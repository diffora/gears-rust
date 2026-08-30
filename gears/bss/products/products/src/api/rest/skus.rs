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
//! @cpt-cf-bss-products-dod-read-door
//! @cpt-cf-bss-products-dod-create-doors
//! @cpt-cf-bss-products-dod-code-reservation
//! @cpt-cf-bss-products-dod-containment
//! @cpt-cf-bss-products-dod-idempotency-store

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::ETAG;
use axum::response::{IntoResponse, Response};
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
use crate::domain::concurrency::InternalRevision;
use crate::domain::containment::{
    EmptyScopeToken, ResolvedScope, ScopeContainment, ScopeInput, ScopePair,
};
use crate::domain::error::DomainError;
use crate::domain::idempotency;
use crate::domain::validation::ValidationReport;
use crate::infra::events;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{self, NewSku, RefusalSubject, SkuRecord};

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

    router.layer(Extension(state))
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
    let looks_like_a_unique_violation = lower.contains("unique constraint")
        || lower.contains("unique constraint failed")
        || lower.contains("duplicate key");
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

#[cfg(test)]
#[path = "skus_tests.rs"]
mod skus_tests;
