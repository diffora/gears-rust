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

pub(super) async fn scope(
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
pub(super) fn conflict(code: &'static str, detail: impl Into<String>) -> TxError {
    TxError::Refused(DomainError::Conflict {
        code,
        detail: detail.into(),
    })
}
pub(super) async fn find(
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
pub(super) async fn resolve(
    state: &ApiState,
    ctx: &SecurityContext,
    content: &SkuContent,
) -> Result<Option<UsageTypeAnswer>, CanonicalError> {
    let Some(reference) = content.usage_type_ref.as_deref() else {
        return Ok(None);
    };
    let answer = state.usage_type_catalog.resolve(ctx, reference).await;
    if matches!(answer, UsageTypeAnswer::Unavailable) {
        return Err(DomainError::UsageTypeUnavailable(reference.into()).into());
    }
    Ok(Some(answer))
}
/// Maintenance never releases a pending unit's fence and compares the observed operation.
pub(super) async fn expire(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    ttl: u32,
    now: OffsetDateTime,
) -> Result<(), TxError> {
    if let Some(row) = repo::find_sku_fence(tx, scope, tenant, id)
        .await
        .map_err(TxError::Repo)?
        && row.pending_unit_id.is_none()
        && row
            .fenced_at
            .is_some_and(|at| now - at >= time::Duration::minutes(i64::from(ttl)))
    {
        repo::unfence_sku(tx, scope, tenant, id, row.fence_op_id)
            .await
            .map_err(TxError::Repo)?;
    }
    Ok(())
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
/// Audit row identifiers belong to a separate aggregate from the authorized resource.
pub(super) async fn audit(
    tx: &impl DBRunner,
    _scope: &AccessScope,
    ctx: &SecurityContext,
    action: &str,
    kind: &str,
    id: Uuid,
    reason: Option<String>,
    now: OffsetDateTime,
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
