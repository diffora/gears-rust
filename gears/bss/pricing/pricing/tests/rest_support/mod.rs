//! The harness every authoring-route suite drives the real router with.
//!
//! It is shared for the reason `tests/common/mod.rs` is: none of it is
//! per-suite. A migrated in-memory `SQLite`, the four PDP doubles the gate can
//! meet, the seeded plan states, and the two readbacks an assertion needs — "did
//! the row move" and "what tag did the response carry". Each suite that copied
//! them would be a second description of the same gate, free to become a weaker
//! one.
//!
//! **The PDP doubles are the point of most of this file.** A suite that only
//! ever drove an allowing resolver would prove the happy path and nothing about
//! the gate; the denying, the erroring and the unconstrained ones are what make
//! "fail closed" observable.

#![allow(
    dead_code,
    reason = "each test binary compiles the whole module and uses part of it"
)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use authz_resolver_sdk::constraints::{Constraint, InPredicate, Predicate};
use authz_resolver_sdk::error::AuthZResolverError;
use authz_resolver_sdk::models::{
    DenyReason, EvaluationRequest, EvaluationResponse, EvaluationResponseContext,
};
use authz_resolver_sdk::{AuthZResolverClient, PolicyEnforcer};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, Response};
use bss_fixtures::ModelKind;
use bss_pricing::api::rest::state::AuthoringState;
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::CurrencyCode;
use bss_pricing::domain::plan::PlanRevision;
use bss_pricing::domain::plan_shape::{
    AddonRule, BillingCycle, DescriptorSet, PhaseKind, PlanPhase,
};
use bss_pricing::domain::price_record::{PriceContent, PriceRecord};
use bss_pricing::domain::price_row::PriceRow;
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::storage::entity::plan;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{
    IdempotencyGate, NewPlanDraft, NewPriceDraft, PlanRepo, PlanShapeRepo, PriceRepo, plan_repo,
    price_repo,
};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use sea_orm_migration::MigratorTrait;
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureEntityExt, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use toolkit_gts::gts_id;
use toolkit_security::{SecurityContext, pep_properties};
use tower::ServiceExt;
use uuid::Uuid;

/// The response header the entity tag rides on, spelled once for every suite.
pub const PLAN_TAG_HEADER: &str = "etag";

/// The actor every seeded row is authored by.
pub const SEED_ACTOR: Uuid = Uuid::from_u128(0xac70);

// ---------------------------------------------------------------------------
// PDP doubles.
// ---------------------------------------------------------------------------

/// Allows, and constrains `owner_tenant_id` to `allowed` — the flat `In` shape
/// the real PDP returns for this gear, since the PEP advertises no tenant-subtree
/// capability and the subtree is pre-expanded.
pub struct FlatInResolver {
    pub allowed: Vec<Uuid>,
}

#[async_trait]
impl AuthZResolverClient for FlatInResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Ok(EvaluationResponse {
            decision: true,
            context: EvaluationResponseContext {
                constraints: vec![Constraint {
                    predicates: vec![Predicate::In(InPredicate::new(
                        pep_properties::OWNER_TENANT_ID,
                        self.allowed.clone(),
                    ))],
                }],
                deny_reason: None,
            },
        })
    }
}

/// Refuses. Every gated route must surface this as 403 and must not have
/// touched a row on the way there.
pub struct DenyingResolver;

#[async_trait]
impl AuthZResolverClient for DenyingResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Ok(EvaluationResponse {
            decision: false,
            context: EvaluationResponseContext {
                constraints: vec![],
                deny_reason: Some(DenyReason {
                    error_code: "no_catalog_role".to_owned(),
                    details: Some("no catalog role grants this pair".to_owned()),
                }),
            },
        })
    }
}

/// Cannot answer. A PDP outage must fail **closed** (503), never degrade into
/// an allow.
pub struct UnavailableResolver;

#[async_trait]
impl AuthZResolverClient for UnavailableResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Err(AuthZResolverError::Internal(
            "the policy decision point is unreachable".to_owned(),
        ))
    }
}

/// Allows with **no** constraints. `require_constraints = true` must refuse it:
/// an unconstrained allow compiles to a scope that filters nothing, which is
/// every tenant's price book.
pub struct UnconstrainedResolver;

#[async_trait]
impl AuthZResolverClient for UnconstrainedResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Ok(EvaluationResponse {
            decision: true,
            context: EvaluationResponseContext {
                constraints: vec![],
                deny_reason: None,
            },
        })
    }
}

/// Allows like [`FlatInResolver`], and **records every request** so a suite can
/// assert which `(resource_type, action)` pair a route actually asked about.
///
/// An allow/deny test cannot catch a route gated on the wrong pair; this can.
pub struct RecordingResolver {
    pub allowed: Vec<Uuid>,
    pub seen: Arc<Mutex<Vec<EvaluationRequest>>>,
}

#[async_trait]
impl AuthZResolverClient for RecordingResolver {
    async fn evaluate(
        &self,
        req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        self.seen.lock().expect("recorder").push(req);
        Ok(EvaluationResponse {
            decision: true,
            context: EvaluationResponseContext {
                constraints: vec![Constraint {
                    predicates: vec![Predicate::In(InPredicate::new(
                        pep_properties::OWNER_TENANT_ID,
                        self.allowed.clone(),
                    ))],
                }],
                deny_reason: None,
            },
        })
    }
}

// ---------------------------------------------------------------------------
// The harness.
// ---------------------------------------------------------------------------

/// A migrated database, the authoring state over it, and two tenants.
pub struct Harness {
    /// The provider every repository and the transaction seam share.
    pub db: DBProvider<DbError>,
    /// The caller's tenant.
    pub tenant: Uuid,
    /// A tenant the caller is never authorized for.
    pub other: Uuid,
    /// The state the routers are built over.
    pub state: Arc<AuthoringState>,
}

impl Harness {
    /// A fresh database with the whole migration chain applied.
    pub async fn new() -> Self {
        let db = connect_db("sqlite::memory:", ConnectOpts::default())
            .await
            .expect("connect in-memory sqlite");
        run_migrations_for_testing(&db, Migrator::migrations())
            .await
            .expect("run the gear migrator");
        let db = DBProvider::<DbError>::new(db);
        let state = Arc::new(AuthoringState {
            db: db.clone(),
            plans: PlanRepo::new(db.clone()),
            shapes: PlanShapeRepo::new(db.clone()),
            prices: PriceRepo::new(db.clone()),
            idempotency: IdempotencyGate::new(Duration::from_hours(1)),
        });
        Self {
            db,
            tenant: Uuid::now_v7(),
            other: Uuid::now_v7(),
            state,
        }
    }

    /// The SQL-level scope a seeding helper writes under.
    pub fn scope(&self) -> AccessScope {
        AccessScope::for_tenant(self.tenant)
    }

    /// The scope of the tenant the caller is never authorized for.
    pub fn other_scope(&self) -> AccessScope {
        AccessScope::for_tenant(self.other)
    }

    fn client(&self, resolver: Arc<dyn AuthZResolverClient>, ctx: Option<Uuid>) -> Client {
        let openapi = OpenApiRegistryImpl::new();
        let router = bss_pricing::api::rest::plans::router(Arc::clone(&self.state), &openapi)
            .merge(bss_pricing::api::rest::prices::router(
                Arc::clone(&self.state),
                &openapi,
            ))
            .layer(axum::Extension(PolicyEnforcer::new(resolver)));
        let router = match ctx {
            Some(tenant) => router.layer(axum::Extension(ctx_for(tenant))),
            None => router,
        };
        Client { router }
    }

    /// An authorized caller in the harness's own tenant.
    pub fn allowed(&self) -> Client {
        self.client(
            Arc::new(FlatInResolver {
                allowed: vec![self.tenant],
            }),
            Some(self.tenant),
        )
    }

    /// An authenticated caller the PDP refuses.
    pub fn denied(&self) -> Client {
        self.client(Arc::new(DenyingResolver), Some(self.tenant))
    }

    /// A caller with no authenticated context at all.
    pub fn anonymous(&self) -> Client {
        self.client(
            Arc::new(FlatInResolver {
                allowed: vec![self.tenant],
            }),
            None,
        )
    }

    /// A caller authorized only for the **other** tenant, reading this one's
    /// rows: the genuine cross-tenant probe.
    pub fn other_tenant(&self) -> Client {
        self.client(
            Arc::new(FlatInResolver {
                allowed: vec![self.other],
            }),
            Some(self.other),
        )
    }

    /// A caller authenticated in **this** tenant whose PDP authorizes only the
    /// **other** one.
    ///
    /// The only shape that exercises `access_scope`'s write-target membership
    /// assertion: the degraded flat-`In` decision does not re-check
    /// `owner_tenant_id`, so a write anchored to a tenant outside the compiled
    /// scope is refused there or nowhere.
    pub fn scope_mismatch(&self) -> Client {
        self.client(
            Arc::new(FlatInResolver {
                allowed: vec![self.other],
            }),
            Some(self.tenant),
        )
    }

    /// A caller whose PDP is down.
    pub fn unavailable(&self) -> Client {
        self.client(Arc::new(UnavailableResolver), Some(self.tenant))
    }

    /// A caller the PDP allows without constraining anything.
    pub fn unconstrained(&self) -> Client {
        self.client(Arc::new(UnconstrainedResolver), Some(self.tenant))
    }

    /// An authorized caller whose every `EvaluationRequest` is captured.
    pub fn recording(&self) -> (Client, Arc<Mutex<Vec<EvaluationRequest>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let client = self.client(
            Arc::new(RecordingResolver {
                allowed: vec![self.tenant],
                seen: Arc::clone(&seen),
            }),
            Some(self.tenant),
        );
        (client, seen)
    }

    /// Publish the plan's revision `0`, so the plan holds a **current**
    /// revision. The publish path is the repository's, not the surface's —
    /// there is no publish route (Slice 5), which is exactly why a test has to
    /// reach for it directly.
    pub async fn publish(&self, plan_id: Uuid, revision: u64) {
        let plan_id = PlanId::new(plan_id);
        let scope = self.scope();
        let tenant = self.tenant;
        let current = self
            .state
            .plans
            .find_revision(&scope, tenant, plan_id, revision)
            .await
            .expect("read the revision")
            .expect("the revision exists");
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<PlanRevision, bss_pricing::infra::storage::RepoError, _>(move |txn| {
                Box::pin(async move {
                    plan_repo::publish_revision(
                        txn,
                        &scope,
                        tenant,
                        plan_id,
                        revision,
                        current.row_version,
                    )
                    .await
                })
            })
            .await;
        outcome.expect("publish the seeded revision");
    }

    /// Publish a seeded price row so a suite can aim a mutating verb at a frozen
    /// one.
    ///
    /// Through the repository's own `publish_rows`, which is the sanctioned
    /// producer of `draft -> published` for a price row; there is no publish
    /// route (Slice 5), which is exactly why a test has to reach for it.
    pub async fn publish_price(&self, plan_id: Uuid, price_id: Uuid) {
        let scope = self.scope();
        let tenant = self.tenant;
        let plan_id = PlanId::new(plan_id);
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<Vec<Uuid>, bss_pricing::infra::storage::RepoError, _>(move |txn| {
                Box::pin(async move {
                    price_repo::publish_rows(
                        txn,
                        &scope,
                        tenant,
                        plan_id,
                        &[(price_id, RowVersion::new(0))],
                    )
                    .await
                })
            })
            .await;
        outcome.expect("publish the seeded price row");
    }

    /// Move a revision straight to `retired`.
    ///
    /// Retirement is Slice 11's publish unit (D-128) and **has no producer in
    /// this gear at all**, so a suite that needs a retired plan has to write the
    /// state itself. The append-only trigger permits the edge: it fires only
    /// once a row is past `draft`, and `published -> retired` is one of the two
    /// flips it whitelists.
    pub async fn retire(&self, plan_id: Uuid, revision: i64) {
        let conn = self.db.conn().expect("conn");
        let result = plan::Entity::update_many()
            .secure()
            .scope_with(&self.scope())
            .col_expr(
                plan::Column::LifecycleState,
                Expr::value(LifecycleState::Retired.as_str()),
            )
            .filter(
                Condition::all()
                    .add(plan::Column::PlanId.eq(plan_id))
                    .add(plan::Column::Revision.eq(revision)),
            )
            .exec(&conn)
            .await
            .expect("retire the seeded revision");
        assert_eq!(result.rows_affected, 1, "the seed must have moved one row");
    }

    /// Open the plan's next revision, so it holds a draft **and** a current one.
    pub async fn open_successor(&self, plan_id: Uuid) {
        self.state
            .plans
            .open_revision(
                &self.scope(),
                self.tenant,
                PlanId::new(plan_id),
                SEED_ACTOR,
                at(11),
            )
            .await
            .expect("open the successor revision");
    }

    /// Abandon the plan's draft revision at `revision`.
    pub async fn abandon_draft(&self, plan_id: Uuid, revision: u64) {
        let plan_id = PlanId::new(plan_id);
        let current = self
            .state
            .plans
            .find_revision(&self.scope(), self.tenant, plan_id, revision)
            .await
            .expect("read the revision")
            .expect("the revision exists");
        self.state
            .plans
            .abandon_draft(
                &self.scope(),
                self.tenant,
                plan_id,
                revision,
                current.row_version,
            )
            .await
            .expect("abandon the seeded draft");
    }

    /// Attach one of each child set to a draft revision, so a read has all three
    /// facets to answer with.
    pub async fn attach_shape(&self, plan_id: Uuid, revision: u64) {
        let plan_id = PlanId::new(plan_id);
        let scope = self.scope();
        let mut version = self
            .state
            .plans
            .find_revision(&scope, self.tenant, plan_id, revision)
            .await
            .expect("read")
            .expect("exists")
            .row_version;

        version = self
            .state
            .shapes
            .replace_phases(
                &scope,
                self.tenant,
                plan_id,
                revision,
                version,
                vec![PlanPhase {
                    phase_id: seeded_phase(),
                    kind: PhaseKind::Evergreen,
                    ordinal: 0,
                    converts_to_phase_id: None,
                    phase_duration_days: None,
                    display_trial_days: None,
                }],
            )
            .await
            .expect("replace phases")
            .row_version;

        version = self
            .state
            .shapes
            .replace_addon_rules(
                &scope,
                self.tenant,
                plan_id,
                revision,
                version,
                vec![AddonRule {
                    addon_sku_id: Uuid::from_u128(0xadd0),
                    required: false,
                    min_qty: None,
                    max_qty: Some(3),
                    step_qty: None,
                    price_override_ref: None,
                    depends_on: Vec::new(),
                    conflicts_with: Vec::new(),
                }],
            )
            .await
            .expect("replace add-on rules")
            .row_version;

        self.state
            .shapes
            .set_descriptor_set(
                &scope,
                self.tenant,
                plan_id,
                revision,
                version,
                DescriptorSet {
                    invoice_line_template: Some("{plan} subscription".to_owned()),
                    gl_code: Some("4000".to_owned()),
                    itemization_rule: Some("per_line".to_owned()),
                    additional: std::collections::BTreeMap::new(),
                },
            )
            .await
            .expect("attach the descriptor set");
    }
}

/// The terminal phase [`Harness::attach_shape`] seeds, for a price row's `phase`
/// scope-key axis.
#[must_use]
pub fn seeded_phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(0x9ba5e))
}

/// A router with one PDP double and one authentication state bound.
pub struct Client {
    router: Router,
}

impl Client {
    /// Drive one request through the whole stack.
    pub async fn send(&self, request: Request<Body>) -> Response<Body> {
        self.router
            .clone()
            .oneshot(request)
            .await
            .expect("the router answers")
    }
}

// ---------------------------------------------------------------------------
// Request and response helpers.
// ---------------------------------------------------------------------------

/// Build a request with an optional JSON body and no preconditions.
pub fn request(method: &str, path: &str, body: Option<serde_json::Value>) -> Request<Body> {
    with_headers(method, path, body, &[])
}

/// Build a request carrying the named headers.
pub fn with_headers(
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
    headers: &[(&str, &str)],
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(json.to_string()))
            .expect("build request"),
        None => builder.body(Body::empty()).expect("build request"),
    }
}

/// The response's `ETag`, when it set one.
pub fn etag_of(response: &Response<Body>) -> Option<String> {
    response
        .headers()
        .get(PLAN_TAG_HEADER)
        .map(|value| value.to_str().expect("the tag is ASCII").to_owned())
}

/// The response's `Location`, when it set one.
pub fn location_of(response: &Response<Body>) -> Option<String> {
    response
        .headers()
        .get("location")
        .map(|value| value.to_str().expect("the location is ASCII").to_owned())
}

/// Read a response body as JSON.
pub async fn body_json(response: Response<Body>) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 4_000_000)
        .await
        .expect("read body");
    if bytes.is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::from_slice(&bytes).expect("the body is JSON")
}

/// The RFC 9457 problem document's machine-readable code.
///
/// Asserted instead of the status wherever a refusal has a code, because §3.3
/// makes the code the discriminator a consumer matches on — several distinct
/// refusals share one status, and a test that only read the status would pass
/// with the wrong one.
pub async fn problem_code(response: Response<Body>) -> String {
    let body = body_json(response).await;
    find_code(&body).unwrap_or_else(|| panic!("no wire code in the problem document: {body}"))
}

/// The code can ride either the `reason` of an aborted/denied error or a
/// precondition violation's `type`; both spellings are the platform's, so both
/// are read rather than one being assumed.
fn find_code(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for key in ["reason", "type", "code"] {
                if let Some(serde_json::Value::String(found)) = map.get(key)
                    && found.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                    && found.len() > 3
                {
                    return Some(found.clone());
                }
            }
            map.values().find_map(find_code)
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_code),
        _ => None,
    }
}

/// A seeded instant, quantized to the millisecond the catalog compares at.
pub fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 3, hour, 0, 0).unwrap()
}

/// A draft plan carrying enough shape to be recognizable, seeded straight
/// through the repository.
pub async fn seed_draft_plan(harness: &Harness, plan_id: Uuid) {
    harness
        .state
        .plans
        .create_draft(&harness.scope(), new_draft(plan_id, harness.tenant))
        .await
        .expect("seed the draft plan");
}

/// A plan whose revision `0` is published, so it holds a **current** revision
/// and no open draft.
pub async fn seed_current_plan(harness: &Harness, plan_id: Uuid) {
    seed_draft_plan(harness, plan_id).await;
    harness.publish(plan_id, 0).await;
}

/// The same, in the **other** tenant, for the cross-tenant probes.
pub async fn seed_foreign_plan(harness: &Harness, plan_id: Uuid) {
    harness
        .state
        .plans
        .create_draft(&harness.other_scope(), new_draft(plan_id, harness.other))
        .await
        .expect("seed the foreign draft plan");
}

/// The row version a revision stands at, or `None` when there is no such row.
///
/// The readback every denial test needs: a 403 alone would also be produced by a
/// handler that wrote first and checked second.
pub async fn plan_row_version(harness: &Harness, plan_id: Uuid, revision: u64) -> Option<u64> {
    harness
        .state
        .plans
        .find_revision(
            &AccessScope::allow_all(),
            harness.tenant,
            PlanId::new(plan_id),
            revision,
        )
        .await
        .expect("read the revision")
        .map(|row| row.row_version.get())
}

/// How many plan revisions the caller's tenant holds.
///
/// The readback a create's denial needs: "403" says nothing about whether a row
/// landed, and a handler that wrote first and checked second would answer the
/// same status.
pub async fn plan_count(harness: &Harness) -> usize {
    let conn = harness.db.conn().expect("conn");
    plan::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(plan::Column::TenantId.eq(harness.tenant)))
        .all(&conn)
        .await
        .expect("count plan revisions")
        .len()
}

/// The lifecycle state a revision stands in, or `None` when there is no such row.
pub async fn plan_state(harness: &Harness, plan_id: Uuid, revision: u64) -> Option<String> {
    harness
        .state
        .plans
        .find_revision(
            &AccessScope::allow_all(),
            harness.tenant,
            PlanId::new(plan_id),
            revision,
        )
        .await
        .expect("read the revision")
        .map(|row| row.lifecycle_state.to_string())
}

fn new_draft(plan_id: Uuid, tenant_id: Uuid) -> NewPlanDraft {
    NewPlanDraft {
        plan_id: PlanId::new(plan_id),
        tenant_id,
        created_by: SEED_ACTOR,
        created_at_utc: at(10),
        sku_id: None,
        plan_tier: Some("gold".to_owned()),
        billing_cycle: Some(BillingCycle::Recurring),
        frequency: None,
        plan_tier_override: false,
        purchase_min_qty: None,
        purchase_max_qty: None,
        invoice_grouping_key: None,
        available_from: None,
        available_to: None,
    }
}

/// The price rows the caller's tenant holds on a plan, in `price_id` order.
///
/// Read with `AccessScope::allow_all()` so a denial test sees what actually
/// landed rather than what the caller was allowed to see.
pub async fn price_rows(harness: &Harness, plan_id: Uuid) -> Vec<PriceRecord> {
    harness
        .state
        .prices
        .list_for_plan(
            &AccessScope::allow_all(),
            harness.tenant,
            PlanId::new(plan_id),
            &[
                LifecycleState::Draft,
                LifecycleState::Published,
                LifecycleState::Superseded,
            ],
        )
        .await
        .expect("list the plan's price rows")
}

/// Seed one draft price row on a distinct region, so several can coexist.
pub async fn seed_price(harness: &Harness, plan_id: Uuid, region: &str) -> PriceRecord {
    let key = ScopeKey::new(
        PlanId::new(plan_id),
        CurrencyCode::new("USD").expect("currency"),
        Region::new(region).expect("region"),
        seeded_phase(),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("scope key");
    harness
        .state
        .prices
        .create_draft(
            &harness.scope(),
            harness.tenant,
            NewPriceDraft {
                price_id: Uuid::now_v7(),
                scope_key: key,
                content: PriceContent {
                    row: PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat)),
                    tax_inclusive: false,
                    billing_timing: None,
                    rounding_policy_ref: None,
                    grandfather_until: None,
                    supersedes_price_id: None,
                },
                created_by: SEED_ACTOR,
                created_at_utc: at(10),
            },
        )
        .await
        .expect("seed a draft price row")
}

/// A `RowVersion`, so a suite never builds one from a magic integer inline.
pub fn version(value: u64) -> RowVersion {
    RowVersion::new(value)
}

fn ctx_for(tenant: Uuid) -> SecurityContext {
    SecurityContext::builder()
        .subject_id(Uuid::now_v7())
        .subject_tenant_id(tenant)
        .subject_type(gts_id!("cf.core.security.subject_user.v1~"))
        .token_scopes(vec!["*".to_owned()])
        .build()
        .expect("authed SecurityContext must build")
}
