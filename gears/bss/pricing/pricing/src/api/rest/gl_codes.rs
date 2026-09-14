//! `GET /bss-pricing/v1/config/vocabularies/gl-codes` and its per-value routes — the
//! general-ledger codes a tenant declares (D-356).
//!
//! # Why this is not `/config/vocabularies/{class}`
//!
//! `rounding_policies`' reason, one vocabulary over:
//! [`TaxonomyClass::scope_class`](crate::domain::taxonomy::TaxonomyClass::scope_class)
//! is a **total** function into [`ScopeClass`](crate::domain::overlay::ScopeClass),
//! so a fifth class would declare
//! that an overlay may be scoped by GL code — a claim that is false and that
//! would reach the `pricing_price_overlay.scope_class` `CHECK`.
//!
//! That argument is about the **enum**, and D-371 measured that it does not by
//! itself decide the **path** — a door may parse one segment onto two enums.
//! What decides the path is the contract: this `PATCH` has no `202` arm and can
//! never have one (D-356), no tax markers in its request type, and no
//! `references` / `editGoverned` / `pendingApprovals` in its response, so one
//! template over six classes would advertise for this code a governance surface
//! it does not have. The route moved under the `vocabularies` prefix in D-371
//! and kept its own segment; see
//! [`vocabulary_values`](crate::api::rest::vocabulary_values) for the full
//! measurement.
//!
//! # What declaring a vocabulary does
//!
//! With **no** values declared, nothing changes: a plan's descriptor `glCode` may
//! be any string, which is where every tenant is today. Declare the first value
//! and `inst-ds-glcode` starts refusing, at publish and under `GL_CODE_UNKNOWN`,
//! a descriptor whose code is outside the active set. That opt-in is the whole
//! reason the vocabulary could land at all: the alternative — an empty set
//! refusing every plan — would have failed every existing catalog on the day of
//! the migration, for a vocabulary nobody had been given a chance to write.
//!
//! # Whose list this is
//!
//! The tenant's. BSS holds no authoritative GL-code list: ledger stores account
//! *class* and takes the concrete code from the Catalog snapshot at post time,
//! so Catalog was already the source and this route makes the source governed.
//! A future ERP gear (D-356 *Owed*) is the anticipated **provider** — it
//! populates or reconciles the same table this route writes, and the publish
//! rule never learns which of the two wrote it.
//!
//! # One value at a time, and the `PUT` that is gone
//!
//! `POST …/values` declares one code; `GET/PATCH …/values/{value}` read and
//! edit one. The whole-set `PUT` this module carried until then is
//! **removed**, not kept beside them, on D-353's own three reasons over this
//! table: the audit record of "the GL-code vocabulary changed" could not say
//! *which* code moved; two admins re-labelling two different codes refused
//! each other on a set they never disagreed about; and a client that saved a
//! filtered list retired every code it did not show. Each per-value record
//! now carries the code with its state before and after
//! (`taxonomy_repo::record_vocabulary_value_mutation`).
//!
//! What the `PUT` was right about survives: a value is **retired, never
//! deleted** — a published revision's descriptor set may still name it, and a
//! deletion would make that reference dangle rather than merely stop being
//! authorable — and the same guard judges the retirement
//! (`TAXONOMY_VALUE_IN_USE`, 409; nothing is written).
//!
//! The collection `GET` keeps its set `ETag`, which is now a **read**
//! validator alone: no write asserts it, and the per-value `PATCH` asserts the
//! value's own tag, which is the false conflict the split exists to remove.
//!
//! # No approval unit
//!
//! `rounding_policies`' reasoning: this narrows what may be authored and can
//! only make publishing harder, so it is `config` configuration under
//! `CatalogAdmin` rather than a D-10 unit. It is audited.

use std::sync::Arc;

use axum::extract::{Extension, Path};
use axum::http::HeaderMap;
use axum::http::header::{ETAG, LOCATION};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::StatusCode};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_security::SecurityContext;

use crate::api::rest::auth_context::require_authenticated;
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::preconditions;
use crate::api::rest::state::AuthoringState;
use crate::api::rest::vocabulary_values::{
    self, DeclareVocabularyValueRequest, PatchVocabularyValueRequest,
};
use crate::domain::taxonomy::{TaxonomyEntry, VocabularyClass};
use crate::infra::storage::repo::taxonomy_repo;
use crate::infra::storage::repo_failure;

/// `OpenAPI` tag (DE0205).
const TAG: &str = "BSS Pricing Configuration";

/// The vocabulary resource.
///
/// The literal is repeated in both `OperationBuilder` calls because DE0801
/// validates a **literal** argument and silently passes a `const` one.
pub const GL_CODES: &str = "/bss-pricing/v1/config/vocabularies/gl-codes";

/// One declared GL code.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct GlCodeValueView {
    /// The code a plan's descriptor `glCode` must match. Never blank.
    pub value: String,
    /// The operator's label for it.
    pub display_name: String,
    /// `active`, `deprecated` or `retired` (D-370). Absent reads as `active`, because a body listing a
    /// value is a body declaring it.
    pub state: Option<String>,
}

/// The vocabulary, whole.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct GlCodesView {
    /// What this document is a representation **of**.
    ///
    /// A constant, and it exists for a client rather than for a reader: a
    /// response carries no URL, so a client that must pair this body with the
    /// `ETag` header — the only source of the `PUT`'s precondition — has nothing
    /// else to attribute it by. `rounding_policies::RoundingPoliciesView` is
    /// structurally identical (`values` alone), and a client reading several
    /// config documents concurrently could pair one's tag with the other's body,
    /// which is a wrong precondition rather than a failed one.
    pub resource: String,
    /// Every declared value, in every state (D-370), ordered by value.
    ///
    /// Retirements are **included** so the round trip is honest: an operator who
    /// reads, edits and writes back can see the value they are about to
    /// re-activate.
    pub values: Vec<GlCodeValueView>,
}

/// One declared value's collection — the per-value create.
pub const GL_CODE_VALUES: &str = "/bss-pricing/v1/config/vocabularies/gl-codes/values";
/// One declared code: read and edit.
pub const GL_CODE_VALUE: &str = "/bss-pricing/v1/config/vocabularies/gl-codes/values/{value}";

/// Build the Axum router for the two operations and register them.
pub fn router(state: Arc<AuthoringState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-pricing/v1/config/vocabularies/gl-codes")
        .operation_id("bss_pricing.get_gl_codes")
        .summary("Read the tenant's declared GL codes")
        .description(
            "The vocabulary a plan's billing-descriptor `glCode` is validated against at publish \
             (D-356). **An empty set constrains nothing** - that is where every tenant starts, and \
             declaring the first value is how a tenant opts in. Retired values are included, \
             because an operator editing the set has to see what they may re-activate. Gates on \
             `config` x `read`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .param(crate::api::rest::plans::if_none_match_param())
        .handler(get_values)
        .json_response_with_schema::<GlCodesView>(
            openapi,
            StatusCode::OK,
            "The declared vocabulary.",
        )
        // The conditional read's answer (RFC 9110 section 15.4.5). Declared
        // because it is reachable: this route emits an `ETag` and honours the
        // `If-None-Match` a caller sends it back in. A read that emits a validator
        // and ignores the conditional is the half-implementation to avoid.
        .no_content_response(
            StatusCode::NOT_MODIFIED,
            "The caller's `If-None-Match` matches the current representation, so the body is \
             not re-sent.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    let router = OperationBuilder::post("/bss-pricing/v1/config/vocabularies/gl-codes/values")
        .operation_id("bss_pricing.declare_gl_code")
        .summary("Declare one GL code")
        .description(
            "Adds **one** code to the vocabulary without re-sending the set. `201` with the \
             value as stored, its own `ETag`, and a `Location` naming it. The code is the \
             resource's natural key, so there is no `Idempotency-Key`: a repeat carrying the \
             **same** body is the create's replay and answers `200`; a body naming a code the \
             tenant already declares with other content - or a retired one - is `409` \
             `TAXONOMY_VALUE_EXISTS`, and the remedy is `PATCH` on that value. `state` defaults \
             to `active`. Declaring the first code turns the publish check on: from then on a \
             plan whose descriptor `glCode` is outside the active set fails publish with \
             `GL_CODE_UNKNOWN`. One audited config mutation naming the code, and no approval \
             unit. Gates on `config` x `write`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<DeclareVocabularyValueRequest>(openapi, "The one code to declare.")
        .handler(post_value)
        .json_response_with_schema::<GlCodeValueView>(
            openapi,
            StatusCode::CREATED,
            "The code as declared, with its own `ETag` and `Location`.",
        )
        .json_response_with_schema::<GlCodeValueView>(
            openapi,
            StatusCode::OK,
            "The code was already declared with exactly this content: the create's replay.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::get("/bss-pricing/v1/config/vocabularies/gl-codes/values/{value}")
        .operation_id("bss_pricing.get_gl_code")
        .summary("Read one declared GL code")
        .description(
            "One code, `active`, `deprecated` or `retired`, with **its own `ETag`** - the tag \
             the per-value \
             `PATCH` demands, and the only place to obtain it (the set's tag from `GET \
             .../config/vocabularies/gl-codes` covers the whole list and does not satisfy the per-value \
             precondition). A code the tenant has never declared is `404`. This GET always \
             returns a fresh body with `Cache-Control: private, no-store`; it does not evaluate \
             `If-None-Match` or return `304`. Gates on `config` x `read`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .param(vocabulary_values::value_param())
        .handler(get_value)
        .json_response_with_schema::<GlCodeValueView>(openapi, StatusCode::OK, "The declared code.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router =
        OperationBuilder::patch("/bss-pricing/v1/config/vocabularies/gl-codes/values/{value}")
            .operation_id("bss_pricing.patch_gl_code")
            .summary("Edit one declared GL code")
            .description(
                "Changes only the fields the body names: `displayName` and `state`. **Retirement \
             is guarded**, at the door and again inside the write transaction: a code a \
             published plan revision's descriptor set still names is `409` \
             `TAXONOMY_VALUE_IN_USE` and nothing is written - re-point them first. `retired -> \
             active` re-activates. A body that changes nothing answers `200` and writes \
             nothing. **`If-Match` is required** and asserts the value's **own** tag from `GET \
             .../values/{value}`; the set's tag does not satisfy it. The commit is one audited \
             config mutation naming the code and its state before and after. This vocabulary \
             opens no approval unit on any edge (D-356). Gates on `config` x `write`.",
            )
            .tag(TAG)
            .authenticated()
            .no_license_required()
            .param(vocabulary_values::value_param())
            .param(vocabulary_values::if_match_value_param())
            .json_request::<PatchVocabularyValueRequest>(openapi, "The fields to change.")
            .handler(patch_value)
            .json_response_with_schema::<GlCodeValueView>(
                openapi,
                StatusCode::OK,
                "The code as it now stands, with its `ETag`.",
            )
            .error_400(openapi)
            .error_401(openapi)
            .error_403(openapi)
            .error_404(openapi)
            .error_409(openapi)
            .error_500(openapi)
            .error_503(openapi)
            .register(router, openapi);

    router
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}

async fn get_values(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = vocabulary_values::read_scope(&enforcer, &ctx).await?;
    let held = state
        .taxonomies
        .list_gl_codes(&scope, ctx.subject_tenant_id())
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    Ok(render(&held, Some(&headers)))
}

/// `POST /config/vocabularies/gl-codes/values`.
async fn post_value(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    body: axum::body::Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    // The gate at the door, not inside the shared body — `vocabulary_values::
    // read_scope`'s doc says why the census depends on it being here.
    let scope = vocabulary_values::write_scope(&enforcer, &ctx).await?;
    let (entry, status) = vocabulary_values::declare_value(
        &state,
        &scope,
        &ctx,
        correlation,
        VocabularyClass::GlCode,
        &body,
    )
    .await?;
    Ok(render_value(&entry, status))
}

/// `GET /config/vocabularies/gl-codes/values/{value}`.
async fn get_value(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(value): Path<String>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = vocabulary_values::read_scope(&enforcer, &ctx).await?;
    let entry = vocabulary_values::read_value(
        &state,
        &scope,
        ctx.subject_tenant_id(),
        VocabularyClass::GlCode,
        &value,
    )
    .await?;
    // Fresh on every read, `taxonomies::get_taxonomy_value`'s posture: the tag
    // this hands back is the `PATCH`'s precondition, and a `304` would leave a
    // caller holding one it could not have compared.
    Ok(preconditions::fresh_read(render_value(
        &entry,
        StatusCode::OK,
    )))
}

/// `PATCH /config/vocabularies/gl-codes/values/{value}`.
async fn patch_value(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path(value): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let scope = vocabulary_values::write_scope(&enforcer, &ctx).await?;
    // **The precondition is read here and only here**, after the gate, for
    // `taxonomies::patch_taxonomy_value`'s ordering — and at the door rather
    // than in the shared body, which `vocabulary_values::read_scope`'s doc
    // explains: `module_test`'s census reads this function's own text.
    let asserted = preconditions::if_match_policy(&headers).map_err(CanonicalError::from)?;
    let entry = vocabulary_values::patch_value(
        &state,
        &scope,
        &ctx,
        correlation,
        VocabularyClass::GlCode,
        &value,
        &asserted,
        &body,
    )
    .await?;
    Ok(render_value(&entry, StatusCode::OK))
}

/// One code's representation, with **its own** tag (and, on a create, where it
/// now lives).
///
/// One renderer for the three per-value verbs, for [`render`]'s reason: the
/// tag a `GET` hands out and the tag a `PATCH` answers with must come from one
/// computation over one reading.
fn render_value(entry: &TaxonomyEntry, status: StatusCode) -> Response {
    let tag = vocabulary_values::value_tag(VocabularyClass::GlCode, entry);
    let body = Json(view_of(entry));
    if status == StatusCode::CREATED {
        let location = vocabulary_values::value_location(VocabularyClass::GlCode, entry);
        return (status, [(ETAG, tag), (LOCATION, location)], body).into_response();
    }
    (status, [(ETAG, tag)], body).into_response()
}

/// One entry as the wire renders it — the collection `GET` and the three
/// per-value verbs share it, so a code cannot read one way in the list and
/// another on its own route.
fn view_of(entry: &TaxonomyEntry) -> GlCodeValueView {
    GlCodeValueView {
        value: entry.value.as_str().to_owned(),
        display_name: entry.display_name.clone(),
        state: Some(entry.state.as_str().to_owned()),
    }
}

/// The whole vocabulary with the set tag that covers it.
///
/// **A read validator alone now.** It served the `GET` and the `PUT` until the
/// per-value doors replaced the whole-set write, and no write asserts it any
/// more: the `PATCH` asserts the value's own tag, which is the false conflict
/// the split exists to remove. The conditional read still compares against
/// *this* string rather than a second rendering, so the comparison and the
/// header cannot disagree. See [`preconditions::if_none_match`].
fn render(entries: &[TaxonomyEntry], conditional: Option<&HeaderMap>) -> Response {
    let tag = preconditions::policy_etag(&taxonomy_repo::vocabulary_tag_of(
        VocabularyClass::GlCode,
        entries,
    ));
    if conditional.is_some_and(|headers| preconditions::if_none_match(headers, &tag)) {
        return preconditions::not_modified(&tag);
    }
    (
        [(ETAG, tag)],
        Json(GlCodesView {
            resource: VocabularyClass::GlCode.resource().to_owned(),
            values: entries.iter().map(view_of).collect(),
        }),
    )
        .into_response()
}
