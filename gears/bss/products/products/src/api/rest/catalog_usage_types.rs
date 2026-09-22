//! The usage-type pick-list — `GET /bss-products/v1/catalog/usage-types`
//! (the plugin-seam decision of 2026-09-22).
//!
//! # Why this door exists
//!
//! A meter declaration is the pair `(unit, usageTypeRef)`. The unit half has a
//! surface: `GET /config/vocabularies/metering_unit` lists a governed local
//! vocabulary. The ref half had none, so an authoring screen either reached
//! into another gear or an operator typed a GTS id by hand and learned about a
//! typo only at publish. This is the ref half's surface, and it is a
//! **pass-through** of whatever catalog `gear.rs` resolved.
//!
//! # `catalog/`, and the grant it spends
//!
//! The prefix is `catalog/` because the content is supplied from **outside**
//! this gear, which is the same reason pricing serves `catalog/tax-categories`
//! rather than putting it under its own config tree.
//!
//! The grant is **`recognized_set × read`** — the pair the meter declaration's
//! *other* half is already judged against. No new authz resource is minted
//! here: a catalog pair is a joint decision this slice may not take alone.
//!
//! # `source` is read, never derived
//!
//! It comes off `ApiState`, where `gear.rs` put it once. Deriving it here from
//! a config mode would report `unconfigured` for an answer a registered
//! supplier gave.
//!
//! # Failure is never an empty success
//!
//! 501 with no catalog configured, 503 when a configured one did not answer,
//! and `200` with `items: []` for a catalog that is configured and empty. A
//! caller that cannot tell the three apart renders silence as a clean verdict,
//! which is the whole failure this surface was built against.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Query};
use axum::http::StatusCode;
use serde::Deserialize;
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_security::SecurityContext;

use crate::api::rest::{ApiState, require_authenticated};

const TAG: &str = "BSS Products";

/// This door's path, named once.
pub const CATALOG_USAGE_TYPES: &str = "/bss-products/v1/catalog/usage-types";

/// The page size a caller gets when it names none, and the ceiling it may not
/// pass — the gear's own `design/01` D-125 walk shape.
const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 1_000;

#[resource_error(toolkit_gts::gts_id!("cf.bss.products.recognized_set.v1~"))]
struct UsageTypeCatalogDoor;

/// `?q=&kind=&limit=&cursor=`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageTypeQuery {
    /// Narrows by substring of the id. The distinguishing part of a usage
    /// type's GTS path is at its **end**, so this is `contains` and not a
    /// prefix match.
    pub q: Option<String>,
    /// `counter` or `gauge`, by equality.
    pub kind: Option<String>,
    /// Page size; absent is [`DEFAULT_LIMIT`], `0` is refused, above
    /// [`MAX_LIMIT`] is refused.
    pub limit: Option<u32>,
    /// The continuation token a previous page handed back.
    pub cursor: Option<String>,
}

/// One usage type as the pick-list renders it.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct UsageTypeView {
    /// The id a meter declaration names.
    pub gts_id: String,
    /// `counter` or `gauge`.
    pub kind: String,
    /// The metadata keys this type declares, in the catalog's order.
    pub metadata_fields: Vec<String>,
}

/// The page, with its provenance.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct UsageTypeListView {
    /// Where the answer came from: `registry`, `usage_collector`,
    /// `local_dev_static` or `unconfigured`.
    ///
    /// **Read this before `items`.** "This deployment has no usage types" and
    /// "nobody could be asked" are opposite facts, and only this field tells
    /// them apart on a `200`.
    pub source: String,
    /// The page's types.
    pub items: Vec<UsageTypeView>,
    /// The walk's cursors.
    pub page_info: UsageTypePageInfoView,
}

/// The cursors, on `design/01`'s D-125 shape.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct UsageTypePageInfoView {
    /// Absent on the last page, not on the page after it.
    pub next_cursor: Option<String>,
    /// Absent on the first.
    pub prev_cursor: Option<String>,
    /// The page size actually applied.
    pub limit: u32,
}

async fn list_usage_types(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Query(query): Query<UsageTypeQuery>,
) -> Result<Json<UsageTypeListView>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    read_gate(&enforcer, &ctx).await?;
    let limit = resolve_limit(query.limit)?;
    let page = state
        .usage_type_catalog
        .list(
            &ctx,
            query.q.as_deref(),
            query.kind.as_deref(),
            limit,
            query.cursor.as_deref(),
        )
        .await?;
    Ok(Json(UsageTypeListView {
        source: state.usage_type_catalog_source.to_owned(),
        items: page
            .items
            .into_iter()
            .map(|binding| UsageTypeView {
                gts_id: binding.gts_id,
                kind: binding.kind,
                metadata_fields: binding.metadata_fields,
            })
            .collect(),
        page_info: UsageTypePageInfoView {
            next_cursor: page.next_cursor,
            prev_cursor: page.prev_cursor,
            limit,
        },
    }))
}

/// `recognized_set × read`, the metering-unit vocabulary's own grant.
async fn read_gate(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<(), CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::RECOGNIZED_SET,
        crate::authz::actions::READ,
        Some(ctx.subject_tenant_id()),
        None,
        true,
    )
    .await
    .map(|_| ())
    .map_err(|e| {
        crate::api::rest::authz_error_to_canonical(e, |reason| {
            UsageTypeCatalogDoor::permission_denied()
                .with_reason(reason)
                .create()
        })
    })
}

/// A caller's `limit`, or the default; `0` and anything past the ceiling are
/// refused rather than clamped, so a screen is never silently given a page it
/// did not ask for.
fn resolve_limit(asked: Option<u32>) -> Result<u32, CanonicalError> {
    match asked {
        None => Ok(DEFAULT_LIMIT),
        Some(0) => Err(UsageTypeCatalogDoor::invalid_argument()
            .with_field_violation("limit", "limit must be at least 1", "out_of_range")
            .create()),
        Some(n) if n > MAX_LIMIT => Err(UsageTypeCatalogDoor::invalid_argument()
            .with_field_violation(
                "limit",
                format!("limit must not exceed {MAX_LIMIT}"),
                "out_of_range",
            )
            .create()),
        Some(n) => Ok(n),
    }
}

/// The one read this module registers.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::get(CATALOG_USAGE_TYPES)
        .operation_id("bss_products.list_usage_types")
        .summary("The usage types a meter declaration may name")
        .description(
            "A pass-through of whatever usage-type catalog this deployment resolved, for the \
             pick-list that authors a SKU's `usageTypeRef`. Gates on `recognized_set x read`, \
             the grant the meter declaration's unit half already spends. **Read `source` \
             before `items`**: a 200 with an empty `items` means the catalog is configured \
             and holds nothing, while no catalog at all answers 501 and a configured one \
             that did not reply answers 503 - three different facts a caller must act on \
             differently. `q` narrows by substring of the id and `kind` by equality.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(list_usage_types)
        .json_response_with_schema::<UsageTypeListView>(
            openapi,
            StatusCode::OK,
            "The catalog's page, with the provenance of the answer.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi)
        .layer(Extension(state))
}
