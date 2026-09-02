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
//! door repeats `products::create_product`'s order
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
//! `products::get_product`'s own doc names for the read
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
//! `products::create_product` does — see that module's
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
//! **The gate phase runs inside both doors** — publish and discard alike,
//! because `inst-fd-pipeline-gate-phase` puts it at *every* mutating door
//! and has it pass trivially where the act is ungated (**P-D-34**). On
//! publish the mode is an **explicit argument** of
//! [`publish_sku_gated`] (`dod-publish-door`, **P-D-30**), which is what
//! lets `04-lifecycle`'s scheduled runner drive this same door; on discard
//! it is the [`GateMode::Gate`] literal, for [`run_discard`]'s stated
//! reason. `inst-fd-gate-mode`'s *"never a wire-visible parameter"* holds
//! structurally: [`GateMode`] reaches no DTO, header reader or extractor in
//! this crate, and [`publish_sku`] and [`discard_sku`] — the only routed
//! handlers — pass literals for both the mode and the host. Nothing a caller
//! sends selects either.
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
//! 07's `CorrectionDoor`'s, and the approval-record consume flip is slice
//! 05's. `composition_pending` was on that list as this slice's own unpaid
//! debt and is no longer: [`run_publish`] reads the gate verdict's
//! `uncomposed_bundle_override` and carries it into both the frozen image and
//! the single head-row `UPDATE`. What remains owed there is the instruction's
//! `bundle` narrowing, which needs slice 03's `type` column.
//!
//!
//! # The save door
//!
//! `PATCH /bss-products/v1/skus/{id}` (`cpt-cf-bss-products-dod-save-door`)
//! is `products::save_product`'s twin and was written against it: the same
//! spine ([`open_act`], the claim, `transaction_with_retry`,
//! [`answer_head_act`]), the same phase order, the same bucket arms, the same
//! refusal codes and the same audit token. [`run_save`] carries the
//! reasoning; only the differences are here.
//!
//! **The column set is the schema's.** Bucket i is `sku_code` **and
//! `product_id`** — the parent link, which §4.1 files with identity on the
//! owner's call of 2026-08-27 — where on the Product the identically named
//! column is the primary key and is admitted in no `UPDATE` at all. Bucket
//! iii is the two scope columns. There is **no `name`**: `products_sku` has
//! no such column, so a `name` field arriving here is a registry miss and is
//! refused by the fail-closed rule rather than routed to the Product's tag.
//!
//! **The one phase the Product door has no analogue for** is the containment
//! re-check: §3.3 puts `SCOPE_NOT_CONTAINED` in the identity phase *"wherever
//! it runs — create, **save**, and the publish re-run"*, and a save is the
//! one door that can widen a child out of its parent's scope. It is
//! [`recheck_parent_containment`] — the publish door's own function, over the
//! image the save *would* store. A Product has no parent, so the asymmetry is
//! the schema's, exactly as it is at the publish doors.
//!
//! **What the `DoD` still owes** is the Product door's list unchanged: the
//! content rows of **slice 02** and the metering declaration of **slice 03**,
//! whose tables do not exist at this commit, so `dod-save-door` reads as
//! **partial** rather than met. [`save_sku_gated`]'s own doc names each.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-meter-atomic:p1
//! @cpt-dod:cpt-cf-bss-products-dod-unit-recognition:p1
//! @cpt-dod:cpt-cf-bss-products-dod-read-door:p1
//! @cpt-dod:cpt-cf-bss-products-dod-create-doors:p1
//! @cpt-dod:cpt-cf-bss-products-dod-code-reservation:p1
//! @cpt-dod:cpt-cf-bss-products-dod-containment:p1
//! @cpt-dod:cpt-cf-bss-products-dod-idempotency-store:p1
//! @cpt-cf-bss-products-dod-publish-door
//! @cpt-dod:cpt-cf-bss-products-dod-transition-guard:p1
//! @cpt-cf-bss-products-dod-save-door

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
use crate::domain::bucket;
use crate::domain::canonical;
use crate::domain::concurrency::InternalRevision;
use crate::domain::containment::{
    EmptyScopeToken, ResolvedScope, ScopeContainment, ScopeInput, ScopePair,
};
use crate::domain::disposition::{self, CLONE_SUGGESTION_ATTEMPTS, SkuCloneSource};
use crate::domain::error::DomainError;
use crate::domain::governance::{
    ApprovalId, EntityRef, GateMode, GateSubject, GovernanceGate, NoMaterialityPolicyGate,
};
use crate::domain::idempotency;
use crate::domain::rules::{
    PublishRevalidationSubject, SkuCodeStillPresent, SkuScopeColumnsStillParse,
};
use crate::domain::transition::{self, ApprovalInvalidation, ApprovalInvalidationHook as _};
use crate::domain::validation::{ValidationPipeline, ValidationReport};
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
/// `products::ProductResource`'s own doc's reason. **That** one, not
/// `infra::error_mapping`'s: there are two types of the name, and only the
/// door's own explains why a door declares a marker the error module already
/// has. Review wave C corrected this citation's *module* and left it pointing
/// at the type that does not carry the reason.
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
    /// The declared metering unit, or absent where no declaration stands.
    pub metering_unit: Option<String>,
    /// The declaration's usage-type reference — present exactly when the
    /// unit is (the paired `CHECK`).
    pub usage_type_ref: Option<String>,
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
            metering_unit: record.metering_unit,
            usage_type_ref: record.usage_type_ref,
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

    let router = register_clone_route(router, openapi);
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

    let router = OperationBuilder::patch("/bss-products/v1/skus/{id}")
        .operation_id("bss_products.save_sku")
        .summary("Save a SKU head")
        .description(
            "Writes the named fields onto the SKU head in one guarded UPDATE, bumps \
             `internal_revision` by one and enqueues `SkuHeadSaved`, and writes no version row \
             and moves no `published_version`, the head being the authoring surface in every \
             non-terminal state. Every field the body names is routed by its field-mutability \
             bucket before any of them is written, so a request naming one refused field applies \
             none of the others. Identity fields (`sku_code`, `product_id`) are admitted only \
             before first publish and refused `ILLEGAL_FIELD_MUTATION` after it; `region_scope` \
             and `brand_scope` are admitted on any non-terminal head, published or not. Every \
             save is then re-checked against the parent Product as it now stands, whatever fields \
             it names: the scope the save would leave must still be contained in the parent's \
             (`SCOPE_NOT_CONTAINED`), and a `retired` or `discarded` parent refuses the save \
             (`PARENT_TERMINAL`), so a save naming only `sku_code` can be refused on its parent's \
             account. A field no bucket registry row names is refused `ILLEGAL_FIELD_MUTATION` \
             rather than routed to a default. Gates on `sku x write` and requires `If-Match`: \
             absent is `VALIDATION`, stale is `STALE_REVISION`. A `retired` or `discarded` head \
             is refused `ENTITY_TERMINAL`. Accepts an optional `Idempotency-Key`, whose digest is \
             taken over this body.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The SKU to save.")
        .json_request::<SaveSkuRequest>(openapi, "The fields to write.")
        .handler(save_sku)
        .json_response_with_schema::<SkuView>(
            openapi,
            StatusCode::OK,
            "The saved SKU head, at its new revision.",
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

/// `GET /skus/{id}`. See `products::get_product`'s doc
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
pub(super) fn scope_input_from_payload(raw: Option<String>) -> Result<ScopeInput, EmptyScopeToken> {
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

/// Translate a [`ScopeContainment::NotContained`] verdict into the gear's
/// one `SCOPE_NOT_CONTAINED` [`DomainError`] (`dod-containment`, P-D-39),
/// for [`create_sku`] to hand to
/// `crate::api::rest::audit_refusal_and_report`.
///
/// **Every door that can raise the code renders it here**: this one,
/// [`recheck_parent_containment`] on the SKU save and publish re-runs, and
/// `products::check_children_stay_contained` on the
/// Product save — where the same verdict is reached from the parent's end.
/// One function rather than a copy per door, so the entity kinds cannot word
/// or code the same verdict two ways.
///
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
pub(super) fn scope_not_contained_domain_err(
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

/// The door-facing face of [`crate::infra::create::insert_sku_with_event`] —
/// see the Product twin's wrapper; identical terms, this surface's view.
pub(crate) async fn insert_sku_with_event(
    state: &ApiState,
    scope: AccessScope,
    new: NewSku,
    claim: Option<IdempotencyClaimInput>,
    actor_ref: Uuid,
) -> Result<CreateOutcome, DbError> {
    crate::infra::create::insert_sku_with_event(
        &state.db,
        &state.sink,
        scope,
        new,
        crate::infra::create::JoinedRecords { claim, stamp: None },
        actor_ref,
        render_created_sku,
    )
    .await
}

/// The created SKU as its `201` answers it — the one rendering both the
/// response and the stored idempotency answer are built from.
fn render_created_sku(record: repo::SkuRecord) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(SkuView::from(record))
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

/// The parent Product's two stored scope columns, parsed into the
/// [`ScopePair`] the containment rule reads.
///
/// **One parse site, two callers**: [`resolve_parent_scope`] on create and
/// [`recheck_parent_containment`] on the publish re-run
/// (§3.3, *"the identity phase raises ... `SCOPE_NOT_CONTAINED` wherever it
/// runs — create, save, and the publish re-run"*). They differ only in what
/// a malformed column costs them, which is why the error is the **column
/// name** rather than a rendered error: each caller renders it into its own
/// failure channel, and neither can drift from the other on how the columns
/// are read.
///
/// # Errors
///
/// The name of the first column that does not parse under
/// [`ResolvedScope::parse`]'s own rule. A stored column that fails is a
/// storage invariant violation rather than any caller's fault, and both
/// callers answer it as an internal failure.
pub(super) fn parent_scope_pair(parent: &repo::ProductRecord) -> Result<ScopePair, &'static str> {
    Ok(ScopePair {
        region: ResolvedScope::parse(&parent.region_scope)
            .map_err(|EmptyScopeToken| "region_scope")?,
        brand: ResolvedScope::parse(&parent.brand_scope)
            .map_err(|EmptyScopeToken| "brand_scope")?,
    })
}

/// [`parent_scope_pair`]'s child-side twin: a stored SKU's two scope columns
/// parsed into the [`ScopePair`] the containment rule reads as the **child**
/// operand.
///
/// **One parse site, two callers**, on [`parent_scope_pair`]'s own terms:
/// [`recheck_parent_containment`] here, and
/// `products::check_children_stay_contained` on the
/// Product door, which asks the same containment question from the other
/// end — a parent narrowing under its live children rather than a child
/// moving out from under its parent. Written once so the two doors cannot
/// read a stored child differently, and so neither can drift into
/// re-resolving the child against the parent: the operand is the row's own
/// stored pair, materialized at create time by [`ScopePair::resolve_child`],
/// and re-resolving it would re-widen it to whatever the parent now carries
/// and turn every narrowing into a silent pass.
///
/// # Errors
///
/// The name of the first column that does not parse. Both callers answer it
/// internally: a stored column that fails is a storage invariant violation,
/// not any caller's fault.
pub(super) fn sku_scope_pair(sku: &SkuRecord) -> Result<ScopePair, &'static str> {
    Ok(ScopePair {
        region: ResolvedScope::parse(&sku.region_scope)
            .map_err(|EmptyScopeToken| "region_scope")?,
        brand: ResolvedScope::parse(&sku.brand_scope).map_err(|EmptyScopeToken| "brand_scope")?,
    })
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
    let parent_scope = parent_scope_pair(&parent).map_err(|column| {
        CanonicalError::internal(format!(
            "bss-products: parent Product's stored {column} contains an empty token"
        ))
        .create()
    })?;

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
/// `products::create_product`'s own doc for the
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
    let now = canonical::write_instant(Utc::now());

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
        // `now` arrives already truncated to microseconds — the handler
        // stamps it through `canonical::write_instant` (P-D-82), which is
        // what closed the cross-engine digest hazard this comment used to
        // carry as a debt: neither engine now holds a fractional digit the
        // other could round differently.
        created_at: now,
        // An ordinary create has no lineage; the clone door is the pair's
        // only writer (P-D-76).
        cloned_from: None,
        cloned_from_version: None,
    };

    let insert_outcome = insert_sku_with_event(&state, scope.clone(), new, claim, actor_ref).await;

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
/// See `products::classify_insert_conflict`'s own doc for
/// the cost this substring match over driver text carries, which applies
/// identically here.
pub(super) fn classify_sku_insert_conflict(message: &str) -> bool {
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

/// Register `POST .../{id}/clone` onto `router` — its own function for
/// [`register_head_act_routes`]'s reason: one door, one place.
fn register_clone_route(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::post("/bss-products/v1/skus/{id}/clone")
        .operation_id("bss_products.clone_sku")
        .summary("Clone a SKU")
        .description(
            "Mints a new draft SKU from the named source (`CloneDoor`, lone-SKU shape): a \
             `draft` source is read at its head, a `published`, `deprecated` or `retired` one \
             at its last frozen version, and a `discarded` one is refused \
             `CLONE_SOURCE_DISCARDED` (409). The parent link copies from the source unless \
             `new_parent_id` overrides it; either way the ordinary create-door checks run \
             (`PARENT_TERMINAL`, `SCOPE_NOT_CONTAINED`), so a lone clone of a retired \
             parent's SKU must name a new parent. The code is suggested `{source}-copy-N`, N \
             the first free integer decided by the reservation; an overridden code's \
             collision is the ordinary `DUPLICATE_CODE`. Gates on `sku x write`. An optional \
             `Idempotency-Key` header claims the clone's own concrete path; a keyed retry \
             replays the first clone.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The source SKU.")
        .json_request::<CloneSkuRequest>(openapi, "The optional overrides.")
        .handler(clone_sku)
        .json_response_with_schema::<SkuView>(
            openapi,
            StatusCode::CREATED,
            "The cloned SKU head, a draft.",
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

/// The lone-SKU clone's body: the overrides and nothing else (**P-D-75**).
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct CloneSkuRequest {
    /// Overrides the suggested code. A collision on it is the ordinary
    /// `DUPLICATE_CODE` — only the *suggested* code walks first-free.
    pub code: Option<String>,
    /// Replaces the copied parent link (§3.1's lone-SKU carve-out): the
    /// create-door checks then run against this parent instead.
    pub new_parent_id: Option<Uuid>,
}

/// The clone body's idempotency digest, over the parsed request
/// (`payload_digest`'s twin for this door's own DTO).
fn clone_sku_payload_digest(request: &CloneSkuRequest) -> Vec<u8> {
    let mut fields = serde_json::Map::new();
    if let Some(code) = request.code.as_ref() {
        fields.insert("code".to_owned(), serde_json::Value::String(code.clone()));
    }
    if let Some(parent) = request.new_parent_id.as_ref() {
        fields.insert(
            "new_parent_id".to_owned(),
            serde_json::Value::String(parent.to_string()),
        );
    }
    crate::domain::idempotency::payload_digest(&serde_json::Value::Object(fields))
}

/// The concrete resource path a SKU clone claims its idempotency key under
/// (**P-D-42**'s rule).
fn clone_sku_endpoint(sku_id: Uuid) -> String {
    format!("/bss-products/v1/skus/{sku_id}/clone")
}

/// One string field out of a decoded frozen rendering (the Product door's
/// `frozen_str`, against the SKU roster's keys).
fn sku_frozen_str(
    content: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    content
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Read the SKU clone's source per `inst-cn-door`: a `draft` at its head,
/// everything else at the last frozen version through
/// [`canonical::decode_rendering`] (**P-D-77**), a `discarded` source
/// refused `CLONE_SOURCE_DISCARDED` (P-D-75).
async fn resolve_sku_clone_source(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
) -> Result<Option<SkuCloneSource>, CanonicalError> {
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&RepoError::Db(format!("clone source connection: {e}")))
    })?;
    let head = repo::find_sku(&conn, scope, tenant_id, sku_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let Some(head) = head else {
        return Ok(None);
    };

    if head.lifecycle_state == LifecycleState::Discarded {
        return Err(CanonicalError::from(DomainError::CloneSourceDiscarded(
            format!("sku {sku_id} is discarded and admits no clone"),
        )));
    }

    if head.lifecycle_state == LifecycleState::Draft {
        return Ok(Some(SkuCloneSource {
            product_id: head.product_id,
            sku_code: head.sku_code,
            region_scope: head.region_scope,
            brand_scope: head.brand_scope,
            read_at_version: None,
        }));
    }

    let frozen =
        repo::latest_entity_version(&conn, scope, tenant_id, VersionedEntityKind::Sku, sku_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?;
    let Some((version, content)) = frozen else {
        return Err(repo_error_to_canonical(&RepoError::CorruptRow(format!(
            "sku {sku_id} is {} with no frozen version row",
            head.lifecycle_state.as_str()
        ))));
    };
    let content = canonical::decode_rendering(&content).map_err(|e| {
        repo_error_to_canonical(&RepoError::CorruptRow(format!(
            "frozen content of sku {sku_id} v{version}: {e}"
        )))
    })?;
    let sku_code = sku_frozen_str(&content, "sku_code").ok_or_else(|| {
        repo_error_to_canonical(&RepoError::CorruptRow(format!(
            "frozen content of sku {sku_id} v{version} carries no sku_code"
        )))
    })?;

    Ok(Some(SkuCloneSource {
        // The parent link is identity, not content: the head's column and
        // the frozen rendering agree by construction, and the head's is the
        // one the carve-out overrides.
        product_id: head.product_id,
        sku_code,
        // The scope keys are always rendered by this gear's own freeze, so
        // their absence is the same corruption class the sku_code check
        // above refuses — never a silent empty scope on the clone.
        region_scope: sku_frozen_str(&content, "region_scope").ok_or_else(|| {
            repo_error_to_canonical(&RepoError::CorruptRow(format!(
                "frozen content of sku {sku_id} v{version} carries no region_scope"
            )))
        })?,
        brand_scope: sku_frozen_str(&content, "brand_scope").ok_or_else(|| {
            repo_error_to_canonical(&RepoError::CorruptRow(format!(
                "frozen content of sku {sku_id} v{version} carries no brand_scope"
            )))
        })?,
        read_at_version: Some(version),
    }))
}

/// `POST /bss-products/v1/skus/{id}/clone` — the lone-SKU shape of
/// `inst-cn-door` (`CloneDoor`), P-D-75's body, P-D-62's walk.
///
/// The act mirrors [`clone_sku`]'s Product twin minus the family phase: a
/// lone SKU is a single-entity act, so its keyed claim keeps the create
/// door's in-transaction answer, and the parent — copied or overridden — is
/// judged by [`resolve_parent_scope`], the ordinary create-door checks.
async fn clone_sku(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
    Path(source_id): Path<Uuid>,
    Json(body): Json<CloneSkuRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let payload_hash = clone_sku_payload_digest(&body);

    let code_override = body.code.as_deref().map(str::trim).map(str::to_owned);
    let new_parent_id = body.new_parent_id;

    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = match crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::SKU,
        crate::authz::actions::WRITE,
        Some(tenant_id),
        None,
        true,
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
                RefusalSubject::Attempted(source_id.to_string()),
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

    // -- shape: a supplied override must survive its own trim. --
    let mut report = ValidationReport::new();
    if code_override.as_deref() == Some("") {
        report.violate("VALIDATION", "code", "code override must not be blank");
    }
    if new_parent_id == Some(Uuid::nil()) {
        report.violate(
            "VALIDATION",
            "new_parent_id",
            "new_parent_id must name a Product",
        );
    }
    if !report.is_empty() {
        return Err(audit_sku_refusal(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            &source_id.to_string(),
            DomainError::Validation(report),
        )
        .await);
    }

    // -- the idempotency claim, on the clone's own concrete path. --
    let client_key = match idempotency_key(&headers) {
        Ok(key) => key,
        Err(domain_err) => {
            return Err(audit_sku_refusal(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                &source_id.to_string(),
                domain_err,
            )
            .await);
        }
    };
    let endpoint = clone_sku_endpoint(source_id);
    let claim = client_key.map(|key| {
        IdempotencyClaimInput::new(
            endpoint,
            key,
            payload_hash,
            now,
            state.idempotency_retention_hours,
        )
    });

    // -- the source, read where its state says. --
    let source = match resolve_sku_clone_source(&state, &scope, tenant_id, source_id).await {
        Ok(Some(source)) => source,
        Ok(None) => {
            return Err(sku_not_found(source_id));
        }
        Err(canonical) => {
            if canonical.status_code() == 409 {
                return Err(crate::api::rest::audit_refusal_and_report(
                    &state,
                    &scope,
                    crate::api::rest::RefusalAuditContext {
                        tenant_id,
                        actor_ref,
                        subject_kind: crate::authz::labels::SKU,
                        error_code: "CLONE_SOURCE_DISCARDED",
                    },
                    RefusalSubject::Attempted(source_id.to_string()),
                    canonical,
                )
                .await);
            }
            return Err(canonical);
        }
    };

    // -- the parent, copied or overridden, judged by the ordinary
    // create-door checks (the carve-out's own rule). --
    let parent_id = new_parent_id.unwrap_or(source.product_id);
    let child_scope = resolve_parent_scope(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        parent_id,
        &source.sku_code,
        PayloadScopes {
            region: Some(source.region_scope.clone()),
            brand: Some(source.brand_scope.clone()),
        },
    )
    .await?;

    // -- the first-free walk (P-D-62). --
    let mut code_n: u32 = 1;
    for _attempt in 0..CLONE_SUGGESTION_ATTEMPTS {
        let code = code_override
            .clone()
            .unwrap_or_else(|| disposition::suggested_sku_code(&source, code_n));
        let new = NewSku {
            sku_id: Uuid::new_v4(),
            tenant_id,
            product_id: parent_id,
            sku_code: code.clone(),
            region_scope: child_scope.region.render(),
            brand_scope: child_scope.brand.render(),
            created_by: actor_ref.to_string(),
            created_at: now,
            cloned_from: Some(source_id),
            cloned_from_version: source.read_at_version,
        };
        match insert_sku_with_event(&state, scope.clone(), new, claim.clone(), actor_ref).await {
            Ok(CreateOutcome::Created {
                internal_revision,
                body,
            }) => {
                let tag = preconditions::etag(InternalRevision::new(internal_revision));
                return Ok((CREATE_RESPONSE_STATUS, [(ETAG, tag)], Json(body)).into_response());
            }
            Ok(CreateOutcome::Replay { status, body }) => {
                return Ok(replay_response(status, body));
            }
            Ok(CreateOutcome::Refused(domain_err)) => {
                return Err(audit_sku_refusal(
                    &state, &scope, tenant_id, actor_ref, &code, domain_err,
                )
                .await);
            }
            Err(db_error) => {
                let message = db_error.to_string();
                if classify_sku_insert_conflict(&message) {
                    if code_override.is_none() {
                        // The suggested candidate lost its reservation: that
                        // is the walk, not a refusal (P-D-62).
                        code_n += 1;
                    } else {
                        // The operator's own collision: the ordinary
                        // audited refusal.
                        return Err(refuse_sku_insert_conflict(
                            &state, &scope, tenant_id, actor_ref, code,
                        )
                        .await);
                    }
                } else {
                    return Err(repo_error_to_canonical(&RepoError::Db(message)));
                }
            }
        }
    }

    // The cap is operational, not semantic (P-D-62): surface the family's
    // own conflict rather than invent a new refusal for it.
    Err(refuse_sku_insert_conflict(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        disposition::suggested_sku_code(&source, code_n),
    )
    .await)
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
/// # `composition_pending` is on the roster, and its own guard is the argument
///
/// It is the one column whose movement **coincides** with the version row
/// instead of bypassing it, which is the exact inverse of the criterion every
/// exclusion above rests on. §4.3 excludes its four columns because they
/// *"move on transitions, which write no version row, so freezing them would
/// need the digest to change on a write that produces no row to digest"*.
/// `composition_pending` cannot move that way: its clause in
/// `m20260829_000003_create_products_sku` admits a change to it **only in the
/// same statement as a `published_version` bump**, so every value it ever
/// takes is a value some frozen row was written alongside. There is no
/// transition that moves it and no save that can.
///
/// `inst-fd-publish-freeze` settles it from the other direction, naming the
/// column outright: the door *"computes the content this act leaves behind —
/// including the `composition_pending` value the same `UPDATE` is about to
/// write — and freezes that"* (**P-D-33**). A roster that omitted it would
/// make that sentence unimplementable.
///
/// It is also the roster's **only** member the publish act itself moves, and
/// therefore the one that makes the choice of image load-bearing: the freeze
/// is taken over [`post_publish_image`] and not over the pre-act head, because
/// the two now differ on this field. That write is [`run_publish`]'s, landed
/// with the flag on the same wave as the statement that carries it — the pair
/// this doc previously named as owed.
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
const SKU_VERSION_CONTENT_ROSTER: [&str; 13] = [
    "brand_scope",
    "cloned_from",
    "cloned_from_version",
    "composition_pending",
    "created_at",
    "created_by",
    "metering_unit",
    "product_id",
    "region_scope",
    "sku_code",
    "sku_id",
    "tenant_id",
    "usage_type_ref",
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
    /// recorded under: [`PUBLISH_AUDIT_ACTION`], [`DISCARD_AUDIT_ACTION`] or
    /// [`SAVE_AUDIT_ACTION`]. It travels in the context rather than being
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
/// from it: the two vocabularies coincide for a publish and diverge for both
/// of the acts that gate on `write` — a discard, which must still be
/// *recorded* as `discard`, and a save, recorded as `save`. Deriving one from
/// the other would file both under `write`, which is the same class of lie
/// this door has just stopped telling by leaving `create` behind.
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

/// The candidate the re-run judges: the head **as it now stands**, reduced to
/// what the registered rules read.
///
/// The Product door's [`crate::api::rest::products`] twin,
/// `publish_candidate`, does the same thing with
/// [`crate::domain::rules::CreateEntityCandidate`], and for the same reason:
/// the rules are the **domain's**, and a domain rule keyed to
/// [`SkuRecord`] — a repository DTO — is one slices 04 and 05 cannot register
/// beside their own without depending on `infra::storage::repo`. The
/// translation from the stored row to the judged subject belongs here, in the
/// door that read the row, which is exactly where a translation between a
/// storage shape and a domain shape belongs.
///
/// Behaviour is unchanged by the move: the two rules read `sku_code`,
/// `region_scope` and `brand_scope` and nothing else, so this carries those
/// three and no more — see [`PublishRevalidationSubject`]'s own doc for why a
/// wider subject would be worse rather than merely larger.
fn publish_revalidation_subject(record: &SkuRecord) -> PublishRevalidationSubject {
    PublishRevalidationSubject {
        sku_code: record.sku_code.clone(),
        region_scope: record.region_scope.clone(),
        brand_scope: record.brand_scope.clone(),
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
/// - **Shape** and **Identity** are [`SkuCodeStillPresent`] and
///   [`SkuScopeColumnsStillParse`], judging the head row as it now stands
///   rather than a payload — which is the point of a re-run: an entity that
///   stopped being publishable since approval fails closed rather than
///   publishing stale. Both live in [`crate::domain::rules`], beside
///   [`crate::domain::rules::NameShapeRule`] and the Product door's own
///   re-run rule, because a rule slices 04 and 05 must be able to register
///   beside their own cannot be declared in an API module over an `infra`
///   type; [`publish_revalidation_subject`] is the translation this door owes
///   in exchange.
/// - **State** is not registered as a rule and is not missing: it runs as
///   [`transition::check_head_write`] and [`transition::guard`] in the door's
///   own steps 3 and 5, and `repo::publish_sku_head` states the same rule a
///   second time in its `WHERE` clause. Registering a third copy here would
///   be a second answer to one question.
/// - **`RegisteredValidators` is empty, and that is a real gap, not a
///   passing phase.** The `→ published` validators the instruction names are
///   `04-lifecycle`'s and `05-governance`'s, and neither exists at this
///   commit; [`crate::domain::validation::Phase::RegisteredValidators`] therefore admits
///   everything. The
///   re-run is fail-closed over the rules that exist and silent over the ones
///   that do not, and no reading of this function should treat the phase's
///   emptiness as the entity having satisfied it.
/// - **`Idempotency`, `Precondition` and `GovernanceGate`** are phases the
///   door runs directly (the claim, the `If-Match`, the gate) rather than as
///   registered rules, for the same reason State is not registered.
fn publish_revalidation_pipeline() -> ValidationPipeline<PublishRevalidationSubject> {
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
/// Both counters move by exactly one here, `updated_at` becomes the act's own
/// instant, and `composition_pending` becomes the flag this act carries —
/// which is exactly the set of columns the single head-row `UPDATE` below
/// writes. The image and the statement are two spellings of one act, and
/// [`sku_version_content`] renders the image, so a column the statement writes
/// and this function forgot would be frozen at its **pre-act** value under the
/// **post-act** key, with a perfectly valid digest over it.
///
/// This image decides the frozen row's **key**: [`freeze_for`] reads
/// `published_version` off it, and `N + 1` is what makes the head table's
/// guard subquery find the frozen row when the `UPDATE` a statement later asks
/// for it.
///
/// It now also decides part of the frozen **content**, and that is new.
/// `published_version`, `internal_revision`, `lifecycle_state` and
/// `updated_at` are all excluded from [`SKU_VERSION_CONTENT_ROSTER`], so for
/// as long as those were the only columns the act moved, the pre-act head and
/// this image rendered identical bytes and the choice between them was
/// invisible. `composition_pending` is **on** the roster — see that constant's
/// own doc for why its own guard clause is the argument for including it — so
/// from this wave on the two images genuinely differ, and
/// `inst-fd-publish-freeze` is unambiguous about which one is frozen: the door
/// *"computes the content this act leaves behind — including the
/// `composition_pending` value the same `UPDATE` is about to write — and
/// freezes that"*.
///
/// `composition_pending` is a **parameter** rather than something derived
/// here, for `repo::publish_sku_head`'s own reason: the operand is the gate
/// verdict's `uncomposed_bundle_override`, and this function and that
/// statement must be handed the same value from the same place or the frozen
/// row and the head row disagree about the act that produced them.
fn post_publish_image(
    head: &SkuRecord,
    composition_pending: bool,
    now: DateTime<Utc>,
) -> SkuRecord {
    SkuRecord {
        lifecycle_state: post_publish_state(head.lifecycle_state),
        internal_revision: head.internal_revision.saturating_add(1),
        published_version: head.published_version.saturating_add(1),
        composition_pending,
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
/// # The image is the operand, and now it has to be
///
/// `inst-fd-publish-freeze` (**P-D-33**) takes the freeze over the image the
/// act leaves behind, and [`post_publish_image`] is what supplies one. That
/// was a distinction without a difference until `composition_pending` gained
/// its write: `published_version`, `internal_revision`, `lifecycle_state` and
/// `updated_at` are the other four columns a publish moves and all four are
/// excluded from [`SKU_VERSION_CONTENT_ROSTER`], so the pre-act head and the
/// post-act image rendered the same bytes and a caller that passed the wrong
/// one produced a correct row by luck.
///
/// It is no longer luck. `composition_pending` is on the roster and the act
/// moves it, so a freeze taken over the **pre-act** head would store the
/// previous flag under the **new** version's key — and the digest over it
/// would be perfectly valid, because the row would agree with itself and lie
/// only about the act that produced it. No reader downstream could detect
/// that: `content_digest` is a function of `content`, and both would be
/// self-consistently wrong. `skus_tests::
/// a_publish_carrying_the_uncomposed_bundle_override_freezes_the_raised_flag`
/// is what holds this closed, by reading the flag back out of
/// `products_entity_version` rather than off any in-memory value.
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
    // A JSON boolean, not a rendered string: canonical::render_into writes
    // `true`/`false` for a Bool, and §4.3's rendering rule has no clause that
    // would turn a flag into text. Rendering it as `"false"` would make the
    // frozen bytes disagree with the column's own type for no reason.
    fields.insert(
        "composition_pending".to_owned(),
        JsonValue::Bool(image.composition_pending),
    );
    fields.insert(
        "created_by".to_owned(),
        JsonValue::String(image.created_by.clone()),
    );
    fields.insert(
        "created_at".to_owned(),
        JsonValue::String(canonical::render_instant(image.created_at)),
    );
    // Lineage joins the roster by its own membership rule — publish does not
    // move it (P-D-76). Omit-when-absent exercises `Absence::Null`, the
    // `product_code` precedent one door over.
    if let Some(source) = image.cloned_from {
        fields.insert(
            "cloned_from".to_owned(),
            JsonValue::String(source.to_string()),
        );
    }
    if let Some(version) = image.cloned_from_version {
        fields.insert(
            "cloned_from_version".to_owned(),
            JsonValue::Number(version.into()),
        );
    }
    // The meter pair is content — 03's declaration freezes at publish like
    // every other content class — on the lineage pair's omit-when-absent
    // terms, so every version frozen before the pair existed keeps its bytes
    // and its digest.
    if let Some(unit) = image.metering_unit.as_ref() {
        fields.insert("metering_unit".to_owned(), JsonValue::String(unit.clone()));
    }
    if let Some(usage) = image.usage_type_ref.as_ref() {
        fields.insert(
            "usage_type_ref".to_owned(),
            JsonValue::String(usage.clone()),
        );
    }
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
    /// Wrap a repository failure, preserving the driver error inside it.
    ///
    /// `repo::RepoError` is a storage failure of this act's own mutation, and
    /// it cannot be classified by `transaction_with_retry` unless it arrives
    /// as a `DbErr` — so it becomes [`Self::Db`] carrying the one
    /// [`RepoError::to_db_err`] answers.
    ///
    /// **Which `DbErr` that is, is the whole of a fix this door's own doc
    /// used to claim without holding.** An earlier version of this
    /// constructor took `&impl Display` and wrapped every failure as
    /// `DbErr::Custom(error.to_string())`; `is_retryable_contention` answers
    /// `false` for `Custom` by construction, so a lock-contention failure —
    /// the loser of two concurrent publishes of the same SKU — reached the
    /// caller as a bare 500 where [`publish_in_one_transaction`]'s doc
    /// promises a retry. `RepoError::Driver` now carries `sea-orm`'s error as
    /// the driver raised it and `to_db_err` hands that variant on unchanged,
    /// so the classifier sees the `Exec`/`Query` it needs.
    ///
    /// The events layer keeps its own inline wrap at the enqueue call: an
    /// `EventsError` never held a `DbErr`, so there is nothing for this
    /// constructor to preserve and no reason to take a second argument type.
    fn from_repo(error: &RepoError) -> Self {
        Self::Db(DbError::Sea(error.to_db_err()))
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
/// none can consume what the next one needs. Every value *carried here* is
/// attempt-independent — `now` was stamped before the first attempt, and the
/// claim's window with it. The envelope id the act eventually enqueues is not
/// one of them; see [`insert_sku_with_event`] for why that is harmless.
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
        .map_err(|e| HeadActError::from_repo(&e))?
    {
        ClaimVerdict::Proceed => Ok(None),
        ClaimVerdict::Replay { status, body } => Ok(Some(MutationOutcome::Replay { status, body })),
        ClaimVerdict::Refused(refusal) => Err(HeadActError::Refused(refusal)),
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
async fn fire_invalidation_hook(
    runner: &impl toolkit_db::secure::DBRunner,
    inputs: &HeadActInputs,
    invalidation: ApprovalInvalidation,
) -> Result<(), HeadActError> {
    if invalidation != ApprovalInvalidation::Fire {
        return Ok(());
    }
    let entity = EntityRef {
        tenant_id: inputs.tenant_id,
        entity_kind: EntityKind::Sku,
        entity_id: inputs.sku_id,
    };
    // The domain seam still runs: it is the pure half of the act, and a host
    // that ever refuses is a refusal of the transition.
    transition::NoApprovalStoreHook
        .invalidate(entity)
        .map_err(HeadActError::Refused)?;
    repo::supersede_open_approval(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        &GateSubject::entity_publish(entity),
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::Db(toolkit_db::DbError::Sea(e.to_db_err())))?;
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
    outbox: &crate::infra::broker::EventSink,
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
            events::enqueue_published(
                outbox,
                runner,
                inputs.sku_id,
                payload_type,
                &core,
                version,
                inputs.actor_ref,
            )
            .await
        }
        None => {
            events::enqueue(
                outbox,
                runner,
                inputs.sku_id,
                payload_type,
                &core,
                inputs.actor_ref,
            )
            .await
        }
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
        .map_err(|e| HeadActError::from_repo(&e))?;
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
        Err(error) => HeadActError::from_repo(&error),
    }
}

/// Re-run the containment check against the **parent as it now stands**, as
/// part of the publish re-validation's identity phase.
///
/// # Why the publish door has to ask this at all
///
/// §3.3: *"the identity phase raises the uniqueness, reservation and
/// containment codes (`DUPLICATE_NAME`, `DUPLICATE_CODE`,
/// `SCOPE_NOT_CONTAINED`) wherever it runs — create, save, and the publish
/// re-run"*. §4.1 puts `region_scope`/`brand_scope` in bucket iii *"in both
/// directions, widening and narrowing alike, so a narrowing that would
/// orphan a live child meets `fr-parent-child-integrity`'s fail-closed check
/// ... ahead of the governance gate"*.
///
/// The operand is the **parent's** row, and it can move after the child is
/// created: nothing freezes a parent's scope when a SKU is minted under it,
/// and the head-row guard admits a bucket-iii narrowing on any non-terminal
/// head. Until this function existed the publish re-run re-parsed the SKU's
/// own two columns and nothing else ([`SkuScopeColumnsStillParse`]), so a
/// parent narrowed out from under a child — or a parent since retired —
/// published that child anyway. `SCOPE_NOT_CONTAINED` is a
/// Foundation-declared code (§3.3, and `04-lifecycle` §C5 carries the
/// reciprocal *"named in 01, registered here"* for its final semantics), so
/// this is not a later slice's debt.
///
/// # Why it is not a registered rule
///
/// [`publish_revalidation_pipeline`] is synchronous and judges the
/// [`SkuRecord`] alone; this check needs the parent row, which is a read.
/// So it runs as a continuation of the same identity phase, immediately
/// after the pipeline and before the edge and the gate — the position §4.1
/// asks for — rather than as a [`crate::domain::validation::Phase::Identity`]
/// rule that cannot reach
/// its operand. The same argument [`publish_revalidation_pipeline`]'s doc
/// makes for State not being registered.
///
/// # The Product door asks the same question from the other end
///
/// A Product has no parent to be contained **in**: `products_product`
/// carries `region_scope` and `brand_scope` as its **own** bucket-iii
/// columns and no `product_id` pointing upwards, so *this* function — a
/// child re-checking itself against its parent — has no analogue there, and
/// `crate::api::rest::products::run_publish` correctly asks nothing of the
/// kind.
///
/// That is a different obligation from the one §4.1 states. A Product has
/// **children**, and they must stay contained in **it**: *"a narrowing that
/// would orphan a live child meets `fr-parent-child-integrity`'s
/// fail-closed check in the registered-validators phase, ahead of the
/// governance gate"*. That obligation is discharged, on the Product save
/// door, by
/// `products::check_children_stay_contained`, which
/// reads the Product's non-terminal children and judges each stored child
/// pair against the pair the save **would** leave. It reaches this module
/// for both halves of the verdict — [`sku_scope_pair`] and
/// [`scope_not_contained_domain_err`] — so the two directions cannot word
/// one refusal two ways.
///
/// # Errors
///
/// [`HeadActError::Refused`] carrying `SCOPE_NOT_CONTAINED` where the
/// child's scope is not provably a subset of the parent's — including the
/// case where the parent does not resolve at all, which is fail-closed for
/// the same reason **P-D-39** makes not-provably-subset a refusal:
/// containment that cannot be evaluated has not been established.
/// [`DomainError::ParentTerminal`] where the parent went `retired` or
/// `discarded` after the child was created, which is `create_sku`'s own
/// refusal asked a second time of a row that has since moved.
/// [`HeadActError::Db`] on a storage failure, or on a stored scope column
/// that no longer parses — a storage invariant violation rather than this
/// request's fault.
/// **Returns the parent row** so a caller needing a second fact about it —
/// `run_publish`'s publish-ordering continuation — reads it once rather than
/// twice in the same act. `run_save` drops the value: `inst-pc-ordering` is a
/// rule about a SKU reaching `published`, and the draft plane edits freely
/// under a parent that has not published yet.
async fn recheck_parent_containment(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    inputs: &HeadActInputs,
    head: &SkuRecord,
) -> Result<repo::ProductRecord, HeadActError> {
    let parent = repo::find_product(runner, &inputs.scope, inputs.tenant_id, head.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;

    let Some(parent) = parent else {
        return Err(HeadActError::Refused(DomainError::ScopeNotContained(
            format!(
                "the parent Product {} does not resolve under the caller's granted scope, so \
                 this SKU's scope cannot be proved contained in it",
                head.product_id
            ),
        )));
    };

    if parent.lifecycle_state.is_terminal() {
        return Err(HeadActError::Refused(DomainError::ParentTerminal(format!(
            "the parent Product {} is `{}`",
            parent.product_id,
            parent.lifecycle_state.as_str()
        ))));
    }

    let parent_scope = parent_scope_pair(&parent).map_err(|column| {
        head_act_internal(format!(
            "bss-products: parent Product's stored {column} contains an empty token"
        ))
    })?;

    // The child's operand is its **own stored pair**, already resolved
    // against the parent at create time (`ScopePair::resolve_child`): an
    // omitted scope was materialized into the column then, so there is
    // nothing left to inherit and re-resolving here would re-widen a child
    // back to whatever the parent now carries — turning the very narrowing
    // this function exists to catch into a silent pass.
    //
    // An `Err` takes the internal channel rather than a caller-facing
    // refusal, and each of the two callers reaches that answer by its own
    // route.
    //
    // On the publish re-run both columns parsed a few lines earlier, in
    // [`SkuScopeColumnsStillParse`], which refuses `INCOMPLETE_ENTITY`
    // first; an `Err` here therefore means the row changed under the
    // transaction, and answering that refusal a second time in a different
    // phase would file it under the wrong one.
    //
    // On the save that pipeline never runs ([`run_save`] does not call it).
    // The operand there is [`post_save_image`], whose scope columns are
    // either the head's own stored pair or a value [`parse_sku_value`] has
    // already refused an empty token in at the shape phase — so an `Err` is
    // again a stored row that no longer parses, which is this gear's
    // invariant and not this request's fault.
    let child_scope = sku_scope_pair(head).map_err(|column| {
        head_act_internal(format!(
            "bss-products: the SKU's stored {column} contains an empty token"
        ))
    })?;

    if let Err(failure) = parent_scope.check_containment(&child_scope) {
        // The identical translation `create_sku` uses, so the two doors
        // cannot word or code the same verdict differently. Its `Err` arm is
        // the `Contained`-on-a-refusal-path impossibility; it answers
        // internally here for the reason that function's own doc gives.
        let domain_err = scope_not_contained_domain_err(failure).map_err(|_| {
            head_act_internal(
                "bss-products: containment check reported Contained on a refusal path".to_owned(),
            )
        })?;
        return Err(HeadActError::Refused(domain_err));
    }

    Ok(parent)
}

/// A failure that is this gear's own rather than any caller's:
/// [`HeadActError::Db`] is the door's internal-failure channel, so a
/// storage-invariant breach rolls the transaction back and answers 5xx
/// instead of being audited as a domain refusal.
fn head_act_internal(detail: String) -> HeadActError {
    HeadActError::Db(DbError::Sea(DbErr::Custom(detail)))
}

/// The publish act itself, **every phase of it on the mutation's own
/// transaction** and in the pipeline's own phase order
/// (`crate::domain::validation::Phase`): the idempotency claim, terminality,
/// the precondition, the re-validation re-run, the containment re-check
/// against the parent as it now stands ([`recheck_parent_containment`]), the
/// edge, the governance gate, then the writes.
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
/// # The mode is an argument, not a literal
///
/// `dod-publish-door` (**P-D-30**): *"the door MUST take a gate mode as an
/// explicit argument"*. `mode` is that argument. Under [`GateMode::Gate`]
/// the host looks for a `satisfied` record and consumes it; under
/// [`GateMode::PreAuthorized`] it verifies the named record and consumes
/// nothing, which is what lets `04-lifecycle`'s scheduled-publish runner
/// drive **this** door rather than a second one — a runner forced through
/// `Gate` would meet an already-`consumed` record and 04's `inst-ar-failure`
/// would wrap that into a terminal `SCHEDULE_STALE_APPROVAL`.
///
/// §2's `inst-fd-gate-mode` calls the mode *"an internal door argument,
/// never a wire-visible parameter"*. Those are two clauses, not one, and an
/// earlier revision of this door read the second as forbidding the first —
/// which left [`GateMode::PreAuthorized`] a type with no call path anywhere
/// in the gear. Wire-invisibility is structural rather than conventional
/// here: [`GateMode`] implements no `Deserialize` — its derive list is
/// `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash`, and
/// `#[domain_model]` adds only a marker trait impl, measured at
/// `toolkit_macros`'s own expansion, 2026-08-30 — so the type cannot be
/// parsed out of a body, a query string or a header at all. Beside that it
/// reaches no request DTO, header reader or query extractor in this crate,
/// and [`publish_sku`], the only routed handler above this function, passes
/// the [`GateMode::Gate`] literal. [`publish_sku_gated`] is the in-process
/// entry point and is not routed.
///
/// The *host* is a parameter too, for [`publish_sku_gated`]'s stated
/// reason. A refusal is
/// `APPROVAL_REQUIRED` and writes nothing (`inst-fd-gate-rejection`); a host
/// that could not **reach** an answer is not a refusal and must not be
/// reported as one, which is why [`GovernanceGate::evaluate`]'s `Err` becomes
/// a bare `500` while `into_authorization`'s becomes the ceremony's refusal.
///
/// # What this act reads off the verdict, and what it does with each
///
/// Two accessors, and both have a destination.
/// `crate::domain::governance::GateAuthorization::approval_ref` goes to
/// `products_entity_version.approval_ref` through [`freeze_for`], under either
/// mode. `uncomposed_bundle_override` is §4.2's `composition_pending` operand
/// and goes to **two** places from one read: [`post_publish_image`], so the
/// frozen content carries the flag this act leaves behind, and
/// `repo::publish_sku_head`'s single `UPDATE`, which is the only statement the
/// head-row guard admits a change to that column in. An earlier revision of
/// this doc recorded the override as *"not read here at all"* because
/// `products_sku` had no such column; the column landed earlier in this phase
/// and §1.5's **In** list names *"the `PublishDoor`'s `composition_pending`
/// write"* as this slice's, so the debt is paid here rather than deferred.
///
/// `approval_to_consume` is the one accessor this act still does not read, and
/// that is slice 05's: [`NoMaterialityPolicyGate`] answers
/// `ApprovalDisposition::NoRecord`, so there is nothing to spend.
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
    mode: GateMode,
    outbox: &crate::infra::broker::EventSink,
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
        .map_err(|e| HeadActError::from_repo(&e))?
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
    if let Some((_phase, report)) =
        publish_revalidation_pipeline().run(&publish_revalidation_subject(&head))
    {
        return Err(HeadActError::Refused(revalidation_refusal(&report)));
    }

    // -- Phase 5 continued: containment, which §3.3 puts in the identity
    // phase "wherever it runs — create, save, and the publish re-run", and
    // §4.1 puts ahead of the governance gate. Its operand is the parent row,
    // so it is a read rather than a registered rule; `recheck_parent_
    // containment`'s own doc argues both. --
    let parent = recheck_parent_containment(runner, inputs, &head).await?;

    // -- Phase 5 continued: `04-lifecycle`'s publish-ordering rule
    // (`inst-pc-ordering`), as **P-D-97's continuation filling** — the operand
    // is the parent row, so it is a read rather than a registered rule, and
    // this is the position §4.1 asks for: after the pipeline, before the edge
    // and the gate. The parent is already in hand from the containment
    // re-check, so this costs no second read.
    //
    // Registered on the **target state**, not the edge (P-D-32): a re-publish
    // re-runs it fail-closed. A **terminal** parent is not this refusal —
    // `recheck_parent_containment` has already answered it `PARENT_TERMINAL`,
    // which is exactly why `parent_must_be_published` admits a terminal state.
    //
    // The registered filling of the same rule, `ParentPublishedRequired`, is
    // not wired here: `publish_revalidation_pipeline` is typed over
    // `PublishRevalidationSubject` and a `ValidationPipeline` is monomorphic
    // in its subject, so the rule's own `PublishOrderingSubject` cannot join
    // it. Both fillings delegate to this one function, so they cannot
    // diverge. --
    if let Err(refusal) = crate::domain::lifecycle::parent_must_be_published(parent.lifecycle_state)
    {
        return Err(HeadActError::Refused(DomainError::ParentNotPublished(
            refusal.detail,
        )));
    }

    // The meter recognition re-runs at publish with the head as its own
    // image: a first publish of a draft whose unit was deprecated since it
    // was authored is a NEW declaration by the PRD's reading, and this is
    // the door that catches it (`inst-mt-recognized`).
    recheck_meter_declaration(runner, inputs, &head, &head, head.published_version == 0).await?;

    // -- The edge, and what the floor says it costs. `post_publish_state`
    // decides the `to` side from the row image, the same way the head-row
    // `UPDATE`'s own `CASE` does. --
    let target = post_publish_state(head.lifecycle_state);
    let decision =
        transition::guard(head.lifecycle_state, target).map_err(HeadActError::Refused)?;

    // -- Phase 7, the governance gate, in the mode this act was entered
    // under (`inst-fd-gate-mode`): `Gate` from every wire surface,
    // `PreAuthorized` only from an in-process caller. --
    let verdict = gate
        .evaluate(
            GateSubject::entity_publish(EntityRef {
                tenant_id: inputs.tenant_id,
                entity_kind: EntityKind::Sku,
                entity_id: inputs.sku_id,
            }),
            InternalRevision::new(inputs.expected),
            mode,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    let authorization = verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    // The one operand of the verdict this act carries into the head row
    // (`inst-fd-publish-freeze`, §4.2, **P-D-32**): *"On a `bundle` SKU that
    // same `UPDATE` also carries `composition_pending` -- set where this
    // publish carried the uncomposed-bundle override, cleared where it did
    // not"*. Read once, here, and handed to **both** the image the freeze
    // renders and the statement that writes the head, because a value the two
    // did not share would freeze one flag under the key of a row carrying the
    // other.
    //
    // The `bundle` narrowing the instruction states is **not** implemented and
    // is not silently dropped: `bundle` is a value of the `type` column, which
    // is slice 03's and is not on `products_sku` at this commit, so there is
    // no operand to test. What runs is the clause with its subject widened to
    // every SKU. `repo::publish_sku_head`'s own doc records the narrowing as
    // owed to 03 and says where it lands.
    let composition_pending = authorization.uncomposed_bundle_override;

    let image = post_publish_image(&head, composition_pending, inputs.now);

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
    .map_err(|e| HeadActError::from_repo(&e))?;

    // -- b. Then exactly one head-row `UPDATE`. An `Err` rather than an
    // outcome, and the whole reason `HeadActError` exists: this rolls the
    // freeze back. --
    let written = repo::publish_sku_head(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.sku_id,
        inputs.expected,
        composition_pending,
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;
    if written == repo::HeadWrite::Unmatched {
        return Err(classify_unmatched(runner, inputs, target).await);
    }

    // -- c. The approval-invalidation hook, where the floor says this edge
    // fires one. It does not on `draft -> published`, nor on a re-publish;
    // the answer is read off `transition::guard` rather than hard-coded. --
    fire_invalidation_hook(runner, inputs, transition::invalidation_for(decision)).await?;

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
/// The one phase a discard does **not** have is as deliberate as the ones it
/// does: there is **no re-validation re-run**, because nothing is being
/// published and `inst-fd-publish-revalidate` is the publish act's clause.
///
/// # The governance-gate phase runs here, and passes trivially
///
/// §3.1's `inst-fd-pipeline-gate-phase` says the phase *"runs at every
/// mutating door and passes trivially where the act is ungated
/// (**P-D-34**)"*, and §1.1 makes governance *"a registered gate phase
/// inside the pipeline, hosting any gated act — publish or transition alike
/// (**P-D-30**) — not a separate path around it"*.
/// [`crate::domain::validation::Phase::GovernanceGate`]'s own doc carries
/// the same rule. So the phase is asked here, in [`GateMode::Gate`], and the
/// gear's default host authorizes naming no record: a discard of a
/// never-published draft consumes no approval and today requires none.
///
/// Behaviourally that is the same answer as not asking. It stops being the
/// same answer the moment slice 05 registers a ceremony on a transition,
/// which is the case the phase exists to make reachable **without reopening
/// every door** — and this door is one of the two that would have to be
/// reopened. An earlier revision cited `inst-fd-governance-gate` as the
/// authority for skipping the phase; that instruction is about the publish
/// door and does not govern this question.
///
/// The mode is the [`GateMode::Gate`] literal rather than an argument, and
/// the asymmetry with [`run_publish`] is measured rather than forgotten:
/// `dod-publish-door` requires the *publish* door to take the mode
/// explicitly because `04-lifecycle`'s scheduled-publish runner needs
/// [`GateMode::PreAuthorized`] to drive it, and no slice schedules or
/// cascades a **discard**. The host is still a parameter, for
/// [`discard_sku_gated`]'s stated reason. `products::run_discard` carries
/// the identical shape and the identical argument.
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
    gate: &(dyn GovernanceGate + Send + Sync),
    outbox: &crate::infra::broker::EventSink,
) -> Result<MutationOutcome, HeadActError> {
    if let Some(replay) = claim_for_head_act(runner, inputs).await? {
        return Ok(replay);
    }

    let head = repo::find_sku(runner, &inputs.scope, inputs.tenant_id, inputs.sku_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
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

    // -- Phase 7, the governance gate: the pipeline's last phase, asked here
    // as it is at every other mutating door (`inst-fd-pipeline-gate-phase`).
    // After the edge, because `Phase::ordered()` puts `State` before
    // `GovernanceGate`, so a `published` head is `ILLEGAL_TRANSITION` rather
    // than an approval question. The two `Err` routes are `run_publish`'s
    // and carry its reasoning. --
    let verdict = gate
        .evaluate(
            GateSubject::entity_publish(EntityRef {
                tenant_id: inputs.tenant_id,
                entity_kind: EntityKind::Sku,
                entity_id: inputs.sku_id,
            }),
            InternalRevision::new(inputs.expected),
            GateMode::Gate,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    // Collapsed into the door's control flow and then dropped, which is the
    // whole of what an ungated act does with a trivial `yes`: a discard
    // freezes no `products_entity_version` row, so the `approval_ref` the
    // verdict may name has no column to reach. The day slice 05 gates a
    // transition, the refusal arm above is already wired and only the
    // record's destination is new.
    verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

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
    .map_err(|e| HeadActError::from_repo(&e))?;
    if written == repo::HeadWrite::Unmatched {
        return Err(classify_unmatched(runner, inputs, LifecycleState::Discarded).await);
    }

    // A discard consumes no approval, so the floor says its edge fires the
    // hook — read off `transition::guard`'s own answer, not decided here.
    fire_invalidation_hook(runner, inputs, transition::invalidation_for(decision)).await?;

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
/// claim rolls back with everything after it, and its inputs are
/// attempt-independent, `now` having been stamped before the first; the
/// envelope's `event_id`, minted per enqueue, is the one value that differs
/// per attempt, and it is harmless for the reason
/// [`insert_sku_with_event`] states.
///
/// # The retry needs the driver's error, not its text
///
/// `is_retryable_contention` matches `DbErr::Exec`/`DbErr::Query` only, so
/// this section's promise holds only while the failure keeps that variant
/// all the way from the driver to [`head_act_contention_db_err`]. It is
/// [`RepoError::Driver`] that carries it and
/// [`HeadActError::from_repo`] that preserves it; a wrap through `Display`
/// anywhere on that path — which is what this door originally did, inheriting
/// it from the create door — turns every collision back into a bare `500`.
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
///
/// `mode` travels beside it, by value: [`GateMode`] is `Copy`, so every retry
/// attempt takes its own copy the way every other input does.
async fn publish_in_one_transaction(
    state: &ApiState,
    inputs: &HeadActInputs,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
    mode: GateMode,
) -> Result<MutationOutcome, HeadActError> {
    let outbox = state.sink.clone();
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
                let outbox = outbox.clone();
                let gate = Arc::clone(&gate);
                let inputs = inputs.clone();
                Box::pin(
                    async move { run_publish(tx, &inputs, gate.as_ref(), mode, &outbox).await },
                )
            },
        )
        .await
}

/// [`publish_in_one_transaction`]'s discard twin, on the same terms —
/// including the gate host, which travels as an `Arc` for that function's
/// stated reason now that the phase runs on this door too
/// (`inst-fd-pipeline-gate-phase`). The mode does not travel: it is
/// [`GateMode::Gate`] inside [`run_discard`], which argues why.
///
/// # Errors
///
/// See [`run_discard`].
async fn discard_in_one_transaction(
    state: &ApiState,
    inputs: &HeadActInputs,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<MutationOutcome, HeadActError> {
    let outbox = state.sink.clone();
    let gate = Arc::clone(gate);
    let inputs = inputs.clone();
    state
        .db
        .db()
        .transaction_with_retry::<MutationOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let gate = Arc::clone(&gate);
                let inputs = inputs.clone();
                Box::pin(async move { run_discard(tx, &inputs, gate.as_ref(), &outbox).await })
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
/// The `digest` is the **act's own**, and it is a parameter rather than a
/// call this function makes for itself. A publish and a discard pass
/// [`bodiless_payload_digest`] — a constant, the two operands that tell one
/// such act from another being already in the key: the entity id through
/// `endpoint`'s concrete path (**P-D-42**) and the caller's own key beside
/// it. A **save** carries a request body and passes
/// [`save_payload_digest`], because two different saves of one head under
/// one client key must be an `IDEMPOTENCY_CONFLICT` and not a replay of
/// each other. The `If-Match` revision is deliberately **not** an operand of
/// either, and [`bodiless_payload_digest`]'s doc gives `inst-fd-idem-hash`'s
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
    digest: Vec<u8>,
    now: DateTime<Utc>,
) -> Result<Option<IdempotencyClaimInput>, DomainError> {
    let client_key = idempotency_key(headers)?;
    Ok(client_key.map(|key| {
        IdempotencyClaimInput::new(
            endpoint,
            key,
            digest,
            now,
            state.idempotency_retention_hours,
        )
    }))
}

/// `POST /skus/{id}/publish`, with the governance gate host **and the gate
/// mode** as arguments — the in-process entry point every non-wire caller of
/// this door uses.
///
/// The handler [`publish_sku`] passes [`NoMaterialityPolicyGate`], which is
/// the only host that exists at this commit, and [`GateMode::Gate`], which
/// is the only mode a wire surface may ever pass. The **host** parameter is
/// what lets a test drive a refusing host, since the default one never
/// refuses under [`GateMode::Gate`] and an `APPROVAL_REQUIRED` path with no
/// test would be an untested one. The **mode** parameter is
/// `dod-publish-door`'s own requirement (**P-D-30**) and the seam
/// `04-lifecycle`'s scheduled-publish runner arrives through; see
/// [`run_publish`]'s doc, which also records that an earlier revision read
/// `inst-fd-gate-mode` as forbidding it. Neither is a seam for a wire
/// caller: this function is not routed, and [`GateMode`] reaches no DTO,
/// header reader or extractor in the crate. Both travel as owned values for
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
///    containment re-check against the parent, the edge, the gate, the
///    freeze, one head-row `UPDATE`, `SkuPublished`, the invalidation hook
///    and the stored answer — one transaction.
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
/// - **The `bundle` narrowing on `composition_pending`.** The write itself is
///   built — [`run_publish`] reads the verdict's uncomposed-bundle override
///   and carries it into the frozen image and the head-row `UPDATE` alike —
///   but `inst-fd-publish-freeze` scopes the clause to a **`bundle`** SKU, and
///   `bundle` is a value of the `type` column, which is **slice 03's** and is
///   not on `products_sku` at this commit. The clause therefore runs with its
///   subject widened to every SKU. Owed to slice **03**; see
///   `repo::publish_sku_head`'s own doc for where the condition lands.
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
    mode: GateMode,
) -> Result<Response, CanonicalError> {
    let now = canonical::write_instant(Utc::now());
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

    let claim = match build_claim(
        state,
        headers,
        publish_endpoint(sku_id),
        bodiless_payload_digest(),
        now,
    ) {
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

    let outcome = publish_in_one_transaction(state, &inputs, gate, mode).await;
    answer_head_act(state, &act, sku_id, head.internal_revision, outcome).await
}

/// `POST /skus/{id}/publish`: freeze version N+1 and move the head, in one
/// transaction.
///
/// The thin `axum` shell over [`publish_sku_gated`], which carries the whole
/// pipeline and its reasoning. The only things decided here are the two a
/// wire request may not choose — the governance host and the gate mode —
/// and they are decided the same way: [`NoMaterialityPolicyGate`] and
/// [`GateMode::Gate`], both literals at this one call site.
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
        GateMode::Gate,
    )
    .await
}

/// `POST /skus/{id}/discard`: discard a never-published draft
/// (`inst-fd-discard`).
///
/// The thin `axum` shell over [`discard_sku_gated`]. The only thing decided
/// here is the governance host, and it is decided the way [`publish_sku`]
/// decides it: [`NoMaterialityPolicyGate`] as a literal, so no wire input
/// chooses one.
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
/// See [`discard_sku_gated`].
async fn discard_sku(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
    Path(sku_id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    discard_sku_gated(
        &state,
        &enforcer,
        &ctx,
        &headers,
        sku_id,
        &(Arc::new(NoMaterialityPolicyGate) as Arc<dyn GovernanceGate + Send + Sync>),
    )
    .await
}

/// `POST /skus/{id}/discard`, with the governance gate host as an argument —
/// [`publish_sku_gated`]'s twin, minus the mode.
///
/// The door's own steps are [`publish_sku_gated`]'s: the `sku x write`
/// grant, the key, the `If-Match`, the head read, then [`run_discard`] on
/// one transaction and [`answer_head_act`]. The legality rule, the effects
/// the guard reports and the reservation the write releases are all argued
/// at [`run_discard`].
///
/// # Why the host is a parameter here too
///
/// The gate phase runs on this door (`inst-fd-pipeline-gate-phase`; see
/// [`run_discard`]), and the gear's only host never refuses under
/// [`GateMode::Gate`]. That makes the phase's refusal arm unreachable
/// through [`discard_sku`] and therefore untestable at the door — the
/// identical argument [`publish_sku_gated`] makes for its own host
/// parameter — and a phase nothing can exercise is one a reader cannot tell
/// from a phase that is absent. This seam is also where slice 05's host
/// arrives the day it gates a transition. The **mode** is not a parameter,
/// and [`run_discard`]'s own doc measures that asymmetry.
///
/// # Errors
///
/// The audited `VALIDATION`, `STALE_REVISION`, `ENTITY_TERMINAL`,
/// `ILLEGAL_TRANSITION`, `APPROVAL_REQUIRED` or idempotency refusal, the
/// `404` a miss answers, or a `500` from storage or an unreachable gate
/// host.
async fn discard_sku_gated(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    headers: &HeaderMap,
    sku_id: Uuid,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<Response, CanonicalError> {
    let now = canonical::write_instant(Utc::now());
    let act = open_act(
        state,
        enforcer,
        ctx,
        sku_id,
        crate::authz::actions::WRITE,
        DISCARD_AUDIT_ACTION,
        now,
    )
    .await?;

    let claim = match build_claim(
        state,
        headers,
        discard_endpoint(sku_id),
        bodiless_payload_digest(),
        now,
    ) {
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
        // The caller's own `If-Match`, for the reason `publish_sku_gated`
        // states at its own copy of this field.
        expected: expected.get(),
        now,
        claim,
    };

    let outcome = discard_in_one_transaction(state, &inputs, gate).await;
    answer_head_act(state, &act, sku_id, head.internal_revision, outcome).await
}

/// The `products_audit_log.action` token every **save** refusal on this door
/// is recorded under. `products::SAVE_AUDIT_ACTION` is the Product door's
/// identical constant, and the two must stay equal: an operator filtering the
/// trail by `action = 'save'` is asking one question of both entity kinds.
const SAVE_AUDIT_ACTION: &str = "save";

/// The concrete resource path a save claims its idempotency key under
/// (**P-D-42**), on [`publish_endpoint`]'s terms — the same string [`router`]
/// registers the `PATCH` at, with `{id}` resolved. A save needs no act suffix
/// to tell it from a read: the method already does.
fn save_endpoint(sku_id: Uuid) -> String {
    format!("/bss-products/v1/skus/{sku_id}")
}

/// `PATCH /bss-products/v1/skus/{id}` request body: **the named field set,
/// and nothing around it** — `products::SaveProductRequest`'s twin, and one
/// decision with it.
///
/// The map shape is not a stylistic echo. Five `Option` fields could tell an
/// omitted field from a sent one but not either from a field this door does
/// not know: `#[toolkit_macros::api_dto(request)]` adds no
/// `#[serde(deny_unknown_fields)]`, so an unrecognized key on a typed `DTO`
/// is silently dropped — and P-D-50's fail-closed rule, which exists to
/// refuse a published-state column carrying no bucket tag, would then have
/// nothing to fire on. The Product door's own `DTO` doc carries the argument
/// in full.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct SaveSkuRequest {
    /// Every field the request named, keyed as the caller spelled it.
    ///
    /// `#[serde(flatten)]`, so the wire shape is the flat object a `PATCH`
    /// caller expects. A [`BTreeMap`](std::collections::BTreeMap) so the
    /// iteration order is the field names' own: the idempotency digest is
    /// taken over this object, and a key-order-dependent digest would hash
    /// one request two ways between processes.
    #[serde(flatten)]
    pub fields: std::collections::BTreeMap<String, JsonValue>,
}

/// The digest of one parsed save request (**P-D-34**), on
/// `products::save_payload_digest`'s terms exactly: the field set itself,
/// through `crate::domain::idempotency::payload_digest`, with nothing of the
/// transport in it and structurally no `If-Match`, this function never being
/// handed the headers.
fn save_payload_digest(request: &SaveSkuRequest) -> Vec<u8> {
    idempotency::payload_digest(&JsonValue::Object(
        request
            .fields
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    ))
}

/// A field the SKU save door **accepts on the wire**, and the column it
/// names — `products::ProductSaveField`'s twin, and a second registry beside
/// `crate::domain::bucket`'s for that type's stated reason: that one answers
/// *what class is this column in*, this one *may a caller author it at all*.
///
/// Two differences from the Product's set, both the schema's. There is **no
/// `name`** — `products_sku` has no such column, so a `name` field on this
/// door is a registry miss and is refused by the fail-closed rule rather than
/// routed to the Product's tag. And `product_id` is here as the **parent
/// link**, bucket-i by the owner's call of 2026-08-27 (*"re-parenting changes
/// whose SKU it is, not how it is described"*), where the identically named
/// column on `products_product` is the primary key and is admitted in no
/// `UPDATE` at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkuSaveField {
    /// Bucket i: the code `uq_products_sku_code` reserves.
    SkuCode,
    /// Bucket i: the parent link.
    ProductId,
    /// Bucket iii, in both directions.
    RegionScope,
    /// Bucket iii, in both directions.
    BrandScope,
    /// Bucket ii: half of 03's atomic `MeterDeclaration`.
    MeteringUnit,
    /// Bucket ii: the declaration's other half.
    UsageTypeRef,
}

impl SkuSaveField {
    /// The wire field this is, or `None` where the caller named something
    /// this door does not author — a refusal ([`unroutable_sku_field`]),
    /// never a silent drop.
    fn from_wire(field: &str) -> Option<Self> {
        match field {
            "sku_code" => Some(Self::SkuCode),
            "product_id" => Some(Self::ProductId),
            "region_scope" => Some(Self::RegionScope),
            "brand_scope" => Some(Self::BrandScope),
            "metering_unit" => Some(Self::MeteringUnit),
            "usage_type_ref" => Some(Self::UsageTypeRef),
            _ => None,
        }
    }

    /// The physical column, as `products_sku` spells it and as
    /// `crate::domain::bucket`'s registry keys on it.
    const fn column(self) -> &'static str {
        match self {
            Self::SkuCode => "sku_code",
            Self::ProductId => "product_id",
            Self::RegionScope => "region_scope",
            Self::BrandScope => "brand_scope",
            Self::MeteringUnit => "metering_unit",
            Self::UsageTypeRef => "usage_type_ref",
        }
    }
}

/// One save field's parsed value — the `Shape` phase's output, before the
/// `State` phase has said whether the column may be written at all.
///
/// Two passes rather than one because
/// `crate::domain::validation::Phase::ordered()` puts `Shape` **before**
/// `State`: a body carrying both a malformed value and an unroutable field is
/// a `VALIDATION`, not an `ILLEGAL_FIELD_MUTATION`.
enum SkuSaveValue {
    /// A parsed, non-blank `sku_code`.
    SkuCode(String),
    /// A parsed `product_id`.
    ProductId(Uuid),
    /// A `region_scope` that parses under [`ResolvedScope::parse`].
    RegionScope(String),
    /// A `brand_scope` that parses under [`ResolvedScope::parse`].
    BrandScope(String),
    /// A non-blank metering unit code, membership judged at the door.
    MeteringUnit(String),
    /// A non-blank usage-type reference — the pair's other half.
    UsageTypeRef(String),
}

/// The `Shape` phase's output — `products::ProductSaveFields`'s twin, and a
/// named alias for that type's stated reason.
type SkuSaveFields = (Vec<(SkuSaveField, SkuSaveValue)>, Vec<String>);

/// Build the `VALIDATION` refusal one or more shape violations ride.
fn sku_shape_refusal(violations: Vec<(String, String)>) -> DomainError {
    let mut report = ValidationReport::new();
    for (subject, detail) in violations {
        report.violate("VALIDATION", &subject, &detail);
    }
    DomainError::Validation(report)
}

/// Read a save field's `JSON` value as a string, or name the shape violation.
fn expect_sku_string(field: &str, value: &JsonValue) -> Result<String, (String, String)> {
    value.as_str().map(str::to_owned).ok_or_else(|| {
        (
            field.to_owned(),
            format!("{field} must be a JSON string on this door"),
        )
    })
}

/// Parse one recognized save field's value (`Phase::Shape`).
///
/// The two scope fields are parsed through [`ResolvedScope::parse`] and not
/// merely read as strings, which is the create door's own rule
/// (`scope_input_from_payload`): a value carrying an empty token — `","`,
/// `"eu,,us"` — is refused rather than silently filtered, so the stored
/// column cannot hold one. The parsed value is discarded and the raw string
/// stored, because the column holds the caller's own spelling; what the parse
/// buys is the refusal.
///
/// `sku_code` is the one field stored **trimmed** rather than as spelled, and
/// that is `create_sku`'s own rule reaching the only other door that writes
/// the column. `uq_products_sku_code` is a byte-comparing partial unique
/// index over `(tenant_id, sku_code)`, so a save storing `" SKU-1 "` would not
/// collide with a live `"SKU-1"` and two rows would hold what an operator
/// reads as one code -- one of them a value no create door could produce.
/// Trimming here is what keeps `fr-skucode-reservation-concurrency`'s
/// reservation the same reservation at both doors. The blank-after-trim
/// refusal stays above it: a code that is all whitespace is not a code.
fn parse_sku_value(
    field: SkuSaveField,
    value: &JsonValue,
) -> Result<SkuSaveValue, (String, String)> {
    let wire = field.column();
    match field {
        SkuSaveField::SkuCode => {
            let raw = expect_sku_string(wire, value)?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err((
                    wire.to_owned(),
                    "sku_code must contain at least one non-whitespace character".to_owned(),
                ));
            }
            Ok(SkuSaveValue::SkuCode(trimmed.to_owned()))
        }
        SkuSaveField::ProductId => {
            let raw = expect_sku_string(wire, value)?;
            Uuid::parse_str(&raw)
                .map(SkuSaveValue::ProductId)
                .map_err(|_| (wire.to_owned(), "product_id must be a UUID".to_owned()))
        }
        SkuSaveField::MeteringUnit | SkuSaveField::UsageTypeRef => {
            let raw = expect_sku_string(wire, value)?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err((
                    wire.to_owned(),
                    format!("{wire} must contain at least one non-whitespace character"),
                ));
            }
            Ok(match field {
                SkuSaveField::MeteringUnit => SkuSaveValue::MeteringUnit(trimmed.to_owned()),
                _ => SkuSaveValue::UsageTypeRef(trimmed.to_owned()),
            })
        }
        SkuSaveField::RegionScope | SkuSaveField::BrandScope => {
            let raw = expect_sku_string(wire, value)?;
            if ResolvedScope::parse(&raw).is_err() {
                return Err((
                    wire.to_owned(),
                    format!("{wire} contains an empty value between separators"),
                ));
            }
            Ok(match field {
                SkuSaveField::RegionScope => SkuSaveValue::RegionScope(raw),
                _ => SkuSaveValue::BrandScope(raw),
            })
        }
    }
}

/// The `Phase::Shape` pass over the whole request: every recognized field
/// parsed, every unrecognized name collected for the `State` phase.
///
/// Both halves are collected before either is answered, so a body with two
/// malformed values reports both (P-D-37: the caller receives every
/// violation, the audit row records one code).
///
/// # Errors
///
/// [`DomainError::Validation`] naming each malformed field.
fn parse_sku_save(request: &SaveSkuRequest) -> Result<SkuSaveFields, DomainError> {
    let mut parsed = Vec::new();
    let mut unrecognized = Vec::new();
    let mut violations = Vec::new();
    for (field, value) in &request.fields {
        match SkuSaveField::from_wire(field) {
            Some(known) => match parse_sku_value(known, value) {
                Ok(parsed_value) => parsed.push((known, parsed_value)),
                Err(violation) => violations.push(violation),
            },
            None => unrecognized.push(field.clone()),
        }
    }
    if violations.is_empty() {
        Ok((parsed, unrecognized))
    } else {
        Err(sku_shape_refusal(violations))
    }
}

/// The refusal a field name this door does not author is answered with —
/// `products::unroutable_product_field`'s twin, asking the registry for the
/// same two readings: **no row** is P-D-50's fail-closed miss, carrying
/// `bucket::classify`'s own message; **a row this door does not accept from a
/// caller** is the mechanical and row-identity set, `cloned_from`'s
/// `CreateOnly` when slice 11 lands it, and any column whose value the gear
/// derives.
fn unroutable_sku_field(field: &str) -> DomainError {
    match bucket::classify(EntityKind::Sku, field) {
        Err(miss) => miss,
        Ok(_) => DomainError::IllegalFieldMutation(format!(
            "SKU column {field} is not authored through this door: it is written by the gear \
             itself or derived from another field, and no save may name it"
        )),
    }
}

/// Route one parsed field through `crate::domain::bucket` and fold it into
/// `save` — the `Phase::State` half, and `products::route_product_field`'s
/// twin arm for arm.
///
/// # Errors
///
/// [`HeadActError::Refused`] for every bucket refusal, and
/// [`HeadActError::Db`] — the gear's internal channel, a `500` — for
/// [`bucket::FieldClass::Outside`], which is **structurally unreachable**:
/// [`SkuSaveField::from_wire`] admits six wire names and every one of their
/// columns is bucket-tagged, so a value reaching that arm means this door's
/// own field table and the registry disagree. The provenance is the gear's
/// rather than the caller's, which is what decides the channel.
fn route_sku_field(
    head: &SkuRecord,
    field: SkuSaveField,
    value: SkuSaveValue,
    save: &mut repo::SkuHeadSave,
) -> Result<(), HeadActError> {
    let column = field.column();
    let class = bucket::classify(EntityKind::Sku, column).map_err(HeadActError::Refused)?;
    let published = head.published_version > 0;
    match class {
        bucket::FieldClass::Bucket(bucket::FieldBucket::Structural) if published => {
            return Err(HeadActError::Refused(sku_structural_after_publish(column)));
        }
        bucket::FieldClass::Bucket(bucket::FieldBucket::Correctable) if published => {
            return Err(HeadActError::Refused(sku_correctable_after_publish(column)));
        }
        bucket::FieldClass::CreateOnly => {
            return Err(HeadActError::Refused(DomainError::IllegalFieldMutation(
                format!(
                    "SKU {column} is create-only: it is writable in the creating statement and \
                     in no update at all, so the lineage stays evidence rather than a claim"
                ),
            )));
        }
        bucket::FieldClass::Outside(reason) => {
            return Err(head_act_internal(format!(
                "bss-products: the save door's wire field {column} resolves to a column outside \
                 the bucket scheme ({reason:?}); the door's own field table and the registry \
                 disagree"
            )));
        }
        bucket::FieldClass::Bucket(_) => {}
    }

    match value {
        SkuSaveValue::SkuCode(code) => save.sku_code = Some(code),
        SkuSaveValue::ProductId(product_id) => save.product_id = Some(product_id),
        SkuSaveValue::RegionScope(scope) => save.region_scope = Some(scope),
        SkuSaveValue::BrandScope(scope) => save.brand_scope = Some(scope),
        SkuSaveValue::MeteringUnit(unit) => save.metering_unit = Some(unit),
        SkuSaveValue::UsageTypeRef(usage) => save.usage_type_ref = Some(usage),
    }
    Ok(())
}

/// The refusal a bucket-ii write after first publish is answered with
/// (`inst-fd-bucket-ii-refusal`) — `products::correctable_after_publish`'s
/// twin, differing only in the entity it names.
///
/// **It names slice 07's correction door and does not forward to it**: the
/// instruction is explicit that the head door refuses rather than forwards —
/// one door, one effect — so a caller is told where the act belongs rather
/// than having this door quietly perform a differently-governed act on its
/// behalf. **The arm became reachable with 03's meter pair** — bucket ii's
/// first membership, `metering_unit` and `usage_type_ref` — and is probed at
/// this door. It was built before it had a member, because the door routes
/// by tag; that foresight is why the pair's arrival needed no second change
/// here.
fn sku_correctable_after_publish(field: &str) -> DomainError {
    let tag = bucket::FieldBucket::Correctable.tag();
    DomainError::IllegalFieldMutation(format!(
        "SKU {field} is a bucket-{tag} correctable column: after first publish it is writable \
         only through the correction door (POST .../corrections, slice 07), which this door \
         names rather than forwards to"
    ))
}

/// The refusal a bucket-i write after first publish is answered with
/// (`inst-fd-bucket-i-refusal`) — `products::structural_after_publish`'s
/// twin, differing only in the entity it names, and taking the numeral from
/// [`bucket::FieldBucket::tag`] for the reason that function's doc gives.
fn sku_structural_after_publish(field: &str) -> DomainError {
    let tag = bucket::FieldBucket::Structural.tag();
    DomainError::IllegalFieldMutation(format!(
        "SKU {field} is a bucket-{tag} identity column: it is writable only before first \
         publish, and a mis-set identity on a published entity is corrected by retire-and-clone \
         rather than by a write"
    ))
}

/// Route **every** field the request carries, and only then hand back the
/// columns to write (`Phase::State`) — `products::route_product_save`'s twin.
///
/// The whole-request discipline is the point: a `PATCH` half-applied because
/// its fourth field was refused would leave the head carrying part of a
/// request the caller was told had failed. Nothing here writes.
///
/// # Errors
///
/// See [`route_sku_field`] and [`unroutable_sku_field`].
fn route_sku_save(
    head: &SkuRecord,
    parsed: Vec<(SkuSaveField, SkuSaveValue)>,
    unrecognized: &[String],
) -> Result<repo::SkuHeadSave, HeadActError> {
    if let Some(field) = unrecognized.first() {
        return Err(HeadActError::Refused(unroutable_sku_field(field)));
    }
    let mut save = repo::SkuHeadSave::default();
    for (field, value) in parsed {
        route_sku_field(head, field, value, &mut save)?;
    }
    Ok(save)
}

/// The meter rules over the row a save would produce, and over a first
/// publish — 03's `inst-mt-atomic-pair` and `inst-mt-recognized`
/// (`dod-meter-atomic`, `dod-unit-recognition`).
///
/// # What counts as a NEW declaration
///
/// The recognition rules bite on a **new** declaration only: an existing
/// published carrier keeps resolving against a `deprecated` unit. New means
/// the image's unit differs from the head's — a save that names the unit the
/// row already carries re-declares nothing — and, at first publish, a
/// `draft` whose unit was deprecated after it was authored, which the PRD
/// treats as a new declaration and rejects. The publish caller passes the
/// head as both arguments and `first_publish = true` for exactly that arm.
///
/// **`first_publish` is a parameter, not `head.published_version == 0`.**
/// Deriving it made every draft save re-judge, so a draft whose unit was
/// deprecated after it was authored could not be edited at all — a
/// `sku_code` change came back `UNIT_DEPRECATED`, naming a declaration the
/// caller never made, while `03` §1.6 has the draft plane editing freely.
/// Only the publish door raises the flag.
///
/// The atomic-pair rule reads the **resulting row**, so a save supplying one
/// half onto a row already carrying the other completes a declaration rather
/// than being refused for arriving alone; the paired `CHECK` refuses the
/// same shape at the physical layer, this door's answer carrying the code.
async fn recheck_meter_declaration(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    inputs: &HeadActInputs,
    head: &SkuRecord,
    image: &SkuRecord,
    first_publish: bool,
) -> Result<(), HeadActError> {
    crate::domain::recognized::meter_pair_complete(
        image.metering_unit.as_deref(),
        image.usage_type_ref.as_deref(),
    )
    .map_err(HeadActError::Refused)?;

    let Some(unit) = image.metering_unit.as_deref() else {
        return Ok(());
    };
    let newly_declared = head.metering_unit.as_deref() != Some(unit);
    if !(newly_declared || first_publish) {
        return Ok(());
    }

    let member = repo::recognized_member(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        crate::domain::recognized::SetKind::MeteringUnit,
        unit,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;
    crate::domain::recognized::declaration_verdict(unit, member.map(|m| m.state))
        .map_err(HeadActError::Refused)
}

/// The head as this save would leave it — the operand the identity phase
/// judges.
///
/// Built rather than re-read because the row this describes has not been
/// written yet: the containment re-check below has to judge the scope the
/// save **would** store, not the one it is replacing. It is deliberately not
/// the operand of the door's *answer*: that is re-read off the committed row
/// ([`run_save`]), so the client is told what the database holds rather than
/// what this door believes it wrote.
fn post_save_image(head: &SkuRecord, save: &repo::SkuHeadSave, now: DateTime<Utc>) -> SkuRecord {
    let mut image = head.clone();
    if let Some(sku_code) = save.sku_code.clone() {
        image.sku_code = sku_code;
    }
    if let Some(product_id) = save.product_id {
        image.product_id = product_id;
    }
    if let Some(region_scope) = save.region_scope.clone() {
        image.region_scope = region_scope;
    }
    if let Some(brand_scope) = save.brand_scope.clone() {
        image.brand_scope = brand_scope;
    }
    if let Some(metering_unit) = save.metering_unit.clone() {
        image.metering_unit = Some(metering_unit);
    }
    if let Some(usage_type_ref) = save.usage_type_ref.clone() {
        image.usage_type_ref = Some(usage_type_ref);
    }
    image.internal_revision = head.internal_revision + 1;
    image.updated_at = now;
    image
}

/// Which refusal a zero-row **save** write was, re-read under the act's own
/// transaction — `products::classify_unmatched_save`'s twin.
///
/// [`repo::save_sku_head`]'s filter carries four conditions, so `Unmatched`
/// has four readings, read here in the order the caller can act on: a moved
/// revision first, then terminality, then a bucket-i write the row was
/// published under. The last arm is the read-then-write race the filter
/// exists to close.
async fn classify_unmatched_save(
    runner: &impl toolkit_db::secure::DBRunner,
    inputs: &HeadActInputs,
    structural: bool,
) -> HeadActError {
    match repo::find_sku(runner, &inputs.scope, inputs.tenant_id, inputs.sku_id).await {
        Ok(Some(head)) if head.internal_revision != inputs.expected => {
            HeadActError::Refused(DomainError::StaleRevision {
                expected: inputs.expected,
                found: head.internal_revision,
            })
        }
        Ok(Some(head)) if head.lifecycle_state.is_terminal() => {
            HeadActError::Refused(DomainError::EntityTerminal(format!(
                "no head write is admitted on a {} entity",
                head.lifecycle_state.as_str()
            )))
        }
        Ok(Some(head)) if structural && head.published_version > 0 => {
            HeadActError::Refused(sku_structural_after_publish("identity column"))
        }
        Ok(Some(head)) => head_act_internal(format!(
            "save matched no row for sku {} at revision {}, yet the head is {} at revision {}",
            head.sku_id,
            inputs.expected,
            head.lifecycle_state.as_str(),
            head.internal_revision
        )),
        Ok(None) => HeadActError::Vanished,
        Err(error) => HeadActError::from_repo(&error),
    }
}

/// Turn a save's storage failure into the refusal it actually was, where the
/// driver's own text names `uq_products_sku_code`.
///
/// §3.3 puts `DUPLICATE_CODE` in the identity phase *"wherever it runs —
/// create, save, and the publish re-run"*, so a re-code onto a held `skuCode`
/// is the same governed refusal here as at create rather than a `500`.
/// [`classify_sku_insert_conflict`] is the create door's own reader of that
/// text, reused unchanged — including its stated cost, that this is a
/// substring match over driver text and not a typed database answer. It
/// answers a `bool` rather than the Product door's two-armed enum because
/// `products_sku` carries exactly one unique index.
fn sku_save_conflict(error: &RepoError) -> HeadActError {
    if classify_sku_insert_conflict(&error.to_string()) {
        return HeadActError::Refused(DomainError::DuplicateCode(
            "another live SKU in this tenant already holds this skuCode".to_owned(),
        ));
    }
    HeadActError::from_repo(error)
}

/// The save act itself, every phase on the mutation's own transaction and in
/// `crate::domain::validation::Phase::ordered()`'s order
/// (`cpt-cf-bss-products-dod-save-door`) — `products::run_save`'s twin,
/// phase for phase, plus the one phase a child has and a parentless entity
/// does not.
///
/// # The order, and the one place it differs from [`run_publish`]
///
/// `Idempotency`, `Precondition`, `Shape`, `State` (terminality, then bucket
/// routing), `Identity` (containment against the parent as it now stands),
/// `RegisteredValidators`, `GovernanceGate`.
///
/// [`run_publish`] and [`run_discard`] ask **terminality before the
/// precondition**; `Phase::ordered()` puts `Precondition` second and `State`
/// fourth, and terminality is a `State` rule. The two orders answer
/// differently in exactly one case — a stale `If-Match` against a head a
/// neighbour has since retired, which this door calls `STALE_REVISION` and
/// the publish door calls `ENTITY_TERMINAL` — and `STALE_REVISION` is the
/// answer the caller can act on. **The publish and discard doors are owed the
/// same swap**, on both entity kinds; it is not made here because those doors
/// are not this slice's subject. `products::run_save` carries the identical
/// note.
///
/// # The containment re-check, and the Product save's mirror of it
///
/// §3.3 puts `SCOPE_NOT_CONTAINED` in the identity phase *"wherever it runs —
/// create, **save**, and the publish re-run"*, and §4.1 puts
/// `region_scope`/`brand_scope` in bucket iii *"in both directions, widening
/// and narrowing alike"*. A save is therefore the one door that can widen a
/// child out of its parent's scope, and [`recheck_parent_containment`] — the
/// publish door's own function, reused rather than restated so the two
/// cannot word the same verdict differently — is asked over
/// [`post_save_image`], the scope this save **would** store.
///
/// `products::run_save` asks the mirror of it, not nothing. A Product has no
/// parent, so it re-checks *itself* against nobody; but §4.1's clause is
/// about its **children**, and a Product save that narrows either scope
/// column can orphan them. `products::check_children_stay_contained` is that
/// half, and it reads this module's [`sku_scope_pair`] and
/// [`scope_not_contained_domain_err`] rather than restating either. The
/// asymmetry is only in the direction of the read: one door loads one
/// parent, the other loads the live children.
///
/// # One head-row `UPDATE`, no version row, no edge, and the hook fires
///
/// The head is the authoring surface in every non-terminal state
/// (`inst-fd-transition-guard`), so a save writes no
/// `products_entity_version` row and does not move `published_version`. It
/// takes no edge, so [`transition::guard`] is not asked. And the
/// approval-invalidation hook **fires**, on `ApprovalInvalidation::Fire`
/// passed directly rather than read off [`transition::invalidation_for`]:
/// that function answers `Skip` for the `NotATransition` arm a save would
/// land on, and it answers it for a **re-publish**, whose exception is *"a
/// transition that consumes an approval in the same transaction"*. A save
/// consumes none, so the exception's reason does not reach it —
/// `transition::invalidation_for`'s own doc says so in as many words, and a
/// later reader who unified the two call sites would silently drop the
/// invalidation this `DoD` requires.
///
/// # Owed: the `state` phase short-circuits where §3.3 collects
///
/// §3.3 uses **a save** as its worked example -- *"a save on a `retired` head
/// that also moves a bucket-i column satisfying `ENTITY_TERMINAL` and
/// `ILLEGAL_FIELD_MUTATION` alike ... the caller's rejection carries all of
/// them regardless; the precedence governs the one code the row stores"* --
/// and §3.1 names `state` as the only phase that may raise more than one
/// code. This door does not: terminality `?`-returns before the routing runs,
/// so a save satisfying both answers `ENTITY_TERMINAL` alone.
///
/// **It is left owed rather than built, and the reason is a measurement of
/// the wire type, not a judgement about effort.** Both codes are 409s and a
/// 409 is `toolkit_canonical_errors`' `Aborted`, whose whole context is one
/// `reason: String` (`AbortedV1`, and `with_reason` is the single builder
/// step that reaches it). There is no second slot for a second code, so
/// "carries all of them" cannot be expressed on this response at all without
/// either changing a shared platform type or demoting the joint refusal to
/// the `Validation` envelope -- which is the only multi-code shape the gear
/// has and which would answer 400 where §3.3 requires 409. Overloading
/// `detail` with the second code would not serve it either: a consumer
/// matches `reason`, exactly as `infra::error_mapping`'s `denied` doc argues
/// for `APPROVAL_REQUIRED`.
///
/// So the clause needs a carrier decided by the taxonomy's owner -- a
/// multi-code refusal shape, or §3.3's clause narrowed to the audit row's
/// precedence alone -- and this door adopts it when there is one. The audit
/// row is already correct under either reading: `ENTITY_TERMINAL` is the
/// highest-precedence code of the pair and it is what the row records today.
///
/// # Errors
///
/// As [`run_publish`], with `ILLEGAL_FIELD_MUTATION` and `DUPLICATE_CODE`
/// added and `APPROVAL_REQUIRED` reachable only through a host
/// [`save_sku_gated`] is handed.
async fn run_save(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    inputs: &HeadActInputs,
    request: &SaveSkuRequest,
    gate: &(dyn GovernanceGate + Send + Sync),
    outbox: &crate::infra::broker::EventSink,
) -> Result<MutationOutcome, HeadActError> {
    // -- Phase 1, idempotency: the claim, and the replay that ends the act
    // before any other phase is judged. --
    if let Some(replay) = claim_for_head_act(runner, inputs).await? {
        return Ok(replay);
    }

    let head = repo::find_sku(runner, &inputs.scope, inputs.tenant_id, inputs.sku_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    // -- Phase 2, the precondition. `repo::save_sku_head` carries the same
    // comparison in its own filter and that copy decides whether the write
    // lands; this one decides whether the rest of the pipeline runs. --
    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    // -- Phase 3, shape: the JSON types and the two scope parses, every
    // violation collected. --
    let (parsed, unrecognized) = parse_sku_save(request).map_err(HeadActError::Refused)?;

    // -- Phase 4, state: terminality — which reaches every head write and not
    // only a transition (`inst-fd-terminal`, P-D-25 widened by P-D-32) — then
    // bucket routing over the whole request before any column is written. --
    transition::check_head_write(head.lifecycle_state).map_err(HeadActError::Refused)?;
    let save = route_sku_save(&head, parsed, &unrecognized)?;

    // -- Phase 5, identity: containment against the parent as it now stands,
    // judged over the image this save would leave. --
    let image = post_save_image(&head, &save, inputs.now);
    recheck_parent_containment(runner, inputs, &image).await?;
    recheck_meter_declaration(runner, inputs, &head, &image, false).await?;

    // -- Phase 7, the governance gate, in `Gate` mode: asked at every
    // mutating door and passing trivially where the act is ungated
    // (`inst-fd-pipeline-gate-phase`). The two `Err` routes are
    // `run_publish`'s and carry its reasoning. --
    let verdict = gate
        .evaluate(
            GateSubject::entity_publish(EntityRef {
                tenant_id: inputs.tenant_id,
                entity_kind: EntityKind::Sku,
                entity_id: inputs.sku_id,
            }),
            InternalRevision::new(inputs.expected),
            GateMode::Gate,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    // Collapsed into the control flow and dropped, as at the discard door: a
    // save freezes no version row, so the `approval_ref` the verdict may name
    // has no column to reach.
    verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    // -- Exactly one head-row `UPDATE`: the routed columns, the revision bump
    // and `updated_at` together, because the guard bumps `internal_revision`
    // on every admitted `UPDATE` without exception. --
    let structural = save.sku_code.is_some() || save.product_id.is_some();
    let written = repo::save_sku_head(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.sku_id,
        inputs.expected,
        &save,
        inputs.now,
    )
    .await
    .map_err(|e| sku_save_conflict(&e))?;
    if written == repo::HeadWrite::Unmatched {
        return Err(classify_unmatched_save(runner, inputs, structural).await);
    }

    // -- The approval-invalidation hook, which a save **fires**: see this
    // function's own doc for why the answer is not read off
    // `transition::invalidation_for`. --
    fire_invalidation_hook(runner, inputs, ApprovalInvalidation::Fire).await?;

    // The committed row, re-read rather than reconstructed. The publish and
    // discard acts hand `announce_and_answer` an image they computed, which
    // they can because each moves a short, fixed column set; a save moves
    // whichever columns the request named, and a door that told the client
    // its own arithmetic would report a `200` describing a row that might
    // differ from the one it committed. `products::announce_and_answer` does
    // this re-read for every head act, and its doc carries the argument.
    let committed = repo::find_sku(runner, &inputs.scope, inputs.tenant_id, inputs.sku_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    announce_and_answer(
        runner,
        outbox,
        inputs,
        &committed,
        (events::SKU_HEAD_SAVED_PAYLOAD_TYPE, None),
    )
    .await
}

/// Run [`run_save`] on one retried transaction —
/// [`publish_in_one_transaction`]'s save twin, on its terms exactly. The
/// request travels as an owned clone for [`HeadActInputs`]'s stated reason.
///
/// # Errors
///
/// See [`run_save`].
async fn save_in_one_transaction(
    state: &ApiState,
    inputs: &HeadActInputs,
    request: SaveSkuRequest,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<MutationOutcome, HeadActError> {
    let outbox = state.sink.clone();
    let gate = Arc::clone(gate);
    let inputs = inputs.clone();
    state
        .db
        .db()
        .transaction_with_retry::<MutationOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let gate = Arc::clone(&gate);
                let inputs = inputs.clone();
                let request = request.clone();
                Box::pin(
                    async move { run_save(tx, &inputs, &request, gate.as_ref(), &outbox).await },
                )
            },
        )
        .await
}

/// `PATCH /skus/{id}`: the save door.
///
/// The thin `axum` shell over [`save_sku_gated`]. The only thing decided here
/// is the governance host, and it is decided the way [`publish_sku`] and
/// [`discard_sku`] decide it: the [`NoMaterialityPolicyGate`] literal, so no
/// wire input chooses one.
///
/// # Errors
///
/// See [`save_sku_gated`].
async fn save_sku(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
    Path(sku_id): Path<Uuid>,
    Json(request): Json<SaveSkuRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    save_sku_gated(
        &state,
        &enforcer,
        &ctx,
        &headers,
        sku_id,
        request,
        &(Arc::new(NoMaterialityPolicyGate) as Arc<dyn GovernanceGate + Send + Sync>),
    )
    .await
}

/// The save door, with its governance host as an explicit argument —
/// [`discard_sku_gated`]'s twin, and a parameter for that function's stated
/// reason: the gate phase runs here (`inst-fd-pipeline-gate-phase`) and the
/// gear's only host never refuses under [`GateMode::Gate`], so the refusal
/// arm is unreachable through [`save_sku`] and a phase nothing can exercise
/// is one a reader cannot tell from a phase that is absent.
///
/// The **mode** is not a parameter, on [`run_discard`]'s measured asymmetry:
/// the explicit-mode requirement is `dod-publish-door`'s, and no slice
/// schedules or cascades a save.
///
/// The door's own steps are [`publish_sku_gated`]'s: the `sku x write` grant
/// ([`open_act`]), the key ([`build_claim`], here with the **body's** digest
/// rather than the bodiless constant), the `If-Match`, the head read, then
/// [`run_save`] on one transaction and [`answer_head_act`].
///
/// # What this door does not build, and which slice owns each
///
/// `cpt-cf-bss-products-dod-save-door` covers a **content-row half this slice
/// cannot build**, and the `DoD` therefore reads as *partial* rather than
/// met. None of it is silently omitted:
///
/// - **Category assignments** — `products_product_category` is **slice 02**'s
///   table and does not exist at this commit, so there is no row for this
///   transaction to write and no field for this door to route.
/// - **Attribute values** — `products_attribute_value`, likewise **slice
///   02**'s.
/// - **The metering declaration** — **slice 03**'s, which owns both the
///   column set and the rules over it.
///
/// Each joins **this** transaction when it lands, beside the single head-row
/// `UPDATE` rather than after it: a content row written on a runner of its
/// own would survive a rolled-back save.
///
/// **Bucket ii and bucket iv have no columns** (`crate::domain::bucket`'s
/// module doc: §4.1 assigns none), so both arms are built and neither is
/// reachable today.
///
/// # Errors
///
/// Every refusal this door raises, each audited on its own transaction
/// through [`audit_act_refusal`]; the bare `404` a miss answers; the `500` a
/// storage or gate-host failure raises.
async fn save_sku_gated(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    headers: &HeaderMap,
    sku_id: Uuid,
    request: SaveSkuRequest,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<Response, CanonicalError> {
    let now = canonical::write_instant(Utc::now());
    let act = open_act(
        state,
        enforcer,
        ctx,
        sku_id,
        crate::authz::actions::WRITE,
        SAVE_AUDIT_ACTION,
        now,
    )
    .await?;

    let claim = match build_claim(
        state,
        headers,
        save_endpoint(sku_id),
        save_payload_digest(&request),
        now,
    ) {
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

    // A save naming no field at all is refused here rather than inside the
    // act: it is a property of the request alone, needs no row to judge, and
    // admitting it would be a bare `internal_revision` bump — a write with no
    // content that still invalidates every `ETag` a client holds.
    if request.fields.is_empty() {
        let mut report = ValidationReport::new();
        report.violate(
            "VALIDATION",
            "body",
            "a save must name at least one field: an empty body would bump the revision and \
             write nothing",
        );
        return Err(audit_act_refusal(
            state,
            &act,
            minted(sku_id, None),
            DomainError::Validation(report),
        )
        .await);
    }

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

    let outcome = save_in_one_transaction(state, &inputs, request, gate).await;
    answer_head_act(state, &act, sku_id, head.internal_revision, outcome).await
}

#[cfg(test)]
#[path = "skus_tests.rs"]
mod skus_tests;
