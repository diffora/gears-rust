//! @cpt-dod:cpt-cf-bss-products-dod-reserve-refused-when-fenced:p1
//! Owner-bound reservations and explicit, audited operator release.
//! @cpt-dod:cpt-cf-bss-products-dod-reference-registry:p1
use super::{
    ApiState, TxError, category_tx_config, contention_db_err,
    dto::{ReferenceReceipt, ReleaseRequest, ReserveRequest},
    governance as g, json_body, replay, require_authenticated, tx_to_canonical,
};
use crate::{
    authz::actions,
    domain::{
        error::DomainError,
        references::{RefKind, reservation_allowed},
    },
    infra::{
        broker, events,
        storage::{RepoError, repo},
    },
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{
    Extension, Json, Router,
    extract::{Path, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use toolkit::api::{
    OpenApiRegistry, canonical_prelude::CanonicalError, operation_builder::OperationBuilder,
};
use toolkit_security::SecurityContext;
use uuid::Uuid;
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = OperationBuilder::post("/bss-products/v1/skus/{id}/references/reserve")
        .operation_id("bss_products.reserve_reference")
        .summary("reserve_reference")
        .tag("References")
        .authenticated()
        .no_license_required()
        .path_param("id", "Resource id")
        .json_request::<ReserveRequest>(openapi, "Request")
        .param(replay::param())
        .handler(reserve)
        .json_response_with_schema::<ReferenceReceipt>(openapi, StatusCode::OK, "Reference")
        .json_response_with_schema::<ReferenceReceipt>(
            openapi,
            StatusCode::CREATED,
            "New reservation",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-products/v1/references/{id}/confirm")
        .operation_id("bss_products.confirm_reference")
        .summary("confirm_reference")
        .tag("References")
        .authenticated()
        .no_license_required()
        .path_param("id", "Resource id")
        .param(replay::param())
        .handler(confirm)
        .json_response_with_schema::<ReferenceReceipt>(openapi, StatusCode::OK, "Reference")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::delete("/bss-products/v1/references/{id}")
        .operation_id("bss_products.release_reference")
        .summary("release_reference")
        .tag("References")
        .authenticated()
        .no_license_required()
        .path_param("id", "Resource id")
        .json_request::<ReleaseRequest>(openapi, "Request")
        .handler(release)
        .json_response_with_schema::<ReferenceReceipt>(openapi, StatusCode::OK, "Reference")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    router.layer(Extension(state))
}
fn owner<'a>(state: &'a ApiState, ctx: &SecurityContext) -> Option<&'a str> {
    state
        .reference_principals
        .get(&ctx.subject_id())
        .map(String::as_str)
}
pub(crate) fn forbidden() -> DomainError {
    DomainError::Forbidden {
        code: "REFERENCE_OWNER_MISMATCH",
        detail: "principal is not the registered owner gear".into(),
    }
}
/// @cpt-cf-bss-products-fr-reference-registry
async fn reserve(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(&enforcer, &ctx, actions::REFERENCE, false).await?;
    let payload = json_body(body)?;
    let claim = replay::input(
        &state,
        &headers,
        format!("/bss-products/v1/skus/{id}/references/reserve"),
        &payload,
    )?;
    let body: ReserveRequest = serde_json::from_value(payload)
        .map_err(|e| CanonicalError::from(g::validation("body", e.to_string())))?;
    if owner(&state, &ctx) != Some(body.owner.as_str()) {
        return Err(forbidden().into());
    }
    let kind = match body.kind.as_str() {
        "price_book_entry" => RefKind::PriceBookEntry,
        "plan_item" => RefKind::PlanItem,
        "sold_as" => RefKind::SoldAs,
        _ => return Err(g::validation("kind", "unknown reference kind").into()),
    };
    let db = state.db.db();
    // A unique loser rolls back before retrying the logical-reference read.
    for attempt in 0..2 {
        let state_tx = state.clone();
        let scope_tx = scope.clone();
        let ctx_tx = ctx.clone();
        let claim_tx = claim.clone();
        let owner_tx = body.owner.clone();
        let result = db
            .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
                let state = state_tx.clone();
                let scope = scope_tx.clone();
                let ctx = ctx_tx.clone();
                let claim = claim_tx.clone();
                let owner = owner_tx.clone();
                Box::pin(async move {
                    let tenant = ctx.subject_tenant_id();
                    g::find(tx, &scope, tenant, id).await?;
                    if let Some(response) = replay::begin(tx, tenant, claim.as_ref()).await? {
                        return Ok(response);
                    }
                    let (row, created) = reserve_tx(
                        tx,
                        &scope,
                        &ctx,
                        &owner,
                        id,
                        kind,
                        body.ref_id,
                        state.fence_ttl_minutes,
                    )
                    .await?;
                    replay::finish(
                        tx,
                        tenant,
                        claim.as_ref(),
                        if created {
                            StatusCode::CREATED
                        } else {
                            StatusCode::OK
                        },
                        &ReferenceReceipt::from(row),
                    )
                    .await
                })
            })
            .await;
        match result {
            Err(TxError::Repo(RepoError::Db(code)))
                if code == "REFERENCE_EXISTS" && attempt == 0 => {}
            other => return other.map_err(tx_to_canonical),
        }
    }
    Err(tx_to_canonical(g::conflict(
        "REFERENCE_EXISTS",
        "logical reference changed; retry",
    )))
}
async fn confirm(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(&enforcer, &ctx, actions::REFERENCE, false).await?;
    let owner = owner(&state, &ctx)
        .ok_or_else(|| CanonicalError::from(forbidden()))?
        .to_owned();
    let claim = replay::input(
        &state,
        &headers,
        format!("/bss-products/v1/references/{id}/confirm"),
        &serde_json::json!({}),
    )?;
    let ttl = state.fence_ttl_minutes;
    let row = state
        .db
        .db()
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let scope = scope.clone();
            let ctx = ctx.clone();
            let owner = owner.clone();
            let claim = claim.clone();
            Box::pin(async move {
                let tenant = ctx.subject_tenant_id();
                let row = repo::find_reference(tx, &scope, tenant, id)
                    .await
                    .map_err(TxError::Repo)?
                    .ok_or(TxError::Refused(DomainError::NotFound {
                        what: "reference",
                        id,
                    }))?;

                if row.owner_gear != owner {
                    return Err(TxError::Refused(forbidden()));
                }
                if let Some(response) = replay::begin(tx, tenant, claim.as_ref()).await? {
                    return Ok(response);
                }
                let row = confirm_tx(tx, &scope, &ctx, &owner, id, ttl).await?;
                replay::finish(
                    tx,
                    tenant,
                    claim.as_ref(),
                    StatusCode::OK,
                    &ReferenceReceipt::from(row),
                )
                .await
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    Ok(row)
}
/// @cpt-cf-bss-products-fr-reference-registry
async fn release(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    body: Result<Json<ReleaseRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    // Authorization precedes body/row disclosure; force explicitly selects the operator route.
    let principal_owner = owner(&state, &ctx).map(str::to_owned);
    let scope = g::scope(
        &enforcer,
        &ctx,
        if principal_owner.is_some() {
            actions::REFERENCE
        } else {
            actions::SUBMIT
        },
        false,
    )
    .await?;
    let body = json_body(body)?;
    let forced = principal_owner.is_none() || body.force;
    let scope = if body.force && principal_owner.is_some() {
        g::scope(&enforcer, &ctx, actions::SUBMIT, false).await?
    } else {
        scope
    };
    if forced && (!body.force || body.reason.as_deref().is_none_or(|s| s.trim().is_empty())) {
        return Err(g::validation(
            "force",
            "operator release requires force and a nonempty reason",
        )
        .into());
    }
    let db = state.db.db();
    let row = db
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let state = state.clone();
            let scope = scope.clone();
            let ctx = ctx.clone();
            let principal_owner = principal_owner.clone();
            let reason = body.reason.clone();
            Box::pin(async move {
                let tenant = ctx.subject_tenant_id();
                if !forced {
                    return release_tx(
                        tx,
                        &scope,
                        &ctx,
                        principal_owner.as_deref().unwrap_or_default(),
                        id,
                        state.fence_ttl_minutes,
                    )
                    .await;
                }
                let now = time::OffsetDateTime::now_utc();
                let row = repo::find_reference(tx, &scope, tenant, id)
                    .await
                    .map_err(TxError::Repo)?
                    .ok_or(TxError::Refused(DomainError::NotFound {
                        what: "reference",
                        id,
                    }))?;
                g::expire(tx, &scope, tenant, row.sku_id, state.fence_ttl_minutes, now).await?;
                if !forced && principal_owner.as_deref() != Some(row.owner_gear.as_str()) {
                    return Err(TxError::Refused(forbidden()));
                }
                let released = repo::release_reference(
                    tx,
                    &scope,
                    tenant,
                    id,
                    ctx.subject_id(),
                    reason.as_deref(),
                    forced,
                    now,
                )
                .await
                .map_err(TxError::Repo)?;
                let repo::HeadWrite::Written(row) = released else {
                    return Ok(row);
                };
                g::audit(
                    tx,
                    &scope,
                    &ctx,
                    if forced {
                        "reference.force_release"
                    } else {
                        "reference.release"
                    },
                    "sku_reference",
                    id,
                    reason.clone(),
                    now,
                )
                .await?;
                if forced {
                    events::enqueue_typed(
                        &state.sink,
                        tx,
                        broker::ReferenceForceReleased {
                            tenant_id: tenant,
                            sku_id: row.sku_id,
                            reference_id: id,
                            owner: row.owner_gear.clone(),
                            kind: row.ref_kind.clone(),
                            ref_id: row.ref_id,
                            actor_ref: ctx.subject_id(),
                            reason: reason.unwrap_or_default(),
                        },
                    )
                    .await
                    .map_err(TxError::from)?;
                }
                Ok(row)
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    Ok(Json(ReferenceReceipt::from(row)).into_response())
}

// Shared transaction operations: REST adds replay envelopes, local callers add owner binding.
use crate::infra::storage::entity::sku_reference;
use toolkit_db::secure::{AccessScope, DBRunner};
#[allow(
    clippy::too_many_arguments,
    reason = "Explicit reservation identity and transaction context"
)]
pub(crate) async fn reserve_tx(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    owner: &str,
    id: Uuid,
    kind: RefKind,
    ref_id: Uuid,
    ttl: u32,
) -> Result<(sku_reference::Model, bool), TxError> {
    let tenant = ctx.subject_tenant_id();
    g::find(tx, scope, tenant, id).await?;
    let scope = AccessScope::for_tenant(tenant);
    let now = time::OffsetDateTime::now_utc();
    g::expire(tx, &scope, tenant, id, ttl, now).await?;
    if let Some(row) = repo::find_live_reference(tx, &scope, tenant, owner, kind, ref_id)
        .await
        .map_err(TxError::Repo)?
    {
        if row.sku_id != id {
            return Err(g::conflict(
                "REFERENCE_EXISTS",
                "logical reference already belongs to another SKU",
            ));
        }
        return Ok((row, false));
    }
    let s = g::find(tx, &scope, tenant, id).await?;
    // Distinguish the actual retirement fence from an unfenced retiring head.
    // Both refuse adoption; normal retire operations retain their existing SKU_FENCED code.
    let retiring_fence = if s.lifecycle == bss_products_sdk::Lifecycle::Retiring {
        repo::find_sku_fence(tx, &scope, tenant, id)
            .await
            .map_err(TxError::Repo)?
            .is_some_and(|row| {
                row.fenced_at.is_some()
                    || row.fence_op_id.is_some()
                    || row.pending_unit_id.is_some()
            })
    } else {
        false
    };
    reservation_allowed(s.lifecycle, s.type_change_pending || retiring_fence)
        .map_err(TxError::Refused)?;
    let row = repo::reserve_reference(
        tx,
        &scope,
        tenant,
        id,
        owner,
        kind,
        ref_id,
        ctx.subject_id(),
        now,
    )
    .await
    .map_err(TxError::Repo)?;
    reference_audit(tx, ctx, owner, "reference.reserve", row.id, now).await?;
    Ok((row, true))
}
pub(crate) async fn owned(
    tx: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    owner: &str,
    id: Uuid,
) -> Result<sku_reference::Model, TxError> {
    let row = repo::find_reference(tx, scope, tenant, id)
        .await
        .map_err(TxError::Repo)?
        .ok_or(TxError::Refused(DomainError::NotFound {
            what: "reference",
            id,
        }))?;
    if row.owner_gear != owner {
        return Err(TxError::Refused(forbidden()));
    }
    Ok(row)
}
pub(crate) async fn confirm_tx(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    owner: &str,
    id: Uuid,
    ttl: u32,
) -> Result<sku_reference::Model, TxError> {
    let tenant = ctx.subject_tenant_id();
    let row = owned(tx, scope, tenant, owner, id).await?;
    let scope = AccessScope::for_tenant(tenant);
    let now = time::OffsetDateTime::now_utc();
    g::expire(tx, &scope, tenant, row.sku_id, ttl, now).await?;
    match repo::confirm_reference(tx, &scope, tenant, id, now)
        .await
        .map_err(TxError::Repo)?
    {
        repo::ConfirmOutcome::Released => {
            return Err(g::conflict(
                "REFERENCE_RELEASED",
                "released attempts cannot be confirmed",
            ));
        }
        repo::ConfirmOutcome::Missing => {
            return Err(TxError::Refused(DomainError::NotFound {
                what: "reference",
                id,
            }));
        }
        repo::ConfirmOutcome::Confirmed => {
            reference_audit(tx, ctx, owner, "reference.confirm", id, now).await?;
        }
        repo::ConfirmOutcome::AlreadyConfirmed => {}
    }
    owned(tx, &scope, tenant, owner, id).await
}
pub(crate) async fn release_tx(
    tx: &impl DBRunner,
    scope: &AccessScope,
    ctx: &SecurityContext,
    owner: &str,
    id: Uuid,
    ttl: u32,
) -> Result<sku_reference::Model, TxError> {
    let tenant = ctx.subject_tenant_id();
    let row = owned(tx, scope, tenant, owner, id).await?;
    let now = time::OffsetDateTime::now_utc();
    g::expire(tx, scope, tenant, row.sku_id, ttl, now).await?;
    if let repo::HeadWrite::Written(row) =
        repo::release_reference(tx, scope, tenant, id, ctx.subject_id(), None, false, now)
            .await
            .map_err(TxError::Repo)?
    {
        reference_audit(tx, ctx, owner, "reference.release", id, now).await?;
        return Ok(row);
    }
    Ok(row)
}
async fn reference_audit(
    tx: &impl DBRunner,
    ctx: &SecurityContext,
    owner: &str,
    action: &str,
    id: Uuid,
    now: time::OffsetDateTime,
) -> Result<(), TxError> {
    let actor_kind = if ctx.subject_type().is_some_and(|s| s.ends_with(".system")) {
        "system"
    } else {
        "subject"
    };
    g::audit(
        tx,
        &AccessScope::for_tenant(ctx.subject_tenant_id()),
        ctx,
        action,
        "sku_reference",
        id,
        Some(format!("owner={owner}; actor_kind={actor_kind}")),
        now,
    )
    .await
}
