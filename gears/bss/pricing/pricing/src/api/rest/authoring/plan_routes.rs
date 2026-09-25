//! The plan, revision and item doors (phase 3): one `plan` label, read and author.
use super::{
    AuthoringState, dto, plan_items, plans,
    support::{authz_failure, header, require_authenticated, response, transaction},
};
use crate::{
    api::rest::{correlation, preconditions},
    authz::{self, OwnerTenant, ResourceRef, actions, resource_types},
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{
    Extension, Router,
    body::Bytes,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::Response,
};
use std::sync::Arc;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Plans and their revisions: create, list, read, rename, copy, clone, and the draft revision's
/// read, PATCH and delete.
#[allow(
    clippy::too_many_lines,
    reason = "one OperationBuilder chain per route keeps every door's contract in one place"
)]
pub(super) fn routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/bss-pricing/v1/plans")
        .operation_id("bss_pricing.create_plan")
        .summary("create_plan")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .json_request::<dto::PricingPlanCreate>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(create_plan)
        .json_response_with_schema::<dto::PricingPlanDto>(openapi, StatusCode::CREATED, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/plans")
        .operation_id("bss_pricing.list_plans")
        .summary("list_plans")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .handler(list_plans)
        .json_response_with_schema::<dto::PricingPlanList>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/plans/{id}")
        .operation_id("bss_pricing.get_plan")
        .summary("get_plan")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan id")
        .handler(get_plan)
        .json_response_with_schema::<dto::PricingPlanDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch("/bss-pricing/v1/plans/{id}")
        .operation_id("bss_pricing.patch_plan")
        .summary("patch_plan")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan id")
        .json_request::<dto::PricingPlanPatch>(openapi, "Request")
        .param(header("If-Match"))
        .handler(patch_plan)
        .json_response_with_schema::<dto::PricingPlanDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-pricing/v1/plans/{id}/revisions")
        .operation_id("bss_pricing.copy_plan_revision")
        .summary("copy_plan_revision")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan id")
        .param(header("Idempotency-Key"))
        .handler(copy_revision)
        .json_response_with_schema::<dto::PricingPlanRevisionDto>(
            openapi,
            StatusCode::CREATED,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-pricing/v1/plans/{id}/clone")
        .operation_id("bss_pricing.clone_plan")
        .summary("clone_plan")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Source plan id")
        .json_request::<dto::PricingPlanClone>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(clone_plan)
        .json_response_with_schema::<dto::PricingPlanDto>(openapi, StatusCode::CREATED, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/plan-revisions/{id}")
        .operation_id("bss_pricing.get_plan_revision")
        .summary("get_plan_revision")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan revision id")
        .handler(get_revision)
        .json_response_with_schema::<dto::PricingPlanRevisionDto>(
            openapi,
            StatusCode::OK,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch("/bss-pricing/v1/plan-revisions/{id}")
        .operation_id("bss_pricing.patch_plan_revision")
        .summary("patch_plan_revision")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan revision id")
        .json_request::<dto::PricingPlanRevisionPatch>(openapi, "Request")
        .param(header("If-Match"))
        .handler(patch_revision)
        .json_response_with_schema::<dto::PricingPlanRevisionDto>(
            openapi,
            StatusCode::OK,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    OperationBuilder::delete("/bss-pricing/v1/plan-revisions/{id}")
        .operation_id("bss_pricing.delete_plan_revision")
        .summary("delete_plan_revision")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan revision id")
        .handler(delete_revision)
        .no_content_response(StatusCode::NO_CONTENT, "Deleted")
        .standard_errors(openapi)
        .register(router, openapi)
}
async fn create_plan(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    corr: Option<Extension<correlation::CorrelationId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let payload: serde_json::Value = preconditions::parse_body(&body)?;
    let digest = preconditions::request_digest(&payload)?;
    let input: dto::PricingPlanCreate = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, input) = (scope.clone(), ctx.clone(), input.clone());
        let (key, digest) = (key.clone(), digest.clone());
        Box::pin(
            async move { plans::create(tx, &scope, &ctx, correlation, &key, &digest, input).await },
        )
    })
    .await
}
async fn list_plans(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let body = plans::list(tx, &scope, ctx.subject_tenant_id()).await?;
            Ok(response(StatusCode::OK, &body, None)?)
        })
    })
    .await
}
async fn get_plan(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::READ,
        None,
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move { plans::get(tx, &scope, ctx.subject_tenant_id(), id).await })
    })
    .await
}
async fn patch_plan(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    corr: Option<Extension<correlation::CorrelationId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let version = preconditions::if_match(&headers)?.get();
    let input: dto::PricingPlanPatch = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, input) = (scope.clone(), ctx.clone(), input.clone());
        Box::pin(
            async move { plans::patch(tx, &scope, &ctx, correlation, id, version, input).await },
        )
    })
    .await
}
async fn copy_revision(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    corr: Option<Extension<correlation::CorrelationId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let digest = preconditions::request_digest(&super::support::empty_body(&body)?)?;
    plans::copy(state, scope, ctx, correlation, id, key, digest).await
}
async fn clone_plan(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    corr: Option<Extension<correlation::CorrelationId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let payload: serde_json::Value = preconditions::parse_body(&body)?;
    let digest = preconditions::request_digest(&payload)?;
    let input: dto::PricingPlanClone = preconditions::parse_body(&body)?;
    plans::clone(state, scope, ctx, correlation, id, key, digest, input).await
}
async fn get_revision(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move { plans::get_revision(tx, &scope, ctx.subject_tenant_id(), id).await })
    })
    .await
}
async fn patch_revision(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    corr: Option<Extension<correlation::CorrelationId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let version = preconditions::if_match(&headers)?.get();
    let input: dto::PricingPlanRevisionPatch = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, input) = (scope.clone(), ctx.clone(), input.clone());
        Box::pin(async move {
            plans::patch_revision(tx, &scope, &ctx, correlation, id, version, input).await
        })
    })
    .await
}
async fn delete_revision(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    corr: Option<Extension<correlation::CorrelationId>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    plans::delete_revision(state, scope, ctx, correlation, id).await
}
/// Items of a draft revision and the revision's checks.
pub(super) fn item_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/bss-pricing/v1/plan-revisions/{id}/items")
        .operation_id("bss_pricing.create_plan_item")
        .summary("create_plan_item")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan revision id")
        .json_request::<dto::PricingPlanItemCreate>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(create_item)
        .json_response_with_schema::<dto::PricingPlanItemDto>(
            openapi,
            StatusCode::CREATED,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch("/bss-pricing/v1/plan-items/{id}")
        .operation_id("bss_pricing.patch_plan_item")
        .summary("patch_plan_item")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan item id")
        .json_request::<dto::PricingPlanItemPatch>(openapi, "Request")
        .param(header("If-Match"))
        .handler(patch_item)
        .json_response_with_schema::<dto::PricingPlanItemDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::delete("/bss-pricing/v1/plan-items/{id}")
        .operation_id("bss_pricing.delete_plan_item")
        .summary("delete_plan_item")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan item id")
        .handler(delete_item)
        .no_content_response(StatusCode::NO_CONTENT, "Deleted")
        .standard_errors(openapi)
        .register(router, openapi);
    OperationBuilder::get("/bss-pricing/v1/plan-revisions/{id}/checks")
        .operation_id("bss_pricing.get_plan_revision_checks")
        .summary("get_plan_revision_checks")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan revision id")
        .handler(get_checks)
        .json_response_with_schema::<dto::PricingPlanChecksDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi)
}
async fn create_item(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    corr: Option<Extension<correlation::CorrelationId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let payload: serde_json::Value = preconditions::parse_body(&body)?;
    let digest = preconditions::request_digest(&payload)?;
    let input: dto::PricingPlanItemCreate = preconditions::parse_body(&body)?;
    plan_items::add(state, scope, ctx, id, correlation, key, digest, input).await
}
async fn patch_item(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    corr: Option<Extension<correlation::CorrelationId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let version = preconditions::if_match(&headers)?.get();
    let input: dto::PricingPlanItemPatch = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, input) = (scope.clone(), ctx.clone(), input.clone());
        Box::pin(async move {
            plan_items::patch(tx, &scope, &ctx, correlation, id, version, input).await
        })
    })
    .await
}
async fn delete_item(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    corr: Option<Extension<correlation::CorrelationId>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    plan_items::delete(state, scope, ctx, correlation, id).await
}
async fn get_checks(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PLAN,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    plans::checks(&state, scope, ctx, id).await
}
