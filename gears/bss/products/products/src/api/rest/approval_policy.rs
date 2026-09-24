//! Direct tenant quorum policy under SETTINGS, with atomic audit.
//! @cpt-dod:cpt-cf-bss-products-dod-quorum-zero-records-unit:p1
use super::{
    ApiState, TxError, category_tx_config, contention_db_err,
    dto::{PolicyDto, PolicyRequest},
    governance as g, json_body, require_authenticated, tx_to_canonical,
};
use crate::{
    authz::actions,
    domain::approvals::{KIND_SKU_CHANGE, KIND_SKU_PUBLISH, KIND_SKU_RETIRE},
    infra::storage::repo,
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{
    Extension, Json, Router,
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use toolkit::api::{
    OpenApiRegistry, canonical_prelude::CanonicalError, operation_builder::OperationBuilder,
};
use toolkit_security::SecurityContext;
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-products/v1/approval-policy")
        .operation_id("bss_products.get_approval_policy")
        .summary("Read approval policy")
        .tag("Approval policy")
        .authenticated()
        .no_license_required()
        .handler(get)
        .json_response_with_schema::<PolicyDto>(openapi, StatusCode::OK, "Policy")
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);
    OperationBuilder::put("/bss-products/v1/approval-policy")
        .operation_id("bss_products.put_approval_policy")
        .summary("Set approval policy")
        .tag("Approval policy")
        .authenticated()
        .no_license_required()
        .json_request::<PolicyRequest>(openapi, "Policy")
        .handler(put)
        .json_response_with_schema::<PolicyDto>(openapi, StatusCode::OK, "Policy")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
        .layer(Extension(state))
}
async fn get(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::settings_read(&enforcer, &ctx).await?;
    let p = repo::read_policy(
        &state.db.conn().map_err(|e| tx_to_canonical(e.into()))?,
        &scope,
        ctx.subject_tenant_id(),
    )
    .await
    .map_err(|e| tx_to_canonical(TxError::Repo(e)))?;
    Ok(Json(PolicyDto::from(p)).into_response())
}
/// @cpt-cf-bss-products-fr-approval-units
async fn put(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    body: Result<Json<PolicyRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(&enforcer, &ctx, actions::SETTINGS, true).await?;
    let p = json_body(body)?;
    let kind = p.kind.unwrap_or_else(|| "*".into());
    if p.quorum > i32::MAX.cast_unsigned()
        || !matches!(
            kind.as_str(),
            "*" | KIND_SKU_PUBLISH | KIND_SKU_CHANGE | KIND_SKU_RETIRE
        )
    {
        return Err(g::validation("policy", "unknown kind or quorum exceeds storage range").into());
    }
    let policy = state
        .db
        .db()
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let scope = scope.clone();
            let ctx = ctx.clone();
            let kind = kind.clone();
            Box::pin(async move {
                repo::write_policy(tx, &scope, ctx.subject_tenant_id(), &kind, p.quorum)
                    .await
                    .map_err(TxError::Repo)?;
                g::audit(
                    tx,
                    &scope,
                    &ctx,
                    "approval_policy.write",
                    "approval_policy",
                    ctx.subject_tenant_id(),
                    None,
                    time::OffsetDateTime::now_utc(),
                )
                .await?;
                repo::read_policy(tx, &scope, ctx.subject_tenant_id())
                    .await
                    .map_err(TxError::Repo)
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    Ok(Json(PolicyDto::from(policy)).into_response())
}
