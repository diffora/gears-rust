//! REST surface.
//!
//! The gear's reserved prefix, `/bss-products/v1`, is claimed here so
//! `gear.rs` never spells it twice. Phase 4 Slice C landed the **read door**
//! — `GET /bss-products/v1/products/{id}` and `GET /bss-products/v1/skus/{id}`
//! — as `products` and `skus`, sibling modules under `api::rest`, following
//! the sibling pricing and ledger gears' shape: one module per resource,
//! composed in [`crate::gear::BssProductsGear::register_rest`] rather than
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
//! [`ApiState`] carries `outbox`, the running
//! [`toolkit_db::outbox::Outbox`] the create doors enqueue their events
//! through (P-D-22, `crate::infra::events`). Both edges are made in
//! `gear.rs`: `Gear::init` declares
//! [`crate::infra::events::QUEUE_NAME`] with
//! [`crate::infra::events::PARTITIONS`] partitions, and `register_rest`
//! populates this field from the handle the runtime holds.
//!
//! A queue cannot be declared without a handler — `.transactional(..)` or
//! `.leased(..)` is mandatory, and `enqueue` refuses an unregistered queue
//! with `OutboxError::QueueNotRegistered`. The handler registered today is
//! [`crate::infra::events::PendingBrokerProducer`], which holds messages
//! rather than delivering them: P-D-47 makes the real processor the broker
//! SDK's `DbProducer`, and the plan puts that in Phase 8's
//! `dod-outbox-eventing`.
//!
//! An earlier draft of this doc described both edges as owed to a future
//! slice, because the slice that wrote it could not touch `gear.rs`. They
//! have since been made, and the description outlived them — which is why
//! this section now names where the wiring is rather than where it is not.
//!

use std::sync::Arc;

use axum::Router;
use axum::extract::Extension;
use chrono::{DateTime, Utc};
use sea_orm::DbErr;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_db::DbError;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::authz::AuthzError;
use crate::domain::error::DomainError;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{self, AuditCommon, RefusalSubject};

pub mod preconditions;
pub mod products;
pub mod skus;

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
#[derive(Clone)]
pub(crate) struct ApiState {
    /// The provider `state.db.conn()` opens a non-transactional runner from,
    /// and `state.db.transaction(..)` opens a transactional one from. The
    /// read door only ever calls `.conn()`; the create door
    /// (`products::create_product`) calls `.transaction(..)` three times —
    /// once for `resolve_actor_ref`, once for the entity insert plus its
    /// outbox row, and once more, conditionally, for a refusal's audit row —
    /// each its own, per this module's doc on the entity/outbox pairing and
    /// `infra::storage::repo`'s own doc on the other two.
    pub(crate) db: toolkit_db::DBProvider<toolkit_db::DbError>,
    /// The running transactional-outbox pipeline the create doors enqueue
    /// their events through, inside the same mutation transaction that
    /// writes the entity row (P-D-22). Populated by `gear.rs`'s
    /// `register_rest` from the handle the runtime holds; see this module's
    /// doc, "The outbox wiring, and where it lives".
    pub(crate) outbox: Arc<toolkit_db::outbox::Outbox>,
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
/// Both variants are refusals of an invariant the request did not cause —
/// a scope/driver failure, or a stored value the database's own `CHECK`
/// constraints should have kept from ever landing — so both render as a bare
/// 500 with no resource marker and no registry code, the same shape
/// `infra::error_mapping`'s `AuditUnavailable` arm uses for the identical
/// reason: neither a `Product` nor a `SKU` refused anything, so tagging
/// either with a resource marker would name something that did not happen.
pub(crate) fn repo_error_to_canonical(err: &RepoError) -> CanonicalError {
    tracing::error!(error = %err, "bss-products: repository failure on the read door");
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
    let common = AuditCommon {
        audit_id: Uuid::new_v4(),
        tenant_id: ctx.tenant_id,
        actor_ref: ctx.actor_ref,
        action: "create".to_owned(),
        subject_kind: ctx.subject_kind.to_owned(),
        reason: Some(format!("{}: refused at create", ctx.error_code)),
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
