//! `GET /bss-products/v1/products/{id}` — the Product read door
//! (`cpt-cf-bss-products-dod-read-door`, `docs/features/foundation.md`
//! "Authoring head read") — and `POST /bss-products/v1/products`, the
//! Product create door (`cpt-cf-bss-products-dod-create-doors`,
//! `cpt-cf-bss-products-dod-code-reservation`, `docs/features/foundation.md`
//! "Create doors" and "Code reservation, atomic at insert").
//!
//! This was the first door this gear opened with a request body an author
//! actually reads, and the plan put it first in Phase 4 for a reason worth
//! restating here: without it an author who has not just written a Product
//! has no `ETag` to send back as `If-Match`, so every mutating door this
//! phase still owes — save, publish, discard — depends on this one existing
//! first. `POST /bss-products/v1/products`, this file's other door, is the
//! first of those; **`POST /bss-products/v1/skus` is the next slice's, not
//! this one's** — no containment or parent check is built here, and
//! `crate::domain::containment` (already present, unused by this file) is
//! where that logic lands.
//!
//! # Authorization
//!
//! [`get_product`] gates on `product x read` (`crate::authz::resource_types
//! ::PRODUCT`, `crate::authz::actions::READ`) with `owner_tenant_id = None` —
//! the PDP derives the scope from the subject rather than from anything the
//! caller sent — and `require_constraints = true`, so an unconstrained
//! *allow* fail-closes instead of exposing every tenant's catalog.
//! `resource_id` is pinned to the requested id, since this is a single-row
//! read. **The scope [`get_product`] hands to [`repo::find_product`] is the
//! one the PDP returned**, never one built from `ctx.subject_tenant_id()`
//! directly — a door that authorized against the compiled scope and then
//! read under a tenant-shaped one would leave the SQL-level filter
//! unenforced on the read that actually touches the row.
//!
//! [`create_product`] gates on `product x write`, `owner_tenant_id =
//! Some(tenant_id)` — the row is written **to** the caller's own tenant, so
//! `crate::authz::access_scope`'s cross-tenant membership assertion applies —
//! and the same `require_constraints = true`. See [`create_product`]'s own
//! doc for why this gate runs **after** [`repo::resolve_actor_ref`], not
//! before it.
//!
//! # The miss: absent and out-of-scope answer identically
//!
//! [`repo::find_product`] already documents why it answers `Ok(None)` both
//! when no such row exists and when one exists outside the caller's
//! authorized scope: a repository that told the two apart would let a caller
//! learn that a row belongs to another tenant just by asking for it, which
//! is the existence leak `docs/design/01-foundation.md` §3.3 keeps this
//! catalog closed against. [`get_product`] preserves that: `Ok(None)` maps to
//! one `not_found` build with the requested id and nothing else, never a
//! distinguishable code or detail for either cause.
//!
//! The status this module answers a miss with is a plain `404`, not a
//! `403`. `docs/design/01-foundation.md` §3.3 is explicit that **a `404` is
//! bare** (P-D-35): "a path segment is judged before the pipeline opens, so
//! no phase raises it ... no rule raises it at all." A row this door cannot
//! show the caller is exactly that kind of judgement — made before any
//! Foundation rule runs over the row's content — so it renders the same way
//! an unmatched path segment would: `404`, carrying a resource type and the
//! id the caller asked for, but no `DomainError`-style registry code.
//!
//! # The hit: `200` plus the `ETag`
//!
//! A found row answers `200` with [`ProductView`] and an `ETag` header built
//! from `internal_revision` (`preconditions::etag`), never
//! `published_version` — see `domain::concurrency`'s own doc for why the two
//! counters are not interchangeable here. Emitting the tag is not optional:
//! a caller who cannot obtain one cannot satisfy a mutating door's `If-Match`
//! precondition on the very next request.
//!
//! # The create door: what it writes, and what it deliberately does not
//!
//! [`create_product`] persists exactly one row and one outbox message, in one
//! transaction: the entity as `draft`, `published_version = 0`,
//! `internal_revision = 1`, and the `ProductCreated` event
//! (`crate::infra::events`, P-D-22, P-D-27) — **and nothing else**. Content
//! rows — 02's category assignments, 03's attribute values — belong to the
//! save door under P-D-46; slice 01's open item 11 asks whether a create
//! should ever admit them, and until that resolves this door writes none.
//!
//! A caller-supplied id is refused, not silently dropped: [`CreateProductRequest`]
//! carries an explicit, optional `id` field, and a present value is refused
//! `VALIDATION` naming `id` — see that type's own doc for why this reading of
//! `dod-create-doors` was chosen over the field-less one.
//!
//! **`brand_id` claims this door cannot check.** `dod-create-doors` and
//! P-D-33 both call for `brand_id` to be "validated against the caller's
//! brand claims", refusing `VALIDATION` when it names a brand the caller
//! does not hold. Measured against what is actually on the wire:
//! `toolkit_security::SecurityContext` (`libs/toolkit-security/src/
//! context.rs`) exposes exactly five things — `subject_id`, `subject_type`,
//! `subject_tenant_id`, `token_scopes`, `bearer_token` — and none of them is
//! a brand claim or a set of held brands. There is nothing on this door's
//! own `SecurityContext` to validate `brand_id` against, and inventing a
//! lookup (a table this slice's target paths exclude, a side-channel this
//! design set never named) would be answering a question the identity layer
//! has not yet been asked. This door therefore validates only that
//! `brand_id` is present and non-nil (`VALIDATION` otherwise) — the part of
//! `dod-create-doors` this door *can* discharge — and the "does the caller
//! hold this brand" half stays open, owed to whoever adds a brand claim to
//! `SecurityContext` (the token-issuer/identity owner, not this gear).
//!
//! **Telling `DUPLICATE_NAME` from `DUPLICATE_CODE` from an unrelated
//! storage failure.** `infra::storage::repo::insert_product`'s own doc
//! states plainly that Phase 1 left the conflict undifferentiated —
//! `RepoError::Db`, one shape for a scope failure, a `CHECK` violation or
//! either unique-index collision — because no caller existed yet to act on
//! a finer answer. This door is that caller, and `infra::storage::repo` is
//! outside this slice's target paths, so the repository's return type could
//! not be widened into a typed conflict here. [`classify_insert_conflict`]
//! instead reads the driver text the failure already carries, anchored to
//! the two facts the migration fixes and this file can cite without
//! guessing: the unique indexes' own names,
//! `uq_products_product_name`/`uq_products_product_code`
//! (`infra/storage/migrations/m20260829_000002_create_products_product.rs`),
//! and the columns each covers. Postgres's driver text names the constraint
//! by that literal index name; `SQLite`'s (the backend this file's own test
//! suite runs against) instead lists the covered columns —
//! `name_normalized` / `product_code` — so [`classify_insert_conflict`]
//! checks for either form. **The cost, stated plainly**: this is a
//! substring match over a driver's own error text, not a typed database
//! answer — a future driver upgrade that reworded either message, or a
//! third index added to the same table whose name also contains
//! `product_code` or `name_normalized`, could misclassify silently. The
//! correct long-term fix is exactly what this slice cannot build: a typed
//! conflict variant on [`RepoError`] itself, returned by `insert_product`
//! rather than inferred after the fact.
//!
//! # The event
//!
//! `ProductCreated`'s envelope is built by `crate::infra::events`, not here —
//! see that module's doc for the body core, the partition formula (P-D-22)
//! and the wiring gap this slice leaves in `gear.rs` for the running
//! [`toolkit_db::outbox::Outbox`] instance [`ApiState`] now carries.
//!
//! # Idempotency: what this door claims, and where
//!
//! A create carrying an `Idempotency-Key` claims
//! `(tenant_id, "/bss-products/v1/products", key)` for the digest of its own
//! parsed body ([`payload_digest`]), and **the claim `INSERT` runs inside the
//! mutation's transaction** ([`insert_product_with_event`], P-D-42) so a
//! rollback frees the key. A create **without** the header skips the phase
//! and creates normally (P-D-34) — it is a skip, not a refusal, and a later
//! edit must not invert it. `endpoint` is the concrete resource path, never
//! a route template ([`CREATE_ENDPOINT`]).
//!
//! The three outcomes: a fresh claim proceeds; a stored answer whose digest
//! matches is replayed with nothing executed; a stored answer under a
//! different digest is `IDEMPOTENCY_CONFLICT` and a live claim is
//! `IDEMPOTENCY_KEY_IN_FLIGHT`. Both refusals go through the same
//! `crate::api::rest::audit_refusal_and_report` every other refusal here
//! uses — an idempotency refusal is a refusal, and gets no fourth path.
//!
//! A committed create **writes its answer back** before the transaction
//! ends: `state = answered` with the `201` and the rendered
//! [`ProductView`], through `crate::api::rest::record_idempotency_answer`,
//! on the same `tx` as the claim and the row
//! ([`insert_product_with_event`]). That is what makes the client's own
//! in-window retry replay the original response instead of being refused
//! `IDEMPOTENCY_KEY_IN_FLIGHT` for an act that already succeeded.
//!
//! @cpt-cf-bss-products-dod-read-door
//! @cpt-cf-bss-products-dod-create-doors
//! @cpt-cf-bss-products-dod-code-reservation
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
use crate::domain::error::DomainError;
use crate::domain::idempotency;
use crate::domain::name;
use crate::domain::validation::ValidationReport;
use crate::infra::events;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{self, NewProduct, ProductRecord, RefusalSubject};

/// `OpenAPI` tag for the Product surface's operations.
const TAG: &str = "BSS Products";

/// The `endpoint` component of every idempotency key this door claims
/// (**P-D-42**: *"`endpoint` MUST be the concrete resource path, not the
/// route template"*).
///
/// A create's concrete path **is** the collection path — there is no id yet
/// to put in it — so this constant and the path [`router`] registers are the
/// same string, and they are two spellings on purpose: the router's is a
/// literal because route registration is what the architecture lint over
/// route shape reads, and this one is what the store is keyed by. They must
/// stay equal; a door claiming a key under a path it does not serve would
/// let one caller's key collide with another door's.
///
/// Three lane names are reserved for callers with no wire surface —
/// `internal:scheduled-activation`, `internal:cascade-leg`,
/// `internal:bulk-row` — and this phase has none, so none is used here. They
/// are named rather than defined as constants because the first non-`HTTP`
/// caller is the one that knows which of the three it is and what it writes
/// in `client_key`.
const CREATE_ENDPOINT: &str = "/bss-products/v1/products";

/// The Product entity's resource marker for this file's own 403/404
/// answers — an authz deny on either door, and the read door's scope-closed
/// miss. Deliberately its own type, distinct from `infra::error_mapping`'s
/// private `ProductResource`: that one renders the `DomainError` ladder
/// [`create_product`]'s conflicts and validation failures raise
/// (`DomainError`'s own `From<DomainError> for CanonicalError`, via
/// `.into()`), carrying its own copy of this same GTS resource type. Both
/// name the same entity from call sites that do not share a `DomainError`
/// ladder to route through, which is `infra::error_mapping`'s own stated
/// reason for not sharing a single marker across every caller either.
#[resource_error(gts_id!("cf.bss.products.product.v1~"))]
struct ProductResource;

/// The read surface of a Product head.
///
/// Carries exactly the fields `docs/features/foundation.md`'s read-door
/// contract names: the ids, the name and its external code, the lifecycle
/// state, both revision counters, both scope columns, the creator and the
/// two timestamps. `name_normalized` is deliberately absent — it is the
/// uniqueness index's own operand, never a field an author reads or writes.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ProductView {
    /// The row's own id.
    pub product_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The brand the Product belongs to.
    pub brand_id: Uuid,
    /// The operator-facing name, as authored.
    pub name: String,
    /// The optional external mapping code.
    pub product_code: Option<String>,
    /// Where the entity sits in the lifecycle machine (`LifecycleState`'s
    /// wire spelling — carried as a plain string on the read surface, the
    /// same way `fx_revaluation_mode::FxRevaluationModeView` carries its own
    /// enum field, since neither enum derives `Serialize` on its own).
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

impl From<ProductRecord> for ProductView {
    fn from(record: ProductRecord) -> Self {
        Self {
            product_id: record.product_id,
            tenant_id: record.tenant_id,
            brand_id: record.brand_id,
            name: record.name,
            product_code: record.product_code,
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

/// Build the Axum router for the Product read and create doors and register
/// both with the supplied `OpenAPI` registry.
///
/// Registers its own absolute paths (`/bss-products/v1/products/{id}`,
/// `/bss-products/v1/products`) rather than being nested under the gear's
/// prefix by a caller — the same shape the sibling ledger gear's door
/// modules use — so `register_rest` merges this router directly onto the
/// host router.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-products/v1/products/{id}")
        .operation_id("bss_products.get_product")
        .summary("Read a Product head")
        .description(
            "Returns the Product head named by `id`: its identity, its lifecycle state, both \
             revision counters and both scope columns. Gates on `product x read`; a Product \
             outside the caller's authorized scope reads exactly like an absent one (`404`, no \
             existence leak). The `ETag` header carries `internal_revision` and is what \
             `PATCH`/`POST .../publish`/`POST .../discard` accept back as `If-Match`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product to read.")
        .handler(get_product)
        .json_response_with_schema::<ProductView>(openapi, StatusCode::OK, "The Product head.")
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    let router = OperationBuilder::post("/bss-products/v1/products")
        .operation_id("bss_products.create_product")
        .summary("Create a Product")
        .description(
            "Mints a new Product head as `draft` (`published_version = 0`, \
             `internal_revision = 1`) and enqueues its `ProductCreated` event in the same \
             transaction, and writes nothing else; content rows are the save door's. Gates on \
             `product x write`. The id is server-minted; a caller-supplied `id` is refused \
             `VALIDATION`. \
             `product_code`'s and the normalized name's uniqueness are reserved by the insert \
             itself: a collision refuses `DUPLICATE_CODE`/`DUPLICATE_NAME`, each with an \
             audited reason. \
             An optional `Idempotency-Key` header claims the key \
             `(tenant, /bss-products/v1/products, key)` in the same transaction as the \
             mutation: a duplicate under a live key is refused \
             `IDEMPOTENCY_KEY_IN_FLIGHT`, and the same key under a different payload is \
             refused `IDEMPOTENCY_CONFLICT`. A request without the header is created \
             normally.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<CreateProductRequest>(openapi, "The Product to create.")
        .handler(create_product)
        .json_response_with_schema::<ProductView>(
            openapi,
            StatusCode::CREATED,
            "The created Product head.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router.layer(Extension(state))
}

/// `GET /products/{id}`.
///
/// See this module's doc for the authorization scope, the miss/hit split and
/// why the miss carries no registry code.
async fn get_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(product_id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();

    // The scope handed to the repository below is exactly this one — never
    // one rebuilt from `tenant_id` — so the read stays under the SQL-level
    // filter the PDP actually granted.
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PRODUCT,
        crate::authz::actions::READ,
        /* owner_tenant_id */ None,
        /* resource_id */ Some(product_id),
        /* require_constraints */ true,
    )
    .await
    .map_err(|e| {
        authz_error_to_canonical(e, |reason| {
            ProductResource::permission_denied()
                .with_reason(reason)
                .create()
        })
    })?;

    let conn = state
        .db
        .conn()
        .map_err(|e| CanonicalError::internal(format!("bss-products: db conn: {e}")).create())?;

    let record = repo::find_product(&conn, &scope, tenant_id, product_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .ok_or_else(|| product_not_found(product_id))?;

    let tag = preconditions::etag(InternalRevision::new(record.internal_revision));
    Ok(([(ETAG, tag)], axum::Json(ProductView::from(record))).into_response())
}

/// The `404` a miss (absent OR out-of-scope, indistinguishably) answers
/// with. Bare on purpose — see this module's doc's "The miss" section.
fn product_not_found(product_id: Uuid) -> CanonicalError {
    ProductResource::not_found("no Product matches this id in the caller's scope")
        .with_resource(product_id.to_string())
        .create()
}

/// `POST /bss-products/v1/products` request body.
///
/// Carries an **explicit, optional `id` field** (`dod-create-doors`: "minting
/// the entity id server-side and refusing a caller-supplied id as
/// `VALIDATION`"). Two shapes could have satisfied that clause: give the
/// type nowhere to put an `id` at all, so one silently never reaches
/// persistence, or accept an optional `id` and refuse the request with
/// `VALIDATION` when one arrives. An earlier revision of this door took the
/// first, field-less reading; measured against the `DoD`'s own words it fell
/// short, because "refusing a caller-supplied id as `VALIDATION`" names a
/// response the caller receives, not merely a property of what lands in
/// storage — a field-less DTO makes an `id` key **silently ignored**
/// (`#[toolkit_macros::api_dto(request)]`'s expansion adds no `#[serde(
/// deny_unknown_fields)]`, matching every other request DTO on this surface),
/// which answers `201`, never the named refusal the `DoD` calls for. This DTO
/// takes the second, explicit-field reading instead: [`create_product`]
/// refuses a present `id` as `VALIDATION` naming the field, which is the
/// property the `DoD` actually asks for. The cost this reading pays that the
/// field-less one did not is a runtime branch a later edit could forget to
/// keep wired — this door's own test suite is what re-proves it stays
/// refused. Deliberately **not** `#[serde(deny_unknown_fields)]`: that would
/// refuse every unrecognized key on this DTO, not only `id`, changing the
/// wire contract for every other field far beyond this one rule.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct CreateProductRequest {
    /// Must be absent. Present only so a caller-supplied value can be
    /// refused `VALIDATION` by name rather than silently dropped — see this
    /// type's own doc for why the explicit-field reading was chosen over the
    /// field-less one. The entity id is always server-minted
    /// (`dod-create-doors`).
    pub id: Option<Uuid>,
    /// Required. Validated for presence only in this slice — see this
    /// module's doc, "`brand_id` claims this door cannot check", for why the
    /// caller's-brand-claims half of `dod-create-doors`/P-D-33 is not built
    /// here.
    pub brand_id: Uuid,
    /// The operator-facing name, as authored. `name_normalized` is derived
    /// from it (`crate::domain::name::normalize`), never accepted from the
    /// caller.
    pub name: String,
    /// The optional external mapping code, reserved by the insert itself
    /// (`dod-code-reservation`). Trimmed, and a blank-after-trim value
    /// collapses to `None` before the insert (F-2 fix): `uq_products_product_code`
    /// is partial `WHERE product_code IS NOT NULL`, so an un-trimmed `""` is a
    /// real, reservable value that two unrelated callers sending an unset
    /// optional field as `""` would otherwise collide on.
    pub product_code: Option<String>,
    /// The region value set, written from the payload when present; empty
    /// (unrestricted) when omitted.
    pub region_scope: Option<String>,
    /// The brand value set, written from the payload when present; empty
    /// (unrestricted) when omitted.
    pub brand_scope: Option<String>,
}

/// The digest of one parsed create request, as the claim is taken against
/// (`crate::domain::idempotency`, **P-D-34**).
///
/// The operand is the **fields this request carries**, each rendered from
/// the parsed value rather than from the received bytes: an omitted optional
/// field is left out of the object entirely rather than rendered `null`, so
/// a client that omits `product_code` and one that clears it are only the
/// same request to the extent this `DTO` can tell them apart — which it
/// cannot, since `Option<String>` collapses an absent key and an explicit
/// `null` into one `None` (`CreateSkuRequest`'s own doc names the same
/// three-state reading from the other side). Telling those two apart is owed
/// to whichever door first needs the distinction on the wire, and it costs a
/// `DTO` change, not a change here.
///
/// The values are rendered as the domain rendering takes them: a `Uuid` as
/// its canonical hyphenated string, a `String` verbatim — deliberately
/// **before** this door's own trimming and `name` normalization, because the
/// hash's subject is the request the caller sent, not the row this door
/// derives from it.
///
/// Nothing about the transport enters the object: no header, no
/// correlation id, no precondition (**P-D-34**; the `If-Match` exclusion is
/// structural here, since this function is never handed the headers).
fn payload_digest(request: &CreateProductRequest) -> Vec<u8> {
    let mut fields = JsonMap::new();
    if let Some(id) = request.id {
        fields.insert("id".to_owned(), JsonValue::String(id.to_string()));
    }
    fields.insert(
        "brand_id".to_owned(),
        JsonValue::String(request.brand_id.to_string()),
    );
    fields.insert("name".to_owned(), JsonValue::String(request.name.clone()));
    if let Some(code) = request.product_code.clone() {
        fields.insert("product_code".to_owned(), JsonValue::String(code));
    }
    if let Some(region) = request.region_scope.clone() {
        fields.insert("region_scope".to_owned(), JsonValue::String(region));
    }
    if let Some(brand) = request.brand_scope.clone() {
        fields.insert("brand_scope".to_owned(), JsonValue::String(brand));
    }
    idempotency::payload_digest(&JsonValue::Object(fields))
}

/// Insert the entity row and enqueue its `ProductCreated` event, in one
/// transaction (`dod-create-doors`) — and nothing else. Split out of
/// [`create_product`] to keep that function's own body to the steps its doc
/// enumerates rather than their expansion.
///
/// Returns the raw [`DbError`] on failure rather than a [`CanonicalError`]:
/// [`create_product`] still needs the driver text this error carries to
/// distinguish a unique-index collision from an unrelated storage failure
/// (`classify_insert_conflict`), which a [`CanonicalError`] would already
/// have discarded.
///
/// # The claim runs here, on the mutation's own runner
///
/// `claim` is `Some` exactly when the request carried an `Idempotency-Key`
/// (a keyless request skips the phase, P-D-34), and its `INSERT` is executed
/// **inside this closure**, on the same `tx` the entity insert and the
/// outbox enqueue run on. That is P-D-42's whole requirement and this
/// function is where it is discharged: the claim, the entity row and the
/// event commit together or not at all, so a rollback frees the key with no
/// release step of its own. Moving the claim to a runner of its own — a
/// second `state.db.transaction(..)`, or `state.db.conn()` before this call
/// — would leave a claim behind for a mutation that never committed, which
/// is the defect `products_tests
/// ::a_rolled_back_mutation_frees_the_key_for_a_later_create` exists to
/// catch.
///
/// The claim is the closure's **first** statement, so a replay or a refusal
/// ends the transaction before the entity insert runs and nothing is
/// written on either path (P-D-38).
///
/// # The answer is the closure's **last** statement, and why it is last
///
/// `record_idempotency_answer` runs after the entity insert and after the
/// outbox enqueue, because it stores the response body and the body cannot
/// be rendered before the row it renders exists. Everything the answer
/// records is therefore already written when it runs, and it commits with
/// them — `inst-fd-idem-claim-write`'s "together or not at all". Ordering it
/// before the enqueue would be no safer and would store an answer for an
/// event that had not been queued yet; ordering it outside the closure would
/// leave a committed create with a `claimed` key, which is the state the
/// write exists to remove.
///
/// The body is rendered here, once, and travels out on
/// [`CreateOutcome::Created`] — the handler answers **that** value rather
/// than re-rendering the view, so what a later replay serves and what the
/// original caller was told are the same bytes.
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
async fn insert_product_with_event(
    state: &ApiState,
    scope: AccessScope,
    new: NewProduct,
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

                    let record = repo::insert_product(tx, &scope, new)
                        .await
                        .map_err(|e| DbError::Sea(DbErr::Custom(e.to_string())))?;

                    let core = events::EventBodyCore {
                        tenant_id: record.tenant_id,
                        entity_kind: events::EntityKind::Product.as_str(),
                        entity_id: record.product_id,
                        internal_revision: record.internal_revision,
                        lifecycle_state: record.lifecycle_state.as_str(),
                    };
                    events::enqueue(
                        &outbox,
                        tx,
                        record.product_id,
                        events::PRODUCT_CREATED_PAYLOAD_TYPE,
                        &core,
                    )
                    .await
                    .map_err(|e| {
                        DbError::Sea(DbErr::Custom(format!("enqueue ProductCreated: {e}")))
                    })?;

                    let internal_revision = record.internal_revision;
                    let body = serde_json::to_value(ProductView::from(record)).map_err(|e| {
                        DbError::Sea(DbErr::Custom(format!("render the created Product: {e}")))
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

/// `POST /products`: mint a Product head as a `draft`.
///
/// See this module's doc, "The create door", for what this writes and what
/// it deliberately does not, and "`brand_id` claims this door cannot check"
/// for the one clause of `dod-create-doors` this slice discharges only
/// partially.
///
/// # Every refusal is audited, on its own runner
///
/// `dod-audit-trail` (`design/01-foundation.md` §4.4, "What the table holds")
/// requires an append-only audit row for **every** refusal a door raises, not
/// only the code-reservation conflicts this door happened to audit first.
/// Every branch below that can return `Err` before the mutation commits —
/// the authorization denial, both shape `VALIDATION`s, and the two
/// code-reservation conflicts — goes through `crate::api::rest::
/// audit_refusal_and_report`, which writes the row on its own transaction
/// and only then answers the refusal, honouring `repo::write_refusal_audit`'s
/// own contract: an unwritable row answers `AUDIT_UNAVAILABLE` instead,
/// never the refusal that would otherwise have been reported.
/// `AUDIT_UNAVAILABLE` itself gets no row of its own (P-D-34; it *is* the row
/// that could not be written), so it alone bypasses this helper.
///
/// Every refusal here is raised **before** the entity is minted, so each one
/// names its subject with `RefusalSubject::Attempted`, carrying `name` —
/// never `RefusalSubject::Minted`, which would require a `subject_id` that,
/// this early, identifies nothing (`design/01-foundation.md` §4.4's three
/// notes on the roster).
///
/// # Order of operations, and why
///
/// 1. The request is destructured up front — before anything that can
///    refuse runs — so `trimmed_name` exists for every refusal below to
///    audit against, the authorization denial included.
/// 2. [`repo::resolve_actor_ref`] (via `crate::api::rest::
///    resolve_creator_actor_ref`), on its own transaction — **before** the
///    authorization gate. `repo::resolve_actor_ref`'s own doc states the
///    reason this door does not reorder: a refusal below rolls this door's
///    own mutation transaction back while the refusal's audit row commits
///    independently and requires an `actor_ref` to attribute to, so the ref
///    must already exist before anything that can refuse runs — and an
///    authorization deny is exactly such a refusal.
/// 3. The `product x write` gate (`crate::authz::access_scope`), anchored to
///    the caller's own tenant. A denial is audited under the caller's own
///    tenant-scoped self access (`crate::api::rest::
///    audit_refusal_and_report`'s own doc explains why no write scope exists
///    yet to reuse).
/// 4. The idempotency phase (`dod-idempotency-store`): read
///    `Idempotency-Key` off the headers and digest the parsed body. §2's own
///    step list puts this in step 2, *with* the authorization gate and after
///    the `actor_ref` resolution, which is why it sits here and not ahead of
///    step 2 — see `crate::api::rest`'s module doc, "The idempotency phase",
///    for the reading of `dod-idempotency-store`'s "first pipeline phase"
///    against §2's step list. **A request with no header skips the phase**
///    (P-D-34); a header present but unusable is `VALIDATION`, audited like
///    every other shape refusal. The claim `INSERT` itself is not made here:
///    it joins the mutation's transaction in step 6 (P-D-42).
/// 5. Shape validation: `name` non-blank, `brand_id` non-nil, and `id`
///    absent (`dod-create-doors`'s server-minted-id clause, F-6). Audited
///    under the gate's own scope.
/// 6. The mutation: the claim, [`repo::insert_product`],
///    `crate::infra::events`'s `ProductCreated` enqueue and the answer
///    written back into the claim, in one transaction
///    (`dod-create-doors`, P-D-42, `inst-fd-idem-claim-write`).
/// 7. On a unique-index collision, [`classify_insert_conflict`] and
///    [`refuse_insert_conflict`]; on an idempotency verdict, a replay served
///    from the stored answer or a refusal audited through
///    `crate::api::rest::audit_refusal_and_report` — never a fourth path.
async fn create_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
    Json(body): Json<CreateProductRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = Utc::now();

    // The digest is taken from the parsed request before it is destructured,
    // so the operand is what the caller sent rather than what the steps
    // below derive from it (`payload_digest`'s own doc).
    let payload_hash = payload_digest(&body);

    // -- 1. Destructure up front: every refusal below, including the
    // authorization denial, audits against `trimmed_name`. --
    let CreateProductRequest {
        id: caller_supplied_id,
        brand_id,
        name: raw_name,
        product_code: raw_product_code,
        region_scope,
        brand_scope,
    } = body;
    let trimmed_name = raw_name.trim().to_owned();

    // -- 2. actor_ref resolution: its own transaction, ahead of the gate. --
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;

    // -- 3. The authorization gate. --
    let scope = match crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PRODUCT,
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
                    subject_kind: crate::authz::labels::PRODUCT,
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(trimmed_name.clone()),
                ProductResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await);
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            return Err(authz_error_to_canonical(err, |reason| {
                ProductResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }));
        }
    };

    // -- 4. The idempotency phase: the key, and the digest taken above. An
    // absent header is the skip (P-D-34), not a refusal; a present but
    // unusable one is `VALIDATION`, audited under the gate's own scope like
    // every other shape refusal. --
    let client_key = match idempotency_key(&headers) {
        Ok(key) => key,
        Err(domain_err) => {
            return Err(crate::api::rest::audit_refusal_and_report(
                &state,
                &scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: crate::authz::labels::PRODUCT,
                    error_code: domain_err.code(),
                },
                RefusalSubject::Attempted(trimmed_name.clone()),
                CanonicalError::from(domain_err),
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
    if trimmed_name.is_empty() {
        report.violate("VALIDATION", "name", "name must not be blank");
    }
    if brand_id.is_nil() {
        report.violate("VALIDATION", "brand_id", "brand_id is required");
    }
    if caller_supplied_id.is_some() {
        report.violate(
            "VALIDATION",
            "id",
            "id is server-minted and must not be supplied",
        );
    }
    if !report.is_empty() {
        let domain_err = DomainError::Validation(report);
        return Err(crate::api::rest::audit_refusal_and_report(
            &state,
            &scope,
            crate::api::rest::RefusalAuditContext {
                tenant_id,
                actor_ref,
                subject_kind: crate::authz::labels::PRODUCT,
                error_code: domain_err.code(),
            },
            RefusalSubject::Attempted(trimmed_name.clone()),
            CanonicalError::from(domain_err),
        )
        .await);
    }

    // `product_code` is trimmed and a blank-after-trim value collapses to
    // `None` (F-2): `uq_products_product_code` is partial `WHERE
    // product_code IS NOT NULL`, so an un-trimmed empty string is a real,
    // reservable value, and two unrelated creates that both send `""` (the
    // value many clients emit for an unset optional text field) would
    // otherwise collide on it.
    let product_code = raw_product_code
        .map(|code| code.trim().to_owned())
        .filter(|code| !code.is_empty());

    // Kept for the conflict path below, which needs it back if the insert
    // this door is about to attempt loses the code-reservation race.
    let attempted_code = product_code.clone();
    let new = NewProduct {
        product_id: Uuid::new_v4(),
        tenant_id,
        brand_id,
        name: trimmed_name.clone(),
        name_normalized: name::normalize(&trimmed_name),
        product_code,
        region_scope: region_scope.unwrap_or_default(),
        brand_scope: brand_scope.unwrap_or_default(),
        created_by: actor_ref.to_string(),
        created_at: now,
    };

    // -- 6. The mutation: the idempotency claim, the entity row, its
    // creation outbox row and the answer written back into the claim, one
    // transaction, nothing else written. --
    let insert_outcome = insert_product_with_event(&state, scope.clone(), new, claim).await;

    match insert_outcome {
        Ok(CreateOutcome::Created {
            internal_revision,
            body,
        }) => {
            let tag = preconditions::etag(InternalRevision::new(internal_revision));
            // `body` is the value rendered inside the mutation transaction
            // and, for a keyed create, stored there as the idempotency
            // answer. Answering it rather than re-rendering the view is what
            // makes a later replay reproduce this response and not a
            // lookalike.
            Ok((CREATE_RESPONSE_STATUS, [(ETAG, tag)], Json(body)).into_response())
        }
        // A replay executes nothing and audits nothing: it is not a refusal,
        // and the act it reproduces was audited (or, being a success,
        // deliberately not — P-D-21) when it originally ran.
        Ok(CreateOutcome::Replay { status, body }) => Ok(replay_response(status, body)),
        // An idempotency refusal is a refusal like any other: the audit row
        // on its own runner first, then the answer — or `AUDIT_UNAVAILABLE`
        // over it if that row cannot be written.
        Ok(CreateOutcome::Refused(domain_err)) => Err(crate::api::rest::audit_refusal_and_report(
            &state,
            &scope,
            crate::api::rest::RefusalAuditContext {
                tenant_id,
                actor_ref,
                subject_kind: crate::authz::labels::PRODUCT,
                error_code: domain_err.code(),
            },
            RefusalSubject::Attempted(trimmed_name),
            CanonicalError::from(domain_err),
        )
        .await),
        Err(db_error) => {
            let message = db_error.to_string();
            match classify_insert_conflict(&message) {
                Some(conflict) => Err(refuse_insert_conflict(
                    &state,
                    &scope,
                    tenant_id,
                    actor_ref,
                    conflict,
                    &trimmed_name,
                    attempted_code.as_deref(),
                )
                .await),
                None => Err(repo_error_to_canonical(&RepoError::Db(message))),
            }
        }
    }
}

/// Which unique index [`classify_insert_conflict`] read an insert failure's
/// driver text as having violated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertConflict {
    /// `uq_products_product_name` — `(tenant_id, brand_id,
    /// name_normalized)`.
    DuplicateName,
    /// `uq_products_product_code` — `(tenant_id, product_code)`.
    DuplicateCode,
}

/// Tell a duplicate name from a duplicate code from an unrelated storage
/// failure, off the driver text an insert failure already carries.
///
/// See this module's doc, "Telling `DUPLICATE_NAME` from `DUPLICATE_CODE`
/// from an unrelated storage failure", for why this is a substring match
/// over driver text rather than a typed answer, and what that costs.
/// `product_code` is checked first because Postgres's own constraint name
/// (`uq_products_product_code`) and `SQLite`'s column-list wording both
/// contain it literally, and it cannot appear in a message the name index
/// produced on either backend.
fn classify_insert_conflict(message: &str) -> Option<InsertConflict> {
    let lower = message.to_ascii_lowercase();
    let looks_like_a_unique_violation = lower.contains("unique constraint")
        || lower.contains("unique constraint failed")
        || lower.contains("duplicate key");
    if !looks_like_a_unique_violation {
        return None;
    }
    if lower.contains("product_code") {
        Some(InsertConflict::DuplicateCode)
    } else if lower.contains("name_normalized") || lower.contains("uq_products_product_name") {
        Some(InsertConflict::DuplicateName)
    } else {
        None
    }
}

/// Refuse an insert conflict: write its audit row on a transaction of its
/// own, then answer the domain refusal — or, if the audit row could not be
/// written, `AUDIT_UNAVAILABLE` instead, never the domain refusal
/// (`crate::api::rest::audit_refusal_and_report`'s own contract;
/// `dod-code-reservation`: "refusing the loser of a concurrent race ... with
/// an audited reason").
///
/// `scope` is the caller's own compiled write scope (from the authorization
/// gate this door already ran) — the refusal audit row is written under the
/// same tenant-scoped access the mutation itself was authorized under, not a
/// fresh, broader one.
async fn refuse_insert_conflict(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    conflict: InsertConflict,
    holder_name: &str,
    holder_code: Option<&str>,
) -> CanonicalError {
    let (error_code, refusal_subject_key, domain_err) = match conflict {
        InsertConflict::DuplicateName => (
            "DUPLICATE_NAME",
            holder_name.to_owned(),
            DomainError::DuplicateName(format!(
                "a Product named \"{holder_name}\" already exists for this tenant and brand"
            )),
        ),
        InsertConflict::DuplicateCode => {
            let code = holder_code.unwrap_or_default().to_owned();
            let domain_err = DomainError::DuplicateCode(format!(
                "product_code \"{code}\" is already reserved for this tenant"
            ));
            ("DUPLICATE_CODE", code, domain_err)
        }
    };

    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: crate::authz::labels::PRODUCT,
            error_code,
        },
        RefusalSubject::Attempted(refusal_subject_key),
        CanonicalError::from(domain_err),
    )
    .await
}

#[cfg(test)]
#[path = "products_tests.rs"]
mod products_tests;
