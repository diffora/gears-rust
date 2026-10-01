//! Route registration. The inbox authorizes nothing itself: the gateway authenticates, and each
//! source door authorizes.

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use toolkit::api::OpenApiRegistry;
use toolkit::api::operation_builder::{OperationBuilder, ParamSpec};

use super::handlers;
use crate::api::ApiState;
use crate::api::rest::dto::{InboxCountsDto, InboxUnitDto, InboxUnitListDto};

const TAG: &str = "Approval units";

pub fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = list_route(router, openapi);
    let router = counts_route(router, openapi);
    let router = card_route(router, openapi);
    let router = vote_route(
        router,
        openapi,
        "/bss-approvals/v1/approval-units/{id}/approve",
        "bss_approvals.approve_unit",
        "approve_unit",
        handlers::approve,
    );
    let router = vote_route(
        router,
        openapi,
        "/bss-approvals/v1/approval-units/{id}/reject",
        "bss_approvals.reject_unit",
        "reject_unit",
        handlers::reject,
    );
    vote_route(
        router,
        openapi,
        "/bss-approvals/v1/approval-units/{id}/withdraw",
        "bss_approvals.withdraw_unit",
        "withdraw_unit",
        handlers::withdraw,
    )
    .layer(axum::Extension(state))
}

fn list_route(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::get("/bss-approvals/v1/approval-units")
        .operation_id("bss_approvals.list_approval_units")
        .summary("list_approval_units")
        .description(
            "One page of approval units from every configured BSS gear, merged by submitted_at \
             and then the unit id, in the same direction. The default order is newest first. \
             `limit` defaults to 50 and is clamped at 200. A cursor carries its order and its \
             narrowing, so a continuation sends no `$orderby`. `impact=false` skips the live \
             impact. `sources` names each configured gear as ok or forbidden; a forbidden gear's \
             units are omitted. `total` on the counts counts the readable gears only. Refusals: \
             400 ORDER_WITH_CURSOR when `$orderby` is sent with a cursor; 400 FILTER_MISMATCH \
             when the narrowing changed; 400 for a cursor or an order that does not read; 403 \
             when every gear forbids the caller; 503 SOURCE_UNAVAILABLE naming a gear that did \
             not answer.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "state",
            false,
            "pending, approved, rejected or withdrawn",
            "string",
        )
        .query_param_typed(
            "kind",
            false,
            "prices, plan_revision, sku_publish, sku_change or sku_retire",
            "string",
        )
        .query_param_typed("ref_id", false, "Referenced aggregate id", "string")
        .query_param_typed(
            "book_id",
            false,
            "Price book id; products treats it as empty",
            "string",
        )
        .query_param_typed(
            "limit",
            false,
            "Page size (default 50, at most 200)",
            "integer",
        )
        .query_param_typed("cursor", false, "Continuation from next_cursor", "string")
        .query_param_typed(
            "$orderby",
            false,
            "submitted_at asc or submitted_at desc (the default when omitted)",
            "string",
        )
        .query_param_typed("impact", false, "false skips the live impact", "boolean")
        .handler(handlers::list_units)
        .json_response_with_schema::<InboxUnitListDto>(openapi, StatusCode::OK, "One merged page")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

fn counts_route(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::get("/bss-approvals/v1/approval-units/counts")
        .operation_id("bss_approvals.count_approval_units")
        .summary("count_approval_units")
        .description(
            "The list's narrowing, summed over the gears the caller can read: by_state, by_kind \
             and total. Every state and every kind is named, 0 when none. total counts the \
             readable gears only. `sources` names each gear as ok or forbidden. A forbidden gear \
             is omitted from the sum. Refusals: 400 for a query that does not parse or a key the \
             narrowing does not take; 403 when every gear forbids the caller; 503 \
             SOURCE_UNAVAILABLE naming a gear that did not answer.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "state",
            false,
            "pending, approved, rejected or withdrawn",
            "string",
        )
        .query_param_typed(
            "kind",
            false,
            "prices, plan_revision, sku_publish, sku_change or sku_retire",
            "string",
        )
        .query_param_typed("ref_id", false, "Referenced aggregate id", "string")
        .query_param_typed(
            "book_id",
            false,
            "Price book id; products treats it as empty",
            "string",
        )
        .handler(handlers::count_units)
        .json_response_with_schema::<InboxCountsDto>(openapi, StatusCode::OK, "Summed counts")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

fn card_route(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::get("/bss-approvals/v1/approval-units/{id}")
        .operation_id("bss_approvals.get_approval_unit")
        .summary("get_approval_unit")
        .description(
            "The unit, from the one configured gear that holds it. Every gear is asked. Two gears \
             holding it is 500 naming both. A gear that does not answer is 503 \
             SOURCE_UNAVAILABLE unless another gear returned the unit. When no gear holds it, a \
             forbidden gear is 403 and the body names no gear; otherwise 404. `impact=false` \
             skips the live impact.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "Unit id")
        .query_param_typed("impact", false, "false skips the live impact", "boolean")
        .handler(handlers::get_unit)
        .json_response_with_schema::<InboxUnitDto>(openapi, StatusCode::OK, "The unit")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

fn vote_route<H, T>(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    path: &'static str,
    operation_id: &'static str,
    summary: &'static str,
    handler: H,
) -> Router
where
    H: axum::handler::Handler<T, ()> + Clone + Send + 'static,
    T: 'static,
{
    OperationBuilder::post(path)
        .operation_id(operation_id)
        .summary(summary)
        .description(
            "Forwards the body and the Idempotency-Key to the gear that holds the unit, and \
             returns that gear's answer unchanged: status, headers and body. The owner is resolved \
             as the card resolves it. Declared door codes: 400 GENERATION_REQUIRED, \
             GENERATION_MISMATCH, UNIT_STALE, NOTE_REQUIRED, NOTE_TOO_LONG, BODY_UNEXPECTED; 403 \
             for the grant and SOD_VIOLATION; 404; 409 DUPLICATE_VOTE, UNIT_ALREADY_DECIDED, \
             IDEMPOTENCY_CONFLICT; 503. These doors do not answer 412.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "Unit id")
        .param(
            ParamSpec::header("Idempotency-Key")
                .description("Passed through to the owning gear's vote door"),
        )
        .handler(handler)
        .json_response(
            StatusCode::OK,
            "The owning gear's vote answer, forwarded unchanged",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}
