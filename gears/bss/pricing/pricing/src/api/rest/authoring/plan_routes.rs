//! The plan, revision and item doors (phase 3): one `plan` label, read and author.
use super::{
    AuthoringState,
    caps::Capped,
    dto, plan_items, plans,
    support::{authz_failure, etag, header, require_authenticated, response, transaction},
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
        .summary("Create a plan")
        .description(
            "Creates a plan with a code and a name and its draft revision 1 on a book of the \
             tenant, with an optional sale date, available_from (YYYY-MM-DD; omitted or null is \
             \"at publish\", D-463); the Idempotency-Key replays the answer. The caller also needs \
             price_book read on that book (D-456). The name is at most 200 characters and the code \
             64 (D-457), and the code follows a rule: 1 to 32 characters of A-Z, 0-9, - and _, \
             starting with a letter or a digit, judged as sent with no trim or case folding \
             (D-468); a code stored before the rule keeps reading. Refusals: 400 FIELD_TOO_LONG on a code or a name over its cap, \
             PLAN_CODE_REQUIRED for a blank code, PLAN_CODE_INVALID for a code off the rule, or \
             DATE_INVALID; 404 for a book the tenant does not hold; 403 \
             PRICE_BOOK_READ_REQUIRED for one the caller may not read; 503 when that grant cannot \
             be judged; 409 PLAN_CODE_TAKEN.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .json_request::<dto::PricingPlanCreate>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(create_plan)
        .json_response_with_schema::<dto::PricingPlanDto>(openapi, StatusCode::CREATED, "Response")
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/plans")
        .operation_id("bss_pricing.list_plans")
        .summary("List the plans")
        .description(
            "Lists the tenant's plans by code, each with the headers of its revisions as they \
             read today (a scheduled revision whose date has come reads published, D-447), each \
             header with its author and when it was submitted and approved (D-461). Each plan names \
             its current revision (the draft or pending one, else the scheduled one, else the \
             published one in effect) with its item count, item SKUs and author, and the \
             published revision in effect (D-460). With sku_id, only the plans that have a draft, \
             pending, scheduled or published revision whose items name the SKU through a price \
             book entry (D-434; an included item without an entry does not count), in the same \
             shape: so a plan's current sku_ids, which name every item, may differ from what the \
             filter keeps. Four statements whatever the number of plans. Refusals: 400 \
             QUERY_INVALID for a malformed sku_id or any other key.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .query_param(
            "sku_id",
            false,
            "Only the plans selling this SKU through an entry",
        )
        .handler(list_plans)
        .json_response_with_schema::<dto::PricingPlanList>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/plans/{id}")
        .operation_id("bss_pricing.get_plan")
        .summary("Read a plan")
        .description(
            "Returns one plan with the headers of its revisions as they read today (D-447) and \
             when each was submitted and approved (D-461), its current revision and the one in \
             effect (D-460), and its version as the ETag a following PATCH sends back as If-Match. \
             Refusals: 404 for a plan the tenant does not hold.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan id")
        .handler(get_plan)
        .json_response_with_schema::<dto::PricingPlanDto>(openapi, StatusCode::OK, "Response")
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch("/bss-pricing/v1/plans/{id}")
        .operation_id("bss_pricing.patch_plan")
        .summary("Rename a plan")
        .description(
            "Renames a plan at the version the caller read (If-Match). The name is at most 200 \
             characters (D-457). Refusals: 400 FIELD_TOO_LONG on a name over its cap; 404 for a \
             plan the tenant does not hold; 409 STALE_REVISION.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan id")
        .json_request::<dto::PricingPlanPatch>(openapi, "Request")
        .param(header("If-Match"))
        .handler(patch_plan)
        .json_response_with_schema::<dto::PricingPlanDto>(openapi, StatusCode::OK, "Response")
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-pricing/v1/plans/{id}/revisions")
        .operation_id("bss_pricing.copy_plan_revision")
        .summary("Copy the published revision")
        .description(
            "Copies the plan's published revision (book, sale date and items) into a new draft \
             revision and attaches each copied item's SKU reference; a scheduled revision whose \
             date has come is switched first, so the copy is of the revision in effect (D-451). \
             Refusals: 404 for an unknown plan; 409 REVISION_DRAFT_EXISTS while a draft or \
             pending revision exists, REVISION_SCHEDULED while a revision waits for its sale date, \
             PLAN_UNPUBLISHED without a published one.",
        )
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
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-pricing/v1/plans/{id}/clone")
        .operation_id("bss_pricing.clone_plan")
        .summary("Clone a plan")
        .description(
            "Creates a new plan with its own code and name whose draft revision 1 copies the \
             source's published revision (the one in effect: a scheduled revision whose date has \
             come is switched first, D-451): its book, its items and its sale date, without \
             anything of its approval. An available_from in the body overrides the sale date, \
             and null clears it (D-463). The caller also needs price_book read on the source's \
             book (D-456). The code is at most 64 characters and the name 200 (D-457), and the \
             new plan's code follows the rule of POST /plans: 1 to 32 characters of A-Z, 0-9, - \
             and _, starting with a letter or a digit, judged as sent (D-468); the source's own \
             code, stored before the rule, is never judged. Refusals: 400 FIELD_TOO_LONG on a code \
             or a name over its cap, PLAN_CODE_REQUIRED for a blank code, PLAN_CODE_INVALID for a \
             code off the rule, or DATE_INVALID; 404 for an unknown plan; 409 CLONE_SOURCE_UNPUBLISHED or PLAN_CODE_TAKEN; 403 \
             PRICE_BOOK_READ_REQUIRED for a book the caller may not read; 503 when that grant \
             cannot be judged.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Source plan id")
        .json_request::<dto::PricingPlanClone>(openapi, "Request")
        .param(header("Idempotency-Key"))
        .handler(clone_plan)
        .json_response_with_schema::<dto::PricingPlanDto>(openapi, StatusCode::CREATED, "Response")
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/plan-revisions/{id}")
        .operation_id("bss_pricing.get_plan_revision")
        .summary("Read a plan revision")
        .description(
            "Returns one plan revision with its items and its state as it reads today (D-447), \
             when it was submitted and approved (D-461) and, while it is pending, its vote \
             progress: the approve votes counted toward the quorum and the quorum, counts only, \
             under plan read (D-462). Its version is the ETag a following PATCH sends back as \
             If-Match. Refusals: 404 for a revision the tenant does not hold.",
        )
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
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch("/bss-pricing/v1/plan-revisions/{id}")
        .operation_id("bss_pricing.patch_plan_revision")
        .summary("Change a draft revision")
        .description(
            "Changes a draft revision's book or its sale date, by its author at the version the \
             author read (If-Match). A new book remaps each item to the new book's entry of the \
             same SKU, charge kind, period and model (D-427), and an item with no such entry keeps \
             its own, so its checks show ITEM_BOOK_FOREIGN; book_id omitted or null leaves the \
             book unchanged. A named book needs the caller's price_book read on it (D-456). \
             Refusals: 400 \
             DATE_INVALID; 403 NOT_DRAFT_AUTHOR; 404; 409 REVISION_NOT_DRAFT or STALE_REVISION; \
             403 PRICE_BOOK_READ_REQUIRED for a book the caller may not read; 503 when that grant \
             cannot be judged.",
        )
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
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    OperationBuilder::delete("/bss-pricing/v1/plan-revisions/{id}")
        .operation_id("bss_pricing.delete_plan_revision")
        .summary("Delete a draft revision")
        .description(
            "Deletes a draft revision of the caller with every item, releasing their SKU \
             references; the last revision of a never-published plan takes the plan with it. \
             Refusals: 403 NOT_DRAFT_AUTHOR; 404; 409 REVISION_NOT_DRAFT, \
             ITEM_CONFIRMATION_PENDING, or STALE_REVISION when a concurrent write changed the \
             revision or one of its items first.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan revision id")
        .handler(delete_revision)
        .no_content_response(StatusCode::NO_CONTENT, "Deleted")
        .standard_errors(openapi)
        .error_503(openapi)
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
    // The plan names a book: its author's `price_book` read, judged a second time (D-456).
    let books = super::money_scope(&enforcer, &ctx).await?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let payload: serde_json::Value = preconditions::parse_body(&body)?;
    let digest = preconditions::request_digest(&payload)?;
    let input: dto::PricingPlanCreate = preconditions::parse_body(&body)?;
    input.caps()?;
    transaction(&state.db.db(), move |tx| {
        let (scope, books, ctx, input) = (scope.clone(), books.clone(), ctx.clone(), input.clone());
        let (key, digest) = (key.clone(), digest.clone());
        Box::pin(async move {
            plans::create(
                tx,
                (&scope, books.as_ref()),
                &ctx,
                correlation,
                (&key, &digest),
                input,
            )
            .await
        })
    })
    .await
}
async fn list_plans(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    uri: axum::http::Uri,
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
    let axum::extract::Query(query) =
        axum::extract::Query::<dto::PricingPlanQuery>::try_from_uri(&uri)
            .map_err(|_| super::support::invalid("query", "QUERY_INVALID"))?;
    transaction(&state.db.db(), move |tx| {
        let (scope, ctx) = (scope.clone(), ctx.clone());
        Box::pin(async move {
            let body = plans::list(tx, &scope, ctx.subject_tenant_id(), query.sku_id).await?;
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
    input.caps()?;
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
    // The clone names its source's book: its author's `price_book` read, judged a second time
    // (D-456).
    let books = super::money_scope(&enforcer, &ctx).await?;
    let correlation = correlation::require_correlation(corr)?;
    let key = preconditions::idempotency_key(&headers)?;
    let payload: serde_json::Value = preconditions::parse_body(&body)?;
    let digest = preconditions::request_digest(&payload)?;
    let input: dto::PricingPlanClone = preconditions::parse_body(&body)?;
    input.caps()?;
    plans::clone(
        state,
        (scope, books),
        ctx,
        correlation,
        id,
        key,
        digest,
        input,
    )
    .await
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
        Box::pin(async move {
            plans::get_revision(tx, &scope, (ctx.subject_tenant_id(), ctx.subject_id()), id).await
        })
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
    // A patch that names a book: its author's `price_book` read, judged a second time (D-456).
    let books = match input.book_id {
        Some(_) => super::money_scope(&enforcer, &ctx).await?,
        None => None,
    };
    transaction(&state.db.db(), move |tx| {
        let (scope, books, ctx, input) = (scope.clone(), books.clone(), ctx.clone(), input.clone());
        Box::pin(async move {
            plans::patch_revision(
                tx,
                (&scope, books.as_ref()),
                &ctx,
                correlation,
                id,
                version,
                input,
            )
            .await
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
        .summary("Add an item to a draft revision")
        .description(
            "Adds an item to a draft revision: a SKU and its entry in the plan's book, the entry \
             required (D-467), reserving the SKU reference in Products; the Idempotency-Key \
             replays the receipt. A deprecated SKU is added only when the plan's published \
             revision in effect carries it (D-465). Refusals: 400 BODY_UNEXPECTED for treatment, \
             included_qty or qty_min, ITEM_ENTRY_MISSING for a missing or null entry, \
             ITEM_BOOK_FOREIGN for an entry of another book, ITEM_ENTRY_SKU_MISMATCH for an entry \
             of another SKU, ITEM_SKU_DEPRECATED, ITEM_BUNDLE_SKU or REVISION_ITEMS_TOO_MANY; 403 \
             NOT_DRAFT_AUTHOR for a draft of another author; 404 for an unknown revision, or an \
             unknown entry (ENTRY_NOT_FOUND); 409 REVISION_NOT_DRAFT, ITEM_SKU_TAKEN, \
             IDEMPOTENCY_CONFLICT or IDEMPOTENCY_KEY_IN_FLIGHT, and SKU_FENCED, SKU_RETIRING or \
             SKU_DRAFT from the item's create op (Products' reserve refusal or its SKU re-read); \
             Products' own refusal of the SKU read, as Products gave it; 503 \
             REGISTRY_UNAVAILABLE.",
        )
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
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-pricing/v1/plan-items/{id}")
        .operation_id("bss_pricing.get_plan_item")
        .summary("Read a plan item")
        .description(
            "Returns one plan item with its revision's number and state and its plan (D-434), its \
             version as the ETag a following PATCH sends back as If-Match. Refusals: 404 for an \
             item the tenant does not hold.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan item id")
        .handler(get_item)
        .json_response_with_schema::<dto::PricingPlanItemReadDto>(
            openapi,
            StatusCode::OK,
            "Response",
        )
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::patch("/bss-pricing/v1/plan-items/{id}")
        .operation_id("bss_pricing.patch_plan_item")
        .summary("Change a plan item")
        .description(
            "Changes a draft item's entry, never its SKU, at the version the caller read \
             (If-Match): a plan item is a SKU and its entry (D-467). Refusals: 400 \
             BODY_UNEXPECTED for treatment, included_qty or qty_min, ITEM_ENTRY_MISSING for a \
             null entry or an item left without one; 403 NOT_DRAFT_AUTHOR; 409 \
             REVISION_NOT_DRAFT or STALE_REVISION.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan item id")
        .json_request::<dto::PricingPlanItemPatch>(openapi, "Request")
        .param(header("If-Match"))
        .handler(patch_item)
        .json_response_with_schema::<dto::PricingPlanItemDto>(openapi, StatusCode::OK, "Response")
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::delete("/bss-pricing/v1/plan-items/{id}")
        .operation_id("bss_pricing.delete_plan_item")
        .summary("Remove a plan item")
        .description(
            "Removes an item from a draft revision and releases its SKU reference. Refusals: 403 \
             NOT_DRAFT_AUTHOR; 404; 409 REVISION_NOT_DRAFT or ITEM_CONFIRMATION_PENDING.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan item id")
        .handler(delete_item)
        .no_content_response(StatusCode::NO_CONTENT, "Deleted")
        .standard_errors(openapi)
        .error_503(openapi)
        .register(router, openapi);
    OperationBuilder::get("/bss-pricing/v1/plan-revisions/{id}/checks")
        .operation_id("bss_pricing.get_plan_revision_checks")
        .summary("Check a plan revision")
        .description(
            "Returns every check of the revision on its sale date (coverage, SKUs, references, \
             book) and whether it may be submitted, from fresh SKU reads. Refusals: 404 for a \
             revision the tenant does not hold; Products' own refusal; 503 REGISTRY_UNAVAILABLE.",
        )
        .tag("Pricing")
        .authenticated()
        .no_license_required()
        .path_param("id", "Plan revision id")
        .handler(get_checks)
        .json_response_with_schema::<dto::PricingPlanChecksDto>(openapi, StatusCode::OK, "Response")
        .standard_errors(openapi)
        .error_503(openapi)
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
    plan_items::judge_body(&payload, true)?;
    let input: dto::PricingPlanItemCreate = preconditions::parse_body(&body)?;
    plan_items::add(state, scope, ctx, id, correlation, key, digest, input).await
}
async fn get_item(
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
        Box::pin(async move { plan_items::get(tx, &scope, ctx.subject_tenant_id(), id).await })
    })
    .await
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
    let payload: serde_json::Value = preconditions::parse_body(&body)?;
    plan_items::judge_body(&payload, false)?;
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
