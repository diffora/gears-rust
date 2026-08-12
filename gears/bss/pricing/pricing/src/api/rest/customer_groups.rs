//! `GET/PUT /bss-pricing/v1/customer-groups/taxonomy` — the BSS customer-group
//! value universe, on its **own** route (`design/09-price-overlays.md` §3
//! `inst-cg-taxonomy`, §5; `design/05-governance.md`'s endpoint map).
//!
//! # Why this is not a fifth arm of `api::rest::taxonomies`
//!
//! `api::rest::taxonomies` mounts `GET/PUT /config/taxonomies/{class}` over
//! `TaxonomyClass`'s four members and gates on `config × read/write`. A first
//! attempt at this surface was briefed as a fifth arm of that enum on that same
//! route, and it was the wrong shape: `design/09-price-overlays.md` §5 gives the
//! customer-group taxonomy its **own** route, and `05-governance.md`'s
//! endpoint-mapping table says so explicitly — *"the customer-group taxonomy is
//! **not** here: it lives at `/bss-pricing/v1/customer-groups/taxonomy` under
//! `customer_group` (more sensitive)"*. `authz.rs`'s own module doc gives the
//! reason: *"`customer_group` is its own resource, not part of `plan` —
//! per-payer membership is payer-level commercial data, more sensitive than
//! plan authoring"* — and the taxonomy of legal values for that data is the same
//! sensitivity class as the membership records it will gate. Filing it under
//! `config × write` would have handed every `CatalogAdmin` who declares a brand
//! or a region the same authority over the customer-group vocabulary, which is
//! exactly the widening `05-governance.md` segregates against.
//!
//! `TaxonomyClass` (`crate::domain::taxonomy`) therefore gains **no** arm for
//! this class — that enum is the shared `{class}` route's own vocabulary, and an
//! arm there is precisely what would make this value set addressable through
//! the route it must not use. The two authz permissions this route consumes,
//! `customer_group × read` and `customer_group × write`, are already declared
//! in `gts/permissions.rs` (review finding Z13-13) and this route is their
//! **first** consumer — no new authz vocabulary is minted here.
//!
//! # The shape otherwise
//!
//! Same resource discipline as `api::rest::taxonomies`, restated because the
//! two are siblings rather than the same handler: the `PUT` carries the
//! **whole** value set (a value the body omits is retired, never deleted,
//! because a value a published overlay scope still names has to keep
//! existing), `If-Match` is required (D-171; a tenant with no values at all is
//! answered `200` with an empty list and a tag, so the bootstrap reads its
//! precondition off the `GET` like every other caller), and a retirement the
//! `inst-cg-taxonomy` guard refuses fails the whole `PUT` with
//! `TAXONOMY_VALUE_IN_USE` (409), writing nothing.
//!
//! This taxonomy carries no `tax_*` markers — those are D-01's, declared on the
//! region taxonomy alone — so its value view is narrower than
//! `TaxonomyValueView`'s.

use std::sync::Arc;

use axum::extract::{Extension, Path};
use axum::http::HeaderMap;
use axum::http::header::ETAG;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::StatusCode};
use chrono::{DateTime, Utc};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::operation_builder::{ParamLocation, ParamSpec};
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::auth_context::{audit_stamp, require_authenticated};
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::plans::idempotency_key_param;
use crate::api::rest::preconditions;
use crate::api::rest::state::AuthoringState;
use crate::domain::concurrency::RowVersion;
use crate::domain::error::DomainError;
use crate::domain::overlay::ScopeValue;
use crate::domain::ports::CatalogVersionRegistryV1;
use crate::domain::taxonomy::{TAXONOMY_VALUE_IN_USE, TaxonomyEntry, TaxonomyState};
use crate::infra::idempotent::{self, Guarded, GuardedRequest};
use crate::infra::membership_publish;
use crate::infra::storage::repo::taxonomy_repo::customer_group_tag_of;
use crate::infra::storage::repo::{IdempotencyGate, NewMembership, group_membership_repo};
use crate::infra::storage::repo_failure;

/// `OpenAPI` tag applied to both operations (DE0205).
const TAG: &str = "BSS Pricing Customer Groups";

/// The customer-group taxonomy resource.
///
/// The literal is repeated in both `OperationBuilder` calls below because
/// DE0801 validates a **literal** argument and silently passes a `const` one;
/// the two spellings are pinned together by `tests/module_test.rs`'s route
/// census, exactly as `taxonomies::TAXONOMY`'s is.
pub const CUSTOMER_GROUP_TAXONOMY: &str = "/bss-pricing/v1/customer-groups/taxonomy";

/// The membership collection of one group (`design/09-price-overlays.md` §5,
/// `POST`).
///
/// The literal is repeated in the `OperationBuilder` call below for
/// [`CUSTOMER_GROUP_TAXONOMY`]'s reason: DE0801 validates a literal argument
/// and silently passes a `const` one.
pub const CUSTOMER_GROUP_MEMBERS: &str = "/bss-pricing/v1/customer-groups/{group}/members";

/// One membership, addressed by its own id (§5, `PATCH`).
pub const CUSTOMER_GROUP_MEMBER: &str = "/bss-pricing/v1/customer-groups/{group}/members/{id}";

/// The atomic move into `{group}` (the **target** group), addressed by the
/// payer (§5, `POST .../move`, `inst-ms-move`, D-09).
pub const CUSTOMER_GROUP_MEMBER_MOVE: &str =
    "/bss-pricing/v1/customer-groups/{group}/members/{payerId}/move";

// ---------------------------------------------------------------------------
// Views.
// ---------------------------------------------------------------------------

/// One declared customer-group value, as an operator reads and writes it.
///
/// Narrower than `taxonomies::TaxonomyValueView`: this table carries no `tax_*`
/// columns (D-01's markers are the region taxonomy's alone), so there is
/// nothing here for them to occupy.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct CustomerGroupValueView {
    /// The declared code — the string a `customerGroup`-scoped overlay's
    /// `scopeValue` must match. Never blank.
    pub value: String,
    /// The operator's label for it.
    pub display_name: String,
    /// `active` or `retired`. Optional on the way in and defaulting to
    /// `active`, because a body listing a value is a body declaring it;
    /// sending `"retired"` is the explicit spelling of the same act as leaving
    /// it out, and both are guarded identically.
    pub state: Option<String>,
}

/// The customer-group taxonomy, whole.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct CustomerGroupTaxonomyView {
    /// Every declared value, `active` and `retired` alike, ordered by value.
    /// Retirements are **included**, for `taxonomies::TaxonomyView`'s reason: an
    /// operator who reads, edits and writes back has to be able to see the
    /// value they are about to re-activate.
    pub values: Vec<CustomerGroupValueView>,
}

/// The body of a `PUT`: the complete value set.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct PutCustomerGroupTaxonomyRequest {
    /// The whole taxonomy, not a patch. A value declared today and absent here
    /// is **retired**.
    pub values: Vec<CustomerGroupValueView>,
}

// ---------------------------------------------------------------------------
// Router.
// ---------------------------------------------------------------------------

/// The `If-Match` header this `PUT` requires (D-171) —
/// `taxonomies::if_match_param`'s wording, restated for this resource.
fn if_match_param() -> ParamSpec {
    ParamSpec {
        name: "If-Match".to_owned(),
        location: ParamLocation::Header,
        required: true,
        description: Some(
            "Mandatory precondition (RFC 9110). The value is the **opaque** tag the `GET` \
             returns in its `ETag` header - copy it back verbatim. It is not a row version: this \
             taxonomy is a value set with no version column, so the tag is a digest over the \
             representation the `GET` serves - every value's code, state and label. A tenant with \
             no values at all is answered `200` with an empty list and carries a tag, so a first \
             `PUT` asserts it like any other. It matters here more than the row count suggests, \
             because this `PUT` replaces the **whole** set: without it, two admins who each add \
             one value would leave a taxonomy carrying one addition and silently retiring the \
             other's. A tag that no longer describes the taxonomy is `409` `STALE_VERSION`; an \
             absent or malformed one is `400`."
                .to_owned(),
        ),
        param_type: "string".to_owned(),
    }
}

/// Build the Axum router for the two customer-group taxonomy operations and
/// register them.
pub fn router(state: Arc<AuthoringState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-pricing/v1/customer-groups/taxonomy")
        .operation_id("bss_pricing.get_customer_group_taxonomy")
        .summary("Read the tenant's BSS customer-group taxonomy")
        .description(
            "Every customer-group value the tenant has declared, `active` and `retired` alike, \
             ordered by value. Retirements are included deliberately: retirement is guarded \
             rather than cascading and `retired -> active` is a legal audited move, so an \
             operator editing this list has to be able to see the value they are about to \
             re-activate. A tenant that has declared nothing is answered `200` with an empty list \
             - that is a state, not an absent resource. **The response carries the `ETag` the \
             `PUT` demands**, and this is the only place to obtain one. This is a **separate** \
             resource from `GET /config/taxonomies/{class}` and gates on `customer_group` x \
             `read`, never `config` x `read` - per-payer commercial data is more sensitive than \
             plan/config authoring (`05-governance.md`).",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(get_customer_group_taxonomy)
        .json_response_with_schema::<CustomerGroupTaxonomyView>(
            openapi,
            StatusCode::OK,
            "The declared customer-group values.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    let router = OperationBuilder::put("/bss-pricing/v1/customer-groups/taxonomy")
        .operation_id("bss_pricing.put_customer_group_taxonomy")
        .summary("Replace the tenant's BSS customer-group taxonomy")
        .description(
            "The body is the **whole** taxonomy, not a patch. A value listed here is declared \
             (and re-activated if it was retired); a value the tenant holds today and this body \
             omits is **retired** - never deleted, because a value a published `customerGroup`-\
             scoped overlay still names has to keep existing. Sending a value with `state: \
             \"retired\"` is the explicit spelling of the same act, and the two are guarded \
             identically. **Retirement is guarded** (`inst-cg-taxonomy`): a value still named by \
             a published `PriceOverlay` scoped to `customerGroup` and that value cannot retire - \
             the response is `409` `TAXONOMY_VALUE_IN_USE` naming the count, and **nothing at all \
             is written**, so the taxonomy an operator re-authors against is the one they had. \
             One transaction, one verdict: there is no partial application. A customer group is \
             never a price-row axis, so unlike `config`'s four taxonomies this guard has only one \
             plane to check. **`If-Match` is required**: send the opaque `ETag` the `GET` \
             returned. Gates on `customer_group` x `write` - a caller holding `config` x `write` \
             alone (declares regions, brands, partners, org tiers) does NOT satisfy this gate, by \
             design: per-payer commercial data is segregated from plan/config authoring.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .param(if_match_param())
        .json_request::<PutCustomerGroupTaxonomyRequest>(
            openapi,
            "The complete customer-group value set.",
        )
        .handler(put_customer_group_taxonomy)
        .json_response_with_schema::<CustomerGroupTaxonomyView>(
            openapi,
            StatusCode::OK,
            "The taxonomy as it now stands.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    // D-178's edge, at this router's own tail — `taxonomies::router`'s reason: a
    // surface reachable without it cannot build an `AuditStamp`.
    router
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

/// `GET /customer-groups/taxonomy`.
async fn get_customer_group_taxonomy(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = read_scope(&enforcer, &ctx).await?;

    let held = state
        .taxonomies
        .list_customer_groups(&scope, ctx.subject_tenant_id())
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;

    Ok(render(&held))
}

/// `PUT /customer-groups/taxonomy`.
async fn put_customer_group_taxonomy(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let scope = write_scope(&enforcer, &ctx).await?;
    let tenant = ctx.subject_tenant_id();
    let now = chrono::Utc::now();

    let asserted = preconditions::if_match_policy(&headers).map_err(CanonicalError::from)?;
    let request: PutCustomerGroupTaxonomyRequest = preconditions::parse_body(&body)?;
    let entries = authored_entries(request.values)?;

    // The premise is handed to the store, not tested here — `taxonomies::
    // put_taxonomy`'s reason (D-186): this handler refuses a request it cannot
    // *understand*, and the store refuses one whose premise has *moved*, inside
    // the transaction that writes.
    let result = state
        .taxonomies
        .replace_customer_groups(
            &scope,
            tenant,
            entries,
            &asserted,
            audit_stamp(&ctx, now, correlation),
        )
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;

    if result.stale {
        return Err(CanonicalError::from(DomainError::StaleVersion(
            "the If-Match tag no longer describes the customer-group taxonomy: it changed after \
             you read it. Re-read the GET and author against the tag it hands back - a PUT \
             replaces the whole value set, so applying yours over a moved one would retire \
             whatever the other author added"
                .to_owned(),
        )));
    }

    if let Some(violation) = result.report.violations.first() {
        debug_assert_eq!(violation.code, TAXONOMY_VALUE_IN_USE);
        return Err(CanonicalError::from(DomainError::TaxonomyValueInUse(
            violation.detail.clone(),
        )));
    }

    Ok(render(&result.entries))
}

// ---------------------------------------------------------------------------
// Rendering and parsing.
// ---------------------------------------------------------------------------

/// The representation, with the tag that covers it — `taxonomies::render`'s
/// reason: one function for both verbs, so the `GET`'s tag and the `PUT`'s
/// response tag cannot come from two renderings of one taxonomy.
fn render(entries: &[TaxonomyEntry]) -> Response {
    let tag = preconditions::policy_etag(&customer_group_tag_of(entries));
    (
        [(ETAG, tag)],
        Json(CustomerGroupTaxonomyView {
            values: entries.iter().map(view_of).collect(),
        }),
    )
        .into_response()
}

fn view_of(entry: &TaxonomyEntry) -> CustomerGroupValueView {
    CustomerGroupValueView {
        value: entry.value.as_str().to_owned(),
        display_name: entry.display_name.clone(),
        state: Some(entry.state.as_str().to_owned()),
    }
}

/// Turn the authored body into domain entries, refusing what is malformed.
fn authored_entries(
    values: Vec<CustomerGroupValueView>,
) -> Result<Vec<TaxonomyEntry>, CanonicalError> {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut entries = Vec::with_capacity(values.len());
    for value in values {
        let declared = ScopeValue::new(&value.value).ok_or_else(|| {
            CanonicalError::from(DomainError::InvalidRequest(
                "a customer-group value must not be blank or whitespace: the empty string is \
                 the store's sentinel for the classless overlay scope, so a blank value here \
                 would make that sentinel forgeable"
                    .to_owned(),
            ))
        })?;
        if !seen.insert(declared.as_str().to_owned()) {
            // Refused rather than de-duplicated: a body naming one value twice
            // has two states for it and no rule says which wins.
            return Err(CanonicalError::from(DomainError::InvalidRequest(format!(
                "value `{declared}` appears twice in this body; a taxonomy declares each value \
                 once, and a repeated one leaves its state undecided"
            ))));
        }
        let state = match value.state.as_deref() {
            None => TaxonomyState::Active,
            Some(token) => TaxonomyState::parse(token).ok_or_else(|| {
                CanonicalError::from(DomainError::InvalidRequest(format!(
                    "value `{declared}` carries state `{token}`; a taxonomy value is `active` or \
                     `retired`, and nothing else"
                )))
            })?,
        };
        entries.push(TaxonomyEntry {
            value: declared,
            display_name: value.display_name,
            state,
            tax: None,
        });
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// The gates.
// ---------------------------------------------------------------------------

/// The `customer_group × read` gate.
///
/// **Deliberately not `config × read`.** See the module doc: filing this
/// surface under `config` would hand every `CatalogAdmin` who declares a
/// region or a brand the same read over the customer-group vocabulary, which
/// is exactly the segregation `05-governance.md` draws.
async fn read_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::CUSTOMER_GROUP,
        crate::authz::actions::READ,
        /* owner_tenant_id */ None,
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)
}

/// The `customer_group × write` gate.
///
/// `owner_tenant_id = Some(caller's tenant)` because this is a write, for
/// `taxonomies::write_scope`'s reason: the membership assertion is what
/// refuses a target outside the compiled scope, the degraded flat-`In`
/// decision not re-checking the property.
async fn write_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::CUSTOMER_GROUP,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(ctx.subject_tenant_id()),
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)
}

// ---------------------------------------------------------------------------
// Membership: `POST .../members`, `PATCH .../members/{id}`,
// `POST .../members/{payerId}/move` (`design/09-price-overlays.md` §3
// `inst-cg-record`/`inst-ms-move`, §5).
//
// Every one of these three routes gates on the **same** `customer_group ×
// write` pair the taxonomy `PUT` above does — [`write_scope`] is reused
// verbatim, so this section mints no new authz vocabulary either. What is new
// is the state: these routes request a `CatalogVersion`
// (`dod-customer-group`'s MUST that every committed membership mutation is
// its own publish unit through the Foundation engine, D-06), which is
// `api::rest::state`'s own criterion for keeping a route off
// [`AuthoringState`]. Rather than widen the crate-wide `GovernanceState` —
// touched by every other authoring/governance route and constructed by hand
// at four call sites — [`MembershipState`] is a small state of its own,
// `api::rest::repricing_runs::ApiState`'s precedent: that state also pairs an
// authoring-shaped read with its own `registry: Arc<dyn
// CatalogVersionRegistryV1>` field, for the same reason stated there — "a
// repricing run's apply is not an authoring act; it commits, exactly as a
// publish does".
// ---------------------------------------------------------------------------

/// The membership routes' dependencies.
///
/// A dedicated state rather than a field on `GovernanceState` — see the
/// section banner above. The same registry `Arc` every other requester in
/// this gear holds: two requesters of one registry is one incrementer, never
/// two — `api::rest::state`'s module doc carries the argument.
#[derive(Clone)]
pub struct MembershipState {
    /// The provider the publish transaction opens its one transaction on.
    pub db: DBProvider<DbError>,
    /// The at-most-once gate `POST .../members` and `POST .../move` claim
    /// under — §5's `Idempotency-Key` column for both. `PATCH .../members/{id}`
    /// needs none of its own: its precondition is `If-Match`, and the
    /// compare-and-swap that answers it is already at-most-once by
    /// construction (a second identical `PATCH` either finds nothing left to
    /// move, or is told its tag is stale).
    pub idempotency: IdempotencyGate,
    /// The sole incrementer of `CatalogVersion` this plane requests from.
    pub registry: Arc<dyn CatalogVersionRegistryV1>,
}

/// One membership, as a caller reads it back.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct MembershipView {
    /// The membership's durable name.
    pub membership_id: Uuid,
    /// The payer's commercial-profile key.
    pub payer_tenant_id: Uuid,
    /// The taxonomy value the payer is enrolled in over this interval.
    pub group_value: String,
    /// Inclusive start, UTC.
    pub effective_from: DateTime<Utc>,
    /// Exclusive end, UTC; `null` is open-ended.
    pub effective_to: Option<DateTime<Utc>>,
    /// The concurrency token a later `PATCH` asserts as its `If-Match`.
    pub row_version: u64,
}

impl From<&group_membership_repo::MembershipRow> for MembershipView {
    fn from(row: &group_membership_repo::MembershipRow) -> Self {
        Self {
            membership_id: row.membership_id,
            payer_tenant_id: row.payer_tenant_id,
            group_value: row.group_value.clone(),
            effective_from: row.effective_from,
            effective_to: row.effective_to,
            row_version: row.row_version,
        }
    }
}

/// What `POST .../members` and `PATCH .../members/{id}` answer with.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct MembershipMutationView {
    /// The membership as committed.
    pub membership: MembershipView,
    /// The registry's **pending** handle — the D-06 publish unit this
    /// mutation opened. The row is durable now; a caller resolving this
    /// payer's group through the read model sees it only once
    /// `CatalogVersionPublished` warms.
    pub pending_version_ref: String,
}

impl From<&membership_publish::MembershipPublishReceipt> for MembershipMutationView {
    fn from(receipt: &membership_publish::MembershipPublishReceipt) -> Self {
        Self {
            membership: MembershipView::from(&receipt.membership),
            pending_version_ref: receipt.pending_ref.clone(),
        }
    }
}

/// What `POST .../move` answers with.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct MembershipMoveView {
    /// The membership that was ended, `null` when the payer held none active
    /// at the move instant — the atomic move degrades to a plain enrollment
    /// rather than refusing.
    pub ended: Option<MembershipView>,
    /// The membership created in the target group.
    pub enrolled: MembershipView,
    /// The one registry handle both rows' publish unit was recorded against.
    pub pending_version_ref: String,
}

impl From<&membership_publish::MembershipMoveReceipt> for MembershipMoveView {
    fn from(receipt: &membership_publish::MembershipMoveReceipt) -> Self {
        Self {
            ended: receipt.ended.as_ref().map(MembershipView::from),
            enrolled: MembershipView::from(&receipt.enrolled),
            pending_version_ref: receipt.pending_ref.clone(),
        }
    }
}

/// The body of `POST /customer-groups/{group}/members`.
// `(request, response)` and not `(request)` alone: the guarded `POST` digests the
// **parsed** request, and that needs the type to serialize —
// `windows::ScheduleWindowRequest`'s own note, verbatim.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct EnrollMembershipRequest {
    /// The payer's commercial-profile key (`inst-cg-record`). AMS supplies
    /// this identity; tenant topology is never modified.
    pub payer_tenant_id: Uuid,
    /// Inclusive start of the half-open interval, UTC.
    pub effective_from: DateTime<Utc>,
    /// Exclusive end, UTC; absent is open-ended.
    pub effective_to: Option<DateTime<Utc>>,
}

/// The body of `PATCH /customer-groups/{group}/members/{id}`.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct EndMembershipRequest {
    /// The new exclusive end, UTC — `inst-ms-time`'s "ending early = setting
    /// `to`".
    pub effective_to: DateTime<Utc>,
}

/// The body of `POST /customer-groups/{group}/members/{payerId}/move`.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct MoveMembershipRequest {
    /// The instant the move pivots on — the ended membership's new
    /// `effectiveTo` and the new one's `effectiveFrom`, both at once (D-09:
    /// "no gap, no overlap").
    pub effective_from: DateTime<Utc>,
}

/// The operation `POST .../members` claims idempotency keys under.
const CREATE_MEMBER_OPERATION: &str = "bss_pricing.create_customer_group_member";

/// The operation `POST .../move` claims idempotency keys under — distinct
/// from [`CREATE_MEMBER_OPERATION`] so one client key used on both verbs does
/// not collide (`pricing_idempotency_dedup`'s key is `(tenant, operation,
/// client_key)`).
const MOVE_MEMBER_OPERATION: &str = "bss_pricing.move_customer_group_member";

/// Build the Axum router for the three membership mutations and register
/// them.
///
/// On [`MembershipState`], not [`AuthoringState`] — see the section banner.
pub fn governance_router(state: Arc<MembershipState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/bss-pricing/v1/customer-groups/{group}/members")
        .operation_id("bss_pricing.create_customer_group_member")
        .summary("Enroll a payer into a customer group")
        .description(
            "Creates an effective-dated membership (`inst-cg-record`) and commits it directly: \
             the default membership change is renewal-aligned and audit-only (`inst-mm-renewal`) \
             - no approval unit opens. The committed mutation is its **own publish unit** through \
             the Foundation engine (D-06, `dod-customer-group`'s MUST): the response carries the \
             registry's **pending** handle, and a caller resolving this payer's group through the \
             read model sees the new membership only once `CatalogVersionPublished` warms it. \
             Interval validation is D-09's: an interval overlapping the payer's existing \
             membership in the **same** group is `MEMBERSHIP_OVERLAP` (409); one overlapping a \
             membership in **another** group is `MEMBERSHIP_CONFLICT` (409 - a payer holds at \
             most one active membership across all groups) - use the move operation instead. \
             **An `Idempotency-Key` header is required and is honoured**: the same key with the \
             same body returns the first answer verbatim and mints no second membership. Gates \
             on `customer_group` x `write`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("group", "The group the payer is enrolled into.")
        .param(idempotency_key_param())
        .json_request::<EnrollMembershipRequest>(openapi, "The payer and the effective interval.")
        .handler(create_membership)
        .json_response_with_schema::<MembershipMutationView>(
            openapi,
            StatusCode::CREATED,
            "The membership as committed, carrying the publish unit's pending handle.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    let router = OperationBuilder::patch("/bss-pricing/v1/customer-groups/{group}/members/{id}")
        .operation_id("bss_pricing.adjust_customer_group_member")
        .summary("End or adjust a membership's interval")
        .description(
            "Moves a membership's `effectiveTo` under the `If-Match` precondition - \
             `inst-ms-time`'s \"ending early = setting `to`\" (audited); records are never \
             mutated in place otherwise, history is retained. Audit-only and commits directly, \
             exactly as the create does, and is its own publish unit (D-06). The overlap checks \
             run again against the narrowed interval: `MEMBERSHIP_OVERLAP` / \
             `MEMBERSHIP_CONFLICT` as the create's. A stale `If-Match` is `409` \
             `STALE_VERSION`. Gates on `customer_group` x `write`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("group", "The membership's own group (addressing only).")
        .path_param("id", "The membership to end or adjust.")
        .param(crate::api::rest::plans::if_match_param(
            "The membership's own `rowVersion`, as an opaque tag - there is no `GET` on a \
             membership, so the tag a caller asserts is the one the create or a previous adjust \
             answered.",
        ))
        .json_request::<EndMembershipRequest>(openapi, "The new exclusive end.")
        .handler(adjust_membership)
        .json_response_with_schema::<MembershipMutationView>(
            openapi,
            StatusCode::OK,
            "The membership as it now stands, carrying the publish unit's pending handle.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router =
        OperationBuilder::post("/bss-pricing/v1/customer-groups/{group}/members/{payerId}/move")
            .operation_id("bss_pricing.move_customer_group_member")
            .summary("Atomically transfer a payer into {group}")
            .description(
                "The atomic move (`inst-ms-move`, D-09): ends the payer's active membership, if \
                 any, and enrolls a new one in `{group}` - the **target** group - both at the \
                 same instant, with no gap and no overlap. Audit-only and commits directly \
                 (`inst-mm-renewal`'s default), on **one** publish unit covering both rows: the \
                 response's `pendingVersionRef` is the one handle both memberships were recorded \
                 against, so the registry's D-47 batching resolves them into the same \
                 `CatalogVersion` together. A payer with no active membership at the move instant \
                 is simply enrolled - `ended` answers `null`. **An `Idempotency-Key` header is \
                 required and is honoured.** Gates on `customer_group` x `write`.",
            )
            .tag(TAG)
            .authenticated()
            .no_license_required()
            .path_param("group", "The target group.")
            .path_param("payerId", "The payer being moved.")
            .param(idempotency_key_param())
            .json_request::<MoveMembershipRequest>(openapi, "The instant the move pivots on.")
            .handler(move_membership)
            .json_response_with_schema::<MembershipMoveView>(
                openapi,
                StatusCode::OK,
                "The ended membership (if any), the new one, and the publish unit's pending \
                 handle.",
            )
            .error_400(openapi)
            .error_401(openapi)
            .error_403(openapi)
            .error_409(openapi)
            .error_500(openapi)
            .error_503(openapi)
            .register(router, openapi);

    // D-178's edge, `taxonomies::router`'s reason: a mutation reachable
    // without it cannot build an `AuditStamp`.
    router
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}

/// The group path segment, non-blank — [`authored_entries`]'s validation,
/// applied to one value instead of a whole set.
fn required_group(group: &str) -> Result<String, CanonicalError> {
    ScopeValue::new(group)
        .map(|value| value.as_str().to_owned())
        .ok_or_else(|| {
            CanonicalError::from(DomainError::InvalidRequest(
                "the group segment must not be blank or whitespace".to_owned(),
            ))
        })
}

/// `POST /customer-groups/{group}/members`.
async fn create_membership(
    Extension(state): Extension<Arc<MembershipState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path(group): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let scope = write_scope(&enforcer, &ctx).await?;
    let tenant = ctx.subject_tenant_id();
    let key = preconditions::idempotency_key(&headers)?;
    let request: EnrollMembershipRequest = preconditions::parse_body(&body)?;
    let digest = preconditions::request_digest(&request)?;
    let group_value = required_group(&group)?;

    let now = Utc::now();
    let stamp = audit_stamp(&ctx, now, correlation);
    let registry = Arc::clone(&state.registry);
    let mutation_ctx = ctx.clone();
    let mutation_scope = scope.clone();
    let payer_tenant_id = request.payer_tenant_id;
    let effective_from = request.effective_from;
    let effective_to = request.effective_to;

    let guarded = idempotent::guarded(
        &state.db,
        &state.idempotency,
        &scope,
        GuardedRequest {
            operation: CREATE_MEMBER_OPERATION,
            client_key: key,
            request_hash: digest,
            tenant_id: tenant,
            status: StatusCode::CREATED.as_u16().into(),
            now,
        },
        move |txn| {
            Box::pin(async move {
                // Minted **inside** the guarded body, `windows::schedule_window`'s rule: an
                // id minted outside would be a second one nobody is ever told about, since
                // a replay does not reach this closure at all.
                let membership_id = Uuid::now_v7();
                membership_publish::enroll_in(
                    txn,
                    registry.as_ref(),
                    &mutation_ctx,
                    &mutation_scope,
                    tenant,
                    NewMembership {
                        membership_id,
                        tenant_id: tenant,
                        payer_tenant_id,
                        group_value,
                        effective_from,
                        effective_to,
                    },
                    stamp,
                )
                .await
            })
        },
        |receipt| body_of(&MembershipMutationView::from(receipt)),
    )
    .await?;

    Ok(match guarded {
        Guarded::Performed(receipt) => (
            StatusCode::CREATED,
            [(
                ETAG,
                preconditions::etag(RowVersion::new(receipt.membership.row_version)),
            )],
            Json(MembershipMutationView::from(&receipt)),
        )
            .into_response(),
        Guarded::Replayed { status, body } => replayed(status, &body),
    })
}

/// `PATCH /customer-groups/{group}/members/{id}`.
///
/// No idempotency gate — the precondition is `If-Match`, and the
/// compare-and-swap [`group_membership_repo::end_membership`] runs is
/// already at-most-once by construction (`windows::adjust_window`'s own
/// shape: parsed and asked for before anything is resolved, compared inside
/// the writing transaction rather than out here).
async fn adjust_membership(
    Extension(state): Extension<Arc<MembershipState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path((group, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let scope = write_scope(&enforcer, &ctx).await?;
    let tenant = ctx.subject_tenant_id();
    let expected = preconditions::if_match(&headers)?;
    let request: EndMembershipRequest = preconditions::parse_body(&body)?;
    // Addressing only (§5's path shape); the row's own group is not
    // re-asserted against it — see the module doc's scope note.
    let _group = required_group(&group)?;

    let now = Utc::now();
    let stamp = audit_stamp(&ctx, now, correlation);
    let registry = Arc::clone(&state.registry);
    let db = state.db.clone();
    let mutation_ctx = ctx.clone();
    let mutation_scope = scope.clone();
    let effective_to = request.effective_to;
    let expected_version = expected.get();

    let (_, outcome) = db
        .db()
        .in_transaction::<membership_publish::MembershipPublishReceipt, DomainError, _>(
            move |txn| {
                Box::pin(async move {
                    membership_publish::end_in(
                        txn,
                        registry.as_ref(),
                        &mutation_ctx,
                        &mutation_scope,
                        tenant,
                        id,
                        effective_to,
                        expected_version,
                        stamp,
                    )
                    .await
                })
            },
        )
        .await;
    let receipt = outcome.map_err(|err| {
        err.into_domain(|infra| {
            DomainError::Internal(format!("bss-pricing: membership end transaction: {infra}"))
        })
    })?;

    Ok((
        StatusCode::OK,
        [(
            ETAG,
            preconditions::etag(RowVersion::new(receipt.membership.row_version)),
        )],
        Json(MembershipMutationView::from(&receipt)),
    )
        .into_response())
}

/// `POST /customer-groups/{group}/members/{payerId}/move`.
async fn move_membership(
    Extension(state): Extension<Arc<MembershipState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path((group, payer_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let scope = write_scope(&enforcer, &ctx).await?;
    let tenant = ctx.subject_tenant_id();
    let key = preconditions::idempotency_key(&headers)?;
    let request: MoveMembershipRequest = preconditions::parse_body(&body)?;
    let digest = preconditions::request_digest(&request)?;
    let group_value = required_group(&group)?;

    let now = Utc::now();
    let stamp = audit_stamp(&ctx, now, correlation);
    let registry = Arc::clone(&state.registry);
    let mutation_ctx = ctx.clone();
    let mutation_scope = scope.clone();
    let effective_from = request.effective_from;

    let guarded = idempotent::guarded(
        &state.db,
        &state.idempotency,
        &scope,
        GuardedRequest {
            operation: MOVE_MEMBER_OPERATION,
            client_key: key,
            request_hash: digest,
            tenant_id: tenant,
            status: StatusCode::OK.as_u16().into(),
            now,
        },
        move |txn| {
            Box::pin(async move {
                let new_membership_id = Uuid::now_v7();
                membership_publish::move_payer_in(
                    txn,
                    registry.as_ref(),
                    &mutation_ctx,
                    &mutation_scope,
                    tenant,
                    payer_id,
                    new_membership_id,
                    group_value,
                    effective_from,
                    stamp,
                )
                .await
            })
        },
        |receipt| body_of(&MembershipMoveView::from(receipt)),
    )
    .await?;

    Ok(match guarded {
        Guarded::Performed(receipt) => {
            (StatusCode::OK, Json(MembershipMoveView::from(&receipt))).into_response()
        }
        Guarded::Replayed { status, body } => replayed(status, &body),
    })
}

/// The recorded body of a guarded membership mutation: the view, as JSON.
///
/// # Errors
/// [`DomainError::Internal`] when the view will not serialize — unreachable,
/// and reported rather than unwrapped for `windows::body_of`'s reason: it
/// would otherwise abort a transaction that has already written.
fn body_of<T: serde::Serialize>(view: &T) -> Result<serde_json::Value, DomainError> {
    serde_json::to_value(view)
        .map_err(|e| DomainError::Internal(format!("cannot render the membership mutation: {e}")))
}

/// The answer a replay is handed back — the stored status and body, verbatim.
fn replayed(status: i32, body: &serde_json::Value) -> Response {
    let status = u16::try_from(status)
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok())
        .unwrap_or(StatusCode::OK);
    (status, Json(body.clone())).into_response()
}
