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

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Query};
use axum::http::StatusCode;
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_security::SecurityContext;

use crate::api::rest::{ApiState, require_authenticated};

const TAG: &str = "BSS Products";

/// This door's path, named once.
pub const CATALOG_USAGE_TYPES: &str = "/bss-products/v1/catalog/usage-types";

/// The four keys this door serves. Anything else is refused by name rather
/// than dropped, because a dropped filter reads as a correct unfiltered answer.
const DECLARED: [&str; 4] = ["q", "kind", "limit", "cursor"];

#[resource_error(toolkit_gts::gts_id!("cf.bss.products.recognized_set.v1~"))]
struct UsageTypeCatalogDoor;

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
    /// The page size the **catalog applied**, which need not be the one the
    /// caller asked for: a catalog may hold a ceiling of its own, and a screen
    /// that sized its pager off the request would size it off a number nobody
    /// honoured.
    pub limit: u32,
}

async fn list_usage_types(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<UsageTypeListView>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    read_gate(&enforcer, &ctx).await?;
    // **The gear's own guard, not serde's.** A typed `deny_unknown_fields`
    // fails inside the extractor, so the handler never runs and the caller
    // gets axum's plain-text rejection instead of the problem body this
    // operation declares - and it names the first offender where this one
    // names every offender at once (P-D-37).
    crate::api::rest::odata::reject_undeclared_query_params(
        &raw,
        crate::api::rest::odata::QueryFamily::OperandsOnly,
        &DECLARED,
    )?;
    let limit = resolve_limit(raw.get("limit").map(String::as_str))?;
    let page = state
        .usage_type_catalog
        .list(
            &ctx,
            raw.get("q").map(String::as_str),
            raw.get("kind").map(String::as_str),
            limit,
            raw.get("cursor").map(String::as_str),
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
            limit: page.limit,
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

/// A caller's `limit`, or the gear's default; `0` and anything past the
/// gear's ceiling are refused rather than clamped, so a screen is never
/// silently handed a page it did not ask for.
///
/// **The numbers are `LISTING_LIMIT_CFG`'s**, not this door's own. That
/// constant is declared as *"the page every list door in this gear serves"*,
/// with the rationale that two doors answering different numbers to the same
/// `$top` is a difference a caller has to learn for no return - and the first
/// cut of this door minted 100/1000 against the gear's 50/200 while its
/// comment claimed conformance.
fn resolve_limit(asked: Option<&str>) -> Result<u32, CanonicalError> {
    use crate::api::rest::odata::LISTING_LIMIT_CFG;

    let Some(raw) = asked.filter(|s| !s.is_empty()) else {
        return Ok(u32::try_from(LISTING_LIMIT_CFG.default).unwrap_or(u32::MAX));
    };
    let parsed: u32 = raw
        .parse()
        .map_err(|_| refuse_limit(format!("`{raw}` is not a page size")))?;
    if parsed == 0 {
        return Err(refuse_limit("limit must be at least 1".to_owned()));
    }
    let ceiling = u32::try_from(LISTING_LIMIT_CFG.max).unwrap_or(u32::MAX);
    if parsed > ceiling {
        return Err(refuse_limit(format!("limit must not exceed {ceiling}")));
    }
    Ok(parsed)
}

/// One spelling of this door's `limit` refusal.
fn refuse_limit(detail: String) -> CanonicalError {
    UsageTypeCatalogDoor::invalid_argument()
        .with_field_violation("limit", detail, "invalid_limit")
        .create()
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
        .query_param("q", false, "Narrow by substring of the usage type's id.")
        .query_param("kind", false, "Narrow by kind: `counter` or `gauge`.")
        .query_param(
            "limit",
            false,
            "Page size. Absent takes the gear's default; `0` and anything past its ceiling are \
             refused rather than clamped.",
        )
        .query_param(
            "cursor",
            false,
            "The continuation token a previous page handed back.",
        )
        .handler(list_usage_types)
        .json_response_with_schema::<UsageTypeListView>(
            openapi,
            StatusCode::OK,
            "The catalog's page, with the provenance of the answer.",
        )
        .problem_response(
            openapi,
            StatusCode::NOT_IMPLEMENTED,
            "No usage-type catalog is configured, so there is nothing to list. This is not an \
             empty page and must not be rendered as one.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi)
        .layer(Extension(state))
}
