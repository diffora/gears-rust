//! Books, prices, rows, approvals, dimension keys and settings REST doors.
mod approvals;
mod books;
mod configuration;
pub mod dto;
mod prices;
pub(crate) mod rows;
pub(crate) mod support;
use super::{correlation, preconditions};
use crate::{
    authz::{self, OwnerTenant, ResourceRef, actions, resource_types},
    infra::storage::repo::book_repo,
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{
    Extension, Router,
    body::Bytes,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::Response,
};
use dto::{
    PriceBookCreate, PriceBookDto, PriceBookExport, PriceBookList, PriceBookPatch,
    PricingDimensions, PricingPriceList, PricingSettingsDto, PricingSettingsPut,
};
use std::sync::Arc;
use support::{authz_failure, header, require_authenticated, response, transaction};
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;
/// Dependencies shared by every authoring request.
pub struct AuthoringState {
    pub db: toolkit_db::DBProvider<toolkit_db::DbError>,
    pub hub: Arc<toolkit::ClientHub>,
    pub outbox: Arc<toolkit_db::outbox::Outbox>,
    pipeline: tokio::sync::Mutex<Option<toolkit_db::outbox::OutboxHandle>>,
}
impl AuthoringState {
    /// Attach the durable event queue to the runtime database.
    /// # Errors
    /// Fails initialization if the toolkit queue cannot start.
    pub async fn new(
        db: toolkit_db::DBProvider<toolkit_db::DbError>,
        hub: Arc<toolkit::ClientHub>,
    ) -> Result<Self, toolkit_db::outbox::OutboxError> {
        let pipeline = toolkit_db::outbox::Outbox::builder(db.db())
            .table_prefix(crate::infra::events::OUTBOX_TABLE_PREFIX)?
            .queue(
                crate::infra::events::QUEUE,
                toolkit_db::outbox::Partitions::of(1),
            )
            .leased(crate::infra::events::PendingProducer)
            .start()
            .await?;
        Ok(Self {
            db,
            hub,
            outbox: pipeline.outbox().clone(),
            pipeline: tokio::sync::Mutex::new(Some(pipeline)),
        })
    }
    pub(crate) async fn stop(&self) {
        if let Some(pipeline) = self.pipeline.lock().await.take() {
            pipeline.stop().await;
        }
    }
}
/// Mount the complete authoring surface and establish one audit correlation per request.
pub fn router(state: Arc<AuthoringState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = OperationBuilder::post("/bss-pricing/v1/price-books")
        .operation_id("bss_pricing.create_book")
        .summary("create_book")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .json_request::<PriceBookCreate>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(create_book)
        .json_response_with_schema::<PriceBookDto>(openapi, StatusCode::CREATED, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/price-books")
        .operation_id("bss_pricing.list_books")
        .summary("list_books")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .handler(list_books)
        .json_response_with_schema::<PriceBookList>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/price-books/{id}")
        .operation_id("bss_pricing.get_book")
        .summary("get_book")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price book id")
        .handler(get_book)
        .json_response_with_schema::<PriceBookDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch("/bss-pricing/v1/price-books/{id}")
        .operation_id("bss_pricing.patch_book")
        .summary("patch_book")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price book id")
        .json_request::<PriceBookPatch>(openapi, "Request")
        .param(header("If-Match"))
        .handler(patch_book)
        .json_response_with_schema::<PriceBookDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/price-books/{id}/prices")
        .operation_id("bss_pricing.list_prices")
        .summary("list_prices")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price book id")
        .handler(list_prices)
        .json_response_with_schema::<PricingPriceList>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/price-books/{id}/export")
        .operation_id("bss_pricing.export_book")
        .summary("export_book")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price book id")
        .handler(export_book)
        .json_response_with_schema::<PriceBookExport>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/settings")
        .operation_id("bss_pricing.get_settings")
        .summary("get_settings")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .handler(get_settings)
        .json_response_with_schema::<PricingSettingsDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::put("/bss-pricing/v1/settings")
        .operation_id("bss_pricing.put_settings")
        .summary("put_settings")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .json_request::<PricingSettingsPut>(openapi, "Request")
        .param(header("If-Match"))
        .handler(put_settings)
        .json_response_with_schema::<PricingSettingsDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/dimension-keys")
        .operation_id("bss_pricing.get_dimensions")
        .summary("get_dimensions")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .handler(get_dimensions)
        .json_response_with_schema::<PricingDimensions>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::put("/bss-pricing/v1/dimension-keys")
        .operation_id("bss_pricing.put_dimensions")
        .summary("put_dimensions")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .json_request::<PricingDimensions>(openapi, "Request")
        .param(header("If-Match"))
        .handler(put_dimensions)
        .json_response_with_schema::<PricingDimensions>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-pricing/v1/price-books/{id}/prices")
        .operation_id("bss_pricing.create_price")
        .summary("create_price")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price or book id")
        .json_request::<dto::PricingPriceCreate>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(create_price)
        .json_response_with_schema::<dto::PricingPriceDto>(openapi, StatusCode::CREATED, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/prices/{id}")
        .operation_id("bss_pricing.get_price")
        .summary("get_price")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price or book id")
        .handler(get_price)
        .json_response_with_schema::<dto::PricingPriceDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch("/bss-pricing/v1/prices/{id}")
        .operation_id("bss_pricing.patch_price")
        .summary("patch_price")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price or book id")
        .json_request::<dto::PricingPricePatch>(openapi, "Request")
        .param(header("If-Match"))
        .handler(patch_price)
        .json_response_with_schema::<dto::PricingPriceDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::delete("/bss-pricing/v1/prices/{id}")
        .operation_id("bss_pricing.delete_price")
        .summary("delete_price")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price or book id")
        .handler(delete_price)
        .no_content_response(StatusCode::NO_CONTENT, "Deleted")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/reference-ops")
        .operation_id("bss_pricing.list_reference_ops")
        .summary("List durable reference work")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .query_param("state", false, "Reference op state")
        .query_param("limit", false, "Batch size, 1 to 1000")
        .query_param("cursor", false, "Exclusive op-id cursor")
        .handler(list_reference_ops)
        .json_response_with_schema::<dto::PricingReferenceOpPage>(
            openapi,
            StatusCode::OK,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    approval_routes(row_routes(router, openapi), openapi)
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(correlation::establish))
}
/// Submission, publish changes, the approval queue, votes and the quorum policy.
#[allow(
    clippy::too_many_lines,
    reason = "one OperationBuilder chain per route keeps every door's contract in one place"
)]
fn approval_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/bss-pricing/v1/rows/{id}/submit")
        .operation_id("bss_pricing.submit_row")
        .summary("submit_row")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price row id")
        .param(header("Idempotency-Key"))
        .handler(submit_row)
        .json_response_with_schema::<dto::PricingSubmitReceipt>(
            openapi,
            StatusCode::CREATED,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/price-books/{id}/publish-changes")
        .operation_id("bss_pricing.list_publish_changes")
        .summary("list_publish_changes")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price book id")
        .handler(list_publish_changes)
        .json_response_with_schema::<dto::PricingPublishChanges>(
            openapi,
            StatusCode::OK,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-pricing/v1/price-books/{id}/publish-changes")
        .operation_id("bss_pricing.publish_changes")
        .summary("publish_changes")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price book id")
        .json_request::<dto::PricingPublishChangesRequest>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(publish_changes)
        .json_response_with_schema::<dto::PricingSubmitReceipt>(
            openapi,
            StatusCode::CREATED,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/approval-units")
        .operation_id("bss_pricing.list_approval_units")
        .summary("list_approval_units")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .query_param("state", false, "Unit state")
        .query_param("kind", false, "Approval kind")
        .query_param("ref_id", false, "Referenced aggregate id")
        .query_param("book_id", false, "Price book id")
        .handler(list_approval_units)
        .json_response_with_schema::<dto::PricingApprovalUnitList>(
            openapi,
            StatusCode::OK,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/approval-units/{id}")
        .operation_id("bss_pricing.get_approval_unit")
        .summary("get_approval_unit")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Approval unit id")
        .handler(get_approval_unit)
        .json_response_with_schema::<dto::PricingApprovalUnitDto>(
            openapi,
            StatusCode::OK,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-pricing/v1/approval-units/{id}/approve")
        .operation_id("bss_pricing.approve_unit")
        .summary("approve_unit")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Approval unit id")
        .json_request::<dto::PricingVoteRequest>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(approve_unit)
        .json_response_with_schema::<dto::PricingVoteReceipt>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-pricing/v1/approval-units/{id}/reject")
        .operation_id("bss_pricing.reject_unit")
        .summary("reject_unit")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Approval unit id")
        .json_request::<dto::PricingVoteRequest>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(reject_unit)
        .json_response_with_schema::<dto::PricingVoteReceipt>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-pricing/v1/approval-units/{id}/withdraw")
        .operation_id("bss_pricing.withdraw_unit")
        .summary("withdraw_unit")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Approval unit id")
        .param(header("Idempotency-Key"))
        .handler(withdraw_unit)
        .json_response_with_schema::<dto::PricingVoteReceipt>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/approval-policy")
        .operation_id("bss_pricing.get_approval_policy")
        .summary("get_approval_policy")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .handler(get_approval_policy)
        .json_response_with_schema::<dto::PricingApprovalPolicyDto>(
            openapi,
            StatusCode::OK,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    OperationBuilder::put("/bss-pricing/v1/approval-policy")
        .operation_id("bss_pricing.put_approval_policy")
        .summary("put_approval_policy")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .json_request::<dto::PricingApprovalPolicyPut>(openapi, "Request")
        .param(header("If-Match"))
        .handler(put_approval_policy)
        .json_response_with_schema::<dto::PricingApprovalPolicyDto>(
            openapi,
            StatusCode::OK,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi)
}
async fn submit_row(
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
        &resource_types::PRICE,
        actions::SUBMIT,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let digest = preconditions::request_digest(&support::empty_body(&body)?)?;
    let cmd = approvals::Command {
        scope,
        ctx,
        hub: state.hub.clone(),
        outbox: state.outbox.clone(),
        correlation,
        key,
        digest,
    };
    approvals::submit_row(&state.db.db(), cmd, id).await
}
async fn list_publish_changes(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRICE_BOOK,
        actions::READ,
        None,
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(
            async move { approvals::publish_list(tx, &scope, ctx.subject_tenant_id(), id).await },
        )
    })
    .await
}
async fn publish_changes(
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
        &resource_types::PRICE_BOOK,
        actions::SUBMIT,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let payload: serde_json::Value = preconditions::parse_body(&body)?;
    let digest = preconditions::request_digest(&payload)?;
    let input: dto::PricingPublishChangesRequest = preconditions::parse_body(&body)?;
    let cmd = approvals::Command {
        scope,
        ctx,
        hub: state.hub.clone(),
        outbox: state.outbox.clone(),
        correlation,
        key,
        digest,
    };
    approvals::publish(&state.db.db(), cmd, id, input).await
}
async fn list_approval_units(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    uri: axum::http::Uri,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::APPROVAL_UNIT,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    let axum::extract::Query(query) =
        axum::extract::Query::<dto::PricingApprovalUnitQuery>::try_from_uri(&uri)
            .map_err(|_| support::invalid("query", "QUERY_INVALID"))?;
    let state_filter = approvals::state_filter(query.state.as_deref())?;
    let reference = match (query.ref_id, query.book_id) {
        (Some(a), Some(b)) if a != b => return Err(support::invalid("book_id", "QUERY_INVALID")),
        (a, b) => a.or(b),
    };
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, kind) = (scope.clone(), ctx.clone(), query.kind.clone());
        Box::pin(async move {
            approvals::list_units(
                tx,
                &scope,
                ctx.subject_tenant_id(),
                state_filter,
                kind.as_deref(),
                reference,
            )
            .await
        })
    })
    .await
}
async fn get_approval_unit(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::APPROVAL_UNIT,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move { approvals::get_unit(tx, &scope, ctx.subject_tenant_id(), id).await })
    })
    .await
}
async fn approve_unit(
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
        &resource_types::APPROVAL_UNIT,
        actions::APPROVE,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let payload: serde_json::Value = preconditions::parse_body(&body)?;
    let digest = preconditions::request_digest(&payload)?;
    let input: dto::PricingVoteRequest = preconditions::parse_body(&body)?;
    let cmd = approvals::Command {
        scope,
        ctx,
        hub: state.hub.clone(),
        outbox: state.outbox.clone(),
        correlation,
        key,
        digest,
    };
    approvals::vote(
        &state.db.db(),
        cmd,
        id,
        approvals::Vote::Approve,
        Some(input),
    )
    .await
}
async fn reject_unit(
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
        &resource_types::APPROVAL_UNIT,
        actions::APPROVE,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let payload: serde_json::Value = preconditions::parse_body(&body)?;
    let digest = preconditions::request_digest(&payload)?;
    let input: dto::PricingVoteRequest = preconditions::parse_body(&body)?;
    let cmd = approvals::Command {
        scope,
        ctx,
        hub: state.hub.clone(),
        outbox: state.outbox.clone(),
        correlation,
        key,
        digest,
    };
    approvals::vote(
        &state.db.db(),
        cmd,
        id,
        approvals::Vote::Reject,
        Some(input),
    )
    .await
}
async fn withdraw_unit(
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
        &resource_types::APPROVAL_UNIT,
        actions::SUBMIT,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let digest = preconditions::request_digest(&support::empty_body(&body)?)?;
    let cmd = approvals::Command {
        scope,
        ctx,
        hub: state.hub.clone(),
        outbox: state.outbox.clone(),
        correlation,
        key,
        digest,
    };
    approvals::vote(&state.db.db(), cmd, id, approvals::Vote::Withdraw, None).await
}
async fn get_approval_policy(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::CONFIG,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move { approvals::get_policy(tx, &scope, ctx.subject_tenant_id()).await })
    })
    .await
}
async fn put_approval_policy(
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
        &resource_types::CONFIG,
        actions::SETTINGS,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let version = preconditions::if_match(&headers)?.get();
    let input: dto::PricingApprovalPolicyPut = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, input) = (scope.clone(), ctx.clone(), input.clone());
        Box::pin(async move {
            approvals::put_policy(tx, &scope, &ctx, correlation, version, input).await
        })
    })
    .await
}
/// Draft row authoring: create, patch and delete.
fn row_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/bss-pricing/v1/prices/{id}/rows")
        .operation_id("bss_pricing.create_row")
        .summary("create_row")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price id")
        .json_request::<dto::PricingPriceRowCreate>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(create_row)
        .json_response_with_schema::<dto::PricingPriceRowCreated>(
            openapi,
            StatusCode::CREATED,
            "Response",
        )
        .standard_errors(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch("/bss-pricing/v1/rows/{id}")
        .operation_id("bss_pricing.patch_row")
        .summary("patch_row")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price row id")
        .json_request::<dto::PricingPriceRowPatch>(openapi, "Request")
        .param(header("If-Match"))
        .handler(patch_row)
        .json_response_with_schema::<dto::PricingPriceRowDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .register(router, openapi);
    OperationBuilder::delete("/bss-pricing/v1/rows/{id}")
        .operation_id("bss_pricing.delete_row")
        .summary("delete_row")
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Price row id")
        .param(header("If-Match"))
        .handler(delete_row)
        .no_content_response(StatusCode::NO_CONTENT, "Deleted")
        .standard_errors(openapi)
        .register(router, openapi)
}
async fn create_row(
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
        &resource_types::PRICE,
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
    let input: dto::PricingPriceRowCreate = preconditions::parse_body(&body)?;
    rows::create(
        &state.db.db(),
        scope,
        ctx,
        correlation,
        id,
        key,
        digest,
        input,
    )
    .await
}
async fn patch_row(
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
        &resource_types::PRICE,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let version = preconditions::if_match(&headers)?.get();
    let input: dto::PricingPriceRowPatch = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, input) = (scope.clone(), ctx.clone(), input.clone());
        Box::pin(
            async move { rows::patch(tx, &scope, &ctx, correlation, id, version, input).await },
        )
    })
    .await
}
async fn delete_row(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    corr: Option<Extension<correlation::CorrelationId>>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRICE,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let version = preconditions::if_match(&headers)?.get();
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move { rows::delete(tx, &scope, &ctx, correlation, id, version).await })
    })
    .await
}
async fn create_book(
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
        &resource_types::PRICE_BOOK,
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
    let body: PriceBookCreate = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, body) = (scope.clone(), ctx.clone(), body.clone());
        let (key, digest) = (key.clone(), digest.clone());
        Box::pin(
            async move { books::create(tx, &scope, &ctx, correlation, &key, &digest, body).await },
        )
    })
    .await
}
async fn list_books(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRICE_BOOK,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let body = PriceBookList {
                items: book_repo::list(tx, &scope, tenant)
                    .await?
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            };
            Ok(response(StatusCode::OK, &body, None)?)
        })
    })
    .await
}
async fn get_book(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRICE_BOOK,
        actions::READ,
        None,
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let m = books::find(tx, &scope, tenant, id).await?;
            let version = preconditions::RowVersion::from_stored(m.version)
                .map_err(CanonicalError::from)?
                .get();
            Ok(response(
                StatusCode::OK,
                &PriceBookDto::from(m),
                Some(version),
            )?)
        })
    })
    .await
}
async fn patch_book(
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
        &resource_types::PRICE_BOOK,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let version = preconditions::if_match(&headers)?.get();
    let body: PriceBookPatch = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, body) = (scope.clone(), ctx.clone(), body.clone());
        Box::pin(
            async move { books::patch(tx, &scope, &ctx, correlation, id, version, body).await },
        )
    })
    .await
}
async fn list_prices(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRICE,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let body = PricingPriceList {
                items: books::prices(tx, &scope, tenant, id)
                    .await?
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            };
            Ok(response(StatusCode::OK, &body, None)?)
        })
    })
    .await
}
async fn export_book(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRICE_BOOK,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            Ok(response(
                StatusCode::OK,
                &books::export(tx, &scope, tenant, id).await?,
                None,
            )?)
        })
    })
    .await
}
async fn get_settings(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::CONFIG,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let body = configuration::settings(tx, &scope, tenant).await?;
            let version = preconditions::RowVersion::from_stored(body.version)
                .map_err(CanonicalError::from)?
                .get();
            Ok(response(StatusCode::OK, &body, Some(version))?)
        })
    })
    .await
}
async fn put_settings(
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
        &resource_types::CONFIG,
        actions::SETTINGS,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let version = preconditions::if_match(&headers)?.get();
    let body: PricingSettingsPut = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, body) = (scope.clone(), ctx.clone(), body.clone());
        Box::pin(async move {
            configuration::put_settings(tx, &scope, &ctx, correlation, version, body).await
        })
    })
    .await
}
async fn get_dimensions(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::CONFIG,
        actions::READ,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let tenant = ctx.subject_tenant_id();
            let (body, tag) = configuration::dimensions(tx, &scope, tenant).await?;
            Ok(response(StatusCode::OK, &body, Some(tag))?)
        })
    })
    .await
}
async fn put_dimensions(
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
        &resource_types::CONFIG,
        actions::SETTINGS,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        None,
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let version = preconditions::if_match(&headers)?.get();
    let body: PricingDimensions = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, body) = (scope.clone(), ctx.clone(), body.clone());
        Box::pin(async move {
            configuration::put_dimensions(tx, &scope, &ctx, correlation, version, body).await
        })
    })
    .await
}

async fn create_price(
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
        &resource_types::PRICE,
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
    let input = preconditions::parse_body(&body)?;
    prices::create(state, scope, ctx, id, correlation, key, digest, input).await
}

async fn get_price(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRICE,
        actions::READ,
        None,
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let m = prices::find(tx, &scope, ctx.subject_tenant_id(), id).await?;
            let version = preconditions::RowVersion::from_stored(m.version)
                .map_err(CanonicalError::from)?
                .get();
            Ok(response(
                StatusCode::OK,
                &dto::PricingPriceDto::from(m),
                Some(version),
            )?)
        })
    })
    .await
}

async fn patch_price(
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
        &resource_types::PRICE,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    let version = preconditions::if_match(&headers)?.get();
    let input: dto::PricingPricePatch = preconditions::parse_body(&body)?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx, input) = (scope.clone(), ctx.clone(), input.clone());
        Box::pin(
            async move { prices::patch(tx, &scope, &ctx, correlation, id, version, input).await },
        )
    })
    .await
}

async fn delete_price(
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
        &resource_types::PRICE,
        actions::AUTHOR,
        Some(OwnerTenant(ctx.subject_tenant_id())),
        Some(ResourceRef(id)),
    )
    .await
    .map_err(authz_failure)?;
    let correlation = correlation::require_correlation(corr)?;
    prices::delete(state, scope, ctx, correlation, id).await
}

async fn list_reference_ops(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    uri: axum::http::Uri,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = authz::access_scope(
        &enforcer,
        &ctx,
        &resource_types::CONFIG,
        actions::SETTINGS,
        None,
        None,
    )
    .await
    .map_err(authz_failure)?;
    let axum::extract::Query(query) =
        axum::extract::Query::<dto::PricingReferenceOpQuery>::try_from_uri(&uri)
            .map_err(|_| support::invalid("query", "QUERY_INVALID"))?;
    let filter = query
        .state
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|_| support::invalid("state", "REFERENCE_OP_STATE_INVALID"))?;
    let limit = query.limit.unwrap_or(100);
    if !(1..=1000).contains(&limit) {
        return Err(support::invalid("limit", "LIMIT_INVALID"));
    }
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let mut items = crate::infra::storage::repo::reference_op_repo::page(
                tx,
                &scope,
                ctx.subject_tenant_id(),
                filter,
                query.cursor,
                limit + 1,
            )
            .await?;
            let next_cursor = if u64::try_from(items.len()).unwrap_or(u64::MAX) > limit {
                items.pop();
                items.last().map(|op| op.op_id)
            } else {
                None
            };
            Ok(response(
                StatusCode::OK,
                &dto::PricingReferenceOpPage {
                    items: items.into_iter().map(Into::into).collect(),
                    next_cursor,
                },
                None,
            )?)
        })
    })
    .await
}
