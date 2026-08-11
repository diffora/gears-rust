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

use axum::extract::Extension;
use axum::http::HeaderMap;
use axum::http::header::ETAG;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::StatusCode};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::operation_builder::{ParamLocation, ParamSpec};
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;

use crate::api::rest::auth_context::{audit_stamp, require_authenticated};
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::preconditions;
use crate::api::rest::state::AuthoringState;
use crate::domain::error::DomainError;
use crate::domain::overlay::ScopeValue;
use crate::domain::taxonomy::{TAXONOMY_VALUE_IN_USE, TaxonomyEntry, TaxonomyState};
use crate::infra::storage::repo::taxonomy_repo::customer_group_tag_of;
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
