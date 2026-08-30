//! `GET /bss-products/v1/skus/{id}` — the SKU read door
//! (`cpt-cf-bss-products-dod-read-door`, `docs/features/foundation.md`
//! "Authoring head read") — and `POST /bss-products/v1/skus`, the SKU create
//! door (`cpt-cf-bss-products-dod-create-doors`,
//! `cpt-cf-bss-products-dod-code-reservation`,
//! `cpt-cf-bss-products-dod-containment`, `docs/features/foundation.md`
//! "Create doors", "Code reservation, atomic at insert" and "Parent and
//! scope containment at SKU create").
//!
//! Structurally the SKU twin of [`crate::api::rest::products`]: same
//! authorization shape, same miss/hit split on the read door, and the create
//! door repeats [`crate::api::rest::products::create_product`]'s order
//! (`actor_ref` resolution ahead of the gate, the authorization gate, shape
//! validation, the mutation, the conflict/audit discipline) and its
//! audit-every-refusal discipline — see that module's own doc for the
//! reasoning behind each of those this file does not repeat.
//! [`crate::api::rest::ApiState`], `require_authenticated`,
//! `repo_error_to_canonical`, `crate::api::rest::resolve_creator_actor_ref`
//! and `crate::api::rest::audit_refusal_and_report` are shared code, because
//! none of those reads which entity is being served; everything else here is
//! this file's own copy for the reason this module's doc, "What is
//! duplicated from the Product door, and why", gives.
//!
//! # What the SKU door adds over the Product door: parent and containment
//!
//! [`create_sku`] refuses three ways `create_product` structurally cannot,
//! because a `Product` has no parent: a `product_id` that does not resolve
//! in the caller's own tenant is `VALIDATION`; a parent that is `retired` or
//! `discarded` is `PARENT_TERMINAL`; a scope not provably contained in the
//! parent's is `SCOPE_NOT_CONTAINED` (`dod-containment`, P-D-39). The
//! containment rule itself is not re-implemented here — it lives in
//! [`crate::domain::containment`], already built for this door to call:
//! [`ResolvedScope::parse`] reads the parent's stored columns,
//! [`scope_input_from_payload`] reads the create payload's three-state input,
//! [`ScopePair::resolve_child`] runs the one inheritance rule, and
//! [`ScopePair::check_containment`] runs the one containment rule. This
//! module's job is only to wire the payload and the stored row into those
//! calls and to translate a [`ScopeContainment::NotContained`] verdict into
//! the wire refusal.
//!
//! **The parent lookup runs under the same [`toolkit_db::secure::AccessScope`]
//! the insert uses** — the one the `sku x write` gate returns, never a
//! second, wider scope built to read `products_product`. A parent read under
//! a broader scope would let a caller attach a SKU to a Product the PDP never
//! actually granted it visibility into, which is exactly the existence leak
//! [`crate::api::rest::products::get_product`]'s own doc names for the read
//! door's miss/hit split — the same failure mode, reached from the write
//! side instead.
//!
//! # The DTO carries the distinction the containment module protects
//!
//! [`CreateSkuRequest::region_scope`] and `::brand_scope` are
//! `Option<String>`, not `String`: under the plain `#[serde(rename_all =
//! "snake_case")]` `#[toolkit_macros::api_dto(request)]` expands to (no
//! `#[serde(default)]`, no custom deserializer), `serde_derive`'s own
//! built-in handling of an `Option<T>`-typed struct field treats an absent
//! JSON key as `None` without any attribute asking it to. That gives this
//! DTO exactly the three states [`crate::domain::containment::ScopeInput`]
//! needs apart: the key omitted (`None`, read as
//! [`ScopeInput::Omitted`]), the key sent as `""` (`Some(String::new())`,
//! read as [`ScopeInput::Unrestricted`] via [`ResolvedScope::parse`]'s own
//! empty-string rule), or the key sent as a non-empty comma-joined list
//! (`Some(_)`, read as [`ScopeInput::Restricted`]). Collapsing to a bare
//! `String` field, the way a hastier reading of "a payload value set" might,
//! would have made every omission read as an explicit empty set, which
//! [`crate::domain::containment`]'s own doc says a restricted parent then
//! refuses outright — a create that should have inherited, refused instead.
//! [`scope_input_from_payload`] is the one place this reading happens, so it
//! is data this module measured, not assumed: see
//! `skus_tests::an_omitted_scope_inherits_the_parents_value` and
//! `skus_tests::an_explicit_unrestricted_scope_against_a_restricted_parent_is_refused`
//! for the pair that proves the two payload shapes are not conflated.
//!
//! # What is duplicated from the Product door, and why
//!
//! [`insert_sku_with_event`] repeats [`crate::api::rest::products::
//! create_product`]'s own split-out `insert_product_with_event`, field for
//! field except which repository function and which `crate::infra::events`
//! payload-type constant it calls. It is not shared: both are private, free
//! functions of their own door module, and a generic form would need a type
//! parameter or a closure whose only job is picking which entity a route
//! serves — the KISS-over-DRY call this gear's own review made for exactly
//! this pair (see `crate::api::rest`'s own module doc). `resolve_creator_actor_ref`
//! **is** now shared (`crate::api::rest::resolve_creator_actor_ref`): unlike
//! the two functions above, it reads nothing about which entity is being
//! created, so keeping two copies bought nothing. The conflict discipline is
//! duplicated for the same reason as the insert helper, but is not a
//! byte-for-byte copy: [`classify_sku_insert_conflict`] answers a `bool`, not
//! `products::InsertConflict`'s two-armed enum, because `uq_products_sku_code`
//! is the *only* unique index `products_sku` carries — a SKU create has no
//! `DUPLICATE_NAME` twin to tell apart from `DUPLICATE_CODE`, so there is
//! nothing here for a second enum variant to name. [`audit_sku_refusal`] and
//! `products::refuse_insert_conflict`'s own inline `AuditCommon`/
//! `write_refusal_audit` construction have both been replaced by the shared
//! `crate::api::rest::audit_refusal_and_report`, so that part is shared too
//! — only the thin, entity-naming wrapper around it stays local to each
//! door.
//!
//! # Idempotency: the same phase, under this door's own endpoint
//!
//! [`create_sku`] runs the identical phase
//! [`crate::api::rest::products::create_product`] does — see that module's
//! own "Idempotency" section for the three outcomes, the keyless skip
//! (P-D-34) and the claim's transaction obligation (P-D-42) — under
//! [`CREATE_ENDPOINT`], its own concrete resource path. The key component
//! that differs is exactly the one that must: two creates under one client
//! key, one of a Product and one of a SKU, are different acts and claim
//! different keys.
//!
//! # The publish and discard doors
//!
//! [`publish_sku`] and [`discard_sku`] are this module's two head acts
//! (`cpt-cf-bss-products-dod-publish-door`,
//! `cpt-cf-bss-products-dod-transition-guard`). They repeat the create
//! door's preamble — `actor_ref` resolution ahead of the authorization gate,
//! the gate's own compiled scope carried through every step below it, the
//! idempotency phase, and the audit-every-refusal discipline — and add the
//! three things a create structurally cannot have, because a create has no
//! prior row: an `If-Match` precondition over an existing revision
//! (**P-D-33**), a terminality check and an edge decision
//! ([`crate::domain::transition`]), and, on publish, the governance gate
//! ([`crate::domain::governance`]) and the version freeze
//! ([`crate::domain::canonical`], `products_entity_version`).
//!
//! **The gate runs inside the door, in [`GateMode::Gate`], always**
//! (`inst-fd-gate-mode`). The mode is an internal argument and never a
//! wire-visible parameter: it is a literal in [`run_publish`], and the host
//! is a literal in [`publish_sku`]. Nothing a caller sends selects either.
//!
//! **Every phase runs inside the act's own transaction, after the idempotency
//! claim.** `Phase::Idempotency` is the pipeline's first and P-D-42 puts the
//! claim `INSERT` inside the mutation, which together put terminality, the
//! precondition, the re-validation re-run, the edge and the gate in there
//! too — see [`run_publish`]'s own doc for the retry a door that judged any
//! of them first would refuse instead of replaying.
//!
//! **The transaction's order is forced by the head-row guard, not chosen.**
//! Freeze the post-act image first, then exactly one head-row `UPDATE`, then
//! the event — see [`run_publish`]'s own doc for why each step cannot move.
//!
//! **Every refusal these two doors raise is audited under this act's own
//! `action` token** — `publish` or `discard`, never the create door's
//! `create`. [`audit_act_refusal`] is the one place that writes it and
//! [`ActContext::audit_action`] is where it is carried; the create doors
//! above keep calling `crate::api::rest::audit_refusal_and_report` and keep
//! writing `create`, so no existing row changes meaning.
//!
//! **`SkuPublished` carries `publishedVersion`** beyond the shared body core
//! (§4.5), on [`events::PublishedEventBody`]; `SkuDiscarded` carries the bare
//! core, because a discard writes no version row and moves no version
//! counter. The number is read off the frozen row the same transaction just
//! wrote, so the version the event announces and the version the row is keyed
//! at are one value.
//!
//! What these doors deliberately do **not** build, and who owns each, is
//! enumerated in [`publish_sku_gated`]'s own doc: the retirement
//! re-announcement is slice 04's, the corrected bucket-ii argument is slice
//! 07's `CorrectionDoor`'s, the approval-record consume flip is slice 05's,
//! and `composition_pending` is **this slice's own unpaid debt** rather than
//! anyone else's arrival.
//!
//! @cpt-cf-bss-products-dod-read-door
//! @cpt-cf-bss-products-dod-create-doors
//! @cpt-cf-bss-products-dod-code-reservation
//! @cpt-cf-bss-products-dod-containment
//! @cpt-cf-bss-products-dod-idempotency-store
//! @cpt-cf-bss-products-dod-publish-door
//! @cpt-cf-bss-products-dod-transition-guard

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::ETAG;
use axum::response::{IntoResponse, Response};
use bss_products_sdk::models::{EntityKind, LifecycleState};
use chrono::{DateTime, Utc};
use sea_orm::DbErr;
use serde_json::{Map as JsonMap, Value as JsonValue};
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_db::DbError;
use toolkit_db::secure::{AccessScope, TxConfig};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::preconditions;
use crate::api::rest::{
    ApiState, CREATE_RESPONSE_STATUS, ClaimVerdict, CreateOutcome, IdempotencyClaimInput,
    authz_error_to_canonical, claim_idempotency, contention_db_err, idempotency_key,
    record_idempotency_answer, replay_response, repo_error_to_canonical, require_authenticated,
};
use crate::domain::canonical;
use crate::domain::concurrency::InternalRevision;
use crate::domain::containment::{
    EmptyScopeToken, ResolvedScope, ScopeContainment, ScopeInput, ScopePair,
};
use crate::domain::error::DomainError;
use crate::domain::governance::{
    ApprovalId, EntityRef, GateMode, GovernanceGate, NoMaterialityPolicyGate,
};
use crate::domain::idempotency;
use crate::domain::transition::{
    self, ApprovalInvalidation, ApprovalInvalidationHook as _, TransitionDecision,
};
use crate::domain::validation::{Phase, ValidationPipeline, ValidationReport, ValidationRule};
use crate::infra::events;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{
    self, NewEntityVersion, NewSku, RefusalSubject, SkuRecord, VersionedEntityKind,
};

/// `OpenAPI` tag for the SKU surface's operations.
const TAG: &str = "BSS Products";

/// The `endpoint` component of every idempotency key this door claims — the
/// **concrete resource path** (**P-D-42**), which for a create is the
/// collection path, since no id exists yet to put in it.
///
/// Distinct from [`crate::api::rest::products`]'s own constant by exactly
/// the property the key needs: two creates under one client key, one of a
/// Product and one of a SKU, are different acts and must not share a key.
/// See that constant's own doc for why this is a second spelling of the path
/// [`router`] registers, and for the three reserved `internal:` lane names
/// this phase does not use.
const CREATE_ENDPOINT: &str = "/bss-products/v1/skus";

/// The SKU entity's resource marker for this door's 403/404 answers. Its own
/// type, distinct from `infra::error_mapping`'s private `SkuResource`, for
/// [`crate::api::rest::products::ProductResource`]'s own doc's reason.
#[resource_error(gts_id!("cf.bss.products.sku.v1~"))]
struct SkuResource;

/// The read surface of a SKU head.
///
/// Mirrors [`crate::api::rest::products::ProductView`], field for field,
/// against the SKU columns: no `product_code`/`brand_id`, but the parent
/// `product_id` in their place. `name_normalized`'s absence has no SKU
/// analogue — a SKU carries no normalized-name column at all.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct SkuView {
    /// The row's own id.
    pub sku_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The parent Product.
    pub product_id: Uuid,
    /// Tenant-unique among non-discarded rows.
    pub sku_code: String,
    /// Where the entity sits in the lifecycle machine (`LifecycleState`'s
    /// wire spelling — carried as a plain string, `ProductView`'s reason).
    pub lifecycle_state: String,
    /// Moves on every admitted write. The operand of this door's `ETag`.
    pub internal_revision: i64,
    /// Moves only on publish.
    pub published_version: i64,
    /// The region value set. Empty means unrestricted.
    pub region_scope: String,
    /// The brand value set. Empty means unrestricted.
    pub brand_scope: String,
    /// The pseudonymous ref of whoever created the row.
    pub created_by: String,
    /// The commit instant.
    pub created_at: DateTime<Utc>,
    /// The instant of the row's last admitted write.
    pub updated_at: DateTime<Utc>,
}

impl From<SkuRecord> for SkuView {
    fn from(record: SkuRecord) -> Self {
        Self {
            sku_id: record.sku_id,
            tenant_id: record.tenant_id,
            product_id: record.product_id,
            sku_code: record.sku_code,
            lifecycle_state: record.lifecycle_state.as_str().to_owned(),
            internal_revision: record.internal_revision,
            published_version: record.published_version,
            region_scope: record.region_scope,
            brand_scope: record.brand_scope,
            created_by: record.created_by,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

/// Build the Axum router for the SKU read door and register it with the
/// supplied `OpenAPI` registry. See
/// [`crate::api::rest::products::router`]'s doc for why this registers its
/// own absolute path rather than being nested by a caller.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-products/v1/skus/{id}")
        .operation_id("bss_products.get_sku")
        .summary("Read a SKU head")
        .description(
            "Returns the SKU head named by `id`: its identity, its parent Product, its \
             lifecycle state, both revision counters and both scope columns. Gates on \
             `sku x read`; a SKU outside the caller's authorized scope reads exactly like an \
             absent one (`404`, no existence leak). The `ETag` header carries \
             `internal_revision` and is what `PATCH`/`POST .../publish`/`POST .../discard` \
             accept back as `If-Match`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The SKU to read.")
        .handler(get_sku)
        .json_response_with_schema::<SkuView>(openapi, StatusCode::OK, "The SKU head.")
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    let router = OperationBuilder::post("/bss-products/v1/skus")
        .operation_id("bss_products.create_sku")
        .summary("Create a SKU")
        .description(
            "Mints a new SKU head as `draft` (`published_version = 0`, `internal_revision = 1`) \
             under a live parent Product and enqueues its `SkuCreated` event in the same \
             transaction, and writes nothing else. Gates on `sku x write`. The id is \
             server-minted; a caller-supplied `id` is refused `VALIDATION`. Refuses a \
             `product_id` that does not resolve in the caller's tenant as `VALIDATION`, a \
             `retired` or `discarded` parent as `PARENT_TERMINAL`, and a scope not provably \
             contained in the parent's as `SCOPE_NOT_CONTAINED`; an omitted scope inherits the \
             parent's. `sku_code`'s uniqueness is reserved by the insert itself: a collision \
             refuses `DUPLICATE_CODE` with an audited reason. \
             An optional `Idempotency-Key` header claims the key \
             `(tenant, /bss-products/v1/skus, key)` in the same transaction as the mutation: \
             a duplicate under a live key is refused `IDEMPOTENCY_KEY_IN_FLIGHT`, and the \
             same key under a different payload is refused `IDEMPOTENCY_CONFLICT`. A request \
             without the header is created normally.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<CreateSkuRequest>(openapi, "The SKU to create.")
        .handler(create_sku)
        .json_response_with_schema::<SkuView>(openapi, StatusCode::CREATED, "The created SKU head.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    register_head_act_routes(router, openapi).layer(Extension(state))
}

/// Register the two head-act routes — `POST .../publish` and
/// `POST .../discard` — onto `router`.
///
/// Split out of [`router`] rather than appended to it because the four
/// operations together run past this crate's `too_many_lines` bar; the split
/// is by act (read/create against publish/discard), so a reader looking for
/// a door finds its whole registration in one place.
fn register_head_act_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/bss-products/v1/skus/{id}/publish")
        .operation_id("bss_products.publish_sku")
        .summary("Publish a SKU")
        .description(
            "Freezes the SKU's next version and moves its head, in one transaction: the \
             post-publish content is rendered canonically, digested and written to the version \
             history, then a single head-row UPDATE bumps `published_version` and \
             `internal_revision` by one each and, on a first publish, moves `lifecycle_state` \
             from `draft` to `published`. A re-publish of a `published` or `deprecated` head \
             changes the version and leaves the state alone. Gates on `sku x publish`. \
             `If-Match` is required and carries the revision the caller authored against: an \
             absent header is refused `VALIDATION`, a stale one `STALE_REVISION`, and nothing \
             is written either way. A `retired` or `discarded` head is refused \
             `ENTITY_TERMINAL`; an entity that is no longer publishable is refused \
             `INCOMPLETE_ENTITY`; a governance refusal is `APPROVAL_REQUIRED` and flips no \
             state. An optional `Idempotency-Key` header is honoured exactly as on create.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The SKU to publish.")
        .handler(publish_sku)
        .json_response_with_schema::<SkuView>(
            openapi,
            StatusCode::OK,
            "The published SKU head, at its new revision.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    OperationBuilder::post("/bss-products/v1/skus/{id}/discard")
        .operation_id("bss_products.discard_sku")
        .summary("Discard a never-published draft SKU")
        .description(
            "Moves a `draft` SKU that has never been published (`published_version = 0`) to \
             the terminal `discarded` state and enqueues `SkuDiscarded`, in one transaction. \
             The `skuCode` and name reservations release by that same write, both unique \
             indexes being partial on non-discarded rows. Gates on `sku x write`. `If-Match` \
             is required: absent is `VALIDATION`, stale is `STALE_REVISION`. A `published` or \
             `deprecated` head is refused `ILLEGAL_TRANSITION`; a `retired` or already \
             `discarded` head is refused `ENTITY_TERMINAL`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The SKU to discard.")
        .handler(discard_sku)
        .json_response_with_schema::<SkuView>(
            openapi,
            StatusCode::OK,
            "The discarded SKU head, at its new revision.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// `GET /skus/{id}`. See [`crate::api::rest::products::get_product`]'s doc
/// for the authorization scope, the miss/hit split and why the miss carries
/// no registry code — this handler is its structural twin over
/// `sku x read` and [`repo::find_sku`].
async fn get_sku(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(sku_id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();

    // The scope handed to the repository below is exactly this one — never
    // one rebuilt from `tenant_id` — so the read stays under the SQL-level
    // filter the PDP actually granted.
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::SKU,
        crate::authz::actions::READ,
        /* owner_tenant_id */ None,
        /* resource_id */ Some(sku_id),
        /* require_constraints */ true,
    )
    .await
    .map_err(|e| {
        authz_error_to_canonical(e, |reason| {
            SkuResource::permission_denied()
                .with_reason(reason)
                .create()
        })
    })?;

    let conn = state
        .db
        .conn()
        .map_err(|e| CanonicalError::internal(format!("bss-products: db conn: {e}")).create())?;

    let record = repo::find_sku(&conn, &scope, tenant_id, sku_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .ok_or_else(|| sku_not_found(sku_id))?;

    let tag = preconditions::etag(InternalRevision::new(record.internal_revision));
    Ok(([(ETAG, tag)], axum::Json(SkuView::from(record))).into_response())
}

/// The `404` a miss (absent OR out-of-scope, indistinguishably) answers
/// with. Bare on purpose — see [`crate::api::rest::products`]'s doc's "The
/// miss" section.
fn sku_not_found(sku_id: Uuid) -> CanonicalError {
    SkuResource::not_found("no SKU matches this id in the caller's scope")
        .with_resource(sku_id.to_string())
        .create()
}

/// `POST /bss-products/v1/skus` request body.
///
/// Carries an **explicit, optional `id` field**, refused `VALIDATION` when
/// present, for [`crate::api::rest::products::CreateProductRequest`]'s own
/// stated reason. `region_scope` and `brand_scope` are `Option<String>`, not
/// `String` — see this module's doc, "The DTO carries the distinction the
/// containment module protects", for why that is load-bearing rather than
/// cosmetic.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct CreateSkuRequest {
    /// Must be absent. Present only so a caller-supplied value can be
    /// refused `VALIDATION` by name rather than silently dropped — see
    /// [`crate::api::rest::products::CreateProductRequest`]'s own doc for why
    /// the explicit-field reading was chosen over the field-less one. The
    /// entity id is always server-minted (`dod-create-doors`).
    pub id: Option<Uuid>,
    /// Required. The parent Product this SKU belongs to. Resolved under the
    /// caller's own authorized `sku x write` scope (`dod-containment`); does
    /// not resolve to `VALIDATION`, resolves but is `retired`/`discarded` to
    /// `PARENT_TERMINAL`.
    pub product_id: Uuid,
    /// Tenant-unique among non-discarded rows, reserved by the insert itself
    /// (`dod-code-reservation`).
    pub sku_code: String,
    /// The region value set. Omitted inherits the parent Product's resolved
    /// value; an explicit empty string is an unrestricted claim the parent
    /// may refuse (`SCOPE_NOT_CONTAINED`); a non-empty comma-joined list is
    /// checked for containment in the parent's own set.
    pub region_scope: Option<String>,
    /// The brand value set. Same three-state reading as `region_scope`.
    pub brand_scope: Option<String>,
}

/// Convert a create payload's raw scope field into the containment module's
/// [`ScopeInput`] — the point where an absent JSON key (`None`) and an
/// explicit empty string (`Some(String::new())`) are read apart, before
/// either reaches [`crate::domain::containment`]. See this module's doc,
/// "The DTO carries the distinction", for why this reading is the one that
/// keeps the two apart rather than collapsing them.
///
/// # Errors
///
/// [`EmptyScopeToken`] when the payload named a non-empty value containing an
/// empty token (`","`, `"eu,,us"`, `",eu"`) — see
/// [`crate::domain::containment::ResolvedScope::parse`]'s own doc for why
/// this is rejected rather than silently filtered.
fn scope_input_from_payload(raw: Option<String>) -> Result<ScopeInput, EmptyScopeToken> {
    match raw {
        None => Ok(ScopeInput::Omitted),
        Some(value) => match ResolvedScope::parse(&value)? {
            ResolvedScope::Unrestricted => Ok(ScopeInput::Unrestricted),
            ResolvedScope::Restricted(values) => Ok(ScopeInput::Restricted(values)),
        },
    }
}

/// A [`ResolvedScope`] rendered for a rejection message: `unrestricted`, or
/// its comma-joined value list.
fn describe_resolved_scope(scope: &ResolvedScope) -> String {
    match scope {
        ResolvedScope::Unrestricted => "unrestricted".to_owned(),
        ResolvedScope::Restricted(_) => scope.render(),
    }
}

/// Translate a [`ScopeContainment::NotContained`] verdict into the door's
/// `SCOPE_NOT_CONTAINED` [`DomainError`] (`dod-containment`, P-D-39), for
/// [`create_sku`] to hand to `crate::api::rest::audit_refusal_and_report`.
/// The containment rule itself already ran, in
/// [`crate::domain::containment::ScopePair::check_containment`]; this
/// function only names, in the message, which dimension failed and what the
/// parent and child resolved to.
///
/// # Errors
///
/// A bare internal [`CanonicalError`] on [`ScopeContainment::Contained`]:
/// `check_containment` only ever wraps a `NotContained` verdict in its `Err`
/// branch — a `Contained` verdict short-circuits to `Ok(())` inside that
/// function's own body — but the type itself does not encode that, so this
/// arm answers a typed internal error rather than reaching for
/// `unreachable!()`, a denied restriction lint in this crate. This branch is
/// not itself an audited refusal — nothing was actually refused — so it
/// answers a plain `CanonicalError`, not a `DomainError` for the caller to
/// audit.
fn scope_not_contained_domain_err(
    failure: ScopeContainment,
) -> Result<DomainError, CanonicalError> {
    match failure {
        ScopeContainment::NotContained {
            dimension,
            parent,
            child,
        } => {
            let detail = format!(
                "{} is not contained in the parent Product's: parent {}, child {}",
                dimension.column_name(),
                describe_resolved_scope(&parent),
                describe_resolved_scope(&child)
            );
            Ok(DomainError::ScopeNotContained(detail))
        }
        ScopeContainment::Contained => Err(CanonicalError::internal(
            "bss-products: containment check reported Contained on a refusal path",
        )
        .create()),
    }
}

/// Pin `crate::api::rest::audit_refusal_and_report` to this door's own
/// `subject_kind` and `RefusalSubject::Attempted(sku_code)` — every pre-mint
/// refusal [`create_sku`] raises (`VALIDATION`, `PARENT_TERMINAL`,
/// `SCOPE_NOT_CONTAINED`) shares those two arguments, since none of them has
/// a minted `sku_id` yet to name instead, so this wrapper keeps each call
/// site in [`create_sku`] down to the one thing that actually differs: the
/// refusal itself.
async fn audit_sku_refusal(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    sku_code: &str,
    domain_err: DomainError,
) -> CanonicalError {
    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: crate::authz::labels::SKU,
            error_code: domain_err.code(),
        },
        RefusalSubject::Attempted(sku_code.to_owned()),
        CanonicalError::from(domain_err),
    )
    .await
}

/// The digest of one parsed create request, as the claim is taken against
/// (`crate::domain::idempotency`, **P-D-34**).
///
/// The SKU twin of [`crate::api::rest::products`]'s own `payload_digest`,
/// over this `DTO`'s own carried fields; that function's doc carries the
/// reasoning this one does not repeat — parsed values rather than received
/// bytes, omitted fields left out rather than rendered `null`, and nothing
/// from the transport in the operand.
///
/// One difference worth naming: `region_scope`/`brand_scope` are the
/// three-state fields this module's own doc calls load-bearing, and only two
/// of those three states survive into the digest, because `Option<String>`
/// renders an absent key and an explicit `null` identically. The state that
/// *is* preserved is the one this door acts on — an explicit `""` renders as
/// an empty string and an omission renders as nothing — so a create that
/// inherits and a create that claims unrestricted never share a digest.
fn payload_digest(request: &CreateSkuRequest) -> Vec<u8> {
    let mut fields = JsonMap::new();
    if let Some(id) = request.id {
        fields.insert("id".to_owned(), JsonValue::String(id.to_string()));
    }
    fields.insert(
        "product_id".to_owned(),
        JsonValue::String(request.product_id.to_string()),
    );
    fields.insert(
        "sku_code".to_owned(),
        JsonValue::String(request.sku_code.clone()),
    );
    if let Some(region) = request.region_scope.clone() {
        fields.insert("region_scope".to_owned(), JsonValue::String(region));
    }
    if let Some(brand) = request.brand_scope.clone() {
        fields.insert("brand_scope".to_owned(), JsonValue::String(brand));
    }
    idempotency::payload_digest(&JsonValue::Object(fields))
}

/// Insert the entity row and enqueue its `SkuCreated` event, in one
/// transaction (`dod-create-doors`) — and nothing else. The SKU door's own
/// copy of [`crate::api::rest::products::create_product`]'s
/// `insert_product_with_event` — see this module's doc, "What is duplicated
/// from the Product door, and why", for why this is not a shared function.
///
/// Returns the raw [`DbError`] on failure rather than a [`CanonicalError`]:
/// [`create_sku`] still needs the driver text this error carries to
/// distinguish a `sku_code` collision from an unrelated storage failure
/// (`classify_sku_insert_conflict`), which a [`CanonicalError`] would already
/// have discarded.
///
/// # The claim runs here, on the mutation's own runner
///
/// `claim` is `Some` exactly when the request carried an `Idempotency-Key`,
/// and its `INSERT` executes inside this closure on the same `tx` the entity
/// insert and the outbox enqueue use — **P-D-42**'s requirement, so that a
/// rollback frees the key with no release step. See
/// [`crate::api::rest::products`]'s `insert_product_with_event` for the same
/// obligation stated in full, and `crate::api::rest::claim_idempotency` for
/// why a runner of its own would break the one property this mechanism
/// exists to provide.
///
/// # The answer runs here too, last
///
/// `record_idempotency_answer` runs after the entity insert and the outbox
/// enqueue, on that same `tx`: it stores the response body, and the body
/// cannot be rendered before the row it renders exists. Claim, mutation and
/// answer therefore commit together or not at all
/// (`inst-fd-idem-claim-write`), and the value stored is the very value
/// [`create_sku`] answers, carried out on [`CreateOutcome::Created`] rather
/// than re-rendered for the wire.
///
/// # The mutation runs under `transaction_with_retry`, not a bare transaction
///
/// `DBProvider::transaction` has no contention retry, and the claim `INSERT`
/// being the gate (P-D-42) makes this transaction one that *concurrent
/// duplicates deliberately collide on*. On `SQLite` "the loser is answered
/// `SQLITE_BUSY` rather than blocking, so the door carries a busy timeout and
/// retries" (`design/01-foundation.md` §3.2 `inst-fd-idem-claim-txn`), and on
/// `PostgreSQL` the same collision can surface as a serialization failure.
/// Without a retry that transaction fails outright, and the failure carries
/// neither "unique constraint" nor "duplicate key", so `classify_insert_conflict`
/// does not recognise it either: the client gets a bare 500 instead of the
/// replay or the `409` the store promises it. `toolkit_db::Db::
/// transaction_with_retry` classifies both through
/// `toolkit_db::contention::is_retryable_contention`, and `contention_db_err`
/// is the accessor it asks the caller for.
///
/// **The closure is safe to re-run.** Its first statement is the claim, and
/// the claim rolls back with everything after it (P-D-38), so a retried
/// attempt starts against exactly the state the first one started against:
/// no key held, no entity row, no outbox row. Nothing in it is derived from
/// the attempt — `now` and `expires_at` were stamped before the first — so
/// the values written are attempt-independent. The body is `FnMut`, so the
/// inputs are cloned per attempt rather than moved in once.
async fn insert_sku_with_event(
    state: &ApiState,
    scope: AccessScope,
    new: NewSku,
    claim: Option<IdempotencyClaimInput>,
) -> Result<CreateOutcome, DbError> {
    let outbox = Arc::clone(&state.outbox);
    let tenant_id = new.tenant_id;
    state
        .db
        .db()
        .transaction_with_retry::<CreateOutcome, DbError, _, _>(
            TxConfig::default(),
            contention_db_err,
            move |tx| {
                // `FnMut`: every attempt gets its own copies, so a retried
                // attempt never finds an input the previous one consumed.
                // Nothing here is derived from the attempt — the claim's
                // `now`/`expires_at` were stamped before the first one — so
                // the second attempt writes exactly what the first tried to.
                let outbox = Arc::clone(&outbox);
                let scope = scope.clone();
                let new = new.clone();
                let claim = claim.clone();
                Box::pin(async move {
                    if let Some(input) = claim.as_ref() {
                        match claim_idempotency(tx, &scope, tenant_id, input)
                            .await
                            .map_err(|e| DbError::Sea(DbErr::Custom(e.to_string())))?
                        {
                            ClaimVerdict::Proceed => {}
                            ClaimVerdict::Replay { status, body } => {
                                return Ok(CreateOutcome::Replay { status, body });
                            }
                            ClaimVerdict::Refused(refusal) => {
                                return Ok(CreateOutcome::Refused(refusal));
                            }
                        }
                    }

                    let record = repo::insert_sku(tx, &scope, new)
                        .await
                        .map_err(|e| DbError::Sea(DbErr::Custom(e.to_string())))?;

                    let core = events::EventBodyCore {
                        tenant_id: record.tenant_id,
                        entity_kind: events::EntityKind::Sku.as_str(),
                        entity_id: record.sku_id,
                        internal_revision: record.internal_revision,
                        lifecycle_state: record.lifecycle_state.as_str(),
                    };
                    events::enqueue(
                        &outbox,
                        tx,
                        record.sku_id,
                        events::SKU_CREATED_PAYLOAD_TYPE,
                        &core,
                    )
                    .await
                    .map_err(|e| DbError::Sea(DbErr::Custom(format!("enqueue SkuCreated: {e}"))))?;

                    let internal_revision = record.internal_revision;
                    let body = serde_json::to_value(SkuView::from(record)).map_err(|e| {
                        DbError::Sea(DbErr::Custom(format!("render the created SKU: {e}")))
                    })?;

                    if let Some(input) = claim.as_ref() {
                        record_idempotency_answer(
                            tx,
                            &scope,
                            tenant_id,
                            input,
                            CREATE_RESPONSE_STATUS,
                            &body,
                        )
                        .await
                        .map_err(|e| DbError::Sea(DbErr::Custom(e.to_string())))?;
                    }

                    Ok(CreateOutcome::Created {
                        internal_revision,
                        body,
                    })
                })
            },
        )
        .await
}

/// Resolve the parent Product and this create's child scope pair
/// (`dod-containment`), under the caller's own granted `scope` — the natural
/// seam [`create_sku`] extracts this into: it takes the granted `scope` and
/// the payload's two scope inputs and returns either the resolved child
/// [`ScopePair`] or an already-audited refusal, and does nothing else.
///
/// Refuses `VALIDATION` when `product_id` does not resolve in the caller's
/// own tenant, `PARENT_TERMINAL` when the parent is `retired`/`discarded`,
/// `VALIDATION` when either scope field contains an empty token (F-5,
/// [`ResolvedScope::parse`]'s own doc), and `SCOPE_NOT_CONTAINED` when the
/// resolved child is not contained in the parent's. Every refusal here goes
/// through [`audit_sku_refusal`], exactly as every other refusal in
/// [`create_sku`] does. The parent's own stored scope columns are parsed too,
/// but as trusted data rather than a caller input: a failure there answers a
/// bare internal `CanonicalError`, never an audited refusal, since it would
/// name a storage invariant violation, not this request's fault.
/// The two scope fields as the payload carried them, grouped so
/// [`resolve_parent_scope`] stays within the argument bar.
///
/// `None` is **omitted**, not empty: an omitted set inherits the parent's,
/// while an explicitly unrestricted one is a claim a restricted parent
/// refuses (P-D-39). Keeping the pair together also keeps the two
/// dimensions from drifting apart at the call site.
pub(crate) struct PayloadScopes {
    pub(crate) region: Option<String>,
    pub(crate) brand: Option<String>,
}

async fn resolve_parent_scope(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    product_id: Uuid,
    sku_code: &str,
    payload_scopes: PayloadScopes,
) -> Result<ScopePair, CanonicalError> {
    let PayloadScopes {
        region: region_scope,
        brand: brand_scope,
    } = payload_scopes;
    let conn = state
        .db
        .conn()
        .map_err(|e| CanonicalError::internal(format!("bss-products: db conn: {e}")).create())?;
    let parent = repo::find_product(&conn, scope, tenant_id, product_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;

    let Some(parent) = parent else {
        let mut report = ValidationReport::new();
        report.violate(
            "VALIDATION",
            "product_id",
            "product_id does not resolve to a Product in the caller's tenant",
        );
        return Err(audit_sku_refusal(
            state,
            scope,
            tenant_id,
            actor_ref,
            sku_code,
            DomainError::Validation(report),
        )
        .await);
    };

    if parent.lifecycle_state.is_terminal() {
        let domain_err = DomainError::ParentTerminal(format!(
            "the parent Product {} is `{}`",
            parent.product_id,
            parent.lifecycle_state.as_str()
        ));
        return Err(
            audit_sku_refusal(state, scope, tenant_id, actor_ref, sku_code, domain_err).await,
        );
    }

    // The parent's own stored columns are trusted data this door itself
    // never wrote a malformed value into (F-5's `EmptyScopeToken` refusal
    // runs on every payload this door admits) — an error here is a storage
    // invariant violation, not the caller's own refusal, so it renders like
    // `repo_error_to_canonical`'s own bare 500 rather than through the
    // refusal-audit discipline above.
    let parent_region = ResolvedScope::parse(&parent.region_scope).map_err(|EmptyScopeToken| {
        CanonicalError::internal(
            "bss-products: parent Product's stored region_scope contains an empty token",
        )
        .create()
    })?;
    let parent_brand = ResolvedScope::parse(&parent.brand_scope).map_err(|EmptyScopeToken| {
        CanonicalError::internal(
            "bss-products: parent Product's stored brand_scope contains an empty token",
        )
        .create()
    })?;
    let parent_scope = ScopePair {
        region: parent_region,
        brand: parent_brand,
    };

    // Both fields are parsed before either is judged, so a payload that gets
    // both wrong is refused with one report naming both, the same
    // one-phase-collects-every-violation shape the shape-validation step
    // in `create_sku` uses (P-D-33).
    let region_result = scope_input_from_payload(region_scope);
    let brand_result = scope_input_from_payload(brand_scope);
    let (region_input, brand_input) = match (region_result, brand_result) {
        (Ok(region_input), Ok(brand_input)) => (region_input, brand_input),
        (region_result, brand_result) => {
            let mut report = ValidationReport::new();
            if region_result.is_err() {
                report.violate(
                    "VALIDATION",
                    "region_scope",
                    "region_scope must not contain an empty value between separators",
                );
            }
            if brand_result.is_err() {
                report.violate(
                    "VALIDATION",
                    "brand_scope",
                    "brand_scope must not contain an empty value between separators",
                );
            }
            return Err(audit_sku_refusal(
                state,
                scope,
                tenant_id,
                actor_ref,
                sku_code,
                DomainError::Validation(report),
            )
            .await);
        }
    };
    let child_scope = parent_scope.resolve_child(region_input, brand_input);
    if let Err(failure) = parent_scope.check_containment(&child_scope) {
        let domain_err = scope_not_contained_domain_err(failure)?;
        return Err(
            audit_sku_refusal(state, scope, tenant_id, actor_ref, sku_code, domain_err).await,
        );
    }

    Ok(child_scope)
}

/// `POST /skus`: mint a SKU head as a `draft` under a live parent Product.
///
/// See this module's doc, "What the SKU door adds over the Product door",
/// for the three parent/containment refusals, and
/// [`crate::api::rest::products::create_product`]'s own doc for the
/// audit-every-refusal discipline this handler repeats.
///
/// # Every refusal is audited, on its own runner
///
/// See `crate::api::rest::products::create_product`'s own doc, "Every
/// refusal is audited, on its own runner" — this door drives the identical
/// set of pre-mint refusals (authorization denial, every shape `VALIDATION`,
/// plus this door's own `PARENT_TERMINAL` and `SCOPE_NOT_CONTAINED`, and the
/// `DUPLICATE_CODE` conflict) through the identical shared
/// `crate::api::rest::audit_refusal_and_report`, naming its subject with
/// `RefusalSubject::Attempted(sku_code)` throughout, since none of them has a
/// minted `sku_id` yet to name instead.
///
/// # Order of operations, and why
///
/// 1. The request is destructured up front — before anything that can
///    refuse runs — so `trimmed_sku_code` exists for every refusal below to
///    audit against, the authorization denial included.
/// 2. [`repo::resolve_actor_ref`] (via `crate::api::rest::
///    resolve_creator_actor_ref`), on its own transaction — ahead of the
///    authorization gate, `crate::api::rest::products::create_product`'s own
///    doc's reason.
/// 3. The `sku x write` gate (`crate::authz::access_scope`), anchored to the
///    caller's own tenant. The returned scope is what every step below
///    uses — the parent lookup never runs under a different one. A denial is
///    audited under the caller's own tenant-scoped self access
///    (`crate::api::rest::audit_refusal_and_report`'s own doc).
/// 4. The idempotency phase (`dod-idempotency-store`): read
///    `Idempotency-Key` off the headers and digest the parsed body, at the
///    position `design/01-foundation.md` §2's step list puts it — in step 2,
///    with the authorization gate and after the `actor_ref` resolution. **A
///    request with no header skips the phase** (P-D-34); a header present
///    but unusable is `VALIDATION`. The claim `INSERT` itself joins the
///    mutation's transaction in step 7 (P-D-42).
/// 5. Shape validation: `sku_code` non-blank, `product_id` non-nil, `id`
///    absent (`dod-create-doors`'s server-minted-id clause, F-6).
/// 6. Parent resolution and containment (`dod-containment`), via
///    [`resolve_parent_scope`]: the parent must resolve in the tenant (else
///    `VALIDATION`), must not be terminal (else `PARENT_TERMINAL`), and the
///    payload's scope, resolved against the parent's, must be contained in
///    it (else `SCOPE_NOT_CONTAINED`). The payload's own scope tokens are
///    parsed there too (`scope_input_from_payload`); an empty token (`","`,
///    `"eu,,us"`, `",eu"`) is refused `VALIDATION`, not silently filtered
///    (`crate::domain::containment::ResolvedScope::parse`'s own doc).
/// 7. The mutation: the idempotency claim, [`repo::insert_sku`],
///    `crate::infra::events`'s `SkuCreated` enqueue and the answer written
///    back into the claim, in one transaction (`dod-create-doors`, P-D-42,
///    `inst-fd-idem-claim-write`).
/// 8. On a `sku_code` collision, [`classify_sku_insert_conflict`] and
///    [`refuse_sku_insert_conflict`]; on an idempotency verdict, a replay
///    served from the stored answer or a refusal audited through the same
///    [`audit_sku_refusal`] wrapper every other refusal here uses.
async fn create_sku(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
    Json(body): Json<CreateSkuRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = Utc::now();

    // Taken from the parsed request before it is destructured, so the
    // operand is what the caller sent rather than what the steps below
    // derive from it (`payload_digest`'s own doc).
    let payload_hash = payload_digest(&body);

    // -- 1. Destructure up front: every refusal below, including the
    // authorization denial, audits against `trimmed_sku_code`. --
    let CreateSkuRequest {
        id: caller_supplied_id,
        product_id,
        sku_code: raw_sku_code,
        region_scope,
        brand_scope,
    } = body;
    let trimmed_sku_code = raw_sku_code.trim().to_owned();

    // -- 2. actor_ref resolution: its own transaction, ahead of the gate. --
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;

    // -- 3. The authorization gate. The scope handed to the parent lookup
    // and the insert below is exactly this one — never one rebuilt from
    // `tenant_id` directly — for this module's doc's reason. --
    let scope = match crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::SKU,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(tenant_id),
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    {
        Ok(scope) => scope,
        Err(crate::authz::AuthzError::Denied(reason)) => {
            let self_scope = AccessScope::for_tenant(tenant_id);
            return Err(crate::api::rest::audit_refusal_and_report(
                &state,
                &self_scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: crate::authz::labels::SKU,
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(trimmed_sku_code.clone()),
                SkuResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await);
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            return Err(authz_error_to_canonical(err, |reason| {
                SkuResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }));
        }
    };

    // -- 4. The idempotency phase: the key, and the digest taken above. An
    // absent header is the skip (P-D-34), not a refusal; a present but
    // unusable one is `VALIDATION`, audited like every other shape refusal.
    // The claim `INSERT` itself joins the mutation's transaction in step 7
    // (P-D-42) — see `crate::api::rest`'s module doc, "The idempotency
    // phase", for why the phase sits here and why it is split in two. --
    let client_key = match idempotency_key(&headers) {
        Ok(key) => key,
        Err(domain_err) => {
            return Err(audit_sku_refusal(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                &trimmed_sku_code,
                domain_err,
            )
            .await);
        }
    };
    let claim = client_key.map(|key| {
        IdempotencyClaimInput::new(
            CREATE_ENDPOINT,
            key,
            payload_hash,
            now,
            state.idempotency_retention_hours,
        )
    });

    // -- 5. Shape validation. --
    let mut report = ValidationReport::new();
    if trimmed_sku_code.is_empty() {
        report.violate("VALIDATION", "sku_code", "sku_code must not be blank");
    }
    if product_id.is_nil() {
        report.violate("VALIDATION", "product_id", "product_id is required");
    }
    if caller_supplied_id.is_some() {
        report.violate(
            "VALIDATION",
            "id",
            "id is server-minted and must not be supplied",
        );
    }
    if !report.is_empty() {
        return Err(audit_sku_refusal(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            &trimmed_sku_code,
            DomainError::Validation(report),
        )
        .await);
    }

    // -- 6. Parent resolution and containment (`dod-containment`), under the
    // same scope the insert (step 7) uses. Extracted into
    // `resolve_parent_scope`: it has one job — taking the granted `scope`
    // and the payload's two scope inputs and returning either the resolved
    // child scope pair or an already-audited refusal — which is what keeps
    // this handler's own body to the seven steps its doc enumerates rather
    // than their expansion.
    let child_scope = resolve_parent_scope(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        product_id,
        &trimmed_sku_code,
        PayloadScopes {
            region: region_scope,
            brand: brand_scope,
        },
    )
    .await?;

    // -- 7. The mutation: the idempotency claim, the entity row, its
    // creation outbox row and the answer written back into the claim, one
    // transaction, nothing else written. --
    let attempted_code = trimmed_sku_code.clone();
    let new = NewSku {
        sku_id: Uuid::new_v4(),
        tenant_id,
        product_id,
        sku_code: trimmed_sku_code,
        region_scope: child_scope.region.render(),
        brand_scope: child_scope.brand.render(),
        created_by: actor_ref.to_string(),
        created_at: now,
    };

    let insert_outcome = insert_sku_with_event(&state, scope.clone(), new, claim).await;

    match insert_outcome {
        Ok(CreateOutcome::Created {
            internal_revision,
            body,
        }) => {
            let tag = preconditions::etag(InternalRevision::new(internal_revision));
            // The body rendered inside the mutation transaction, and stored
            // there as this key's answer: answering it rather than
            // re-rendering the view is what makes a replay reproduce this
            // response exactly.
            Ok((CREATE_RESPONSE_STATUS, [(ETAG, tag)], Json(body)).into_response())
        }
        // A replay executes nothing and audits nothing: it is not a refusal,
        // and the act it reproduces was audited, or deliberately not
        // (P-D-21), when it originally ran.
        Ok(CreateOutcome::Replay { status, body }) => Ok(replay_response(status, body)),
        // An idempotency refusal takes the same audit-then-answer discipline
        // as every other refusal this door raises, through the same
        // `audit_sku_refusal` wrapper — never a path of its own.
        Ok(CreateOutcome::Refused(domain_err)) => Err(audit_sku_refusal(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            &attempted_code,
            domain_err,
        )
        .await),
        Err(db_error) => {
            let message = db_error.to_string();
            if classify_sku_insert_conflict(&message) {
                Err(refuse_sku_insert_conflict(
                    &state,
                    &scope,
                    tenant_id,
                    actor_ref,
                    attempted_code,
                )
                .await)
            } else {
                Err(repo_error_to_canonical(&RepoError::Db(message)))
            }
        }
    }
}

/// Tell a `sku_code` collision from an unrelated storage failure, off the
/// driver text an insert failure already carries. A `bool`, not
/// [`crate::api::rest::products`]'s `InsertConflict` enum — see this
/// module's doc, "What is duplicated from the Product door, and why", for
/// why `products_sku` has only the one unique index to classify.
///
/// See [`crate::api::rest::products::classify_insert_conflict`]'s own doc for
/// the cost this substring match over driver text carries, which applies
/// identically here.
fn classify_sku_insert_conflict(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let looks_like_a_unique_violation =
        lower.contains("unique constraint") || lower.contains("duplicate key");
    looks_like_a_unique_violation && lower.contains("sku_code")
}

/// Refuse a `sku_code` insert conflict: write its audit row on a transaction
/// of its own, then answer `DUPLICATE_CODE` — or, if the audit row could not
/// be written, `AUDIT_UNAVAILABLE` instead, never the domain refusal
/// (`crate::api::rest::audit_refusal_and_report`'s own contract;
/// `dod-code-reservation`).
///
/// `scope` is the caller's own compiled write scope from the authorization
/// gate this door already ran — the refusal audit row is written under the
/// same tenant-scoped access the mutation itself was authorized under, not a
/// fresh, broader one.
async fn refuse_sku_insert_conflict(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    holder_code: String,
) -> CanonicalError {
    let domain_err = DomainError::DuplicateCode(format!(
        "sku_code \"{holder_code}\" is already reserved for this tenant"
    ));

    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: crate::authz::labels::SKU,
            error_code: domain_err.code(),
        },
        RefusalSubject::Attempted(holder_code),
        CanonicalError::from(domain_err),
    )
    .await
}

// ---------------------------------------------------------------------------
// The publish and discard doors (`dod-publish-door`, `dod-transition-guard`)
// ---------------------------------------------------------------------------

/// The `endpoint` component of the idempotency key the publish door claims:
/// the **concrete resource path** (**P-D-42**), which for a head act carries
/// the entity's own id rather than the route template's placeholder.
///
/// A function rather than a constant for exactly that reason — the path is
/// not knowable until the request names an id — and the reason it can be one
/// is that `crate::api::rest::IdempotencyClaimInput::endpoint` is a `String`.
/// Two publishes of two different SKUs under one client key are different
/// acts and claim different keys, which is the same property
/// [`CREATE_ENDPOINT`] gives the create doors, held one level further down.
fn publish_endpoint(sku_id: Uuid) -> String {
    format!("/bss-products/v1/skus/{sku_id}/publish")
}

/// [`publish_endpoint`]'s discard twin. A publish and a discard of one SKU
/// under one client key are likewise different acts, and the `/publish` and
/// `/discard` suffixes are what keep them apart.
fn discard_endpoint(sku_id: Uuid) -> String {
    format!("/bss-products/v1/skus/{sku_id}/discard")
}

/// The status a publish or a discard answers on success, and therefore the
/// status a replay of one reproduces. One spelling, read by the response and
/// by the stored answer alike, for `CREATE_RESPONSE_STATUS`'s own reason.
const ACT_RESPONSE_STATUS: StatusCode = StatusCode::OK;

/// The **complete named field set** a frozen SKU version row's content is
/// rendered against (§4.3, `canonical::Absence::Null`).
///
/// Named here, once, so a later slice adding a content-bearing column to
/// `products_sku` has one place to change rather than a roster spread across
/// a builder — and `skus_tests::
/// the_sku_content_roster_is_the_head_table_minus_the_excluded_columns` is
/// what fails if it forgets.
///
/// # The rule this is derived from
///
/// §4.3 scopes the frozen content as **the publish-time entity**, excluding
/// the metadata map and excluding `lifecycle_state`,
/// `deprecation_provenance`, `replaced_by_sku_id` and `internal_revision`
/// (**P-D-24**, extended by **P-D-35**). The roster is therefore
/// `products_sku`'s own column list minus those, and this is the identical
/// derivation `products::PRODUCT_CONTENT_ROSTER` states for its own table:
/// the two rosters differ only where the two tables differ, never in how
/// they are read off §4.3.
///
/// **What is excluded, and why each:**
///
/// - **`lifecycle_state` and `internal_revision`** — §4.3's own, verbatim
///   (**P-D-24**, **P-D-35**). Both move for reasons that are not content.
/// - **`deprecation_provenance` and `replaced_by_sku_id`**, §4.3's other
///   two, are not columns on `products_sku` at this commit; they arrive with
///   `04-lifecycle`, and the exclusion is already stated here so that adding
///   them does not read as an omission from this roster.
/// - **The metadata map** is excluded and is likewise not a column yet.
/// - **`updated_at` and `published_version`** — neither is in §4.3's
///   enumeration, and both are excluded anyway. §4.3 enumerates its
///   exclusions as a closed list of four columns plus the metadata map, and
///   neither of these two is on it, so each is a reading the code states.
///   The sections below are the arguments, and they are the same arguments
///   `products::PRODUCT_CONTENT_ROSTER` makes.
///
/// **`brand_id` is not here because a SKU does not carry one, and §4.2's
/// silence is the whole of the evidence.** §4.2's column roster for
/// `products_sku` simply has no `brand_id`, so the freeze stores what the SKU
/// row holds — its `product_id` — and a reader looking for `brand_id` beside
/// `brand_scope` is looking for a column that does not exist. Nothing in
/// `01-foundation.md` says a SKU *inherits* a brand from its parent: the only
/// inheritance the document states is of **scope**
/// (`inst-fd-containment-scope`), which is a different column and a different
/// rule. An earlier revision of this doc attributed the inheritance claim to
/// §4.2; the claim is not there, and the exclusion rests on the roster's
/// silence alone. That is a difference in the **table**, not in the reading
/// of §4.3.
///
/// # `updated_at` is excluded by P-D-35's own criterion
///
/// This is an **application of a stated criterion to a column the
/// enumeration does not list**, not a new rule this code invented. §4.3
/// gives P-D-35's criterion in words: those columns *"move on transitions,
/// which write no version row, so freezing them would need the digest to
/// change on a write that produces no row to digest"* — and it adds
/// `internal_revision` on exactly that ground, noting it *"was left out of
/// the original enumeration"*. `updated_at` moves on every transition and
/// every save; it meets the criterion verbatim, and it was left out of the
/// enumeration the same way `internal_revision` was. §5 corroborates from a
/// second direction: it already counts the update timestamp among the
/// mechanical columns that sit outside the bucket comparison. Nothing is
/// lost by leaving it out — the instant of the write that produced this
/// version is on the version row already, as `published_at`.
///
/// # `published_version` is excluded because it is the row's own key
///
/// `products_entity_version`'s primary key is `(tenant_id, entity_kind,
/// entity_id, published_version)`
/// (`m20260829_000007_create_products_entity_version`). Rendering the
/// version number into the content therefore writes the key **inside the
/// payload it keys**: the row states which version it is twice, once where a
/// reader looks and once inside the bytes the digest is taken over. And
/// because the number moves on every publish by construction, that copy
/// moves the digest on every publish whether or not one content field
/// changed.
///
/// # What the two exclusions buy together
///
/// *The same content produces the same digest.* That property is what lets a
/// reader answer *"did the content change between version N and N+1"* by
/// comparing two rows' digests — the question §2's
/// `inst-fd-publish-reannounce` raises when it contemplates re-announcing
/// unchanged content, the question slice 06's `CatalogVersion` is built on,
/// and the one slice 10's restore drill asks of a pair of rows.
///
/// It belongs to **both** exclusions and to neither alone. An earlier
/// revision of this doc claimed it for the `updated_at` exclusion by itself,
/// and that claim was measurably false while `published_version` was still
/// on the roster: the digest moved on every publish regardless, so excluding
/// `updated_at` bought nothing on its own.
///
/// **The design set's §4.3 enumeration is owed both additions** — it should
/// name `updated_at` and `published_version` beside `internal_revision` as
/// columns the original enumeration missed. Until it does, this doc and its
/// Product twin are where the reading is recorded.
const SKU_VERSION_CONTENT_ROSTER: [&str; 8] = [
    "brand_scope",
    "created_at",
    "created_by",
    "product_id",
    "region_scope",
    "sku_code",
    "sku_id",
    "tenant_id",
];

/// The operands every step of a head act shares: who is acting, in which
/// tenant, and under which compiled scope.
///
/// Grouped for `crate::api::rest::RefusalAuditContext`'s own reason — the
/// three always travel together, and passing them loose would put two `Uuid`s
/// side by side at every call site for the compiler to happily transpose.
struct ActContext {
    /// Owning tenant, the caller's own.
    tenant_id: Uuid,
    /// The pseudonymous ref this act and its refusals are attributed to.
    actor_ref: Uuid,
    /// The `products_audit_log.action` token **every** refusal of this act is
    /// recorded under: [`PUBLISH_AUDIT_ACTION`] or
    /// [`DISCARD_AUDIT_ACTION`]. It travels in the context rather than being
    /// passed to each refusal helper because a door raises refusals from a
    /// dozen branches and a per-call argument is a per-call chance to write
    /// the wrong one; the door names it once, when it opens.
    audit_action: &'static str,
    /// The scope the authorization gate returned. Every read and every write
    /// below runs under exactly this one, never one rebuilt from
    /// `tenant_id` — [`crate::api::rest::skus`]'s own module doc gives the
    /// existence-leak reason.
    scope: AccessScope,
}

/// Resolve the actor ref and run the authorization gate for a head act.
///
/// The first two steps of both doors, in the order
/// [`create_sku`] establishes and for the reasons its own doc gives:
/// [`repo::resolve_actor_ref`] on its own transaction **ahead of** the gate,
/// so a denial has a ref to attribute its audit row to, and the denial itself
/// audited under the caller's tenant-scoped self access, since the gate is
/// what refused and there is no granted scope to reuse.
///
/// `authz_action` is the door's own (`sku x publish` for a publish,
/// `sku x write` for a discard): a discard is an ordinary head write, while a
/// publish is the act the catalog's `publish` permission exists to govern.
///
/// **`owner_tenant_id` is `Some(tenant_id)`, because both doors write.**
/// `crate::authz::access_scope` fixes the contract: a read passes `None` and
/// uses the compiled scope as its SQL filter, while a **write** passes the
/// tenant the row is written to and the function then asserts that tenant is
/// a member of the compiled scope, denying a cross-tenant target — a check
/// that is gated on the argument being `is_some()` and so is simply absent
/// when a write passes `None`. Both head acts move a row, so both take the
/// write shape, exactly as [`create_sku`] and `products::open_head_door` do.
/// The `.secure().scope_with(scope)` filter on the head write still keeps
/// another tenant's row out either way; what the assertion adds is the
/// audited `403 PERMISSION_DENIED` in place of a bare, unaudited `404`.
///
/// `audit_action` is a **separate** argument and deliberately not derived
/// from it: the two vocabularies coincide for a publish and diverge for a
/// discard, which gates on `write` and must still be *recorded* as
/// `discard`. Deriving one from the other would file every discard refusal
/// under `write`, which is the same class of lie this door has just stopped
/// telling by leaving `create` behind.
///
/// # Errors
///
/// The audited `403` on a denial, the unmapped 403/503 on an unreachable PDP
/// (`authz_error_to_canonical`), or the `500` a failed `actor_ref` mint
/// raises.
async fn open_act(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    sku_id: Uuid,
    authz_action: &'static str,
    audit_action: &'static str,
    now: DateTime<Utc>,
) -> Result<ActContext, CanonicalError> {
    let tenant_id = ctx.subject_tenant_id();
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(state, tenant_id, ctx.subject_id(), now)
            .await?;

    let scope = match crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::SKU,
        authz_action,
        /* owner_tenant_id */ Some(tenant_id),
        /* resource_id */ Some(sku_id),
        /* require_constraints */ true,
    )
    .await
    {
        Ok(scope) => scope,
        Err(crate::authz::AuthzError::Denied(reason)) => {
            let self_scope = AccessScope::for_tenant(tenant_id);
            return Err(crate::api::rest::audit_refusal_of_action_and_report(
                state,
                &self_scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: crate::authz::labels::SKU,
                    error_code: "PERMISSION_DENIED",
                },
                audit_action,
                minted(sku_id, None),
                SkuResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await);
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            return Err(authz_error_to_canonical(err, |reason| {
                SkuResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }));
        }
    };

    Ok(ActContext {
        tenant_id,
        actor_ref,
        audit_action,
        scope,
    })
}

/// The audit subject a head act names: the row's own id, which exists before
/// the act runs, unlike a create's [`RefusalSubject::Attempted`] natural key.
const fn minted(sku_id: Uuid, revision: Option<i64>) -> RefusalSubject {
    RefusalSubject::Minted {
        subject_id: sku_id,
        subject_revision: revision,
    }
}

/// [`audit_sku_refusal`]'s head-act twin: the same shared audit-then-answer
/// discipline, pinned to this door's `subject_kind`, to a
/// [`RefusalSubject::Minted`] subject and to **this act's own `action`
/// token**.
///
/// **Every refusal below goes through it**, on its own transaction, and an
/// unwritable audit row answers `AUDIT_UNAVAILABLE` 503 rather than the
/// domain refusal it would otherwise have reported — that contract is the
/// shared function's and is not re-implemented here.
///
/// It calls `crate::api::rest::audit_refusal_of_action_and_report`, not
/// `audit_refusal_and_report`. The latter delegates with the literal
/// `"create"`, so every publish and discard refusal this door raised used to
/// write a row saying the operator was refused at **create** — the
/// `error_code` and the subject were right and only the `action` label was a
/// lie, which is the worst shape for an operator reading
/// `products_audit_log` to catch. The create doors below still call
/// `audit_refusal_and_report` and still write `create`, so no existing row
/// changes meaning. The Product doors reached the same function first;
/// `products::audit_and_refuse` is the twin.
async fn audit_act_refusal(
    state: &ApiState,
    act: &ActContext,
    subject: RefusalSubject,
    domain_err: DomainError,
) -> CanonicalError {
    let error_code = domain_err.code();
    crate::api::rest::audit_refusal_of_action_and_report(
        state,
        &act.scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id: act.tenant_id,
            actor_ref: act.actor_ref,
            subject_kind: crate::authz::labels::SKU,
            error_code,
        },
        act.audit_action,
        subject,
        CanonicalError::from(domain_err),
    )
    .await
}

/// The `products_audit_log.action` token every **publish** refusal on this
/// door is recorded under. Named, not spelled at each call site, so the
/// trail is greppable by one string — `products::PUBLISH_AUDIT_ACTION` is
/// the Product door's identical constant, and the two tokens must stay
/// equal: an operator filtering the trail by `action = 'publish'` is asking
/// one question of both entity kinds.
const PUBLISH_AUDIT_ACTION: &str = "publish";

/// [`PUBLISH_AUDIT_ACTION`]'s discard twin. Note that the discard door gates
/// on `sku x write` and records `discard`: the authorization vocabulary and
/// the audit vocabulary are two different sets, and [`open_act`] takes them
/// as two arguments for that reason.
const DISCARD_AUDIT_ACTION: &str = "discard";

/// Read the head a head act is about, under the act's own granted scope.
///
/// The connection is checked out and dropped inside this function on purpose:
/// the mutation below needs the pool's connection, and holding this one
/// across it would deadlock a single-connection pool.
///
/// # Errors
///
/// The bare `404` [`sku_not_found`] builds on a miss — absent and
/// out-of-scope alike, the read door's own indistinguishable answer — or the
/// `500` a storage failure raises.
async fn load_head(
    state: &ApiState,
    act: &ActContext,
    sku_id: Uuid,
) -> Result<SkuRecord, CanonicalError> {
    let conn = state
        .db
        .conn()
        .map_err(|e| CanonicalError::internal(format!("bss-products: db conn: {e}")).create())?;
    repo::find_sku(&conn, &act.scope, act.tenant_id, sku_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .ok_or_else(|| sku_not_found(sku_id))
}

/// The `sku_code` a head must still carry to be publishable.
///
/// A [`ValidationRule`] rather than an `if` in the door, because
/// `inst-fd-publish-revalidate` re-runs **the pipeline**, and a check written
/// as an `if` is one slice 04 and 05 cannot register beside their own.
struct SkuCodeStillPresent;

impl ValidationRule<SkuRecord> for SkuCodeStillPresent {
    fn name(&self) -> &'static str {
        "inst-fd-publish-revalidate/sku_code"
    }

    fn phase(&self) -> Phase {
        Phase::Shape
    }

    fn evaluate(&self, subject: &SkuRecord, report: &mut ValidationReport) {
        if subject.sku_code.trim().is_empty() {
            report.violate(
                "INCOMPLETE_ENTITY",
                "sku_code",
                "sku_code is blank, so this entity is no longer publishable",
            );
        }
    }
}

/// The two stored scope columns must still parse under
/// [`ResolvedScope::parse`]'s own rule.
///
/// [`Phase::Identity`] because §4.2 files containment and reservation there,
/// and this is the operand the containment rule reads.
struct SkuScopeColumnsStillParse;

impl ValidationRule<SkuRecord> for SkuScopeColumnsStillParse {
    fn name(&self) -> &'static str {
        "inst-fd-publish-revalidate/scope-columns"
    }

    fn phase(&self) -> Phase {
        Phase::Identity
    }

    fn evaluate(&self, subject: &SkuRecord, report: &mut ValidationReport) {
        if ResolvedScope::parse(&subject.region_scope).is_err() {
            report.violate(
                "INCOMPLETE_ENTITY",
                "region_scope",
                "region_scope contains an empty value between separators",
            );
        }
        if ResolvedScope::parse(&subject.brand_scope).is_err() {
            report.violate(
                "INCOMPLETE_ENTITY",
                "brand_scope",
                "brand_scope contains an empty value between separators",
            );
        }
    }
}

/// The pipeline `inst-fd-publish-revalidate` re-runs at publish.
///
/// # This re-run is real, and it is not yet complete — who owns the rest
///
/// The instruction asks for the **full** pipeline: shape, state, identity and
/// *every registered validator for `→ published`*. What is here is the
/// Foundation's own share of it, and only that:
///
/// - **Shape** and **Identity** are the two rules above, over the head row as
///   it now stands rather than over a payload — which is the point of a
///   re-run: an entity that stopped being publishable since approval fails
///   closed rather than publishing stale.
/// - **State** is not registered as a rule and is not missing: it runs as
///   [`transition::check_head_write`] and [`transition::guard`] in the door's
///   own steps 3 and 5, and `repo::publish_sku_head` states the same rule a
///   second time in its `WHERE` clause. Registering a third copy here would
///   be a second answer to one question.
/// - **`RegisteredValidators` is empty, and that is a real gap, not a
///   passing phase.** The `→ published` validators the instruction names are
///   `04-lifecycle`'s and `05-governance`'s, and neither exists at this
///   commit; [`Phase::RegisteredValidators`] therefore admits everything. The
///   re-run is fail-closed over the rules that exist and silent over the ones
///   that do not, and no reading of this function should treat the phase's
///   emptiness as the entity having satisfied it.
/// - **`Idempotency`, `Precondition` and `GovernanceGate`** are phases the
///   door runs directly (the claim, the `If-Match`, the gate) rather than as
///   registered rules, for the same reason State is not registered.
fn publish_revalidation_pipeline() -> ValidationPipeline<SkuRecord> {
    ValidationPipeline::new()
        .with_rule(Box::new(SkuCodeStillPresent))
        .with_rule(Box::new(SkuScopeColumnsStillParse))
}

/// Turn a failing re-validation phase into the door's refusal.
///
/// `INCOMPLETE_ENTITY` rather than `VALIDATION`: `inst-fd-publish-revalidate`
/// names "`INCOMPLETE_ENTITY`/rule-named code" for an entity that stopped
/// being publishable, and `VALIDATION` would say the *request* was malformed
/// when the request was fine and the row was not.
fn revalidation_refusal(report: &ValidationReport) -> DomainError {
    let detail = report
        .violations()
        .iter()
        .map(|violation| format!("{}: {}", violation.subject, violation.detail))
        .collect::<Vec<_>>()
        .join("; ");
    DomainError::IncompleteEntity(format!("the entity is no longer publishable: {detail}"))
}

/// The `lifecycle_state` a publish leaves behind.
///
/// **This function and `repo::publish_sku_head`'s `CASE` expression are two
/// spellings of one rule and must agree**: `draft` becomes `published`, and
/// every other admitted state stands (`inst-fd-publish-freeze`, *"a
/// re-publish changes the version, never the state"*). They are kept apart
/// because the statement decides on the row image the write lands on while
/// this one decides what the door freezes and answers, and a door that
/// assumed `published` unconditionally would flip a `deprecated` head back —
/// a state change the transition door owns and the two-person ceremony
/// governs.
fn post_publish_state(from: LifecycleState) -> LifecycleState {
    if from == LifecycleState::Draft {
        LifecycleState::Published
    } else {
        from
    }
}

/// The head row **as this act leaves it** — the post-act image
/// `inst-fd-publish-freeze` and **P-D-33** require the freeze to be taken
/// over.
///
/// Both counters move by exactly one here, and `updated_at` becomes the
/// act's own instant, which is what the single head-row `UPDATE` below will
/// write.
///
/// What this image now decides is the frozen row's **key**:
/// [`freeze_for`] reads `published_version` off it, and `N + 1` is what
/// makes the head table's guard subquery find the frozen row when the
/// `UPDATE` a statement later asks for it. It no longer decides the frozen
/// **content**: every column this function moves is excluded from
/// [`SKU_VERSION_CONTENT_ROSTER`], so [`sku_version_content`] renders the
/// same bytes from either image. The two come apart again the moment a slice
/// makes a *content* column move inside the act.
fn post_publish_image(head: &SkuRecord, now: DateTime<Utc>) -> SkuRecord {
    SkuRecord {
        lifecycle_state: post_publish_state(head.lifecycle_state),
        internal_revision: head.internal_revision.saturating_add(1),
        published_version: head.published_version.saturating_add(1),
        updated_at: now,
        ..head.clone()
    }
}

/// The head row as a discard leaves it: `discarded`, one revision on, the
/// version counter untouched (`inst-fd-discard` admits the act only from
/// `published_version = 0`, and a discard publishes nothing).
fn post_discard_image(head: &SkuRecord, now: DateTime<Utc>) -> SkuRecord {
    SkuRecord {
        lifecycle_state: LifecycleState::Discarded,
        internal_revision: head.internal_revision.saturating_add(1),
        updated_at: now,
        ..head.clone()
    }
}

/// One timestamp, rendered as §4.3 pins them: `RFC 3339`, `UTC`, microsecond
/// precision.
///
/// `crate::domain::canonical` deliberately converts no instant — its own doc
/// says the caller renders its instant into a string field, so the precision
/// decision stays with whoever owns the column. This is that decision for
/// `products_sku`'s `created_at`, the one timestamp
/// [`SKU_VERSION_CONTENT_ROSTER`] still names — `updated_at` left the roster
/// with P-D-35's criterion, see that constant's own doc. It is spelled
/// identically in `products::render_instant`, and the two copies are owed a
/// single home in `crate::domain::canonical`, the module that owns the
/// rendering rules.
fn render_instant(instant: DateTime<Utc>) -> String {
    instant.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// The content a frozen SKU version row carries, as the value
/// [`SKU_VERSION_CONTENT_ROSTER`] is the complete set of.
///
/// Every name in the roster is written here, and the roster is what makes a
/// name this function forgot render as `null` rather than vanish — see
/// [`canonical::Absence::Null`]'s own doc for why the roster travels with the
/// mode instead of being inferred from the value. `skus_tests::
/// the_sku_content_builder_writes_exactly_the_roster` is what fails if it
/// does forget one, because `Absence::Null` alone would make the omission
/// silent.
///
/// # The pre-act/post-act hazard is structurally absent here
///
/// `inst-fd-publish-freeze` (**P-D-33**) takes the freeze over the image the
/// act leaves behind, and [`post_publish_image`] is what supplies one. On
/// the **roster's** fields, though, that image and the pre-act head are
/// equal: `published_version`, `internal_revision`, `lifecycle_state` and
/// `updated_at` are the four columns a publish moves, and all four are
/// excluded ([`SKU_VERSION_CONTENT_ROSTER`]'s own doc argues each). No
/// caller can therefore freeze a pre-act value under a post-act key — the
/// image decides the key in [`freeze_for`] and nothing else. The hazard
/// returns the moment a slice makes a *content* column move inside the act.
fn sku_version_content(image: &SkuRecord) -> JsonValue {
    let mut fields = JsonMap::new();
    fields.insert(
        "sku_id".to_owned(),
        JsonValue::String(image.sku_id.to_string()),
    );
    fields.insert(
        "tenant_id".to_owned(),
        JsonValue::String(image.tenant_id.to_string()),
    );
    fields.insert(
        "product_id".to_owned(),
        JsonValue::String(image.product_id.to_string()),
    );
    fields.insert(
        "sku_code".to_owned(),
        JsonValue::String(image.sku_code.clone()),
    );
    fields.insert(
        "region_scope".to_owned(),
        JsonValue::String(image.region_scope.clone()),
    );
    fields.insert(
        "brand_scope".to_owned(),
        JsonValue::String(image.brand_scope.clone()),
    );
    fields.insert(
        "created_by".to_owned(),
        JsonValue::String(image.created_by.clone()),
    );
    fields.insert(
        "created_at".to_owned(),
        JsonValue::String(render_instant(image.created_at)),
    );
    JsonValue::Object(fields)
}

/// Build the version row this publish freezes, digest and all.
///
/// The rendering and the `SHA-256` over it are computed **here**, in the
/// door, and handed to `repo::insert_entity_version` as inputs — that
/// function's own doc states why it re-renders nothing on the way to storage.
fn freeze_for(
    image: &SkuRecord,
    actor_ref: Uuid,
    approval_ref: Option<Uuid>,
    now: DateTime<Utc>,
) -> NewEntityVersion {
    let content = canonical::canonical_rendering(
        &sku_version_content(image),
        canonical::Absence::Null {
            roster: &SKU_VERSION_CONTENT_ROSTER,
        },
    );
    let content_digest = canonical::content_digest(&content);
    NewEntityVersion {
        tenant_id: image.tenant_id,
        entity_kind: VersionedEntityKind::Sku,
        entity_id: image.sku_id,
        published_version: image.published_version,
        content,
        content_digest,
        digest_version: canonical::DIGEST_VERSION,
        approval_ref,
        actor_ref,
        published_at: now,
    }
}

/// The idempotency digest of a **bodiless** head act (`crate::domain::
/// idempotency`, **P-D-34**).
///
/// Neither of this file's head doors carries a request body, so the parsed
/// request's named field set is **empty** and every request under one
/// endpoint renders identically — the digest is a constant, and deliberately
/// so.
///
/// # The precondition is not an operand, and that is the whole point
///
/// `inst-fd-idem-hash` (**P-D-34**) takes the hash over *"the body's present
/// fields and not the precondition"*, and states the failure a door that
/// folded `If-Match` in would ship: *"hashing the precondition in would
/// answer that retry `IDEMPOTENCY_CONFLICT` instead of running it"*. The
/// reachable case is the ordinary one this store exists for — a publish
/// commits, the response is lost, the client re-reads the head to recover
/// and so holds a **fresher** `ETag`, and retries under the same key. With
/// the revision hashed in, the two digests differ and the store answers
/// `409` for the whole retention window instead of replaying the stored
/// `200`. `crate::domain::idempotency_tests::
/// folding_a_precondition_into_the_operand_would_change_the_digest`
/// demonstrates the hazard on the digest itself, and `skus_tests::
/// a_retry_under_a_different_if_match_replays_the_stored_answer` holds this
/// door closed against it.
///
/// `sku_id` is out for a second reason rather than the same one: it is
/// already the key's discriminator, through P-D-42's concrete-path
/// `endpoint` ([`publish_endpoint`]), so hashing it into the payload too
/// would be hashing a value the key is keyed by.
///
/// `products::bodiless_payload_digest` is the Product door's identical
/// function. The two are one rule and must stay equal.
fn bodiless_payload_digest() -> Vec<u8> {
    idempotency::payload_digest(&JsonValue::Object(JsonMap::new()))
}

/// What a head act's mutation transaction produced.
///
/// [`crate::api::rest::CreateOutcome`]'s two success shapes, restated
/// locally because its `Created` arm is named for a door that mints a row
/// and this one does not: a publish and a discard both answer `200` over a
/// row that already existed.
///
/// It has no refusal arm, and that is the point of [`HeadActError`]: a
/// refusal decided inside the transaction has to roll it back, and
/// `transaction_with_retry` commits on `Ok`.
enum MutationOutcome {
    /// The act ran: the revision the `ETag` is minted from, and the body
    /// rendered and stored inside the transaction that wrote it.
    Applied {
        /// The committed `internal_revision`.
        internal_revision: i64,
        /// The response body, and the stored idempotency answer.
        body: JsonValue,
    },
    /// A stored answer was replayed; nothing was written.
    Replay {
        /// The stored status.
        status: i32,
        /// The stored body.
        body: JsonValue,
    },
}

/// Why a head act's transaction ended without applying — a **typed** control
/// signal, not a marker string.
///
/// # Every variant rolls the transaction back, and that is the point
///
/// `Db::transaction_with_retry` **commits on `Ok`**. The create doors above
/// can return their idempotency refusal as an `Ok` variant, because the
/// claim `INSERT` is their first statement and committing an empty
/// transaction is harmless. Neither head door can: `inst-fd-publish-txn`
/// forces the freeze **before** the head-row `UPDATE`, so by the time
/// `repo::publish_sku_head` can answer [`repo::HeadWrite::Unmatched`] a
/// `products_entity_version` row is already written on this transaction, and
/// committing it would leave a frozen version for a publish that never
/// happened — one the head-row guard would then accept as the missing
/// prerequisite for a later bump nobody authorized.
///
/// # Why this is an enum and not a sentinel in `DbErr::Custom`
///
/// An earlier shape stuffed a marker string into `DbErr::Custom` and matched
/// it back out of the error's rendered text. Two things were wrong with it,
/// and both are closed here. A refusal was **indistinguishable from a
/// storage failure in the type system**, so a door reading the error had to
/// re-derive which one it held from a substring. And the marker was handed
/// to `transaction_with_retry`'s contention classifier with no arm saying
/// *never retry this*, so a decided answer sat one classifier change away
/// from being re-attempted. [`head_act_contention_db_err`] now answers
/// `None` for every variant but [`Self::Db`], which is that arm, stated in
/// the type. `products::HeadActError` is the Product door's identical enum;
/// the two are owed a single home, and `crate::api::rest` is where it would
/// go — that module is a neighbour's in this phase, so the copy stays local
/// and the sharing is owed.
enum HeadActError {
    /// A domain refusal decided inside the transaction: the idempotency
    /// phase's, terminality, the precondition, the re-validation re-run, the
    /// edge, the gate, or the head-row write's own `Unmatched` once
    /// [`classify_unmatched_head_write`] has read which of its several
    /// meanings applied.
    Refused(DomainError),
    /// The head vanished from the caller's scope between the door's own read
    /// and its write. Answered as this module's bare `404`, unaudited, for
    /// [`load_head`]'s stated reason.
    Vanished,
    /// A storage failure, including one the contention classifier may decide
    /// to retry.
    Db(DbError),
}

impl From<DbError> for HeadActError {
    fn from(error: DbError) -> Self {
        Self::Db(error)
    }
}

impl HeadActError {
    /// Wrap any error whose text is all this door needs.
    ///
    /// `repo::RepoError` and `events::EventsError` are each a storage or
    /// infrastructure failure of this act's own mutation, and neither can be
    /// classified by `transaction_with_retry` unless it arrives as a
    /// `DbErr`; each becomes [`Self::Db`] carrying the text that named it.
    fn from_storage(error: &impl core::fmt::Display) -> Self {
        Self::Db(DbError::Sea(DbErr::Custom(error.to_string())))
    }
}

/// The `DbErr` inside a [`HeadActError`], for `transaction_with_retry`'s
/// contention classifier.
///
/// Only [`HeadActError::Db`] can carry one. A refusal and a vanished head
/// answer `None` deliberately: both are decided answers, and retrying either
/// would re-run an act whose outcome this door has already established.
fn head_act_contention_db_err(error: &HeadActError) -> Option<&DbErr> {
    match error {
        HeadActError::Db(db_error) => contention_db_err(db_error),
        HeadActError::Refused(_) | HeadActError::Vanished => None,
    }
}

/// The operands one head act's transaction runs on, owned so the
/// `transaction_with_retry` closure can hold them.
///
/// A copy of the door's state rather than a borrow of it, and the retry
/// helper is the reason: its body is
/// `for<'a> FnMut(&'a DbTx<'a>) -> Pin<Box<dyn Future + Send + 'a>>`, and the
/// higher-ranked `'a` cannot be bounded by any lifetime the caller holds — a
/// borrow of the door's state simply does not typecheck there. [`Clone`] for
/// the same helper's other reason: the body is `FnMut` and may be re-entered
/// on a retryable contention failure, so every attempt takes its own copy and
/// none can consume what the next one needs. Every value is
/// attempt-independent — `now` was stamped before the first attempt, and the
/// claim's window with it.
#[derive(Clone)]
struct HeadActInputs {
    /// The compiled scope every read and write of the act runs under.
    scope: AccessScope,
    /// Owning tenant.
    tenant_id: Uuid,
    /// The head being acted on.
    sku_id: Uuid,
    /// The pseudonymous ref the frozen version row attributes the publish to.
    actor_ref: Uuid,
    /// The revision the caller pinned, as the head-row filter compares it
    /// (**P-D-33**) — never a value this door re-read.
    expected: i64,
    /// The act's instant, stamped once before the first attempt.
    now: DateTime<Utc>,
    /// The claim to take as the transaction's first statement, or `None`
    /// where the request carried no key (P-D-34's skip).
    claim: Option<IdempotencyClaimInput>,
}

/// Take the act's idempotency claim, if it carries one, on the mutation's own
/// runner (**P-D-42**) — the first statement of every head act's
/// transaction.
///
/// `Ok(None)` means proceed; `Ok(Some(outcome))` is a replay to serve with
/// nothing executed; an `Err` is a refusal, and it rolls the transaction back
/// rather than committing an empty one — see [`HeadActError`].
///
/// # Errors
///
/// [`HeadActError::Refused`] on `IDEMPOTENCY_CONFLICT` or
/// `IDEMPOTENCY_KEY_IN_FLIGHT`, [`HeadActError::Db`] on a storage failure.
async fn claim_for_head_act(
    runner: &impl toolkit_db::secure::DBRunner,
    inputs: &HeadActInputs,
) -> Result<Option<MutationOutcome>, HeadActError> {
    let Some(input) = inputs.claim.as_ref() else {
        return Ok(None);
    };
    match claim_idempotency(runner, &inputs.scope, inputs.tenant_id, input)
        .await
        .map_err(|e| HeadActError::from_storage(&e))?
    {
        ClaimVerdict::Proceed => Ok(None),
        ClaimVerdict::Replay { status, body } => Ok(Some(MutationOutcome::Replay { status, body })),
        ClaimVerdict::Refused(refusal) => Err(HeadActError::Refused(refusal)),
    }
}

/// Whether this act fires the approval-invalidation hook, **read off
/// [`transition::guard`]'s own answer** rather than decided at the call site
/// (`inst-fd-transition-bump`: *"Every transition bumps `internal_revision`
/// and fires the approval-invalidation hook, except a transition that
/// consumes an approval in the same transaction, which bumps once with no
/// hook"*).
///
/// [`transition::TransitionDecision::NotATransition`] reaches this from one
/// place only: a **re-publish**, where the head is already `published` or
/// `deprecated` and the act moves the version rather than the state. The
/// answer is [`transition::ApprovalInvalidation::Skip`], the same one the
/// gated `draft -> published` edge carries, and on the identical argument:
/// the instruction's exception is *a transition that consumes an approval in
/// the same transaction*, because *a hook firing against the record the act
/// is consuming has no defined ordering* — and a re-publish is exactly such
/// an act, this transaction being the one that spends the approval for
/// version N+1. Answering `Fire` here would, the moment slice 05 supplies a
/// real hook, invalidate the very `ApprovalRecord` the re-publish is
/// spending. `products::head_act_invalidation` is the Product door's
/// identical function and answers the same constant; the two are one rule
/// and `crate::domain::transition` — which already houses `ADMITTED_EDGES`
/// and `GATED_EDGES` — is where the single copy belongs. That module is not
/// open in this fix.
///
/// A discard cannot reach that arm at all: the only same-value discard is
/// `discarded -> discarded`, which [`transition::guard`] has already refused
/// as terminal.
const fn head_act_invalidation(decision: TransitionDecision) -> ApprovalInvalidation {
    match decision {
        TransitionDecision::Transition(effects) => effects.invalidation,
        TransitionDecision::NotATransition => ApprovalInvalidation::Skip,
    }
}

/// Fire the approval-invalidation hook where the transition floor says this
/// act's edge fires one, inside the act's own transaction.
///
/// The hook runs on the transaction rather than after it because
/// [`transition::ApprovalInvalidationHook`]'s own contract says so: a failure
/// fails the transition rather than leaving an approval standing against a
/// head that has moved. The host is [`transition::NoApprovalStoreHook`] until
/// slice 05 supplies a record store — a no-op that **succeeds**, because
/// there is no store and therefore no record that could be stale.
///
/// # Errors
///
/// [`HeadActError::Refused`]: a hook failure is the domain's own refusal of
/// the transition, and it rolls the act back like every other refusal decided
/// inside the transaction.
fn fire_invalidation_hook(
    inputs: &HeadActInputs,
    invalidation: ApprovalInvalidation,
) -> Result<(), HeadActError> {
    if invalidation == ApprovalInvalidation::Fire {
        transition::NoApprovalStoreHook
            .invalidate(EntityRef {
                tenant_id: inputs.tenant_id,
                entity_kind: EntityKind::Sku,
                entity_id: inputs.sku_id,
            })
            .map_err(HeadActError::Refused)?;
    }
    Ok(())
}

/// Enqueue the act's event and — where the request carried a key — store the
/// rendered answer, on the act's own transaction. The tail both head acts
/// share.
///
/// `published_version` is `Some` on a publish and `None` on a discard, and it
/// is the operand that decides the body shape: §4.5 puts `publishedVersion`
/// on `SkuPublished` beyond the shared core, while a discard writes no
/// version row and moves no version counter, so there is no number it could
/// truthfully carry.
///
/// The body is rendered here and answered from here, so a later replay
/// reproduces this response rather than a lookalike re-rendered by a second
/// call site.
///
/// # Errors
///
/// [`HeadActError::Db`] on a failed enqueue, a failed rendering or a failed
/// answer write.
async fn announce_and_answer(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    outbox: &toolkit_db::outbox::Outbox,
    inputs: &HeadActInputs,
    image: &SkuRecord,
    announcement: (&'static str, Option<i64>),
) -> Result<MutationOutcome, HeadActError> {
    let (payload_type, published_version) = announcement;
    let core = events::EventBodyCore {
        tenant_id: inputs.tenant_id,
        entity_kind: events::EntityKind::Sku.as_str(),
        entity_id: inputs.sku_id,
        internal_revision: image.internal_revision,
        lifecycle_state: image.lifecycle_state.as_str(),
    };
    match published_version {
        Some(version) => {
            events::enqueue_published(outbox, runner, inputs.sku_id, payload_type, &core, version)
                .await
        }
        None => events::enqueue(outbox, runner, inputs.sku_id, payload_type, &core).await,
    }
    .map_err(|e| {
        HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
            "enqueue {payload_type}: {e}"
        ))))
    })?;

    let body = serde_json::to_value(SkuView::from(image.clone())).map_err(|e| {
        HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
            "bss-products: render the head act's answer: {e}"
        ))))
    })?;

    if let Some(input) = inputs.claim.as_ref() {
        record_idempotency_answer(
            runner,
            &inputs.scope,
            inputs.tenant_id,
            input,
            ACT_RESPONSE_STATUS,
            &body,
        )
        .await
        .map_err(|e| HeadActError::from_storage(&e))?;
    }

    Ok(MutationOutcome::Applied {
        internal_revision: image.internal_revision,
        body,
    })
}

/// Which refusal a zero-row head write actually was, re-read under the act's
/// own transaction.
///
/// [`classify_unmatched_head_write`] decides the message; this reads the row
/// it decides from, and turns a head that has left the caller's scope
/// entirely into [`HeadActError::Vanished`] rather than into a refusal naming
/// a row nothing measured.
async fn classify_unmatched(
    runner: &impl toolkit_db::secure::DBRunner,
    inputs: &HeadActInputs,
    requested_state: LifecycleState,
) -> HeadActError {
    match repo::find_sku(runner, &inputs.scope, inputs.tenant_id, inputs.sku_id).await {
        Ok(Some(head)) => HeadActError::Refused(classify_unmatched_head_write(
            &head,
            InternalRevision::new(inputs.expected),
            requested_state,
        )),
        Ok(None) => HeadActError::Vanished,
        Err(error) => HeadActError::from_storage(&error),
    }
}

/// The publish act itself, **every phase of it on the mutation's own
/// transaction** and in the pipeline's own phase order
/// (`crate::domain::validation::Phase`): the idempotency claim, terminality,
/// the precondition, the re-validation re-run, the edge, the governance
/// gate, then the writes.
///
/// # Why every phase is in here, and not half of them outside
///
/// `Phase::Idempotency` runs **before** `Phase::Precondition`, and P-D-42
/// puts the claim `INSERT` inside the guarded mutation's transaction.
/// Together they put every later phase inside that transaction too, and the
/// consequence is functional rather than stylistic: a client whose publish
/// committed and whose response was lost retries under the key it still
/// holds, against a head a neighbour may since have deprecated and retired.
/// A door that judged terminality, or the precondition, or the gate, before
/// the claim would refuse that retry `ENTITY_TERMINAL` or `STALE_REVISION`
/// and would never reach the stored answer — leaving the idempotency store
/// inert at exactly the door it exists for.
/// `skus_tests::a_retried_publish_replays_after_the_head_has_gone_terminal`
/// is what holds this closed; it answers `409 ENTITY_TERMINAL` against the
/// other order.
///
/// # The write order is forced, not chosen
///
/// §4.2's head-row guard admits a `published_version` bump **only where the
/// matching frozen row already exists**, so the freeze has to precede the
/// bump and both have to be on this one transaction. Then exactly **one**
/// head-row `UPDATE`, which carries the version bump, the revision bump, the
/// edge and `updated_at` together: the guard bumps `internal_revision` on
/// every admitted `UPDATE` without exception, so a second statement would
/// move it twice for one act and the `ETag` a client holds would skip a value
/// the door never returned (`inst-fd-publish-bump`).
///
/// # The gate runs in `Gate` mode, always
///
/// `inst-fd-gate-mode`, and the owner's call of 2026-08-27: the mode is a
/// literal here and reachable from nowhere else — no request field, header or
/// query parameter selects [`GateMode::PreAuthorized`]. The *host* is a
/// parameter, for [`publish_sku_gated`]'s stated reason. A refusal is
/// `APPROVAL_REQUIRED` and writes nothing (`inst-fd-gate-rejection`); a host
/// that could not **reach** an answer is not a refusal and must not be
/// reported as one, which is why [`GovernanceGate::evaluate`]'s `Err` becomes
/// a bare `500` while `into_authorization`'s becomes the ceremony's refusal.
///
/// # What the verdict carries that this door drops on the floor
///
/// `crate::domain::governance::GateAuthorization::uncomposed_bundle_override`
/// is **not read here at all** — nothing in this file binds it, and
/// `GateAuthorization` is not even imported. That is not an oversight to
/// tidy: it is §4.2's `composition_pending` operand and `products_sku` has no
/// such column at this commit, so there is nowhere for a reader to put it.
/// The column is **this slice's** to add — §1.5's **In** list names *"the
/// `PublishDoor`'s `composition_pending` write"* and leaves slice 06 only the
/// composition semantics — so this is an unpaid debt of this phase, not a
/// later slice's arrival; its clause joins `repo::publish_sku_head`'s
/// single `UPDATE` when it lands. The accessor's only reader today is
/// `governance_tests`. `approval_ref` is the one accessor this act does read,
/// and it goes to `products_entity_version.approval_ref` through
/// [`freeze_for`].
///
/// # Errors
///
/// [`HeadActError::Refused`] for every domain refusal above, each rolled back
/// and audited by [`answer_head_act`]; [`HeadActError::Vanished`] where the
/// head left the caller's scope; [`HeadActError::Db`] on storage or on an
/// unreachable gate host.
async fn run_publish(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    inputs: &HeadActInputs,
    gate: &(dyn GovernanceGate + Send + Sync),
    outbox: &toolkit_db::outbox::Outbox,
) -> Result<MutationOutcome, HeadActError> {
    // -- Phase 1, idempotency: the claim, and the replay that ends the act
    // before any other phase is judged. --
    if let Some(replay) = claim_for_head_act(runner, inputs).await? {
        return Ok(replay);
    }

    // The head as it stands **under the write**. A miss here is the head
    // vanishing from the caller's scope between the door's own read and this
    // one, and it answers the same bare `404`.
    let head = repo::find_sku(runner, &inputs.scope, inputs.tenant_id, inputs.sku_id)
        .await
        .map_err(|e| HeadActError::from_storage(&e))?
        .ok_or(HeadActError::Vanished)?;

    // -- Terminality, which reaches every head write and not only a
    // transition (`inst-fd-terminal`, P-D-25 widened by P-D-32). Asked
    // directly rather than left to `transition::guard` below, because a
    // re-publish takes no edge at all and an edge-keyed check would let
    // exactly this write through. --
    transition::check_head_write(head.lifecycle_state).map_err(HeadActError::Refused)?;

    // -- Phase 2, the precondition (P-D-33). `repo::publish_sku_head` carries
    // the same comparison in its own filter and that copy is what decides
    // whether the write lands; this one decides whether the gate is asked at
    // all, an approval being usable only against the revision it pinned. --
    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    // -- Phases 3 to 5, the re-validation re-run
    // (`inst-fd-publish-revalidate`), over the head as it now stands. --
    if let Some((_phase, report)) = publish_revalidation_pipeline().run(&head) {
        return Err(HeadActError::Refused(revalidation_refusal(&report)));
    }

    // -- The edge, and what the floor says it costs. `post_publish_state`
    // decides the `to` side from the row image, the same way the head-row
    // `UPDATE`'s own `CASE` does. --
    let target = post_publish_state(head.lifecycle_state);
    let decision =
        transition::guard(head.lifecycle_state, target).map_err(HeadActError::Refused)?;

    // -- Phase 7, the governance gate. --
    let verdict = gate
        .evaluate(
            EntityRef {
                tenant_id: inputs.tenant_id,
                entity_kind: EntityKind::Sku,
                entity_id: inputs.sku_id,
            },
            InternalRevision::new(inputs.expected),
            GateMode::Gate,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    let authorization = verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    let image = post_publish_image(&head, inputs.now);

    // -- a. Freeze the post-act image, at `published_version + 1`. --
    repo::insert_entity_version(
        runner,
        &inputs.scope,
        freeze_for(
            &image,
            inputs.actor_ref,
            authorization.approval_ref().map(ApprovalId::get),
            inputs.now,
        ),
    )
    .await
    .map_err(|e| HeadActError::from_storage(&e))?;

    // -- b. Then exactly one head-row `UPDATE`. An `Err` rather than an
    // outcome, and the whole reason `HeadActError` exists: this rolls the
    // freeze back. --
    let written = repo::publish_sku_head(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.sku_id,
        inputs.expected,
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::from_storage(&e))?;
    if written == repo::HeadWrite::Unmatched {
        return Err(classify_unmatched(runner, inputs, target).await);
    }

    // -- c. The approval-invalidation hook, where the floor says this edge
    // fires one. It does not on `draft -> published`, nor on a re-publish;
    // the answer is read off `transition::guard` rather than hard-coded. --
    fire_invalidation_hook(inputs, head_act_invalidation(decision))?;

    // -- d. Then the event, and the stored answer. --
    announce_and_answer(
        runner,
        outbox,
        inputs,
        &image,
        (
            events::SKU_PUBLISHED_PAYLOAD_TYPE,
            Some(image.published_version),
        ),
    )
    .await
}

/// The discard act itself, on [`run_publish`]'s terms exactly: every phase on
/// the mutation's own transaction, the idempotency claim first, and the head
/// read under the write.
///
/// The phases a discard does **not** have are as deliberate as the ones it
/// does. There is **no re-validation re-run** — nothing is being published,
/// and `inst-fd-publish-revalidate` is the publish act's clause — and **no
/// governance gate**: `inst-fd-governance-gate` puts the gate on the publish
/// door, and discarding a never-published draft consumes and requires no
/// approval.
///
/// # The edge decision is the guard's, and the legality is the statement's
///
/// [`transition::guard`] judges `draft -> discarded`: it asks terminality
/// first, so a `retired` or `discarded` head is `ENTITY_TERMINAL` while a
/// `published` or `deprecated` one is `ILLEGAL_TRANSITION` — two refusals for
/// two different reasons, which a single "is this legal" test would have
/// collapsed into one. `published_version = 0` is checked here **and** in
/// `repo::discard_sku_head`'s own `WHERE` clause, and the second copy is the
/// load-bearing one: the statement judges the row image the write actually
/// lands on, which no prior read can.
///
/// # The reservations release by the same write
///
/// `uq_products_sku_code` is partial on `lifecycle_state <> 'discarded'`, so
/// the row leaves that index the moment this `UPDATE` commits and the
/// `skuCode` is free for the next holder. There is no release statement,
/// because there is nothing left to release. It is the **only** unique index
/// `products_sku` carries and there is no name index beside it — a SKU has no
/// `name` column at all, which is also why [`classify_sku_insert_conflict`]
/// answers a `bool` where the Product door needs a two-armed enum.
///
/// # Errors
///
/// As [`run_publish`], minus the gate's.
async fn run_discard(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    inputs: &HeadActInputs,
    outbox: &toolkit_db::outbox::Outbox,
) -> Result<MutationOutcome, HeadActError> {
    if let Some(replay) = claim_for_head_act(runner, inputs).await? {
        return Ok(replay);
    }

    let head = repo::find_sku(runner, &inputs.scope, inputs.tenant_id, inputs.sku_id)
        .await
        .map_err(|e| HeadActError::from_storage(&e))?
        .ok_or(HeadActError::Vanished)?;

    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    let decision = transition::guard(head.lifecycle_state, LifecycleState::Discarded)
        .map_err(HeadActError::Refused)?;

    if head.published_version != 0 {
        return Err(HeadActError::Refused(DomainError::IllegalTransition {
            from: head.lifecycle_state.as_str().to_owned(),
            to: LifecycleState::Discarded.as_str().to_owned(),
        }));
    }

    let image = post_discard_image(&head, inputs.now);

    let written = repo::discard_sku_head(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.sku_id,
        inputs.expected,
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::from_storage(&e))?;
    if written == repo::HeadWrite::Unmatched {
        return Err(classify_unmatched(runner, inputs, LifecycleState::Discarded).await);
    }

    // A discard consumes no approval, so the floor says its edge fires the
    // hook — read off `transition::guard`'s own answer, not decided here.
    fire_invalidation_hook(inputs, head_act_invalidation(decision))?;

    announce_and_answer(
        runner,
        outbox,
        inputs,
        &image,
        (events::SKU_DISCARDED_PAYLOAD_TYPE, None),
    )
    .await
}

/// Run [`run_publish`] on one retried transaction.
///
/// # `transaction_with_retry`, not a bare transaction
///
/// The claim `INSERT` is this transaction's first statement and is the gate
/// (P-D-42), which makes this the transaction concurrent duplicates
/// deliberately collide on; `DBProvider::transaction` has no contention
/// retry, so that collision would surface as a bare `500` instead of the
/// replay or the `409` the store promises. The body is safe to re-run: the
/// claim rolls back with everything after it, and nothing it writes is
/// derived from the attempt, `now` having been stamped before the first.
///
/// # The gate arrives as an `Arc`, and it has to
///
/// `transaction_with_retry`'s body is
/// `for<'a> FnMut(&'a DbTx<'a>) -> Pin<Box<dyn Future + Send + 'a>>`: the
/// higher-ranked `'a` cannot be bounded by any lifetime the caller holds, so
/// a borrowed `&impl GovernanceGate` cannot be captured by the closure at
/// all — the same constraint [`HeadActInputs`] exists for. An owned,
/// cheaply-cloned handle can be, which is why the host travels as
/// `Arc<dyn GovernanceGate + Send + Sync>` from the handler down. It is the
/// same shape `products::publish_in_one_transaction` carries, for the same
/// reason.
///
/// # Errors
///
/// See [`run_publish`].
async fn publish_in_one_transaction(
    state: &ApiState,
    inputs: &HeadActInputs,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<MutationOutcome, HeadActError> {
    let outbox = Arc::clone(&state.outbox);
    let gate = Arc::clone(gate);
    let inputs = inputs.clone();
    state
        .db
        .db()
        .transaction_with_retry::<MutationOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                // `FnMut`: every attempt takes its own copies, so a retried
                // attempt never finds an input the previous one consumed.
                let outbox = Arc::clone(&outbox);
                let gate = Arc::clone(&gate);
                let inputs = inputs.clone();
                Box::pin(async move { run_publish(tx, &inputs, gate.as_ref(), &outbox).await })
            },
        )
        .await
}

/// [`publish_in_one_transaction`]'s discard twin, on the same terms and with
/// no gate to carry.
///
/// # Errors
///
/// See [`run_discard`].
async fn discard_in_one_transaction(
    state: &ApiState,
    inputs: &HeadActInputs,
) -> Result<MutationOutcome, HeadActError> {
    let outbox = Arc::clone(&state.outbox);
    let inputs = inputs.clone();
    state
        .db
        .db()
        .transaction_with_retry::<MutationOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                let outbox = Arc::clone(&outbox);
                let inputs = inputs.clone();
                Box::pin(async move { run_discard(tx, &inputs, &outbox).await })
            },
        )
        .await
}

/// Turn a head act's outcome into the door's response, auditing every
/// refusal.
///
/// Shared by both doors, because the five outcomes read identically on each:
/// a success answers `200` with the `ETag` the act committed; a replay
/// executes nothing and audits nothing (it is not a refusal, and the act it
/// reproduces was audited — or, being a success, deliberately not, P-D-21 —
/// when it originally ran); every refusal takes the same audit-then-answer
/// discipline; a vanished head is the bare, unaudited `404`; and an unrelated
/// storage failure is a `500`.
///
/// `subject_revision` is the revision the head carried when the door read it,
/// so an operator reading the trail can see which image of the head was
/// refused. It is the door's own read rather than a fresh one: a refusal
/// decided inside the transaction rolled that transaction back, and a second
/// read taken afterwards would report an image nothing judged.
///
/// # Errors
///
/// The audited refusal, the `404`, or the `500` a storage failure raises.
async fn answer_head_act(
    state: &ApiState,
    act: &ActContext,
    sku_id: Uuid,
    subject_revision: i64,
    outcome: Result<MutationOutcome, HeadActError>,
) -> Result<Response, CanonicalError> {
    match outcome {
        Ok(MutationOutcome::Applied {
            internal_revision,
            body,
        }) => {
            let tag = preconditions::etag(InternalRevision::new(internal_revision));
            Ok((ACT_RESPONSE_STATUS, [(ETAG, tag)], Json(body)).into_response())
        }
        Ok(MutationOutcome::Replay { status, body }) => Ok(replay_response(status, body)),
        Err(HeadActError::Refused(domain_err)) => Err(audit_act_refusal(
            state,
            act,
            minted(sku_id, Some(subject_revision)),
            domain_err,
        )
        .await),
        Err(HeadActError::Vanished) => Err(sku_not_found(sku_id)),
        Err(HeadActError::Db(db_error)) => Err(repo_error_to_canonical(&RepoError::Db(
            db_error.to_string(),
        ))),
    }
}

/// Which refusal a [`repo::HeadWrite::Unmatched`] head write names, read off
/// the head as it now stands.
///
/// The order matters. A moved revision is `STALE_REVISION` first, because it
/// is both the commonest cause and the one the caller can act on; then
/// terminality, through [`transition::check_head_write`], so the answer names
/// the rule that actually refused rather than the edge; then the edge itself.
/// The last arm is reachable where the row moved between the act's own checks
/// and the write, which is exactly the read-then-write race the statement's
/// own filter exists to close.
fn classify_unmatched_head_write(
    head: &SkuRecord,
    expected: InternalRevision,
    requested_state: LifecycleState,
) -> DomainError {
    if head.internal_revision != expected.get() {
        return DomainError::StaleRevision {
            expected: expected.get(),
            found: head.internal_revision,
        };
    }
    if let Err(terminal) = transition::check_head_write(head.lifecycle_state) {
        return terminal;
    }
    match transition::guard(head.lifecycle_state, requested_state) {
        Err(refusal) => refusal,
        Ok(_) => DomainError::IllegalTransition {
            from: head.lifecycle_state.as_str().to_owned(),
            to: requested_state.as_str().to_owned(),
        },
    }
}

/// Read the `Idempotency-Key` header and build the claim both doors take.
///
/// An absent header is the **skip** (**P-D-34**), not a refusal; a header
/// present but unusable is `VALIDATION`, audited like every other refusal by
/// whichever door called this.
///
/// The digest is [`bodiless_payload_digest`] — a constant — and the two
/// operands that tell one act from another are already in the key: the
/// entity id through `endpoint`'s concrete path (**P-D-42**) and the
/// caller's own key beside it. The `If-Match` revision is deliberately **not**
/// an operand, and that constant's own doc gives `inst-fd-idem-hash`'s
/// wording for why.
///
/// # Errors
///
/// [`DomainError::Validation`] where the header is present but unusable, for
/// `crate::api::rest::idempotency_key` to name the reason.
fn build_claim(
    state: &ApiState,
    headers: &HeaderMap,
    endpoint: String,
    now: DateTime<Utc>,
) -> Result<Option<IdempotencyClaimInput>, DomainError> {
    let client_key = idempotency_key(headers)?;
    Ok(client_key.map(|key| {
        IdempotencyClaimInput::new(
            endpoint,
            key,
            bodiless_payload_digest(),
            now,
            state.idempotency_retention_hours,
        )
    }))
}

/// `POST /skus/{id}/publish`, with the governance gate host as an argument.
///
/// The handler [`publish_sku`] passes [`NoMaterialityPolicyGate`], which is
/// the only host that exists at this commit; the parameter is what lets a
/// test drive a refusing host, since the default one never refuses under
/// [`GateMode::Gate`] and an `APPROVAL_REQUIRED` path with no test would be
/// an untested one. It is **not** a seam for a caller: the mode stays a
/// literal in [`run_publish`], so no wire input reaches either the host or
/// the mode. It arrives as an `Arc<dyn ...>` for
/// [`publish_in_one_transaction`]'s stated reason.
///
/// # Order of operations
///
/// 1. `actor_ref` resolution and the `sku x publish` gate ([`open_act`]).
/// 2. The idempotency phase — the key off the header, and the bodiless digest
///    ([`build_claim`]). `Phase::Idempotency` is the pipeline's **first**, so
///    it is read before the precondition rather than after it; the claim
///    `INSERT` itself joins the mutation (**P-D-42**).
/// 3. `If-Match` (**P-D-33**), through [`preconditions::if_match`]: absent or
///    unreadable is `VALIDATION`, and a *stale* one is not judged here at
///    all — the comparison belongs under the write.
/// 4. The head read ([`load_head`]), whose only refusal is the bare `404`.
/// 5. The act ([`run_publish`], on [`publish_in_one_transaction`]): the
///    claim, terminality, the precondition, the re-validation re-run, the
///    edge, the gate, the freeze, one head-row `UPDATE`, `SkuPublished`, the
///    invalidation hook and the stored answer — one transaction.
/// 6. [`answer_head_act`].
///
/// # What this door does not build, and who owns each
///
/// - **The retirement re-announcement** (`inst-fd-publish-reannounce`,
///   **P-D-48**) needs a live retire intent to detect, and the
///   `ScheduledTransition` that carries one is `04-lifecycle`'s and does not
///   exist at this commit. Owed to **slice 04**.
/// - **The corrected bucket-ii argument** (`inst-fd-publish-correction`,
///   **P-D-41**) is supplied only by slice **07**'s `CorrectionDoor`, which
///   has no caller here to hand one in. When 07 lands, its value must ride
///   `repo::publish_sku_head`'s single `UPDATE` rather than a second
///   statement.
/// - **`composition_pending`** is not a column yet, so the uncomposed-bundle
///   override the gate verdict can carry has nowhere to be written. Owed to
///   slice **07**; see [`run_publish`]'s own doc.
/// - **Consuming the gate's `satisfied` record** (`inst-fd-publish-consume`)
///   has nothing to consume: [`NoMaterialityPolicyGate`] answers
///   `ApprovalDisposition::NoRecord`, and
///   `GateAuthorization::approval_to_consume` is `None` by construction.
///   The consume flip lands with slice **05**'s record store, in this same
///   transaction.
///
/// # Errors
///
/// Every refusal this door raises, each audited on its own transaction
/// through [`audit_act_refusal`], plus the `404` a miss answers and the `500`
/// a storage or gate-host failure raises.
async fn publish_sku_gated(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    headers: &HeaderMap,
    sku_id: Uuid,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<Response, CanonicalError> {
    let now = Utc::now();
    let act = open_act(
        state,
        enforcer,
        ctx,
        sku_id,
        crate::authz::actions::PUBLISH,
        PUBLISH_AUDIT_ACTION,
        now,
    )
    .await?;

    let claim = match build_claim(state, headers, publish_endpoint(sku_id), now) {
        Ok(claim) => claim,
        Err(refusal) => {
            return Err(audit_act_refusal(state, &act, minted(sku_id, None), refusal).await);
        }
    };
    let expected = match preconditions::if_match(headers) {
        Ok(expected) => expected,
        Err(refusal) => {
            return Err(audit_act_refusal(state, &act, minted(sku_id, None), refusal).await);
        }
    };

    let head = load_head(state, &act, sku_id).await?;
    let inputs = HeadActInputs {
        scope: act.scope.clone(),
        tenant_id: act.tenant_id,
        sku_id,
        actor_ref: act.actor_ref,
        // The caller's own `If-Match`, never the revision this door just
        // read: the head write is a compare-and-swap against exactly what the
        // caller pinned (P-D-33), and swapping in the freshly-read value
        // would make every publish unconditional and `STALE_REVISION`
        // unreachable.
        expected: expected.get(),
        now,
        claim,
    };

    let outcome = publish_in_one_transaction(state, &inputs, gate).await;
    answer_head_act(state, &act, sku_id, head.internal_revision, outcome).await
}

/// `POST /skus/{id}/publish`: freeze version N+1 and move the head, in one
/// transaction.
///
/// The thin `axum` shell over [`publish_sku_gated`], which carries the whole
/// pipeline and its reasoning. The only thing decided here is the governance
/// host and, with it, that no wire input can choose one:
/// [`NoMaterialityPolicyGate`] is passed as a literal, exactly as
/// [`GateMode::Gate`] is inside.
///
/// # Errors
///
/// See [`publish_sku_gated`].
async fn publish_sku(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
    Path(sku_id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    publish_sku_gated(
        &state,
        &enforcer,
        &ctx,
        &headers,
        sku_id,
        &(Arc::new(NoMaterialityPolicyGate) as Arc<dyn GovernanceGate + Send + Sync>),
    )
    .await
}

/// `POST /skus/{id}/discard`: discard a never-published draft
/// (`inst-fd-discard`).
///
/// The door's own steps are [`publish_sku_gated`]'s, minus the gate: the
/// `sku x write` grant, the key, the `If-Match`, the head read, then
/// [`run_discard`] on one transaction and [`answer_head_act`]. The legality
/// rule, the effects the guard reports and the reservation the write releases
/// are all argued at [`run_discard`].
///
/// # The grant is `write`, not `discard`
///
/// §2 narrates this door under `sku × discard`, and `crate::authz` declares
/// no `discard` action: `05-governance.md` §3.2's own RBAC catalog rows the
/// same door under `× write`, and that document's open-items list records the
/// contradiction as unresolved with the decision owned by that slice. This
/// door therefore gates on the action the normative catalog table currently
/// grants it, and still **records** `discard` — [`open_act`] takes the two
/// vocabularies as two arguments for exactly that reason.
///
/// # Errors
///
/// The audited `VALIDATION`, `STALE_REVISION`, `ENTITY_TERMINAL`,
/// `ILLEGAL_TRANSITION` or idempotency refusal, the `404` a miss answers, or
/// a `500` from storage.
async fn discard_sku(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
    Path(sku_id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let now = Utc::now();
    let act = open_act(
        &state,
        &enforcer,
        &ctx,
        sku_id,
        crate::authz::actions::WRITE,
        DISCARD_AUDIT_ACTION,
        now,
    )
    .await?;

    let claim = match build_claim(&state, &headers, discard_endpoint(sku_id), now) {
        Ok(claim) => claim,
        Err(refusal) => {
            return Err(audit_act_refusal(&state, &act, minted(sku_id, None), refusal).await);
        }
    };
    let expected = match preconditions::if_match(&headers) {
        Ok(expected) => expected,
        Err(refusal) => {
            return Err(audit_act_refusal(&state, &act, minted(sku_id, None), refusal).await);
        }
    };

    let head = load_head(&state, &act, sku_id).await?;
    let inputs = HeadActInputs {
        scope: act.scope.clone(),
        tenant_id: act.tenant_id,
        sku_id,
        actor_ref: act.actor_ref,
        // The caller's own `If-Match`, for the reason `publish_sku_gated`
        // states at its own copy of this field.
        expected: expected.get(),
        now,
        claim,
    };

    let outcome = discard_in_one_transaction(&state, &inputs).await;
    answer_head_act(&state, &act, sku_id, head.internal_revision, outcome).await
}

#[cfg(test)]
#[path = "skus_tests.rs"]
mod skus_tests;
