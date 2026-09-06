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

pub mod approvals;
pub mod bulk;
pub mod catalog_version;
pub mod dto;
pub mod materiality_policy;
pub mod preconditions;
pub mod products;
pub mod read;
pub mod recognized_sets;
pub mod reference;
pub mod retention;
pub mod scheduled_transitions;
pub mod sdk_bindings;
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
    /// `attribute_values_max_per_patch` — bounds one live-value patch's
    /// transaction (P-D-163).
    pub(crate) attribute_values_max_per_patch: u32,
}

impl From<&crate::config::ProductsConfig> for TaxonomyCaps {
    fn from(cfg: &crate::config::ProductsConfig) -> Self {
        Self {
            max_depth: cfg.taxonomy_max_depth,
            max_children_per_node: cfg.taxonomy_max_children_per_node,
            metadata_max_keys: cfg.metadata_max_keys,
            metadata_max_key_bytes: cfg.metadata_max_key_bytes,
            metadata_max_value_bytes: cfg.metadata_max_value_bytes,
            attribute_values_max_per_patch: cfg.attribute_values_max_per_patch,
        }
    }
}

/// `07-reference-signal`'s three door-side knobs, read off `ProductsConfig`
/// once at boot (P-D-87 arm 1 homes; P-D-147 puts them on the state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReferenceKnobs {
    /// How old a watermark may be before its producer reads *stale*.
    pub(crate) freshness: std::time::Duration,
    /// The tripwire's rate: break-glass overrides per rolling 30 days above
    /// which `reference_breakglass_tripwire` fires.
    pub(crate) tripwire_max_overrides_per_30_days: u32,
    /// Whether break-glass arm (a) is admissible at all (**P-D-71**,
    /// default off). Arm (b) is not behind it (**P-D-48**).
    pub(crate) breakglass_correction_enabled: bool,
}

impl From<&crate::config::ProductsConfig> for ReferenceKnobs {
    fn from(cfg: &crate::config::ProductsConfig) -> Self {
        Self {
            freshness: cfg.reference_freshness(),
            tripwire_max_overrides_per_30_days: cfg.tripwire_max_overrides_per_30_days,
            breakglass_correction_enabled: cfg.breakglass_correction_enabled,
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
    /// `07`'s knobs the correction and retirement doors read
    /// (`dod-reference-config`'s four minus the skew, which sits above).
    pub(crate) reference: ReferenceKnobs,
    /// The elevation window, in hours (**P-D-132**, interim 4, zero refused
    /// at boot). Carried here for the reason every other configured number
    /// is: the break-glass door computes `valid_until` from it and **must not
    /// inline** a literal, or the operator's own setting stops reaching the
    /// one door it governs.
    pub(crate) breakglass_window_hours: u32,
    /// The post-hoc review SLA, in hours (**P-D-133**, interim 24). The
    /// elevation's alert carries it, so an operator reading the alert knows
    /// when the obligation lapses without looking the number up.
    pub(crate) breakglass_review_sla_hours: u32,
    /// `04`'s EOL flag (`ProductsConfig::eol_enabled`), read by the retire doors.
    pub(crate) eol_enabled: bool,
    /// `03`'s usage-type resolver (`dod-usage-type-resolution`, P-D-141):
    /// the collector's client behind a trait, `NoCollector` where none is
    /// wired. Carried here for the same reason the detector is built per
    /// door: the policy has an operand outside the process, and a literal in
    /// the door would be a second program under test.
    pub(crate) usage_type_resolver:
        std::sync::Arc<dyn crate::infra::usage_types::UsageTypeResolver>,
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
/// Which governance host a door runs under (P-D-142).
///
/// A route handler passes [`GateHost::Real`]: the door builds the **stored**
/// host for its act once it knows the subject — the head's revision from
/// `If-Match`, the entity's id — and the act's materiality. A probe passes
/// [`GateHost::Given`] to drive the door under a double (`NoMaterialityPolicyGate`,
/// an overriding gate, a refusing one); that is the only route to a
/// non-stored host, and it is a test's.
pub(crate) enum GateHost {
    /// The stored host, built by [`resolve_host`] for the act.
    Real,
    /// A host the caller supplies — the in-process seam the probes enter
    /// through; no routed handler constructs it.
    #[cfg_attr(not(test), allow(dead_code))]
    Given(std::sync::Arc<dyn crate::domain::governance::GovernanceGate + Send + Sync>),
}

/// What a door is about to do, as the host builder needs to know it.
pub(crate) enum HostFor {
    /// A governed act whose materiality is settled by enumeration — every
    /// lifecycle transition, a governed cancel, a live op, a set write: the
    /// stored host over the store's candidates for `subject`.
    Governed(crate::domain::governance::GateSubject),
    /// An entity publish: material or not by the columns it touches against
    /// the last frozen version (`MaterialityEvaluator::verdict`), under the
    /// tenant's policy. `NonMaterial` publishes run ungoverned; `Material` ones
    /// run the stored host.
    Publish {
        entity: crate::domain::governance::EntityRef,
        revision: crate::domain::concurrency::InternalRevision,
    },
}

/// [`resolve_host`]'s two failure classes — the doors map them apart.
pub(crate) enum HostError {
    /// The act is refused before any host is consulted: a publish touching a
    /// bucket-ii column outside the correction door, an unregistered column.
    Refused(DomainError),
    /// A storage failure reading the policy, the candidates or the head.
    Repo(RepoError),
}

/// Build the host a door runs under (P-D-142).
///
/// `Given` is returned as is. `Real` is built from the store:
/// - `Ungoverned` → [`crate::domain::approval::StoredApprovalGate::ungoverned`].
/// - `Governed(subject)` → the stored host over `repo::gate_candidates` for
///   that subject — the record must be `satisfied` and pinned to the subject,
///   or the gate refuses `APPROVAL_REQUIRED`.
/// - `Publish { entity, revision }` → the tenant's materiality policy is read
///   (an absent row is the default, P-D-112 arm 2), the touched set is
///   measured against the last frozen version through the submit door's own
///   `resolve_entity_subject`, and the evaluator judges it: `NonMaterial`
///   runs ungoverned, `Material` runs the stored host for
///   `GateSubject::entity_publish(entity, revision)`. A bucket-ii touch is
///   refused `ILLEGAL_FIELD_MUTATION` naming the correction door — it cannot
///   arise on a first publish (P-D-142 excludes bucket ii from a first
///   publish's touched set) and after one the head guard admits no such save,
///   so reaching it means the head and its last version disagree on a column
///   only the correction door may move.
///
/// @cpt-dod:cpt-cf-bss-products-dod-gate-host:p1
/// One finding of the dry-run publish lint (`validate` doors; **P-D-125**
/// row 14, P-D-148): the code a publish would refuse with, the field or
/// rule it names, and the detail. The per-entity report `fr-prepublish-lint`
/// requires, and `dod-override-ceremony`'s operand — approvers acknowledge
/// these codes by name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LintFinding {
    pub code: String,
    pub subject: String,
    pub detail: String,
}

impl LintFinding {
    pub(crate) fn of(refusal: &crate::domain::error::DomainError) -> Self {
        Self {
            code: refusal.code().to_owned(),
            subject: String::new(),
            detail: refusal.to_string(),
        }
    }

    pub(crate) fn from_report(report: &crate::domain::validation::ValidationReport) -> Vec<Self> {
        report
            .violations()
            .iter()
            .map(|violation| Self {
                code: violation.code.to_owned(),
                subject: violation.subject.clone(),
                detail: violation.detail.clone(),
            })
            .collect()
    }
}

pub(crate) async fn resolve_host(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    host: GateHost,
    act: HostFor,
) -> Result<std::sync::Arc<dyn crate::domain::governance::GovernanceGate + Send + Sync>, HostError>
{
    use crate::domain::approval::StoredApprovalGate;
    use crate::domain::governance::GateSubject;
    use crate::domain::materiality::{
        MaterialAct, Materiality, MaterialityEvaluator, MaterialityRefusal, Resolution,
    };
    if let GateHost::Given(gate) = host {
        return Ok(gate);
    }
    let subject = match act {
        HostFor::Governed(subject) => subject,
        HostFor::Publish { entity, revision } => {
            let policy = repo::resolve_materiality_policy(runner, scope, tenant_id)
                .await
                .map_err(HostError::Repo)?;
            let Resolution::Resolved(policy) = policy else {
                return Err(HostError::Repo(RepoError::Db(
                    "the materiality policy could not be read: a failed read is not a verdict \
                     (P-D-119 row 3), so the act does not run"
                        .to_owned(),
                )));
            };
            let resolved =
                crate::api::rest::approvals::resolve_entity_subject(runner, scope, entity)
                    .await
                    .map_err(HostError::Repo)?;
            let Some((_, _, _, touched)) = resolved else {
                // The head vanished under the door; the door's own read
                // answers its 404, and an ungoverned host lets it get there.
                return Ok(std::sync::Arc::new(StoredApprovalGate::ungoverned()));
            };
            let touched: Vec<&str> = touched.iter().map(String::as_str).collect();
            let verdict = MaterialityEvaluator::new(Resolution::Resolved(&policy)).verdict(
                &MaterialAct::EntityPublish {
                    kind: entity.entity_kind,
                    touched: &touched,
                },
            );
            match verdict {
                Ok(Materiality::NonMaterial) => {
                    return Ok(std::sync::Arc::new(StoredApprovalGate::ungoverned()));
                }
                Ok(Materiality::Material) => GateSubject::entity_publish(entity, revision),
                Err(MaterialityRefusal::CorrectableTouch(column)) => {
                    return Err(HostError::Refused(DomainError::IllegalFieldMutation(
                        format!(
                            "{column} is bucket ii and differs from the last frozen version: after \
                         first publish it moves only through the correction door \
                         (POST .../corrections, slice 07), never through a publish"
                        ),
                    )));
                }
                Err(MaterialityRefusal::Registry(error)) => return Err(HostError::Refused(error)),
                Err(other) => {
                    return Err(HostError::Repo(RepoError::Db(format!(
                        "the materiality verdict could not be formed: {other}"
                    ))));
                }
            }
        }
    };
    let candidates = repo::gate_candidates(runner, scope, &subject)
        .await
        .map_err(HostError::Repo)?;
    Ok(std::sync::Arc::new(StoredApprovalGate::governed(
        candidates,
    )))
}

pub(crate) use crate::infra::storage::repo::{SettleError, settle_authorization};

/// The gate a live-op door runs **before** its transaction (P-D-144): the
/// stored host over the store's candidates for `subject`, evaluated in `Gate`
/// mode. The door then spends the record it was authorized on **inside** its
/// own transaction with [`settle_authorization`], so the one-shot commits with
/// the act or rolls back with it.
pub(crate) async fn authorize_live_op(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    subject: crate::domain::governance::GateSubject,
) -> Result<crate::domain::governance::GateAuthorization, HostError> {
    let conn = state
        .db
        .conn()
        .map_err(|e| HostError::Repo(RepoError::Db(e.to_string())))?;
    let host = resolve_host(
        &conn,
        scope,
        tenant_id,
        GateHost::Real,
        HostFor::Governed(subject.clone()),
    )
    .await?;
    let verdict = host
        .evaluate(subject, crate::domain::governance::GateMode::Gate)
        .map_err(HostError::Refused)?;
    verdict.into_authorization().map_err(HostError::Refused)
}

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
        // The W3C trace id off the ambient span, `None` outside a traced
        // request (P-D-118: the column is `text` since 2026-09-04, so the
        // 32-hex rendering that joins the access log and the error envelope
        // is stored as is).
        correlation_id: crate::infra::events::correlation_id(),
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

/// The longest `Idempotency-Key` a door admits, in bytes. The key is stored
/// on the claim row and compared on replay, so an unbounded one is an
/// unbounded row and an unbounded comparison; 255 holds every UUID, ULID and
/// hash a client would send and refuses a payload smuggled in as a key.
pub(crate) const IDEMPOTENCY_KEY_MAX_BYTES: usize = 255;

/// The one wording every over-length shape refusal carries, so a `name`, a
/// code and a key refused for length read the same and name their ceiling.
pub(crate) fn over_length_detail(field: &str, len: usize, max: usize) -> String {
    format!("{field} is {len} bytes long and at most {max} are admitted")
}

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
    if value.len() > IDEMPOTENCY_KEY_MAX_BYTES {
        return Err(refuse_idempotency_key(&format!(
            "the header value is {} bytes long and a key is at most {IDEMPOTENCY_KEY_MAX_BYTES}",
            value.len()
        )));
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

// ---------------------------------------------------------------------------
// The break-glass elevation gate (`design/05` §2's break-glass flow;
// **P-D-133** row 18, **P-D-68** arm 2, **P-D-132**'s window).
// ---------------------------------------------------------------------------

/// The platform's own 403 for a session that is unknown, unparseable, or not
/// the caller's.
///
/// **One answer for all three**, and no gear code. P-D-119 row 3: the gear
/// mints codes for its own refusals and never for an authorization denial, so
/// this carries `permission_denied`'s shape. One answer, because
/// distinguishing "no such session" from "not yours" would let a caller
/// enumerate other principals' elevations by id.
fn break_glass_session_unknown() -> CanonicalError {
    ElevationResource::permission_denied()
        .with_reason("BREAK_GLASS_SESSION_UNKNOWN")
        .create()
}

/// The canonical-error identity the elevation gate's own refusals carry.
#[toolkit::api::canonical_prelude::resource_error(gts_id!("cf.bss.products.breakglass.v1~"))]
struct ElevationResource;

/// The header an elevated call names its session in.
///
/// A header rather than a query parameter or a body field, for the reason
/// P-D-133 row 18 gives: the operand is read in the **pre-pipeline gate**,
/// before any route's own extractor runs, and a header is the only part of a
/// request that layer can read without knowing which door it is bound for.
/// @cpt-dod:cpt-cf-bss-products-dod-breakglass-readonly:p1
/// @cpt-dod:cpt-cf-bss-products-dod-breakglass-expiry:p1
/// @cpt-dod:cpt-cf-bss-products-dod-governance-audit:p1
pub(crate) const BREAK_GLASS_SESSION_HEADER: &str = "x-break-glass-session";

/// The pre-pipeline elevation gate — **one function, one call site**.
///
/// # What an elevation changes, and what it deliberately does not
///
/// **P-D-133** row 18: *"the session names its target tenant; the door reads
/// the session id from a header in the pre-pipeline gate, checks the window
/// and substitutes `AccessScope::for_tenant(target)` **read-only** for the
/// caller's own scope; every write is refused; `ToolKit` is unchanged."*
///
/// The substitution happens **on the `SecurityContext`**, not on a scope
/// handed to each door. Every door in this gear reads `ctx.subject_tenant_id()`
/// and passes it to `crate::authz::access_scope`, so rewriting the context's
/// tenant is the one edit that reaches all of them — and it keeps the policy
/// point in the loop, because the door still asks the PDP about the *target*
/// pair rather than being handed a scope nobody authorized. `AccessScope`
/// already builds for any tenant, so `ToolKit` is untouched exactly as the
/// decision says.
///
/// # The order of the three refusals is the decision's order
///
/// 1. **Whose session is it.** The row is read on an unconstrained scope,
///    because the target tenant is the thing being looked up and a
///    caller-scoped read could never find a cross-tenant session. That would
///    be fail-open on its own, so the gate then requires the caller to be the
///    session's own `principal`; anyone else gets the platform's 403 with no
///    gear code (**P-D-119** row 3 — the gear mints codes for its own
///    refusals, never for an authorization denial).
/// 2. **The window**, before the method check, because *expiry gates
///    admission* (P-D-68 arm 2): a post-expiry **write** is a post-expiry act
///    and must be the one that emits `BreakGlassExpired`, not one that is
///    turned away for its verb first and leaves the stamp unflipped.
/// 3. **The method.** v1 is read and audit-export only, so every mutating
///    verb is `BREAKGLASS_WRITE_FORBIDDEN` with no exception.
///
/// An admitted read is audited **individually** — session id, reason and
/// correlation id — before it runs, which is `dod-breakglass-readonly`'s
/// *"every access is individually audited"* rather than a sampled one.
///
/// # Errors
///
/// [`CanonicalError`] when the session is unknown or not the caller's (403,
/// the platform's), past its window (`BREAKGLASS_EXPIRED`), or the request is
/// a write (`BREAKGLASS_WRITE_FORBIDDEN`).
pub(crate) async fn elevation_gate(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<ApiState>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, CanonicalError> {
    let Some(session_id) = request
        .headers()
        .get(BREAK_GLASS_SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        // No header, no elevation. The overwhelming majority of requests take
        // this arm and must cost nothing.
        return Ok(next.run(request).await);
    };
    let Ok(session_id) = uuid::Uuid::parse_str(session_id) else {
        return Err(break_glass_session_unknown());
    };
    // The **bare** `SecurityContext`, not an `Extension<_>` wrapper: axum's
    // `Extension` extractor looks the inner type up directly, so that is what
    // the authenticating layer inserts and what every door reads. Asking for
    // the wrapper finds nothing and answers 401 on every elevated call.
    let Some(ctx) = request.extensions().get::<SecurityContext>().cloned() else {
        return Err(unauthenticated());
    };
    let now = crate::domain::canonical::write_instant(Utc::now());

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    // Unconstrained, because the target tenant is what this read resolves and
    // a caller-scoped read of a cross-tenant session finds nothing by
    // construction. The fail-open this could be is closed by the principal
    // check immediately below, not by the scope.
    let platform_scope = AccessScope::allow_all();
    let session =
        crate::infra::storage::repo::read_breakglass_session(&conn, &platform_scope, session_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?;
    let Some(session) = session else {
        return Err(break_glass_session_unknown());
    };

    let actor_ref =
        resolve_creator_actor_ref(&state, session.target_tenant, ctx.subject_id(), now).await?;
    if actor_ref != session.principal {
        // Not the platform principal who opened it. The platform's own 403,
        // and no gear code: this is an authorization denial, and P-D-119
        // row 3 reserves the gear's roster for the gear's own refusals.
        return Err(break_glass_session_unknown());
    }

    let target_scope = AccessScope::for_tenant(session.target_tenant);
    // **The CAS and the emission are one transaction** — `dod-breakglass-expiry`'s
    // *"in the same transaction as that refusal"*, which `admit_elevated_call`
    // leaves to its caller because it opens none itself. A committed flip
    // beside a failed enqueue is the exactly-once guarantee **inverted**: the
    // stamp says the event went out and no later caller will send it, so the
    // expiry is announced zero times. Rolling both back leaves the next
    // post-expiry call to emit, which is what "the first post-expiry act"
    // means when the first one fails.
    let scope_tx = target_scope.clone();
    let sink = state.sink.clone();
    let target_tenant = session.target_tenant;
    let admission = state
        .db
        .db()
        .transaction_with_retry::<crate::infra::storage::repo::Elevation, ElevationTxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            elevation_contention,
            move |tx| {
                let scope = scope_tx.clone();
                let sink = sink.clone();
                Box::pin(async move {
                    let admission = crate::infra::storage::repo::admit_elevated_call(
                        tx, &scope, session_id, now,
                    )
                    .await
                    .map_err(ElevationTxError::Repo)?;
                    if matches!(
                        admission,
                        crate::infra::storage::repo::Elevation::Expired { emit_expired: true }
                    ) {
                        crate::infra::events::enqueue_governance(
                            &sink,
                            tx,
                            session_id,
                            crate::infra::events::BREAK_GLASS_EXPIRED_PAYLOAD_TYPE,
                            &crate::infra::events::GovernanceEventBody {
                                tenant_id: target_tenant,
                                act: "expired",
                                approval_id: None,
                                session_id: Some(session_id),
                                verdict: None,
                                state: None,
                                target_tenant_id: Some(target_tenant),
                            },
                            actor_ref,
                        )
                        .await
                        .map_err(ElevationTxError::Events)?;
                    }
                    Ok(admission)
                })
            },
        )
        .await
        .map_err(|error| match error {
            ElevationTxError::Repo(e) => repo_error_to_canonical(&e),
            ElevationTxError::Events(e) => {
                repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
            }
        })?;

    // `NotYetValid` refuses with the same code and emits **nothing**: a
    // session that has not begun has not expired, and folding the two arms
    // would announce `BreakGlassExpired` for a window still ahead.
    match admission {
        crate::infra::storage::repo::Elevation::Admitted => {}
        // Both refuse `BREAKGLASS_EXPIRED`, and the two are kept **distinct
        // upstream rather than here**: `admit_elevated_call` answers
        // `NotYetValid` for a window still ahead precisely so nothing flips
        // the stamp or emits for a session that has not begun. By the time
        // the answer reaches this `match` the difference has already been
        // acted on — the emission, if this caller won the CAS, committed with
        // the flip inside the transaction above — so one arm is the honest
        // shape and two identical ones would be a distinction that no longer
        // exists.
        crate::infra::storage::repo::Elevation::NotYetValid
        | crate::infra::storage::repo::Elevation::Expired { .. } => {
            return Err(elevation_outside_window(
                &state,
                &target_scope,
                ElevationRefusal {
                    session: &session,
                    actor_ref,
                    now,
                },
            )
            .await);
        }
    }

    if !matches!(
        request.method(),
        &axum::http::Method::GET | &axum::http::Method::HEAD
    ) {
        return Err(
            elevated_write_forbidden(&state, &target_scope, &session, actor_ref, now).await,
        );
    }

    // `dod-breakglass-readonly`: **every** admitted access, not a sample.
    audit_elevated_access(&state, &target_scope, &session, actor_ref, now, "read").await?;

    // The substitution. Everything below this line runs under the target
    // tenant, through the policy point, read-only by the check above.
    let elevated = elevated_context(&ctx, &session)?;
    request.extensions_mut().insert(elevated);
    Ok(next.run(request).await)
}

/// The prefix of the token scope an elevated context carries, followed by
/// the session id. See [`elevated_context`].
pub(crate) const BREAK_GLASS_SCOPE_PREFIX: &str = "bss-products.breakglass:";

/// The substituted context an admitted elevated read runs under: the
/// caller's own subject, the **target** tenant, every scope the caller
/// carried, **plus one marker scope naming the session**.
///
/// The marker is what makes an elevated context distinguishable from an
/// ordinary one once the middleware has run. Before it, a door, an audit row
/// or a log line downstream saw a context indistinguishable from a native
/// principal of the target tenant — and the audit trail the gate writes was
/// the only place the elevation was visible at all. A door that must answer
/// differently under elevation reads [`breakglass_session_of`].
///
/// # Errors
///
/// The builder refusing the context — answered as `unauthenticated`, the same
/// word the gate uses for a context it cannot form.
pub(crate) fn elevated_context(
    ctx: &SecurityContext,
    session: &crate::infra::storage::entity::breakglass_session::Model,
) -> Result<SecurityContext, CanonicalError> {
    let mut scopes = ctx.token_scopes().to_vec();
    scopes.push(format!("{BREAK_GLASS_SCOPE_PREFIX}{}", session.session_id));
    let elevated = SecurityContext::builder()
        .subject_id(ctx.subject_id())
        .subject_tenant_id(session.target_tenant)
        .token_scopes(scopes);
    let elevated = match ctx.subject_type() {
        Some(subject_type) => elevated.subject_type(subject_type),
        None => elevated,
    };
    elevated.build().map_err(|_| unauthenticated())
}

/// The break-glass session a context runs under, read off the marker scope
/// [`elevated_context`] adds; `None` for an ordinary context.
#[must_use]
pub(crate) fn breakglass_session_of(ctx: &SecurityContext) -> Option<uuid::Uuid> {
    ctx.token_scopes()
        .iter()
        .find_map(|scope| scope.strip_prefix(BREAK_GLASS_SCOPE_PREFIX))
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
}

/// The elevation gate's own transaction error.
enum ElevationTxError {
    Repo(crate::infra::storage::RepoError),
    Events(crate::infra::events::EventsError),
}

impl From<toolkit_db::DbError> for ElevationTxError {
    fn from(error: toolkit_db::DbError) -> Self {
        Self::Repo(crate::infra::storage::RepoError::Db(error.to_string()))
    }
}

/// The retry loop classifies `sea-orm`'s own error.
fn elevation_contention(error: &ElevationTxError) -> Option<&sea_orm::DbErr> {
    match error {
        ElevationTxError::Repo(crate::infra::storage::RepoError::Driver { source, .. }) => {
            Some(source)
        }
        ElevationTxError::Repo(_) | ElevationTxError::Events(_) => None,
    }
}

/// What an out-of-window refusal needs, grouped so the two `Uuid`s cannot be
/// transposed at the call site.
///
/// **It carries no `emit_expired` flag.** It used to, and the flag was the
/// operand of an emission this function no longer makes: the CAS and
/// `BreakGlassExpired` now commit together in the gate's own transaction, so
/// by the time a refusal is built the emission has already happened or the
/// whole thing rolled back.
struct ElevationRefusal<'a> {
    session: &'a crate::infra::storage::entity::breakglass_session::Model,
    actor_ref: uuid::Uuid,
    now: DateTime<Utc>,
}

/// Refuse a call past the window, emitting `BreakGlassExpired` for exactly
/// the caller that won the CAS (**P-D-68** arm 2).
async fn elevation_outside_window(
    state: &ApiState,
    scope: &AccessScope,
    refusal: ElevationRefusal<'_>,
) -> CanonicalError {
    // **No emission here.** The CAS and `BreakGlassExpired` commit together
    // in the caller's transaction; this function only answers the refusal
    // that accompanied them.
    audit_refusal_and_report_for_elevation(
        state,
        scope,
        refusal.session,
        refusal.actor_ref,
        refusal.now,
        DomainError::BreakGlassExpired(format!(
            "elevation {} is outside its window [{}, {})",
            refusal.session.session_id, refusal.session.valid_from, refusal.session.valid_until
        )),
    )
    .await
}

/// Refuse a write under an elevation. **No exception in v1.**
async fn elevated_write_forbidden(
    state: &ApiState,
    scope: &AccessScope,
    session: &crate::infra::storage::entity::breakglass_session::Model,
    actor_ref: uuid::Uuid,
    now: DateTime<Utc>,
) -> CanonicalError {
    audit_refusal_and_report_for_elevation(
        state,
        scope,
        session,
        actor_ref,
        now,
        DomainError::BreakGlassWriteForbidden(format!(
            "elevation {} is read and audit-export only",
            session.session_id
        )),
    )
    .await
}

/// Audit one elevated refusal and answer it.
async fn audit_refusal_and_report_for_elevation(
    state: &ApiState,
    scope: &AccessScope,
    session: &crate::infra::storage::entity::breakglass_session::Model,
    actor_ref: uuid::Uuid,
    now: DateTime<Utc>,
    refusal: DomainError,
) -> CanonicalError {
    let code = refusal.code();
    // **The refusal is answered whether or not its audit row landed.** An
    // audit failure here would otherwise turn a legitimate 403 into a 500 and
    // tell the operator the wrong thing about their own request; the loss is
    // named on the alert channel instead, which is where a missing audit row
    // is actionable.
    if let Err(error) = audit_elevated_access(state, scope, session, actor_ref, now, code).await {
        tracing::warn!(
            event = "products_breakglass_access_unaudited",
            session_id = %session.session_id,
            %error,
            "an elevated refusal could not be audited"
        );
    }
    CanonicalError::from(refusal)
}

/// One audit row per elevated access — admitted or refused.
///
/// `dod-breakglass-readonly` names the three operands: the session id, the
/// reason, and the correlation id. All three are real values now —
/// `correlation_id` became `text` under **P-D-118**, so the row carries the
/// trace rather than the `None` the old doc excused.
async fn audit_elevated_access(
    state: &ApiState,
    scope: &AccessScope,
    session: &crate::infra::storage::entity::breakglass_session::Model,
    actor_ref: uuid::Uuid,
    now: DateTime<Utc>,
    action: &str,
) -> Result<(), CanonicalError> {
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    crate::infra::storage::repo::write_eventless_act_audit(
        &conn,
        scope,
        crate::infra::storage::repo::AuditCommon {
            audit_id: uuid::Uuid::now_v7(),
            tenant_id: session.target_tenant,
            actor_ref,
            action: format!("breakglass.{action}"),
            subject_kind: "breakglass".to_owned(),
            reason: Some(session.reason.clone()),
            correlation_id: crate::infra::events::correlation_id(),
            written_at: now,
        },
        session.session_id,
        None,
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))
}

// Declared last on purpose: `retention_tests` reads this file's production
// half as everything before its first `#[cfg(test)]`.
#[cfg(test)]
mod elevation_tests;
