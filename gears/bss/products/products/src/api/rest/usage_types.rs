//! The usage-type picker (P-D-207): `GET /bss-products/v1/usage-types?q&kind&limit&cursor`.
//!
//! Products serves the authoring pick-list itself, from the one `UsageTypeCatalog` its publish
//! gate resolves against (P-D-184), so the screen offers exactly what the gate accepts. The
//! 09-22 catalog design named the path `/bss-products/v1/catalog/usage-types` and gated it on
//! `recognized_set × read`; the route is `/usage-types` under the products SKU-author grant,
//! because picking a usage type is authoring a SKU and no `recognized_set` resource exists.
//!
//! The catalog is read **as the caller** (owner option b): a collector that refuses the caller answers
//! 403, an unconfigured catalog 501, an unreachable one 503, and an empty configured one 200 with
//! `items: []` — never one of these as another. Over the usage collector, `q` is products' own
//! search (`infra::usage_types`): the collector's plugin takes no `contains`.
use super::{ApiState, authz_error_to_canonical, require_authenticated};
use crate::{
    authz::{access_scope, actions, resource_types},
    domain::{error::DomainError, validation::ValidationReport},
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{
    Extension, Json, Router,
    extract::{Query, rejection::QueryRejection},
    http::StatusCode,
};
use bss_products_sdk::usage_types::{UsageTypeBinding, UsageTypePage};
use std::sync::Arc;
use toolkit::api::{
    OpenApiRegistry,
    canonical_prelude::{CanonicalError, resource_error},
    operation_builder::OperationBuilder,
};
use toolkit_security::SecurityContext;

const PICKER: &str = "/bss-products/v1/usage-types";
/// The page asked for when the caller names none.
const DEFAULT_LIMIT: u32 = 50;
/// The largest page asked for; a larger `limit` is clamped, never refused.
const MAX_LIMIT: u32 = 200;

#[resource_error(gts_id!("cf.bss.products.sku.v1~"))]
struct SkuResource;

#[toolkit_macros::api_dto(request)]
struct PickerQuery {
    q: Option<String>,
    kind: Option<String>,
    limit: Option<u32>,
    cursor: Option<String>,
}

/// One usage type as the picker offers it. Prefixed: the usage collector's own `UsageTypeDto` is
/// in the same served document.
#[toolkit_macros::api_dto(response)]
pub struct ProductsUsageTypeDto {
    pub gts_id: String,
    pub kind: String,
    pub metadata_fields: Vec<String>,
}
impl From<UsageTypeBinding> for ProductsUsageTypeDto {
    fn from(b: UsageTypeBinding) -> Self {
        Self {
            gts_id: b.gts_id,
            kind: b.kind,
            metadata_fields: b.metadata_fields,
        }
    }
}
/// The page's cursors and the size the catalog actually applied.
#[toolkit_macros::api_dto(response)]
pub struct ProductsUsageTypePageInfo {
    pub next_cursor: Option<String>,
    pub prev_cursor: Option<String>,
    pub limit: u32,
}
/// A page of the picker, with the catalog's provenance (`registry`, `usage_collector`,
/// `local_dev_static`).
#[toolkit_macros::api_dto(response)]
pub struct ProductsUsageTypeList {
    pub source: String,
    pub items: Vec<ProductsUsageTypeDto>,
    pub page_info: ProductsUsageTypePageInfo,
}

pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::get(PICKER)
        .operation_id("bss_products.list_usage_types")
        .summary("List usage types for SKU authoring")
        .description(
            "The usage-type catalog the publish gate resolves against, read as the caller \
             (P-D-207). `q` narrows by case-insensitive substring of the id, `kind` by equality; \
             `limit` defaults to 50 and is clamped at 200. Over the usage collector, `q` searches \
             at most 1000 types of the asked kind, in id order, and its cursor is bound to `q` \
             and `kind`. Refusals: 403 without products SKU author, or when the catalog refuses \
             the caller; 400 for a malformed query, or a cursor of another query; 501 when no \
             catalog is configured; 503 when the configured one does not answer, or \
             `USAGE_TYPE_CATALOG_TOO_LARGE` when `q` would search more than 1000 types. A \
             configured catalog with no types answers 200 with no items.",
        )
        .tag("SKUs")
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "q",
            false,
            "Case-insensitive substring of the usage type's GTS id",
            "string",
        )
        .query_param_typed("kind", false, "counter or gauge", "string")
        .query_param_typed(
            "limit",
            false,
            "Page size (default 50, at most 200)",
            "integer",
        )
        .query_param_typed(
            "cursor",
            false,
            "Continuation token from page_info.next_cursor",
            "string",
        )
        .handler(list_usage_types)
        .json_response_with_schema::<ProductsUsageTypeList>(openapi, StatusCode::OK, "Usage types")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .problem_response(openapi, StatusCode::NOT_IMPLEMENTED, "Not Implemented")
        .error_503(openapi)
        .register(Router::new(), openapi)
        .layer(Extension(state))
}

/// A malformed query is the gear's 400 envelope.
fn refuse(field: &str, detail: impl Into<String>) -> CanonicalError {
    let mut r = ValidationReport::new();
    r.violate("VALIDATION", field, detail);
    DomainError::Validation(r).into()
}

async fn list_usage_types(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    query: Result<Query<PickerQuery>, QueryRejection>,
) -> Result<Json<ProductsUsageTypeList>, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    // The products SKU-author grant: picking a usage type is authoring a SKU.
    access_scope(
        &enforcer,
        &ctx,
        &resource_types::SKU,
        actions::AUTHOR,
        Some(ctx.subject_tenant_id()),
        None,
        true,
    )
    .await
    .map_err(|e| {
        authz_error_to_canonical(e, |reason| {
            SkuResource::permission_denied()
                .with_reason(reason)
                .create()
        })
    })?;
    let Query(q) = query.map_err(|e| refuse("query", e.body_text()))?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 {
        return Err(refuse("limit", "limit must be at least one"));
    }
    let page: UsageTypePage = state
        .usage_type_catalog
        .list(
            &ctx,
            q.q.as_deref(),
            q.kind.as_deref(),
            limit.min(MAX_LIMIT),
            q.cursor.as_deref(),
        )
        .await?;
    Ok(Json(ProductsUsageTypeList {
        source: state.usage_type_catalog_source.to_owned(),
        items: page.items.into_iter().map(Into::into).collect(),
        page_info: ProductsUsageTypePageInfo {
            next_cursor: page.next_cursor,
            prev_cursor: page.prev_cursor,
            limit: page.limit,
        },
    }))
}

#[cfg(test)]
#[path = "usage_types_tests.rs"]
mod usage_types_tests;
