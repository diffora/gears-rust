//! @cpt-dod:cpt-cf-bss-products-dod-reserve-refused-when-fenced:p1
//! Owner-bound reservations and explicit, audited operator release.
use super::{
    ApiState, TxError, category_tx_config, contention_db_err,
    dto::{ReferenceReceipt, ReleaseRequest, ReserveRequest},
    governance as g, json_body, require_authenticated, tx_to_canonical,
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
    http::StatusCode,
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
fn forbidden() -> DomainError {
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
    body: Result<Json<ReserveRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(&enforcer, &ctx, actions::REFERENCE, false).await?;
    let body = json_body(body)?;
    if owner(&state, &ctx) != Some(body.owner.as_str()) {
        return Err(forbidden().into());
    }
    let kind = match body.kind.as_str() {
        "price" => RefKind::Price,
        "plan_item" => RefKind::PlanItem,
        "sold_as" => RefKind::SoldAs,
        _ => return Err(g::validation("kind", "unknown reference kind").into()),
    };
    let db = state.db.db();
    let replay_scope = scope.clone();
    let replay_owner = body.owner.clone();
    let replay_tenant = ctx.subject_tenant_id();
    let result = db
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let state = state.clone();
            let scope = scope.clone();
            let ctx = ctx.clone();
            let owner = body.owner.clone();
            Box::pin(async move {
                let tenant = ctx.subject_tenant_id();
                let now = time::OffsetDateTime::now_utc();
                g::expire(tx, &scope, tenant, id, state.fence_ttl_minutes, now).await?;
                let s = g::find(tx, &scope, tenant, id).await?;
                if let Some(row) =
                    repo::find_live_reference(tx, &scope, tenant, &owner, kind, body.ref_id)
                        .await
                        .map_err(TxError::Repo)?
                {
                    if row.sku_id != id {
                        return Err(g::conflict(
                            "REFERENCE_EXISTS",
                            "logical reference already belongs to another SKU",
                        ));
                    }
                    return Ok((false, row));
                }
                reservation_allowed(
                    s.lifecycle,
                    s.type_change_pending
                        || s.lifecycle == bss_products_sdk::models::Lifecycle::Retiring,
                )
                .map_err(TxError::Refused)?;
                let row = repo::reserve_reference(
                    tx,
                    &scope,
                    tenant,
                    id,
                    &owner,
                    kind,
                    body.ref_id,
                    ctx.subject_id(),
                    now,
                )
                .await
                .map_err(TxError::Repo)?;
                g::audit(
                    tx,
                    &scope,
                    &ctx,
                    "reference.reserve",
                    "sku_reference",
                    row.id,
                    None,
                    now,
                )
                .await?;
                Ok((true, row))
            })
        })
        .await;
    // A unique loser must roll back before reading the winner on PostgreSQL.
    let (created, row) = match result {
        Ok(result) => result,
        Err(TxError::Repo(RepoError::Db(code))) if code == "REFERENCE_EXISTS" => {
            let conn = db.conn().map_err(|e| tx_to_canonical(e.into()))?;
            let row = repo::find_live_reference(
                &conn,
                &replay_scope,
                replay_tenant,
                &replay_owner,
                kind,
                body.ref_id,
            )
            .await
            .map_err(|e| tx_to_canonical(TxError::Repo(e)))?
            .ok_or_else(|| {
                tx_to_canonical(g::conflict(
                    "REFERENCE_EXISTS",
                    "winning reference was released; retry",
                ))
            })?;
            if row.sku_id != id {
                return Err(tx_to_canonical(g::conflict(
                    "REFERENCE_EXISTS",
                    "logical reference belongs to another SKU",
                )));
            }
            (false, row)
        }
        Err(e) => return Err(tx_to_canonical(e)),
    };
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(ReferenceReceipt::from(row)),
    )
        .into_response())
}
async fn confirm(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(&enforcer, &ctx, actions::REFERENCE, false).await?;
    let owner = owner(&state, &ctx)
        .ok_or_else(|| CanonicalError::from(forbidden()))?
        .to_owned();
    let ttl = state.fence_ttl_minutes;
    let row = state
        .db
        .db()
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let scope = scope.clone();
            let ctx = ctx.clone();
            let owner = owner.clone();
            Box::pin(async move {
                let tenant = ctx.subject_tenant_id();
                let now = time::OffsetDateTime::now_utc();
                let row = repo::find_reference(tx, &scope, tenant, id)
                    .await
                    .map_err(TxError::Repo)?
                    .ok_or(TxError::Refused(DomainError::NotFound {
                        what: "reference",
                        id,
                    }))?;
                g::expire(tx, &scope, tenant, row.sku_id, ttl, now).await?;
                if row.owner_gear != owner {
                    return Err(TxError::Refused(forbidden()));
                }
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
                        g::audit(
                            tx,
                            &scope,
                            &ctx,
                            "reference.confirm",
                            "sku_reference",
                            id,
                            None,
                            now,
                        )
                        .await?;
                    }
                    repo::ConfirmOutcome::AlreadyConfirmed => {}
                }
                repo::find_reference(tx, &scope, tenant, id)
                    .await
                    .map_err(TxError::Repo)?
                    .ok_or(TxError::Refused(DomainError::NotFound {
                        what: "reference",
                        id,
                    }))
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    Ok(Json(ReferenceReceipt::from(row)).into_response())
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
                    .map_err(|e| TxError::Repo(RepoError::Db(e.to_string())))?;
                }
                Ok(row)
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    Ok(Json(ReferenceReceipt::from(row)).into_response())
}
