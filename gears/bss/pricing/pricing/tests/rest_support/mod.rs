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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
use bss_pricing::api::rest::frontier::ApiState as FrontierState;
use bss_pricing::api::rest::state::{AuthoringState, GovernanceState};
use bss_pricing::config::LimitsConfig;
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::CurrencyCode;
use bss_pricing::domain::money::MinorAmount;
use bss_pricing::domain::plan::PlanRevision;
use bss_pricing::domain::plan_shape::Frequency;
use bss_pricing::domain::plan_shape::{
    AddonRule, BillingCycle, DescriptorSet, PhaseKind, PlanPhase,
};
use bss_pricing::domain::price_record::PriceContent as PriceContentAlias;
use bss_pricing::domain::price_record::{PriceContent, PriceRecord};
use bss_pricing::domain::price_row::PriceRow;
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::approval::ApprovalService;
use bss_pricing::infra::fixture_gate::FixtureGate;
use bss_pricing::infra::publish::PublishService;
use bss_pricing::infra::storage::entity::{approval, audit_log, catalog_version_ref, outbox, plan};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::approval_repo::ApprovalRecord;
use bss_pricing::infra::storage::repo::{
    IdempotencyGate, NewPlanDraft, NewPriceDraft, PinFrontierRepo, PlanRepo, PlanShapeRepo,
    PriceRepo, plan_repo, price_repo,
};
use bss_pricing::infra::window::WindowService;
use bss_pricing_sdk::catalog_version::CatalogVersion;
use bss_pricing_sdk::catalog_version_registry::{
    CatalogVersionRegistryError, CatalogVersionRegistryV1, PendingVersionRef,
};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use sea_orm_migration::MigratorTrait;
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureEntityExt, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use toolkit_gts::gts_id;
use toolkit_security::{SecurityContext, pep_properties};
use tower::ServiceExt;
use uuid::Uuid;

/// One value for a whole test binary: these suites drive a repository or a
/// service directly, where the value the HTTP edge would have established has
/// no producer. What each suite asserts *about* it is stated where it asserts
/// it.
const TEST_CORRELATION: uuid::Uuid = uuid::Uuid::from_u128(0x_c0_11_a7_10);

/// The stamp an audited repository call is made under: who acted, when, and the
/// request's correlation.
///
/// Public, so a suite driving `ApprovalService` directly can open a unit as a
/// **named submitter**: the submitter *is* the actor of the open (`NewApproval`'s
/// doc), and `seed_stamp`'s `SEED_ACTOR` would make submitter and approver the same
/// principal wherever the approver is a third identity — which `inst-tp-distinct`
/// refuses on identity rather than on role.
pub fn stamp_of(
    actor: uuid::Uuid,
    when: chrono::DateTime<chrono::Utc>,
) -> bss_pricing::domain::audit::AuditStamp {
    bss_pricing::domain::audit::AuditStamp {
        actor_principal_id: actor,
        recorded_at: when,
        correlation_id: TEST_CORRELATION,
    }
}

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

/// The `CatalogVersion` registry double, hoisted here because two route suites
/// need it.
///
/// One pending handle per `request_id`, idempotently: the commit derives its
/// `request_id` from `(tenant, plan, revision)` so a rolled-back-and-retried
/// publish re-requests the same assignment, and a double that minted a fresh
/// handle per call would let that property pass untested. It lives in the test
/// crate and must never move into `src/` — the single failure the registry port
/// exists to prevent is a second version incrementer shipping.
/// It also **answers commits**, which is what lets a route suite drive the projector
/// over a handle a mutation requested. `committed_version` answered `None`
/// unconditionally until D-99's re-projection half needed asserting: a pending ref
/// nothing can resolve is a ref the warm sweep skips forever, so the delta a window
/// mutation produces was unobservable and the write that carries it could be deleted
/// with the whole suite green.
#[derive(Default)]
pub struct RegistryDouble {
    issued: Mutex<HashMap<String, String>>,
    commits: Mutex<HashMap<String, CatalogVersion>>,
}

impl RegistryDouble {
    /// Answer `pending_ref` with `version` from now on — the registry's batching, as
    /// a suite scripts it.
    pub fn commit(&self, pending_ref: &str, version: u64) {
        self.commits
            .lock()
            .expect("no panics in the double")
            .insert(pending_ref.to_owned(), CatalogVersion::new(version));
    }
}

#[async_trait]
impl CatalogVersionRegistryV1 for RegistryDouble {
    async fn request_version(
        &self,
        _ctx: &SecurityContext,
        request_id: &str,
    ) -> Result<PendingVersionRef, CatalogVersionRegistryError> {
        let mut issued = self.issued.lock().expect("no panics in the double");
        let next = issued.len();
        let pending = issued
            .entry(request_id.to_owned())
            .or_insert_with(|| format!("pend-{next}"))
            .clone();
        Ok(PendingVersionRef {
            request_id: request_id.to_owned(),
            pending_ref: pending,
        })
    }

    async fn committed_version(
        &self,
        _ctx: &SecurityContext,
        pending_ref: &str,
    ) -> Result<Option<CatalogVersion>, CatalogVersionRegistryError> {
        Ok(self
            .commits
            .lock()
            .expect("no panics in the double")
            .get(pending_ref)
            .copied())
    }
}

/// The committed corpus registry, resolved from this crate's manifest so a suite
/// does not depend on the directory `cargo test` ran in.
#[must_use]
pub fn committed_registry_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/registry.toml")
}

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
    /// The state the approval routes and the publish mount are built over.
    pub governance: Arc<GovernanceState>,
    /// The state the read-only frontier route is built over.
    ///
    /// Mounted here — rather than left to `tests/rest_frontier.rs`'s own
    /// harness — so the router this file builds is the **whole** gear, every
    /// mounted route of it. A set-level property that ranged over all but one
    /// would be a census with a hole in exactly the place a census exists to
    /// close.
    pub frontier: Arc<FrontierState>,
    /// The one registry both services hold, kept concretely so a suite can script a
    /// commit on it. See [`RegistryDouble::commit`].
    pub registry: Arc<RegistryDouble>,
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
        // **One registry, handed to both services**, which is `src/module.rs`'s own
        // rule — *"The same registry `Arc` the engine holds. Two requesters of one
        // registry, never two incrementers."* Two doubles are not a weaker version
        // of that arrangement, they are a different one: the double numbers its
        // handles per instance, so a second instance re-issues `pend-0` after the
        // first has already written it, and `pricing_catalog_version_ref`'s
        // `pending_version_ref` is unique. The staging that hit it is the ordinary
        // one — publish a plan through the route, then mutate a window on it — which
        // answered **500** out of a `UNIQUE` violation the caller never touched, and
        // which is why nothing in the suite covered that pair. Production shares one
        // `Arc` and `PublishUnitKind::request_token` keeps the two units' handles
        // apart, so the fault was the harness's alone.
        let registry = Arc::new(RegistryDouble::default());
        let governance = Arc::new(GovernanceState {
            db: db.clone(),
            plans: PlanRepo::new(db.clone()),
            prices: PriceRepo::new(db.clone()),
            approvals: ApprovalService::new(db.clone()),
            windows: WindowService::new(db.clone(), Arc::clone(&registry) as Arc<_>),
            supersessions: bss_pricing::infra::supersession::SupersessionService::new(
                db.clone(),
                Arc::clone(&registry) as Arc<_>,
            ),
            publish: PublishService::new(
                db.clone(),
                &LimitsConfig::default(),
                FixtureGate::load(&committed_registry_path()),
                Arc::clone(&registry) as Arc<_>,
            ),
            thresholds: bss_pricing::infra::threshold::ThresholdService::new(db.clone()),
            // The window `POST`'s gate (D-191), under the production default TTL: a
            // harness with a different expiry would make the replay tests pass or fail
            // on a knob rather than on the guarantee.
            idempotency: bss_pricing::infra::storage::repo::IdempotencyGate::new(
                LimitsConfig::default().idempotency_key_ttl(),
            ),
        });
        let frontier = Arc::new(FrontierState {
            pin_frontier: PinFrontierRepo::new(db.clone()),
        });
        Self {
            db,
            tenant: Uuid::now_v7(),
            other: Uuid::now_v7(),
            state,
            governance,
            frontier,
            registry,
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
        self.client_as(resolver, ctx.map(|tenant| (tenant, Uuid::now_v7())))
    }

    /// The same, with the caller's **principal** named.
    ///
    /// The two-person rule is about identity, so every approval test has to be
    /// able to say who is calling: `client` mints a fresh subject id per call,
    /// which is right for the authoring suites and would make a submitter and
    /// an approver accidentally distinct here — a self-approval test that could
    /// never stage a self-approval.
    fn client_as(
        &self,
        resolver: Arc<dyn AuthZResolverClient>,
        ctx: Option<(Uuid, Uuid)>,
    ) -> Client {
        let openapi = OpenApiRegistryImpl::new();
        let router = bss_pricing::api::rest::frontier::router(Arc::clone(&self.frontier), &openapi)
            .merge(bss_pricing::api::rest::plans::router(
                Arc::clone(&self.state),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::prices::router(
                Arc::clone(&self.state),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::windows::router(
                Arc::clone(&self.governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::supersessions::router(
                Arc::clone(&self.governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::approvals::router(
                Arc::clone(&self.governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::publish::router(
                Arc::clone(&self.governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::threshold_policy::router(
                Arc::clone(&self.governance),
                &openapi,
            ))
            .layer(axum::Extension(PolicyEnforcer::new(resolver)));
        let router = match ctx {
            Some((tenant, principal)) => {
                router.layer(axum::Extension(ctx_for_principal(tenant, principal)))
            }
            None => router,
        };
        Client { router }
    }

    /// An authorized caller in this tenant, acting as `principal`.
    pub fn allowed_as(&self, principal: Uuid) -> Client {
        self.client_as(
            Arc::new(FlatInResolver {
                allowed: vec![self.tenant],
            }),
            Some((self.tenant, principal)),
        )
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
    /// revision.
    ///
    /// Straight through `plan_repo::publish_revision` and **not** through
    /// `POST /plans/{planId}/publish`, and the reason is isolation rather than
    /// absence: the route is mounted, but reaching its commit arm takes a
    /// publishable shape, a submit, an independent approve and a registry that
    /// answers — four preconditions a suite whose subject is something else would
    /// then be asserting about. `rest_publish.rs` and `rest_approvals.rs` drive
    /// that sequence deliberately, and
    /// `rest_windows.rs::a_plans_first_window_is_authorable_through_the_routes_after_an_empty_publish`
    /// drives it to prove the routes compose.
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
    /// producer of `draft -> published` for a price row. [`Harness::publish`]'s
    /// note applies verbatim: the route exists, and what this avoids is staging
    /// its four preconditions in a suite that is about something else.
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
                stamp_of(SEED_ACTOR, at(11)),
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
                stamp(),
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
                stamp(),
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
                stamp(),
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
                stamp(),
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
    code_in(&body)
}

/// The same code, off a problem document a caller already holds.
///
/// [`problem_code`] consumes the response, so a case that asserts the code **and**
/// something else about the body — that a refusal names the unit holding a key, say —
/// could otherwise only read one of the two. Both spellings go through [`find_code`],
/// which is the point: a second reading of the document here would be free to disagree
/// with the one every other case uses.
#[must_use]
pub fn code_in(body: &serde_json::Value) -> String {
    find_code(body).unwrap_or_else(|| panic!("no wire code in the problem document: {body}"))
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
        correlation_id: TEST_CORRELATION,
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

/// Every audit record the caller's tenant holds, in `(chain_id, seq)` order.
///
/// Read with `AccessScope::allow_all()` for `price_rows`' reason: a test asserting
/// what a mutation recorded must see what landed rather than what the caller was
/// allowed to see.
pub async fn audit_rows(harness: &Harness) -> Vec<audit_log::Model> {
    let conn = harness.db.conn().expect("conn");
    audit_log::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(audit_log::Column::TenantId.eq(harness.tenant)))
        .order_by(audit_log::Column::ChainId, Order::Asc)
        .order_by(audit_log::Column::Seq, Order::Asc)
        .all(&conn)
        .await
        .expect("read the audit chain")
}

/// The correlation id of every `pricing_outbox` row the caller's tenant holds.
///
/// The column is `NOT NULL` there, unlike the audit log's, which is D-178
/// clause (1) already made physical on one of the two stores.
pub async fn outbox_correlations(harness: &Harness) -> Vec<Uuid> {
    let conn = harness.db.conn().expect("conn");
    outbox::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(outbox::Column::TenantId.eq(harness.tenant)))
        .order_by(outbox::Column::Seq, Order::Asc)
        .all(&conn)
        .await
        .expect("read the outbox")
        .into_iter()
        // `PriceCreated` is the **authoring** door's (S3 §17.5), so it belongs to
        // whatever seeded the row rather than to the act under test. Excluded here
        // for `sqlite_publish_commit::outbox_rows`' reason: the callers assert what
        // one commit emitted, and counting the seed's event in would have been
        // repaired by bumping each expectation, which leaves the assertions named
        // for guarantees they stopped checking.
        .filter(|row| row.event_name != "PriceCreated")
        .map(|row| row.correlation_id)
        .collect()
}

/// Every `pricing_catalog_version_ref` row the caller's tenant holds, in
/// `requested_at` then `pending_ref` order.
///
/// The readback D-99's re-projection half needs, and it had none: a mutation's
/// pending ref is what carries the plan subject to the projector, so a test that only
/// read the *response* body's handle was asserting about a value the registry supplied
/// **before** the row was written. Deleting the write left the whole suite green.
///
/// Read with `AccessScope::allow_all()` for `price_rows`' reason: what landed, not
/// what the caller was allowed to see.
pub async fn pending_version_refs(harness: &Harness) -> Vec<catalog_version_ref::Model> {
    let conn = harness.db.conn().expect("conn");
    catalog_version_ref::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(catalog_version_ref::Column::TenantId.eq(harness.tenant)))
        .order_by(catalog_version_ref::Column::RequestedAt, Order::Asc)
        .order_by(catalog_version_ref::Column::PendingRef, Order::Asc)
        .all(&conn)
        .await
        .expect("read the pending version refs")
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
                correlation_id: TEST_CORRELATION,
            },
        )
        .await
        .expect("seed a draft price row")
}

/// Seed one `scheduled` window on a price row, straight through the repository.
///
/// **The repository and not the route**, deliberately: a suite that needs a window
/// to exist in order to drive `PATCH`/`DELETE` must not depend on `POST` working,
/// or a single defect in the schedule path would redden every window suite at once
/// and none of them would say which rule broke.
///
/// The interval is dated **2099** and carries no relation to the wall clock, which
/// is the fixtures' standing rule: a window dated *today* makes the suite race the
/// activation sweep, and one dated relatively goes stale the day the test data
/// ages past it.
pub async fn seed_window(harness: &Harness, price_id: Uuid) -> Uuid {
    /// Far enough out that no wall clock reaches it and no sweep activates the
    /// row mid-suite. A **fact**, not a date derived from `Utc::now()`.
    const SEEDED_WINDOW_YEAR: i32 = 2099;
    let effective_from =
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, SEEDED_WINDOW_YEAR, 1, 1, 0, 0, 0)
            .single()
            .expect("a real instant");
    let window_id = Uuid::now_v7();
    let conn = harness.db.conn().expect("conn");
    bss_pricing::infra::storage::repo::window_repo::schedule(
        &conn,
        &harness.scope(),
        bss_pricing::infra::storage::repo::window_repo::NewWindow {
            window_id,
            tenant_id: harness.tenant,
            price_id,
            effective_from,
            effective_to: None,
            reason_code: "seeded".to_owned(),
        },
        seed_stamp(),
    )
    .await
    .expect("seed a scheduled window");
    window_id
}

/// A `RowVersion`, so a suite never builds one from a magic integer inline.
pub fn version(value: u64) -> RowVersion {
    RowVersion::new(value)
}

/// The authenticated context a service call needs when no route is minting one.
///
/// The same builder the harness's own clients use, exposed so a suite driving
/// `WindowService` or `ApprovalService` directly asks with the identity a route
/// would have compiled rather than with a second, weaker spelling of it.
#[must_use]
pub fn security_context(principal: Uuid, tenant: Uuid) -> SecurityContext {
    ctx_for_principal(tenant, principal)
}

fn ctx_for_principal(tenant: Uuid, principal: Uuid) -> SecurityContext {
    SecurityContext::builder()
        .subject_id(principal)
        .subject_tenant_id(tenant)
        .subject_type(gts_id!("cf.core.security.subject_user.v1~"))
        .token_scopes(vec!["*".to_owned()])
        .build()
        .expect("authed SecurityContext must build")
}

/// The actor and instant every mutating repository call now records (D-135 - the
/// audit row commits inside the mutation's own transaction).
///
/// Public as `seed_stamp` so a suite can stage a repository-level mutation that
/// no route offers - the price-row edit that stages the approve→commit window
/// is one, because the authoring route would need its own tag round trip and
/// the suite's subject is the publish.
pub fn seed_stamp() -> bss_pricing::domain::audit::AuditStamp {
    stamp()
}

fn stamp() -> bss_pricing::domain::audit::AuditStamp {
    bss_pricing::domain::audit::AuditStamp {
        actor_principal_id: uuid::Uuid::from_u128(0xac_10),
        recorded_at: chrono::Utc::now(),
        correlation_id: TEST_CORRELATION,
    }
}

// ---------------------------------------------------------------------------
// The publishable seed, and reading the approval store back.
// ---------------------------------------------------------------------------

/// The revision and version a publishable seed left behind.
pub struct Publishable {
    /// The terminal phase this plan's rows are filed under.
    ///
    /// **Minted per plan, and that is not tidiness.** `pricing_plan_phase`'s
    /// primary key is `(phase_id, plan_revision)` and does **not** carry
    /// `plan_id`, so two plans of one tenant - or of two tenants - cannot share
    /// a phase id at the same revision number. A fixed `seeded_phase()` here
    /// made the second publishable plan in one test fail on a UNIQUE violation.
    /// Reported as a schema finding; worked around here.
    pub phase: PhaseId,
    /// The open draft revision.
    pub revision: u64,
    /// Its row version — what the composite `If-Match` tag has to carry.
    pub version: RowVersion,
    /// The one price row the plan carries.
    pub price_id: Uuid,
}

impl Publishable {
    /// The composite plan-plane entity tag: the revision **and** its version.
    ///
    /// Built here rather than in each suite because a plan route's tag names
    /// both, and a test that quoted a bare version would be testing a
    /// precondition the surface does not offer.
    #[must_use]
    pub fn etag(&self) -> String {
        format!("\"{}-{}\"", self.revision, self.version.get())
    }
}

/// A plan whose **shape** the publish rule set passes, carrying **no price row at
/// all**.
///
/// The other half of [`seed_publishable_plan`], split out rather than copied
/// because a plan with no rows is a state the rule set genuinely admits — the
/// coverage rule ranges over the *billable* set, and an empty set has no key to
/// find uncovered — so it is the world an empty first publish happens in. One
/// description of the shape, two seeds over it.
pub struct PublishableShape {
    /// The terminal phase this plan's rows would be filed under.
    pub phase: PhaseId,
    /// The open draft revision.
    pub revision: u64,
    /// Its row version — what the composite `If-Match` tag has to carry.
    pub version: RowVersion,
}

impl PublishableShape {
    /// The composite plan-plane entity tag: the revision **and** its version.
    #[must_use]
    pub fn etag(&self) -> String {
        format!("\"{}-{}\"", self.revision, self.version.get())
    }
}

/// A plan the publish rule set passes on its shape alone: one evergreen terminal
/// phase, a descriptor set, a tier and a frequency, and nothing priced.
pub async fn seed_publishable_shape(harness: &Harness, plan_id: Uuid) -> PublishableShape {
    let plan = PlanId::new(plan_id);
    let phase = PhaseId::new(Uuid::now_v7());
    let scope = harness.scope();
    let created = harness
        .state
        .plans
        .create_draft(
            &scope,
            NewPlanDraft {
                plan_id: plan,
                tenant_id: harness.tenant,
                created_by: SEED_ACTOR,
                created_at_utc: at(10),
                sku_id: Some(Uuid::from_u128(0x5_c1)),
                plan_tier: Some("gold".to_owned()),
                billing_cycle: Some(BillingCycle::Recurring),
                frequency: Some(Frequency::Monthly),
                plan_tier_override: false,
                purchase_min_qty: None,
                purchase_max_qty: None,
                invoice_grouping_key: None,
                available_from: None,
                available_to: None,
                correlation_id: TEST_CORRELATION,
            },
        )
        .await
        .expect("seed the publishable draft plan");

    let after_phases = harness
        .state
        .shapes
        .replace_phases(
            &scope,
            harness.tenant,
            plan,
            created.revision,
            created.row_version,
            vec![PlanPhase {
                phase_id: phase,
                kind: PhaseKind::Evergreen,
                ordinal: 0,
                converts_to_phase_id: None,
                phase_duration_days: None,
                display_trial_days: None,
            }],
            stamp(),
        )
        .await
        .expect("attach the phase chain");

    let after_descriptors = harness
        .state
        .shapes
        .set_descriptor_set(
            &scope,
            harness.tenant,
            plan,
            created.revision,
            after_phases.row_version,
            DescriptorSet {
                invoice_line_template: Some("{plan}".to_owned()),
                gl_code: Some("4000".to_owned()),
                itemization_rule: Some("per_charge".to_owned()),
                additional: std::collections::BTreeMap::new(),
            },
            stamp(),
        )
        .await
        .expect("attach the descriptor set");

    PublishableShape {
        phase,
        revision: created.revision,
        version: after_descriptors.row_version,
    }
}

/// A plan the **whole** publish rule set passes: [`seed_publishable_shape`] plus
/// one flat recurring row carrying an amount and a rounding policy, and the
/// window `inst-wc-required` demands of it.
///
/// `seed_draft_plan` + `attach_shape` + `seed_price` do not add up to this and
/// deliberately are not changed to: those seeds exist so the authoring suites
/// can drive save-time refusals over half-finished drafts, which is the state
/// §4.2 says authoring is allowed to be in. A publish suite needs the other
/// state, and it is a different seed rather than a widened one.
pub async fn seed_publishable_plan(harness: &Harness, plan_id: Uuid) -> Publishable {
    let plan = PlanId::new(plan_id);
    let scope = harness.scope();
    let shape = seed_publishable_shape(harness, plan_id).await;
    let phase = shape.phase;

    let price_id = Uuid::now_v7();
    harness
        .state
        .prices
        .create_draft(
            &scope,
            harness.tenant,
            NewPriceDraft {
                price_id,
                scope_key: publishable_scope_key(plan, phase, "eu"),
                content: publishable_row(),
                created_by: SEED_ACTOR,
                created_at_utc: at(10),
                correlation_id: TEST_CORRELATION,
            },
        )
        .await
        .expect("author the price row");

    // `inst-wc-required`: the row cannot publish until its canonical scope key
    // holds an active or scheduled window. Twenty-four tests across
    // `rest_publish.rs` and `rest_approvals.rs` reach the publish route through
    // this one seed, and every one of them asserted a 202 that the rule refuses
    // without this.
    //
    // The decision is `common::schedule_coverage_window`'s and not this file's,
    // so the two `sqlite_*` seeds that owe the same window cannot drift from it.
    //
    // **Scheduled through the repository for isolation and speed, and no longer
    // because the route could not do it.** The sentence here used to read that
    // `POST …/prices/{priceId}/windows` "is not mounted: a seed cannot use a door
    // that does not exist yet", and it is withdrawn rather than edited: the route
    // is mounted and this suite's own cases drive it. What the route cannot do is
    // author *this* window, because a window mutation resolves the plan's current
    // revision and this seed's plan has not published yet — the seed exists to make
    // that publish possible. Reaching the route from here would also mean every
    // publish fixture in the crate depended on the schedule path, so one defect in
    // it would redden four suites at once with none of them naming the rule that
    // broke. `rest_support::seed_window` carries the same note for the same reason.
    let conn = harness.state.db.conn().expect("conn");
    crate::common::schedule_coverage_window(&conn, &scope, harness.tenant, price_id, stamp()).await;

    Publishable {
        phase,
        revision: shape.revision,
        version: shape.version,
        price_id,
    }
}

/// A flat recurring row that passes the Slice-3 rule set.
#[must_use]
pub fn publishable_row() -> PriceContentAlias {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    PriceContentAlias {
        row,
        tax_inclusive: false,
        billing_timing: Some("advance".to_owned()),
        // Its own policy, so the Foundation's rounding rule resolves without a
        // tenant policy row.
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

/// The canonical scope key a publishable row sits on.
#[must_use]
pub fn publishable_scope_key(plan_id: PlanId, phase: PhaseId, region: &str) -> ScopeKey {
    ScopeKey::new(
        plan_id,
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new(region).expect("a non-blank region"),
        phase,
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("the class pairs with cohort none")
}

/// Every approval record the caller's tenant holds, in `submitted_at` order.
///
/// Read with `AccessScope::allow_all()` for `price_rows`' reason: a test
/// asserting what a call did to the store must see what landed rather than what
/// the caller was allowed to see.
pub async fn approval_rows(harness: &Harness) -> Vec<approval::Model> {
    let conn = harness.db.conn().expect("conn");
    approval::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(approval::Column::TenantId.eq(harness.tenant)))
        .order_by(approval::Column::SubmittedAt, Order::Asc)
        .order_by(approval::Column::ApprovalId, Order::Asc)
        .all(&conn)
        .await
        .expect("read the approval store")
}

/// One approval record, by id, read outside the caller's scope.
pub async fn approval_row(harness: &Harness, approval_id: Uuid) -> ApprovalRecord {
    let conn = harness.db.conn().expect("conn");
    bss_pricing::infra::storage::repo::approval_repo::read(
        &conn,
        &AccessScope::allow_all(),
        harness.tenant,
        approval_id,
    )
    .await
    .expect("read the approval record")
    .expect("the record is there")
}

/// The principal who proposes a threshold policy in
/// [`approve_threshold_policy`], and the independent one who approves it.
///
/// Two ids because `chk_pricing_approval_distinct_principals` is a real constraint and
/// D-10's whole content is that the two-person rule's own foundation takes two people.
/// They are deliberately unlike any suite's own actors, so a suite asserting on *its*
/// submitter never matches these.
pub const POLICY_PROPOSER: Uuid = Uuid::from_u128(0x_7005_ec01);
/// The independent `FinanceReviewer` of [`approve_threshold_policy`].
pub const POLICY_REVIEWER: Uuid = Uuid::from_u128(0x_7005_ec02);

/// Configure the tenant's approval-threshold policy **through the real surfaces**,
/// and carry it to effective by approving it with a second principal.
///
/// # Why a suite ever needs this
///
/// Without it every act this gear can perform is material, and a suite of nothing but
/// refusals passes against a service that refuses everything. `inst-mat-failsafe` is
/// the reason: a tenant with no configured policy has *every* change material, so the
/// commit arm of any mutation — a plan publish, a window schedule, a lengthening
/// `PATCH` — is unreachable until one currency has an entry.
///
/// # Why it goes through `PUT` + approve rather than writing the rows
///
/// `infra::threshold::effective_version` resolves the policy by finding the **approved
/// unit whose pin still matches**, so a fixture that inserted version rows directly
/// would install a policy the evaluator never sees, and every test built on it would
/// assert the fail-safe while claiming to assert the comparison. The two calls here
/// are the same two an operator makes.
///
/// `entries` is `(currency, absolute_minor)`. A bar of any size is *below* nothing —
/// what matters for a zero-delta act is only that the currency **has** an entry.
///
/// # Why the start is in the past
///
/// It was `2099-01-01T00:00:00Z`, and under D-188 that is a policy which is **not in
/// force**: a version is not effective before its authored `effectiveFrom`, so this
/// fixture would have installed a policy no publish and no window mutation could
/// see, and every suite built on it would have gone back to asserting the fail-safe
/// while claiming to assert the comparison — the exact failure this function's own
/// doc warns about one paragraph up.
pub async fn approve_threshold_policy(harness: &Harness, entries: &[(&str, i64)]) {
    approve_threshold_policy_from(harness, "2020-01-01T00:00:00Z", entries).await;
}

/// The tenant's current policy `ETag`, read the way a caller has to read it.
///
/// A helper rather than an inline `GET` because every policy `PUT` in every suite
/// needs one and there is no other way to get it: the tag is opaque by construction
/// (D-186), so a fixture cannot compute one and a fixture that hard-coded one would
/// pin the digest's framing instead of the precondition.
pub async fn policy_etag_of(harness: &Harness, principal: Uuid) -> String {
    let read = harness
        .allowed_as(principal)
        .send(with_headers(
            "GET",
            bss_pricing::api::rest::threshold_policy::APPROVAL_THRESHOLD_POLICY,
            None,
            &[],
        ))
        .await;
    assert_eq!(
        read.status(),
        axum::http::StatusCode::OK,
        "the policy read always answers, unset or not"
    );
    etag_of(&read).expect(
        "the policy GET carries the ETag its PUT demands; without it the precondition is \
         unsatisfiable",
    )
}

/// [`approve_threshold_policy`] over an authored start, for the suites whose
/// subject is the start itself.
pub async fn approve_threshold_policy_from(
    harness: &Harness,
    effective_from: &str,
    entries: &[(&str, i64)],
) {
    let body = serde_json::json!({
        "effective_from": effective_from,
        "entries": entries
            .iter()
            .map(|(currency, minor)| serde_json::json!({
                "currency": currency,
                "absolute_minor": minor,
            }))
            .collect::<Vec<_>>(),
    });
    // The `PUT` requires the policy's `ETag` (D-186), and the read that supplies it
    // is the same read an operator makes: there is no tag to fabricate, because the
    // value is an opaque digest over the representation the `GET` serves. A tenant
    // with no policy at all is answered 200 and carries one, which is what makes this
    // fixture work on a fresh harness.
    let tag = policy_etag_of(harness, POLICY_PROPOSER).await;
    let proposed = harness
        .allowed_as(POLICY_PROPOSER)
        .send(with_headers(
            "PUT",
            bss_pricing::api::rest::threshold_policy::APPROVAL_THRESHOLD_POLICY,
            Some(body),
            &[("if-match", tag.as_str())],
        ))
        .await;
    assert_eq!(
        proposed.status(),
        axum::http::StatusCode::ACCEPTED,
        "the D-10 unit opens; the diff is not applied yet"
    );
    let opened = body_json(proposed).await;
    let approval_id = opened["approval"]["approval_id"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .unwrap_or_else(|| panic!("the 202 names the unit it opened: {opened}"));
    let approved = harness
        .allowed_as(POLICY_REVIEWER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{approval_id}/approve"),
            None,
            &[],
        ))
        .await;
    assert_eq!(
        approved.status(),
        axum::http::StatusCode::OK,
        "an independent principal is what puts the policy in force (D-10)"
    );
}
