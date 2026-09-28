//! @cpt-dod:cpt-cf-bss-products-dod-usage-type-resolves:p1
//! @cpt-dod:cpt-cf-bss-products-dod-terminal-audit-and-event:p1
//! Shared scoped transaction plumbing for approval and reference operations.
use super::{ApiState, TxError, authz_error_to_canonical, contention_db_err, tx_to_canonical};
use crate::{
    authz::{access_scope, actions, resource_types},
    domain::{error::DomainError, recognized::UsageTypeAnswer, validation::ValidationReport},
    infra::{broker, events, storage::repo},
};
use authz_resolver_sdk::PolicyEnforcer;
use bss_approval::{Store, Unit};
use bss_products_sdk::models::{Sku, SkuContent};
use time::OffsetDateTime;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit_db::{
    DbTx,
    secure::{AccessScope, DBRunner},
};
use toolkit_security::SecurityContext;
use uuid::Uuid;
#[resource_error(gts_id!("cf.bss.products.sku.v1~"))]
struct Resource;

pub async fn scope(
    enforcer: &PolicyEnforcer,
    ctx: &SecurityContext,
    action: &str,
    units: bool,
) -> Result<AccessScope, CanonicalError> {
    let resource = if units {
        resource_types::APPROVAL_UNIT
    } else {
        resource_types::SKU
    };
    access_scope(
        enforcer,
        ctx,
        &resource,
        action,
        (action != actions::READ).then(|| ctx.subject_tenant_id()),
        None,
        true,
    )
    .await
    .map_err(|e| {
        authz_error_to_canonical(e, |reason| {
            Resource::permission_denied().with_reason(reason).create()
        })
    })
}
pub(super) fn validation(field: &str, detail: impl Into<String>) -> DomainError {
    let mut r = ValidationReport::new();
    r.violate("VALIDATION", field, detail);
    DomainError::Validation(r)
}
pub fn conflict(code: &'static str, detail: impl Into<String>) -> TxError {
    TxError::Refused(DomainError::Conflict {
        code,
        detail: detail.into(),
    })
}
pub async fn find(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Sku, TxError> {
    repo::find_sku(tx, scope, tenant, id)
        .await
        .map_err(TxError::Repo)?
        .ok_or(TxError::Refused(DomainError::NotFound { what: "sku", id }))
}
/// The SKU's lifecycle now, in the caller's transaction: the observed half of an audit row's
/// lifecycle move (P-D-213).
pub async fn lifecycle(
    tx: &impl DBRunner,
    tenant: Uuid,
    id: Uuid,
) -> Result<bss_products_sdk::models::Lifecycle, TxError> {
    find(tx, &AccessScope::for_tenant(tenant), tenant, id)
        .await
        .map(|s| s.lifecycle)
}
pub(super) async fn resolve(
    state: &ApiState,
    ctx: &SecurityContext,
    content: &SkuContent,
) -> Result<Option<UsageTypeAnswer>, CanonicalError> {
    let Some(reference) = content.usage_type_ref.as_deref() else {
        return Ok(None);
    };
    let answer = state.usage_type_catalog.resolve(ctx, reference).await;
    match answer {
        UsageTypeAnswer::Unavailable => {
            Err(DomainError::UsageTypeUnavailable(reference.into()).into())
        }
        // P-D-207: read as the caller; a denial is the caller's 403, not an outage.
        UsageTypeAnswer::Forbidden => Err(DomainError::UsageTypeForbidden(reference.into()).into()),
        UsageTypeAnswer::Resolved(_) | UsageTypeAnswer::Unresolved => Ok(Some(answer)),
    }
}
/// Maintenance never releases a pending unit's fence and compares the observed operation; a fence
/// it lifts is the system's act, with its audit row (P-D-213).
pub async fn expire(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    ttl: u32,
    now: OffsetDateTime,
) -> Result<(), TxError> {
    let cutoff = now - time::Duration::minutes(i64::from(ttl));
    if let Some(expired) = repo::expire_orphan_fence(tx, scope, tenant, id, cutoff)
        .await
        .map_err(TxError::Repo)?
    {
        expiry_audit(tx, tenant, expired, ttl, now).await?;
    }
    Ok(())
}
/// The audit row of an orphan fence the maintenance lifted (P-D-213): the system's act
/// (`repo::SYSTEM_ACTOR`) `sku.fence_expired` on the SKU, with the move it made and the TTL it
/// applied.
pub async fn expiry_audit(
    tx: &impl DBRunner,
    tenant: Uuid,
    expired: repo::ExpiredFence,
    ttl: u32,
    now: OffsetDateTime,
) -> Result<(), TxError> {
    repo::write_eventless_act_audit(
        tx,
        &AccessScope::for_tenant(tenant),
        expiry_row(tenant, &expired, ttl, now),
        expired.id,
        Some(expired.revision),
    )
    .await
    .map_err(TxError::Repo)
}
/// The audit rows of every orphan fence one read's expiry lifted (P-D-213), as ONE multi-row
/// insert whatever their number (P-D-211): each row is [`expiry_audit`]'s.
pub async fn expiry_audits(
    tx: &impl DBRunner,
    tenant: Uuid,
    expired: &[repo::ExpiredFence],
    ttl: u32,
    now: OffsetDateTime,
) -> Result<(), TxError> {
    repo::write_eventless_act_audits(
        tx,
        tenant,
        expired
            .iter()
            .map(|e| (expiry_row(tenant, e, ttl, now), e.id, Some(e.revision)))
            .collect(),
    )
    .await
    .map_err(TxError::Repo)
}
/// An expiry's audit row: the system's act on the SKU, the move it made, the TTL it applied.
fn expiry_row(
    tenant: Uuid,
    expired: &repo::ExpiredFence,
    ttl: u32,
    now: OffsetDateTime,
) -> repo::AuditCommon {
    repo::AuditCommon {
        audit_id: Uuid::now_v7(),
        tenant_id: tenant,
        actor_ref: repo::SYSTEM_ACTOR,
        action: "sku.fence_expired".into(),
        subject_kind: "sku".into(),
        reason: Some(format!("fence_ttl_minutes={ttl}")),
        correlation_id: None,
        written_at: now,
        lifecycle: repo::LifecycleMove::between(expired.from, expired.to),
    }
}
pub(super) async fn touch(
    state: &ApiState,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<(), CanonicalError> {
    let scope = scope.clone();
    let ttl = state.fence_ttl_minutes;
    state
        .db
        .db()
        .transaction_with_retry(
            super::category_tx_config(state),
            contention_db_err,
            move |tx| {
                let scope = scope.clone();
                Box::pin(async move {
                    expire(tx, &scope, tenant, id, ttl, OffsetDateTime::now_utc()).await
                })
            },
        )
        .await
        .map_err(tx_to_canonical)
}
#[allow(
    clippy::too_many_arguments,
    reason = "Audit inputs explicitly bind subject and actor to the caller transaction"
)]
/// Audit row identifiers belong to a separate aggregate from the authorized resource. `lifecycle`
/// is the SKU lifecycle move the act made (P-D-213): [`repo::LifecycleMove::NONE`] for an act on
/// no SKU (the policy, a reference).
pub async fn audit(
    tx: &impl DBRunner,
    _scope: &AccessScope,
    ctx: &SecurityContext,
    action: &str,
    kind: &str,
    id: Uuid,
    reason: Option<String>,
    now: OffsetDateTime,
    lifecycle: repo::LifecycleMove,
) -> Result<(), TxError> {
    repo::write_eventless_act_audit(
        tx,
        &AccessScope::for_tenant(ctx.subject_tenant_id()),
        repo::AuditCommon {
            audit_id: Uuid::now_v7(),
            tenant_id: ctx.subject_tenant_id(),
            actor_ref: ctx.subject_id(),
            action: action.into(),
            subject_kind: kind.into(),
            reason,
            correlation_id: None,
            written_at: now,
            lifecycle,
        },
        id,
        None,
    )
    .await
    .map_err(TxError::Repo)
}
pub(super) async fn decided(
    state: &ApiState,
    tx: &DbTx<'_>,
    store: &repo::ProductsApprovalStore,
    unit: &Unit,
    actor: Uuid,
) -> Result<(), TxError> {
    let mut actors = store
        .decisions(tx, unit.id)
        .await?
        .into_iter()
        .filter(|d| !d.stale)
        .map(|d| d.actor)
        .collect::<Vec<_>>();
    actors.push(actor);
    actors.sort();
    actors.dedup();
    events::enqueue_typed(
        &state.sink,
        tx,
        broker::ApprovalUnitDecided {
            tenant_id: unit.tenant_id,
            unit_id: unit.id,
            kind: unit.kind.clone(),
            state: unit.state.as_str().into(),
            generation: unit.generation,
            actors,
        },
    )
    .await
    .map_err(TxError::from)
}

/// Retain the canonical violation and expose a numeric generation for reviewer clients.
pub(super) fn generation_problem(
    error: CanonicalError,
    generation: i32,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let mut problem = toolkit::api::canonical_prelude::Problem::from(error);
    problem.context["generation"] = serde_json::json!(generation);
    problem.into_response()
}

/// Policy reads use SETTINGS while retaining the read-side absent tenant hint.
pub(super) async fn settings_read(
    enforcer: &PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<AccessScope, CanonicalError> {
    access_scope(
        enforcer,
        ctx,
        &resource_types::APPROVAL_UNIT,
        actions::SETTINGS,
        None,
        None,
        true,
    )
    .await
    .map_err(|e| {
        authz_error_to_canonical(e, |reason| {
            Resource::permission_denied().with_reason(reason).create()
        })
    })
}
