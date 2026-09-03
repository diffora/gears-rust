//! REST surface.
//!
//! The gear's reserved prefix, `/bss-products/v1`, is claimed here so
//! `gear.rs` never spells it twice. Phase 4 Slice C landed the **read door**
//! — `GET /bss-products/v1/products/{id}` and `GET /bss-products/v1/skus/{id}`
//! — as `products` and `skus`, sibling modules under `api::rest`, following
//! the sibling pricing and ledger gears' shape: one module per resource,
//! composed in `BssProductsGear::register_rest` rather than
//! wired inline. Slice D1 adds the Product half of the create doors
//! (`POST /bss-products/v1/products`, `products::create_product`) in the same
//! module; the SKU half follows the same shape in a later slice.
//!
//! Every mutating door Phase 4 still owes depends on this slice existing: an
//! author who has not just written a row has no `ETag` to send back as
//! `If-Match`, so the read door has to land before any door that checks one.
//!
//! [`router`] itself still nests an empty router: neither `OperationBuilder`
//! registration below needs it, because both doors register their own
//! absolute paths (`/bss-products/v1/products/{id}`,
//! `/bss-products/v1/skus/{id}`) and are `.merge()`d onto the host router
//! directly, the same way the sibling ledger gear's `fx_revaluation_mode`
//! and friends are. [`router`] is kept for the one caller that still needs an
//! unconditionally-claimed, routeless prefix: `register_rest`'s
//! runtime-absent branch.
//!
//! # What is shared between `products` and `skus`, and what is not
//!
//! [`ApiState`], [`require_authenticated`], [`unauthenticated`],
//! [`repo_error_to_canonical`], [`resolve_creator_actor_ref`] and
//! [`audit_refusal_and_report`] are identical for both doors — none of them
//! reads which entity is being served, so duplicating them would just be two
//! copies to keep in sync. The 403/404 builders are **not** shared: each
//! resource answers with its own GTS resource type
//! (`cf.bss.products.product.v1~` / `cf.bss.products.sku.v1~`), mirroring
//! `infra::error_mapping`'s own "two resource markers, not one" split, and a
//! shared builder generic enough to cover both would need a type parameter or
//! a trait object whose only job was picking which resource a route serves —
//! exactly the obscuring the task's own instructions warn against for two
//! four-line functions.
//!
//! `insert_product_with_event`/`insert_sku_with_event` and each door's own
//! conflict-classification pair stay two readable copies rather than one
//! shared, entity-generic form — see `products`'s and `skus`'s own module
//! docs, "What is duplicated from the Product door, and why", for the reason:
//! the only way to share them is a generic whose sole job is picking which
//! entity a route serves, which is the obscuring this section already names.
//!
//! # The outbox wiring, and where it lives
//!
//! [`ApiState`] carries `sink`, the [`crate::infra::broker::EventSink`] the
//! create doors enqueue their events
//! through (P-D-22, `crate::infra::events`). Both edges are made in
//! `gear.rs`: `Gear::init` declares
//! [`crate::infra::events::QUEUE_NAME`] with
//! [`crate::infra::events::PARTITIONS`] partitions, and `register_rest`
//! populates this field from the handle the runtime holds.
//!
//! A queue cannot be declared without a handler — `.transactional(..)` or
//! `.leased(..)` is mandatory, and `enqueue` refuses an unregistered queue
//! with `OutboxError::QueueNotRegistered`. **Which handler is registered is a
//! boot-time fork**, and the field above holds the answer: with an
//! `EventBrokerApi` in the `ClientHub`, the processor is the broker SDK's own
//! producer (P-D-47) and this field is
//! [`crate::infra::broker::EventSink::Broker`]; without one it is
//! [`crate::infra::events::PendingBrokerProducer`], which holds messages rather
//! than delivering them, and the field is `EventSink::Interim`. See
//! `crate::infra::broker`'s module doc for the fork, its deviation from
//! P-D-47's letter, and the measurement that **no gear in this workspace
//! registers that client yet**, so today every deployment takes the second arm.
//!
//! Two earlier drafts of this paragraph outlived their subject — one said both
//! edges were owed to a future slice, the next said delivery was still owed
//! after the producer had landed. Neither is a safe shape to restate; name the
//! fork and point at the type that holds it.
//!
//! # The idempotency phase, and why its parts sit here
//!
//! [`idempotency_key`], `idempotency_expiry` (now
//! `crate::infra::idempotency`'s own), [`IdempotencyClaimInput`],
//! [`claim_idempotency`], [`ClaimVerdict`], [`CreateOutcome`] and
//! [`replay_response`] are shared by both create doors for this module's own
//! stated reason: not one of them reads which entity is being served. The
//! key is read off a header, the expiry off the retention window, the claim
//! off `(tenant, endpoint, client key)`, and the replay off two stored
//! columns — a second copy in `skus` would only be a copy to keep in sync.
//!
//! **Where the phase sits in the flow.** `design/01-foundation.md` §2's
//! create-flow step list puts it in step **2**: *"Authorize `product × write`
//! ...; resolve the idempotency key `(tenant, endpoint, client key)`"* —
//! after step 1's `actor_ref` resolution, and in the same step as the
//! authorization gate. Both doors follow §2 rather than
//! `dod-idempotency-store`'s summary phrase "the first pipeline phase":
//! the `DoD` names the phase's rank among the *pipeline* phases, and §2 is
//! the step list that says which of this door's own steps precede it. So the
//! order is `actor_ref` → authorization gate → **read the key and digest the
//! payload** → shape validation → the mutation, with the claim `INSERT`
//! itself executed inside the mutation's transaction (P-D-42; see
//! [`claim_idempotency`]).
//!
//! **The phase is split between two places on purpose.** Reading the header
//! and digesting the parsed body happen in the handler, at the position
//! above; the claim `INSERT` happens inside `insert_*_with_event`'s
//! transaction closure, because P-D-42 makes that `INSERT` the gate and
//! requires it to join the guarded mutation's transaction so a rollback
//! frees the key. Nothing observable rides on the gap: a refusal raised
//! between the two stores nothing either way (P-D-38).
//!
//! **A keyless request skips the phase, it does not fail it** (P-D-34,
//! `dod-idempotency-store`). [`idempotency_key`] answers `Ok(None)` for an
//! absent header and both doors then claim nothing and create normally. A
//! later edit that made the header mandatory would contradict §2's own
//! opening sentence, which says every mutating door *accepts*
//! `Idempotency-Key` and none requires it.
//!
//! **The answer write closes the loop.** §3.2 `inst-fd-idem-claim-write`
//! requires the door to set `state = answered` with `response_status` and
//! `response_body` together, in the mutation's own transaction, on
//! completion. [`record_idempotency_answer`] is where both doors do it,
//! called from inside the same `insert_*_with_event` closure that took the
//! claim and wrote the entity row, so all three commit together or not at
//! all. An earlier version of this doc recorded the gap this left instead:
//! until the write existed, a committed create left its key `claimed` and
//! the client's own in-window retry was refused
//! `IDEMPOTENCY_KEY_IN_FLIGHT` rather than replaying the original `201`.
//! That is the case the store exists for, and
//! `products_tests::a_retry_after_a_committed_create_replays_the_original_response`
//! is what holds it closed.
//!
//! **What is stored is what was answered.** The `201`'s body is rendered
//! **inside** the transaction, stored there, and then returned by the
//! handler from that same rendered value ([`CreateOutcome::Created`] carries
//! it) — a door that re-rendered the view for the wire could drift from the
//! bytes it stored, and a replay that reproduces a different response is
//! worse than no replay. The one thing a replay cannot reproduce is the
//! `ETag`, and [`replay_response`]'s own doc says why: the table stores a
//! status and a body and no headers at all.
//!

use axum::Router;
use axum::extract::Extension;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use sea_orm::DbErr;
use serde_json::Value as JsonValue;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_db::DbError;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::authz::AuthzError;
use crate::domain::error::DomainError;
use crate::domain::validation::ValidationReport;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{self, AuditCommon, RefusalSubject};

pub mod bulk;
pub mod catalog_version;
pub mod dto;
pub mod materiality_policy;
pub mod preconditions;
pub mod products;
pub mod recognized_sets;
pub mod reference;
pub mod retention;
pub mod skus;
pub mod taxonomy;

/// The gear's reserved service prefix.
const PREFIX: &str = "/bss-products/v1";

/// Mounts the products REST surface onto `host_router`.
///
/// Nests an empty router under [`PREFIX`] so an unconfigured boot answers
/// `404` under the gear's own namespace rather than leaving the prefix
/// unclaimed. Used only by `register_rest`'s runtime-absent branch — the
/// present branch merges [`products::router`] and [`skus::router`] directly,
/// since both register absolute paths under [`PREFIX`] and need no nesting.
pub fn router(host_router: Router) -> Router {
    host_router.nest(PREFIX, Router::new())
}

/// Per-request state shared by every resource module on this surface.
///
/// Carries only the database provider: the repositories in
/// `infra::storage::repo` are free functions taking `&impl DBRunner` (see
/// that module's own doc for why), so a handler's only per-request dependency
/// is a runner to hand them — the `PolicyEnforcer` arrives through its own
/// `Extension`, layered once in `register_rest`, exactly as the sibling
/// ledger gear's door modules take it.
/// The five interim ceilings **P-D-107 arm 1** put in `ProductsConfig`,
/// resolved once at init as `watermark_skew_tolerance` is.
///
/// Bundled into one field rather than five, because `ApiState` is built at
/// fifteen sites — three in `gear.rs` and twelve in door harnesses that are
/// other strands' files — and five fields would be five lines of churn at
/// each. Read from configuration and never inlined: the numbers are interim
/// and the NFR workshop overrides them by configuration with no code change.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaxonomyCaps {
    /// `taxonomy_max_depth` — bounds the hold on the per-tenant taxonomy
    /// writer lock, since the walk runs inside the write transaction.
    pub(crate) max_depth: u32,
    /// `taxonomy_max_children_per_node`.
    pub(crate) max_children_per_node: u32,
    /// `metadata_max_keys`.
    pub(crate) metadata_max_keys: u32,
    /// `metadata_max_key_bytes`.
    pub(crate) metadata_max_key_bytes: u32,
    /// `metadata_max_value_bytes`.
    pub(crate) metadata_max_value_bytes: u32,
}

impl From<&crate::config::ProductsConfig> for TaxonomyCaps {
    fn from(cfg: &crate::config::ProductsConfig) -> Self {
        Self {
            max_depth: cfg.taxonomy_max_depth,
            max_children_per_node: cfg.taxonomy_max_children_per_node,
            metadata_max_keys: cfg.metadata_max_keys,
            metadata_max_key_bytes: cfg.metadata_max_key_bytes,
            metadata_max_value_bytes: cfg.metadata_max_value_bytes,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ApiState {
    /// The provider `state.db.conn()` opens a non-transactional runner from,
    /// and `state.db.transaction(..)` opens a transactional one from. The
    /// read door only ever calls `.conn()`; the create door
    /// (`products::create_product`) opens three transactions — one for
    /// `resolve_actor_ref`, one for the entity insert plus its outbox row,
    /// and one more, conditionally, for a refusal's audit row — each its
    /// own, per this module's doc on the entity/outbox pairing and
    /// `infra::storage::repo`'s own doc on the other two.
    ///
    /// The middle one is opened through `state.db.db().
    /// transaction_with_retry(..)` rather than `state.db.transaction(..)`:
    /// it is the transaction concurrent duplicates collide on by design, and
    /// `DBProvider::transaction` has no contention retry. See
    /// `products::insert_product_with_event`'s own doc.
    pub(crate) db: toolkit_db::DBProvider<toolkit_db::DbError>,
    /// The running transactional-outbox pipeline the create doors enqueue
    /// their events through, inside the same mutation transaction that
    /// writes the entity row (P-D-22). Populated by `gear.rs`'s
    /// `register_rest` from the handle the runtime holds; see this module's
    /// doc, "The outbox wiring, and where it lives".
    /// Either the SDK producer's queue (P-D-47) or the interim one, decided
    /// once at `Gear::init` — see [`crate::infra::broker::EventSink`].
    pub(crate) sink: crate::infra::broker::EventSink,
    /// The taxonomy and metadata ceilings the `02` doors enforce
    /// (**P-D-107** arm 1). `TAXONOMY_LIMIT` and `METADATA_LIMIT` are rules
    /// with no number without them.
    pub(crate) taxonomy_caps: TaxonomyCaps,
    /// The operator's own `idempotency_retention_hours`
    /// ([`crate::config::ProductsConfig`]), resolved once in `gear.rs`'s `init` from
    /// `ctx.config_or_default()` and carried here for the same reason the
    /// enforcer, the outbox and the provider are: a door reads per-request
    /// state, never a configuration source of its own. [`idempotency_expiry`]
    /// is its only reader. An earlier version read
    /// `ProductsConfig::default()` here, which silently gave every operator
    /// the design's 24-hour floor however they had configured the window.
    pub(crate) idempotency_retention_hours: u32,
    /// `inst-bm-limits`' first operand, carried here for the same reason
    /// as the retention window: the import door reads per-request state,
    /// never a configuration source of its own.
    pub(crate) bulk_max_rows_per_batch: u32,
    /// `inst-bm-limits`' second operand — the tenant's concurrent-batch
    /// ceiling, checked here and re-checked by the worker at claim.
    pub(crate) bulk_max_concurrent_batches_per_tenant: u32,
    /// The watermark door's own bound (P-D-87 arm 1), resolved once at
    /// `init` from `ProductsConfig::watermark_skew_tolerance`.
    pub(crate) watermark_skew_tolerance: std::time::Duration,
}

/// Extract the authenticated [`SecurityContext`] from the request
/// extensions, refusing with 401 when it is missing or carries no positive
/// identity — the same check the sibling ledger gear's own
/// `auth_context::require_authenticated` makes, reimplemented here rather
/// than imported because this gear has no `auth_context` module of its own
/// yet.
///
/// # Errors
///
/// [`CanonicalError`] (401) when no authenticated context is present, or the
/// one present carries the all-zero placeholder identity or no
/// `subject_type` at all.
pub(crate) fn require_authenticated(
    extension_ctx: Option<Extension<SecurityContext>>,
) -> Result<SecurityContext, CanonicalError> {
    let Some(Extension(ctx)) = extension_ctx else {
        return Err(unauthenticated());
    };
    if ctx.subject_id().is_nil() || ctx.subject_tenant_id().is_nil() {
        return Err(unauthenticated());
    }
    if ctx.subject_type().is_none() {
        return Err(unauthenticated());
    }
    Ok(ctx)
}

/// Build the 401 `CanonicalError` [`require_authenticated`] refuses with —
/// distinct from a permission denial (403), which is [`AuthzError::Denied`]'s
/// own wire shape.
pub(crate) fn unauthenticated() -> CanonicalError {
    CanonicalError::unauthenticated()
        .with_reason("AUTHENTICATION_REQUIRED")
        .create()
}

/// Map a storage-layer [`RepoError`] to a [`CanonicalError`].
///
/// Read and write doors alike route through this one helper — the create,
/// publish and discard paths as much as the two `GET`s — which is why its
/// log line names no door. An earlier version said "repository failure on
/// the read door", written when the read door was the only caller, and it
/// went on reporting a failed publish as a read-door failure once the
/// write doors were attached. If a future edit needs the doors told apart
/// in the log, take the context as an argument rather than restoring a
/// constant that only one caller matches.
///
/// Both variants are refusals of an invariant the request did not cause —
/// a scope/driver failure, or a stored value the database's own `CHECK`
/// constraints should have kept from ever landing — so both render as a bare
/// 500 with no resource marker and no registry code, the same shape
/// `infra::error_mapping`'s `AuditUnavailable` arm uses for the identical
/// reason: neither a `Product` nor a `SKU` refused anything, so tagging
/// either with a resource marker would name something that did not happen.
///
/// # The diagnostic passed to `CanonicalError::internal` does not reach the wire
///
/// `err` renders driver text — on a constraint violation, the constraint,
/// table and column names — and the publish/discard doors additionally hand
/// this helper a message built from a head row's own `lifecycle_state` and
/// `internal_revision`. None of that is answered to a caller, and the
/// suppression is the toolkit's, not this gear's, so it is recorded here
/// rather than re-implemented: `ResourceErrorBuilder::create`
/// (`libs/toolkit-canonical-errors/src/builder.rs`) routes the string into
/// `Internal::description` and is the one arm that deliberately skips
/// `with_detail`, so the wire `detail` stays the fixed
/// `"An internal error occurred. Please retry later."` that
/// `CanonicalError::__internal` mints; `InternalV1::description` is
/// `#[serde(skip)]`, so it is absent from `Problem::context` too. Only
/// `Problem::from_error_debug`, behind the `debug-problem` feature no
/// manifest in this workspace enables, surfaces it — and its own doc forbids
/// production use.
///
/// The consequence for an edit here: keep giving this constructor the full
/// diagnostic, but do not move that string onto a category whose `detail`
/// *is* serialized (every category except `Internal` and `Unknown`), and do
/// not reach for `.with_detail(..)` with it. Either would put driver text on
/// the wire and breach `api-contracts.md`'s *"do not expose internal
/// diagnostics in production `Problem` responses"*.
pub(crate) fn repo_error_to_canonical(err: &RepoError) -> CanonicalError {
    tracing::error!(error = %err, "bss-products: repository failure");
    CanonicalError::internal(format!("bss-products: {err}")).create()
}

/// Map an [`AuthzError`] from the PEP gate to a [`CanonicalError`], generic
/// over which resource's builder renders the 403 — the one place `products`
/// and `skus` still share a body, parameterised by the tiny closure each
/// caller supplies rather than by a resource-type constant, so the call site
/// still reads which resource is refusing.
///
/// `Denied` becomes a 403 carrying the deny reason, built by `denied` (each
/// caller's own resource marker); `Unavailable` becomes a fail-closed 503
/// whose diagnostic stays server-side.
pub(crate) fn authz_error_to_canonical(
    err: AuthzError,
    denied: impl FnOnce(String) -> CanonicalError,
) -> CanonicalError {
    match err {
        AuthzError::Denied(reason) => denied(reason),
        AuthzError::Unavailable(detail) => {
            tracing::error!(detail, "bss-products: authorization service unavailable");
            CanonicalError::service_unavailable().create()
        }
    }
}

/// Resolve `principal_id`'s `actor_ref`, on its own transaction, distinct
/// from a create door's own mutation transaction (P-D-26;
/// `repo::resolve_actor_ref`'s own doc). Shared by `products::create_product`
/// and `skus::create_sku`: neither entity is read here, so a byte-for-byte
/// copy in each door module would just be two copies to keep in sync — see
/// this module's own doc, "What is shared between `products` and `skus`, and
/// what is not".
pub(crate) async fn resolve_creator_actor_ref(
    state: &ApiState,
    tenant_id: Uuid,
    principal_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Uuid, CanonicalError> {
    let principal_ref = principal_id.to_string();
    let self_scope = AccessScope::for_tenant(tenant_id);
    state
        .db
        .transaction(move |tx| {
            Box::pin(async move {
                repo::resolve_actor_ref(tx, &self_scope, tenant_id, &principal_ref, now)
                    .await
                    .map_err(|e| DbError::Sea(DbErr::Custom(e.to_string())))
            })
        })
        .await
        .map_err(|e| {
            CanonicalError::internal(format!("bss-products: resolve actor ref: {e}")).create()
        })
}

/// The audit row's identity fields for one refusal, grouped exactly the way
/// `infra::storage::repo::AuditCommon` groups the fields every audit-row
/// class carries — the fields [`audit_refusal_and_report`] does not itself
/// derive (`audit_id`, `action`, `reason`, `correlation_id`, `written_at`
/// come from the call, not the caller). Grouping these four is what keeps
/// [`audit_refusal_and_report`] under this crate's argument-count lint
/// without reaching for an `allow`: they always travel together — every
/// caller has a `tenant_id` and an `actor_ref` to attribute to and a
/// `subject_kind`/`error_code` pair to name the refusal by, never a subset.
pub(crate) struct RefusalAuditContext<'a> {
    /// Owning tenant.
    pub(crate) tenant_id: Uuid,
    /// The pseudonymous ref of whoever, or whatever refused act, this row
    /// attributes to.
    pub(crate) actor_ref: Uuid,
    /// `crate::authz::labels::PRODUCT` or `::SKU` — the one thing that
    /// differs between callers, so it is the one field naming which door is
    /// refusing, the same shape [`authz_error_to_canonical`] already uses to
    /// stay shared across both doors.
    pub(crate) subject_kind: &'a str,
    /// The refusal's stable wire code, e.g. `VALIDATION` or
    /// `PERMISSION_DENIED`.
    pub(crate) error_code: &'a str,
}

/// Write a refusal's audit row on a transaction of its own, then answer the
/// refusal it names — or, if the audit row could not be written,
/// `AUDIT_UNAVAILABLE` instead of the refusal it would otherwise have
/// reported (`repo::write_refusal_audit`'s own contract).
///
/// Shared by every refusal branch of both create doors: `create_product` and
/// `create_sku` alike drive the authorization denial, every shape
/// `VALIDATION`, `PARENT_TERMINAL`, `SCOPE_NOT_CONTAINED`, `DUPLICATE_NAME`
/// and `DUPLICATE_CODE` through this one function, so the "answer only after
/// the row commits" discipline cannot be forgotten on a branch a future edit
/// adds.
///
/// `scope` is the caller's own compiled write scope from the door's
/// authorization gate — except for an authorization denial itself, which has
/// none to reuse (the gate is exactly what refused it) and instead audits
/// under the caller's own tenant-scoped self access
/// (`toolkit_db::secure::AccessScope::for_tenant`), the same self-scope
/// [`resolve_creator_actor_ref`] mints the actor ref's own row under.
///
/// `refusal` is the fully-built [`CanonicalError`] to answer with once the
/// audit row commits — a [`DomainError`] refusal via `.into()`, or the
/// authorization denial's own 403 — so this function stays generic over both
/// callers, which build different response shapes for the identical audit
/// discipline.
pub(crate) async fn audit_refusal_and_report(
    state: &ApiState,
    scope: &AccessScope,
    ctx: RefusalAuditContext<'_>,
    subject: RefusalSubject,
    refusal: CanonicalError,
) -> CanonicalError {
    audit_refusal_of_action_and_report(state, scope, ctx, "create", subject, refusal).await
}

/// [`audit_refusal_and_report`] with the audit row's `action` token supplied
/// by the caller — the same discipline, for a door that is not a create.
///
/// The `action` arrived as an argument rather than as a sixth field on
/// [`RefusalAuditContext`] deliberately: that struct is built by literal at
/// every call site in this crate, including in a module a concurrently
/// running slice owns, and adding a field to it would break every one of
/// them at once for a value only the non-create doors vary. A create keeps
/// calling [`audit_refusal_and_report`] and keeps writing `create`, so no
/// row's `action` changes meaning; the publish and discard doors write
/// `publish` and `discard`, which is what an operator reading
/// `products_audit_log` needs the column to say. There is no vocabulary
/// `CHECK` on the column yet (the audit-log migration's own doc records
/// that as an owed debt), so this crate's own tokens are what keep it a
/// closed set in practice.
pub(crate) async fn audit_refusal_of_action_and_report(
    state: &ApiState,
    scope: &AccessScope,
    ctx: RefusalAuditContext<'_>,
    action: &str,
    subject: RefusalSubject,
    refusal: CanonicalError,
) -> CanonicalError {
    let common = AuditCommon {
        audit_id: Uuid::new_v4(),
        tenant_id: ctx.tenant_id,
        actor_ref: ctx.actor_ref,
        action: action.to_owned(),
        subject_kind: ctx.subject_kind.to_owned(),
        reason: Some(format!("{}: refused at {action}", ctx.error_code)),
        // Reserved and unwritable, and no longer for want of a value: the
        // gear does read a request-scoped correlation id
        // (`infra::events::correlation_id`), but it is 32 hex characters and
        // this column is `uuid`, so it cannot be written without a migration.
        // `repo::AuditCommon::correlation_id`'s own doc carries the two
        // shapes that migration could take and why the choice is owed. This
        // is deliberate, not a forgotten field.
        correlation_id: None,
        written_at: Utc::now(),
    };

    let scope_for_audit = scope.clone();
    let error_code_owned = ctx.error_code.to_owned();
    let audit_result = state
        .db
        .transaction(move |tx| {
            Box::pin(async move {
                repo::write_refusal_audit(tx, &scope_for_audit, common, error_code_owned, subject)
                    .await
                    .map_err(|e| DbError::Sea(DbErr::Custom(e.to_string())))
            })
        })
        .await;

    match audit_result {
        Ok(()) => refusal,
        Err(db_error) => {
            tracing::error!(
                error = %db_error,
                "bss-products: refusal audit row could not be written; withholding the refusal"
            );
            CanonicalError::from(DomainError::AuditUnavailable(db_error.to_string()))
        }
    }
}

/// The header a caller carries its idempotency key in
/// (`design/01-foundation.md` §2: every mutating door *accepts*
/// `Idempotency-Key`, and none requires it).
pub(crate) const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";

/// The idempotency key on this request, if it carries one.
///
/// `Ok(None)` is the **skip**, not a failure: a create without the header
/// proceeds normally and claims nothing (`dod-idempotency-store`, P-D-34:
/// "skipping the phase on a keyless request rather than failing it"). A
/// later edit that turned this into a refusal would make the header
/// mandatory on every mutating door, which is the opposite of what §2 says
/// the doors accept.
///
/// # Errors
///
/// [`DomainError::Validation`] naming `Idempotency-Key` when the header is
/// **present but unusable** — not valid `UTF-8`, or blank after trimming.
/// Present-but-unusable is not the same as absent: the caller asked for
/// at-most-once semantics and this door cannot key them, and silently
/// skipping the phase would hand back a `201` the caller would reasonably
/// read as protected. The same `VALIDATION` code and the same
/// audit-then-answer discipline as every other shape refusal, following
/// `preconditions::if_match`'s own reading of an unreadable header.
pub(crate) fn idempotency_key(headers: &HeaderMap) -> Result<Option<String>, DomainError> {
    let Some(raw) = headers.get(IDEMPOTENCY_KEY_HEADER) else {
        return Ok(None);
    };
    let value = raw
        .to_str()
        .map_err(|_| refuse_idempotency_key("the header value is not valid UTF-8"))?
        .trim();
    if value.is_empty() {
        return Err(refuse_idempotency_key(
            "the header is present but blank; send a stable, caller-chosen key or omit the \
             header entirely",
        ));
    }
    Ok(Some(value.to_owned()))
}

/// Build the [`DomainError::Validation`] an unusable `Idempotency-Key` is
/// refused with — one site, so every case [`idempotency_key`] raises carries
/// the same subject and the same wire code.
fn refuse_idempotency_key(detail: &str) -> DomainError {
    let mut report = ValidationReport::new();
    report.violate("VALIDATION", IDEMPOTENCY_KEY_HEADER, detail);
    DomainError::Validation(report)
}

// The idempotency phase and the shared create transaction live in infra
// (`crate::infra::idempotency`, `crate::infra::create`) so the batch worker
// reaches them without depending on this layer; the doors keep their old
// paths through these re-exports.
pub(crate) use crate::infra::create::{CREATE_RESPONSE_STATUS, CreateOutcome};
pub(crate) use crate::infra::idempotency::{
    ClaimVerdict, CompositeClaimVerdict, IdempotencyClaimInput, claim_composite_idempotency,
    claim_idempotency, record_idempotency_answer,
};
pub(crate) use crate::infra::storage::contention_db_err;

/// Serve a stored answer as a replay: the recorded status and body, and
/// nothing else.
///
/// **No `ETag`.** `products_idempotency` stores a status and a body and no
/// headers at all (§3.2 `inst-fd-idem-claim-write`, §4.4's two response
/// columns), so a replay cannot reproduce the original `ETag`. A client that
/// needs the tag reads the head, which is the door that mints tags. This is
/// a property of the stored shape rather than of this function, and is
/// stated here because it is where a reader would look for the missing
/// header.
///
/// A stored status outside `u16`, or outside the status range, answers a
/// bare 500 rather than a fabricated status: reaching it means a row was
/// written around this gear, since the only writer is a door that stores the
/// status it just answered.
pub(crate) fn replay_response(status: i32, body: JsonValue) -> Response {
    let recorded = u16::try_from(status)
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok());
    if let Some(code) = recorded {
        return (code, axum::Json(body)).into_response();
    }
    tracing::error!(
        status,
        "bss-products: stored idempotency response_status is not a status code"
    );
    CanonicalError::internal(format!(
        "bss-products: stored idempotency response_status {status} is not a status code"
    ))
    .create()
    .into_response()
}
