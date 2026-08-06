//! `GET /bss-pricing/v1/plans/{planId}/preview` — the base-price preview
//! (`design/04-currency-tax.md` §2's second flow, `inst-pv-api`,
//! `inst-pv-resolve`, `inst-pv-return`).
//!
//! # The published read model only, and that is the whole rule
//!
//! `inst-pv-resolve`: *"Resolve from the **published read model only** (no draft
//! read); base list price rows only, `PriceOverlay` adjustments disclaimed"*. So
//! this handler reaches the same way `api::rest::windows`' sellability read does
//! — the pin frontier, then the delta at that version — and never touches
//! `pricing_price`. A preview served off the truth side would show a partner a
//! draft nobody has approved.
//!
//! # Fail closed, and never FX
//!
//! `inst-mc-nofx` is the sharpest sentence in the slice: *"No FX derivation ever:
//! a missing `(currency, region)` row is simply absent — preview/publish paths
//! fail closed on it, no base-currency fallback"*. So an absent market is
//! [`PRICE_ROW_ABSENT`] (404) and there is deliberately **no** nearest-currency,
//! no base-currency and no region-fallback branch anywhere below. `Future
//! currencyFallbackPolicy` is named in §1.5 as exactly that — future.
//!
//! The 404 is the design set's own status for it, and it is right: the caller
//! asked for a price on a market this plan does not sell, so the resource they
//! named does not exist. It is **not** an empty 200 — a preview that answered
//! "no price" with a success would be indistinguishable, at a partner's end, from
//! a price of zero.
//!
//! # The disclaimer is part of the contract
//!
//! §2's success scenario requires *"an explicit disclaimer that
//! Contract/`PriceOverlays` apply at purchase (Tariffs evaluates)"*. It is a
//! field of the response rather than prose in the description, because a machine
//! consumer has to be able to carry it into whatever it renders: this is a
//! **list** price and the amount actually charged is Tariffs' to compute.
//!
//! Overlays are excluded from the preview for the same reason `inst-plv-disclosure`
//! gives one slice over — the base-price preview is the surface an overlay's
//! `restricted` disclosure keeps it out of.
//!
//! # The gate is `plan × preview`, which is not `plan × read`
//!
//! §10 and §2 both say so: the preview needs *"the explicit preview grant … an
//! **extra assignment** beyond the `FinanceManager` role: the default role matrix
//! does not carry `plan × preview`"*. Gating it on `read` would hand every holder
//! of the ordinary catalog read a surface the matrix deliberately withholds, and
//! no allow/deny fixture can see the difference — which is why `rest_authz`'s
//! census asserts the pair.

use std::sync::Arc;

use axum::extract::{Extension, Path, Query};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::StatusCode};
use serde::Deserialize;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::operation_builder::{ParamLocation, ParamSpec};
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::auth_context::require_authenticated;
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::state::GovernanceState;
use crate::domain::error::DomainError;
use crate::domain::money::CurrencyCode;
use crate::domain::read_model::SubjectRef;
use crate::domain::scope_key::{PlanId, Region};
use crate::infra::storage::repo::{pin_frontier_repo, read_model_repo};
use crate::infra::storage::repo_failure;

/// `OpenAPI` tag (DE0205).
const TAG: &str = "BSS Pricing Catalog";

/// The preview resource.
///
/// The literal is repeated in the `OperationBuilder` call because DE0801
/// validates a **literal** argument and silently passes a `const` one; the two
/// spellings are pinned together by `tests/module_test.rs`'s route census.
pub const PLAN_PREVIEW: &str = "/bss-pricing/v1/plans/{planId}/preview";

/// A market with no published row (§5, 404 — fail closed, no FX).
pub const PRICE_ROW_ABSENT: &str = "PRICE_ROW_ABSENT";

/// The disclaimer §2 requires on every preview.
///
/// A constant, so the sentence a partner is shown cannot drift between two
/// responses of one deployment.
const OVERLAY_DISCLAIMER: &str = "This is the catalog base list price for the requested market. Contract terms and \
     PriceOverlays may apply at purchase and are evaluated by Tariffs; the amount actually \
     charged may differ.";

/// `?currency=&region=` — both required.
#[derive(Debug, Deserialize)]
struct PreviewQuery {
    currency: Option<String>,
    region: Option<String>,
}

// ---------------------------------------------------------------------------
// Views.
// ---------------------------------------------------------------------------

/// One market's base list price.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct PreviewView {
    /// The plan previewed.
    pub plan_id: Uuid,
    /// The catalog version the answer was resolved at, so a caller can pin what
    /// they were shown.
    pub catalog_version: u64,
    /// The requested currency, echoed.
    pub currency: String,
    /// The requested region, echoed.
    pub region: String,
    /// The base list amount in minor units.
    pub amount_minor: Option<i64>,
    /// Whether that amount is tax-inclusive (§2's success scenario).
    pub tax_inclusive: bool,
    /// D-154's resolved effective tax category, as the version froze it.
    pub resolved_tax_category: Option<String>,
    /// C3's gate: authorable and **previewable**, but not sellable on this
    /// market until Tax Engine GA. Carried because §2 explicitly permits the
    /// preview of a gated row — a caller has to be able to tell the two apart.
    pub not_sellable_ga: bool,
    /// The trial days a consumer surface shows, when the plan declares any.
    pub display_trial_days: Option<i64>,
    /// §2's required disclaimer.
    pub disclaimer: String,
}

// ---------------------------------------------------------------------------
// Router.
// ---------------------------------------------------------------------------

fn currency_param() -> ParamSpec {
    ParamSpec {
        name: "currency".to_owned(),
        location: ParamLocation::Query,
        required: true,
        description: Some(
            "The ISO 4217 code of the market to preview. Required: the catalog performs no FX \
             and has no base currency, so there is no answer to `what does this plan cost` \
             without one."
                .to_owned(),
        ),
        param_type: "string".to_owned(),
    }
}

fn region_param() -> ParamSpec {
    ParamSpec {
        name: "region".to_owned(),
        location: ParamLocation::Query,
        required: true,
        description: Some(
            "The commercial region of the market to preview. Required for `currency`'s reason: a \
             price row is keyed on the pair, and a region-less query names no row. This is the \
             pricing region, not the IdP authorization-region claim."
                .to_owned(),
        ),
        param_type: "string".to_owned(),
    }
}

/// Build the Axum router for the preview and register it.
pub fn router(state: Arc<GovernanceState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-pricing/v1/plans/{planId}/preview")
        .operation_id("bss_pricing.preview_plan_price")
        .summary("Preview a plan's base list price on one market")
        .description(
            "The catalog **base list price** for one `(currency, region)` market, resolved from \
             the **published read model only** - never from a draft, so a preview cannot show a \
             price nobody has approved. The response carries the amount, its `taxInclusive` \
             display basis, the resolved tax category the catalog version froze, the trial days \
             a consumer surface shows, and an explicit **disclaimer**: Contract terms and \
             PriceOverlays may apply at purchase and are evaluated by Tariffs, so the amount \
             actually charged may differ. Overlay adjustments are deliberately not applied here. \
             **Fails closed on an absent market**: if the plan publishes no row for the \
             requested pair the answer is `404` `PRICE_ROW_ABSENT`, never a converted price and \
             never a base-currency fallback - the catalog performs no FX under any circumstance, \
             and a currency fallback policy is a named Future item rather than an omission. A \
             row flagged `notSellableGa` is previewable and is returned with the flag set, which \
             is what lets a caller distinguish `not sold here` from `not sellable yet`. Gates on \
             `plan` x `preview`, which is deliberately **not** `plan` x `read`: the preview \
             grant is an extra assignment the default role matrix does not carry.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .param(currency_param())
        .param(region_param())
        .handler(preview_plan_price)
        .json_response_with_schema::<PreviewView>(
            openapi,
            StatusCode::OK,
            "The base list price for the requested market.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    router.layer(Extension(state))
}

// ---------------------------------------------------------------------------
// Handler.
// ---------------------------------------------------------------------------

async fn preview_plan_price(
    Extension(state): Extension<Arc<GovernanceState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(plan_id): Path<Uuid>,
    Query(query): Query<PreviewQuery>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let plan_id = PlanId::new(plan_id);
    let scope = preview_scope(&enforcer, &ctx, plan_id.get(), ctx.subject_tenant_id()).await?;

    // **After the gate.** A caller without the preview grant is told that, rather
    // than that their query is malformed — the ordering `schedule_window` argues
    // for its own parameters and `rest_authz`'s catalogued-pair property depends
    // on.
    let (currency, region) = market_of(&query)?;
    let tenant = ctx.subject_tenant_id();

    let conn = state
        .db
        .conn()
        .map_err(|e| CanonicalError::from(DomainError::Internal(format!("preview conn: {e}"))))?;

    let absent = || {
        CanonicalError::from(DomainError::PriceRowAbsent(format!(
            "plan {plan_id} publishes no price row on {}/{region}. The catalog performs no FX \
             and has no base-currency fallback, so an absent market is an absent price rather \
             than a converted one",
            currency.as_str()
        )))
    };

    // The frontier first, then the delta at it — `windows::sellability_facts`'
    // arrangement, and for its reason: a read taken at "the latest row" rather
    // than at the pinned version could serve two callers two different answers
    // while the projector is mid-sweep.
    let Some(frontier) = pin_frontier_repo::read_at(&conn, &scope, tenant)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?
    else {
        return Err(absent());
    };
    let Some(delta) = read_model_repo::delta_at(
        &conn,
        &scope,
        tenant,
        &SubjectRef::Plan(plan_id.get()),
        frontier.catalog_version,
    )
    .await
    .map_err(|e| CanonicalError::from(repo_failure(&e)))?
    else {
        return Err(absent());
    };

    let row = base_row(&delta.payload, currency.as_str(), region.as_str()).ok_or_else(absent)?;

    Ok(Json(PreviewView {
        plan_id: plan_id.get(),
        catalog_version: delta.catalog_version.get(),
        currency: currency.as_str().to_owned(),
        region: region.as_str().to_owned(),
        amount_minor: row["amountMinor"].as_i64(),
        tax_inclusive: row["taxInclusive"].as_bool().unwrap_or(false),
        resolved_tax_category: row["resolvedTaxCategory"].as_str().map(ToOwned::to_owned),
        not_sellable_ga: row["notSellableGa"].as_bool().unwrap_or(false),
        display_trial_days: delta.payload["phases"]
            .as_array()
            .and_then(|phases| phases.iter().find_map(|p| p["displayTrialDays"].as_i64())),
        disclaimer: OVERLAY_DISCLAIMER.to_owned(),
    })
    .into_response())
}

/// The published base row for one market, if the version froze one.
///
/// **Base list rows only** (`inst-pv-resolve`): `priceOverlay` is `base` on every
/// row this gear authors, and a grandfathered generation is not what a *new*
/// purchaser would be quoted — quoting one would show a prospective customer a
/// price only an existing subscriber can have.
fn base_row<'a>(
    payload: &'a serde_json::Value,
    currency: &str,
    region: &str,
) -> Option<&'a serde_json::Value> {
    payload["prices"].as_array()?.iter().find(|row| {
        let key = &row["scopeKey"];
        key["currency"] == currency
            && key["region"] == region
            && key["priceOverlay"] == "base"
            && key["priceEligibility"] != "existing_grandfathered"
    })
}

/// Both query parameters, parsed and non-blank.
fn market_of(query: &PreviewQuery) -> Result<(CurrencyCode, Region), CanonicalError> {
    let currency = query.currency.as_deref().unwrap_or_default();
    let region = query.region.as_deref().unwrap_or_default();
    let currency = CurrencyCode::new(currency).map_err(CanonicalError::from)?;
    let region = Region::new(region).map_err(CanonicalError::from)?;
    Ok((currency, region))
}

/// The `plan × preview` gate.
///
/// **Not `plan × read`.** §2 and §10 both make the preview grant an extra
/// assignment the default role matrix does not carry, so gating on `read` would
/// hand it to every holder of the ordinary catalog read. No allow/deny fixture
/// can see that difference, which is why `rest_authz`'s census asserts the pair —
/// the same argument `plan × publish` carries.
async fn preview_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    plan_id: Uuid,
    tenant: Uuid,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::PREVIEW,
        /* owner_tenant_id */ Some(tenant),
        /* resource_id */ Some(plan_id),
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)
}
