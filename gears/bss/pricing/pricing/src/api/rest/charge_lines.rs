//! Line-first authoring: `/plans/{planId}/charge-lines` and the market prices
//! nested under a line version.
//!
//! # Structure is authored once; money is authored per market
//!
//! A charge line carries what every market of it shares: the eight structural
//! axes, the calculation model, tier *geometry*, the usage policy, the descriptor
//! and the proration contract. A market price carries what differs per currency
//! and region: the amounts and rates that fill that structure, and the market's
//! tax and rounding policy. The two are separate requests on separate routes, and
//! **each refuses the other's fields** -- `currency` on a structure, a tier bound
//! on a price -- rather than ignoring them, because a field silently dropped is a
//! client that believes it authored something.
//!
//! # Which tag guards which act
//!
//! Creating, replacing or deleting a line is an edit of the *plan's* content, so
//! those three take the plan revision's tag (`If-Match: "<revision>-<version>"`)
//! and answer with the moved one. A market price is a row of its own: no
//! precondition on create, its own entity tag on `PATCH`/`DELETE` of
//! `…/prices/{priceId}`, exactly as before.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path};
use axum::http::header::{ETAG, LOCATION};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::HeaderMap, http::StatusCode};
use time::OffsetDateTime;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_db::secure::{AccessScope, DbTx};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::auth_context::{audit_stamp, require_authenticated};
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::preconditions;
use crate::api::rest::prices::{
    self, IncludedAllowanceView, PriceContentView, TierBandView, authoring_sku_context, content_of,
    read_scope, require_declared_region, wire_token, write_scope,
};
use crate::api::rest::state::AuthoringState;
use crate::domain::charge_line::TierGeometry;
use crate::domain::contracts::AnchorDay;
use crate::domain::error::DomainError;
use crate::domain::instant::rfc3339;
use crate::domain::money::{CurrencyCode, MinorAmount, RateMinor};
use crate::domain::price_record::PriceContent;
use crate::domain::price_row::{BandTop, TierBand, model_kind_wire};
use crate::domain::scope_key::{
    ChargeKind, ChargeLineScopeKey, Cohort, DimensionKey, MarketPriceScopeKey, PhaseId, PlanId,
    PriceEligibility, Region, SkuId,
};
use crate::domain::validation::ValidationReport;
use crate::infra::charge_line::{self, LineWrite, PlanTag};
use crate::infra::idempotent::{self, Guarded, GuardedRequest, TxFuture};
use crate::infra::storage::repo::price_repo::{self, LineRecord, MarketPriceRecord};
use crate::infra::storage::repo::{NewPriceDraft, window_guard_repo};
use crate::infra::storage::repo_failure;

const TAG: &str = "BSS Pricing Charge Lines";

const CREATE_LINE_OPERATION: &str = "bss_pricing.create_charge_line";
const CREATE_MARKET_PRICE_OPERATION: &str = "bss_pricing.create_market_price";

/// The collection of a plan's charge lines.
pub const PLAN_CHARGE_LINES: &str = "/bss-pricing/v1/plans/{planId}/charge-lines";
/// One line version of a plan.
pub const PLAN_CHARGE_LINE: &str = "/bss-pricing/v1/plans/{planId}/charge-lines/{lineVersionId}";
/// The market prices filed under one line version.
///
/// On one line on purpose: the route census resolves a registration's path by
/// reading `pub const NAME: &str = "…"` off a single source line, so a wrapped
/// declaration is a route it cannot see -- and it says so by count, not by name.
#[rustfmt::skip]
pub const PLAN_CHARGE_LINE_PRICES: &str = "/bss-pricing/v1/plans/{planId}/charge-lines/{lineVersionId}/prices";

/// The code a market's tier rates answer with when they do not fit the line's
/// geometry -- the domain's own, so the wire and the resolver agree on one name.
const TIER_RATE_COUNT_MISMATCH: &str = crate::domain::market_price::MARKET_TIER_RATE_COUNT_MISMATCH;

// ---------------------------------------------------------------------------
// Views and requests.
// ---------------------------------------------------------------------------

/// The axes of a line a caller authors; the plan is the `{planId}` segment and the
/// overlay is always `base` on this surface.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
#[serde(deny_unknown_fields)]
pub struct LineScopeKeyRequest {
    /// The phase the line belongs to. Every phase carries its own complete set.
    pub phase: Uuid,
    /// The SKU the line charges for; a usage line's meter derives from it.
    pub sku_id: Uuid,
    /// `all_subscriptions`, `new_subscriptions_only` or `existing_grandfathered`.
    pub price_eligibility: String,
    /// `one_time`, `recurring` or `usage`.
    pub charge_kind: String,
    /// The grandfathering generation; present exactly when the class is
    /// `existing_grandfathered`.
    #[serde(default, with = "rfc3339::option")]
    pub cohort: Option<OffsetDateTime>,
    /// The usage dimension, `""` or absent for an undimensioned line.
    #[serde(default)]
    pub dimension_key: Option<String>,
}

/// A line's eight axes, as stored.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct LineScopeKeyView {
    /// The plan the line belongs to.
    pub plan_id: Uuid,
    /// Always `base` on this surface.
    pub price_overlay: String,
    /// The phase.
    pub phase: Uuid,
    /// The eligibility class.
    pub price_eligibility: String,
    /// The charge kind.
    pub charge_kind: String,
    /// The grandfathering generation, when the class has one.
    #[serde(default, with = "rfc3339::option")]
    pub cohort: Option<OffsetDateTime>,
    /// The SKU.
    pub sku_id: Uuid,
    /// The usage dimension, `null` when undimensioned.
    pub dimension_key: Option<String>,
}

impl From<&ChargeLineScopeKey> for LineScopeKeyView {
    fn from(key: &ChargeLineScopeKey) -> Self {
        Self {
            plan_id: key.plan_id().get(),
            price_overlay: key.price_overlay().as_str().to_owned(),
            phase: key.phase().get(),
            price_eligibility: key.price_eligibility().as_str().to_owned(),
            charge_kind: key.charge_kind().as_str().to_owned(),
            cohort: key.cohort().generation(),
            sku_id: key.sku_id().as_uuid(),
            dimension_key: (!key.dimension_key().is_none())
                .then(|| key.dimension_key().as_str().to_owned()),
        }
    }
}

/// One tier's bounds. The rate that fills it is a market's, not the line's.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
#[serde(deny_unknown_fields)]
pub struct TierView {
    /// Inclusive lower bound.
    pub from_qty: u64,
    /// Exclusive upper bound; `null` for the open top.
    pub to_qty: Option<u64>,
}

/// What every market of a line shares.
///
/// Every member is optional because a line, like a price row, is assembled over
/// several calls and judged at publish. Unknown members are refused -- including
/// every monetary one, which is how `currency` or `amount_minor` on a structure
/// is an error rather than a no-op.
#[derive(Debug, Clone, Default)]
#[toolkit_macros::api_dto(request, response)]
#[serde(deny_unknown_fields)]
pub struct StructureView {
    /// `flat`, `per_unit`, `graduated`, `volume` or `package`.
    pub model_kind: Option<String>,
    /// Tier geometry, for the two banded kinds.
    pub tiers: Option<Vec<TierView>>,
    /// Units per package, for `package`.
    pub package_size: Option<u64>,
    /// `subscription_seat_count` or `manual`.
    pub quantity_source: Option<String>,
    /// The quantity, when the source is `manual`.
    pub manual_quantity: Option<u64>,
    /// **Derived from the SKU; never accepted on write.** Present on reads of a
    /// usage line. Read through `authored_meter` so that an explicit `null` is an
    /// authored member like any other -- a plain `Option` would fold it into
    /// "absent" and let it through.
    #[serde(
        default,
        deserialize_with = "crate::api::rest::prices::authored_meter",
        skip_serializing_if = "Option::is_none"
    )]
    pub meter: Option<String>,
    /// Usage billing granularity.
    pub billing_granularity: Option<String>,
    /// Tier aggregation window.
    pub tier_aggregation_window: Option<String>,
    /// Tier qualification window.
    pub tier_qualification_window: Option<String>,
    /// Usage aggregation function.
    pub aggregation_function: Option<String>,
    /// Usage aggregation granularity.
    pub aggregation_granularity: Option<String>,
    /// Reservation hold bound.
    pub max_hold_granules: Option<u64>,
    /// D-45's included allowance.
    pub included_allowance: Option<IncludedAllowanceView>,
    /// Reservation flavor.
    pub reservation_flavor: Option<String>,
    /// Minimum purchasable quantity.
    pub min_qty_purchase: Option<u64>,
    /// Minimum billable usage.
    pub min_qty_usage: Option<u64>,
    /// What happens beneath the usage minimum.
    pub min_qty_usage_fallback: Option<String>,
    /// The discount instrument hook.
    pub discount_ref: Option<String>,
    /// Invoice line template override.
    pub invoice_line_template: Option<String>,
    /// GL code override.
    pub gl_code_ref: Option<String>,
    /// `advance` or `arrears`, for a recurring line.
    pub billing_timing: Option<String>,
    /// Proration contract: anchor policy.
    pub billing_anchor_policy: Option<String>,
    /// Proration contract: anchor day.
    pub anchor_day: Option<u8>,
    /// Proration contract: basis.
    pub proration_basis: Option<String>,
    /// Proration contract: credit on downgrade.
    pub credit_on_downgrade: Option<bool>,
}

/// The amounts and rates one market fills a line's structure with.
///
/// Every member names its own scale, which is why they share a suffix: an
/// `amount` and a `rate` here are different quantities in different units, and
/// dropping the unit from the wire name is exactly the ambiguity the existing
/// price content view avoids the same way.
#[allow(
    clippy::struct_field_names,
    reason = "the suffix is the unit, not noise"
)]
#[derive(Debug, Clone, Default)]
#[toolkit_macros::api_dto(request, response)]
#[serde(deny_unknown_fields)]
pub struct MoneyView {
    /// The flat amount, in minor units.
    pub amount_minor: Option<i64>,
    /// The per-unit rate, in nano-minor units.
    pub unit_rate_nano_minor: Option<i64>,
    /// One rate per tier of the line, in the line's quantity order.
    pub tier_rates_nano_minor: Option<Vec<i64>>,
    /// The package price, in minor units.
    pub package_price_minor: Option<i64>,
    /// The reserved-capacity rate, in nano-minor units.
    pub reserved_rate_nano_minor: Option<i64>,
}

/// The per-market policy that travels with the money.
#[derive(Debug, Clone, Default)]
#[toolkit_macros::api_dto(request, response)]
#[serde(deny_unknown_fields)]
pub struct MarketPolicyView {
    /// Whether the amounts include tax. Defaults to `false`.
    pub tax_inclusive: Option<bool>,
    /// Tax category override.
    pub tax_category_ref: Option<String>,
    /// Rounding policy override.
    pub rounding_policy_ref: Option<String>,
    /// The grandfathering horizon, on an `existing_grandfathered` line.
    #[serde(default, with = "rfc3339::option")]
    pub grandfather_until: Option<OffsetDateTime>,
}

/// Draft a line: its axes and its shared structure. No market yet.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
#[serde(deny_unknown_fields)]
pub struct CreateChargeLineRequest {
    /// The axes the line is filed under.
    pub scope_key: LineScopeKeyRequest,
    /// What every market of it will share.
    pub structure: StructureView,
}

/// Replace a draft line version's whole shared structure.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
#[serde(deny_unknown_fields)]
pub struct PatchChargeLineRequest {
    /// The whole structure, replacing what is there.
    pub structure: StructureView,
}

/// File a market price under a line version.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
#[serde(deny_unknown_fields)]
pub struct CreateMarketPriceRequest {
    /// ISO 4217 currency of the market.
    pub currency: String,
    /// The market's region; must be one the tenant declared.
    pub region: String,
    /// The amounts and rates.
    pub money: MoneyView,
    /// The market's tax and rounding policy.
    #[serde(default)]
    pub market_policy: MarketPolicyView,
}

/// Replace a draft market price's monetary content.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
#[serde(deny_unknown_fields)]
pub struct PatchMarketPriceRequest {
    /// The amounts and rates, replacing what is there.
    pub money: MoneyView,
    /// The market's tax and rounding policy, replacing what is there.
    #[serde(default)]
    pub market_policy: MarketPolicyView,
}

/// One monetary version under a line.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct MarketPriceView {
    /// The stable currency/region variant.
    pub market_price_id: Uuid,
    /// This monetary version. Windows and history address it.
    pub price_id: Uuid,
    /// The exact structure version the money is priced against.
    pub line_version_id: Uuid,
    /// The stable logical line.
    pub charge_line_id: Uuid,
    /// The market's currency.
    pub currency: String,
    /// The market's region.
    pub region: String,
    /// The amounts and rates.
    pub money: MoneyView,
    /// The market's policy.
    pub market_policy: MarketPolicyView,
    /// `draft`, `published` or `superseded`.
    pub lifecycle_state: String,
    /// The row's own entity-tag version; `PATCH`/`DELETE` of the price take it.
    pub row_version: u64,
}

impl From<&MarketPriceRecord> for MarketPriceView {
    fn from(found: &MarketPriceRecord) -> Self {
        let record = &found.record;
        let row = &record.row;
        Self {
            market_price_id: found.market_price_id,
            price_id: record.price_id,
            line_version_id: found.line_version_id,
            charge_line_id: found.charge_line_id,
            currency: record.scope_key.currency().as_str().to_owned(),
            region: record.scope_key.region().as_str().to_owned(),
            money: MoneyView {
                amount_minor: row.amount_minor.map(MinorAmount::get),
                unit_rate_nano_minor: row.unit_rate.map(RateMinor::nano_minor),
                tier_rates_nano_minor: (!row.bands.is_empty()).then(|| {
                    row.bands
                        .iter()
                        .map(|band| band.unit_price_rate.nano_minor())
                        .collect()
                }),
                package_price_minor: row.package_price_minor.map(MinorAmount::get),
                reserved_rate_nano_minor: row.reserved_rate.map(RateMinor::nano_minor),
            },
            market_policy: MarketPolicyView {
                tax_inclusive: Some(record.tax_inclusive),
                tax_category_ref: record.tax_category_ref.clone(),
                rounding_policy_ref: record.rounding_policy_ref.clone(),
                grandfather_until: record.grandfather_until,
            },
            lifecycle_state: record.lifecycle_state.as_str().to_owned(),
            row_version: record.row_version.get(),
        }
    }
}

/// A line version with the market prices filed under it.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ChargeLineView {
    /// The stable logical line: survives revisions.
    pub charge_line_id: Uuid,
    /// This version of its shared content: a revision allocates a new one.
    pub line_version_id: Uuid,
    /// The plan revision that owns the version.
    pub plan_revision: u64,
    /// `draft` while editable; frozen on publication.
    pub lifecycle_state: String,
    /// The version's own entity-tag version.
    pub row_version: u64,
    /// The eight axes.
    pub scope_key: LineScopeKeyView,
    /// What every market shares.
    pub structure: StructureView,
    /// The market prices filed under this version, in market order.
    pub prices: Vec<MarketPriceView>,
}

impl ChargeLineView {
    fn of(line: &LineRecord, prices: &[MarketPriceRecord]) -> Self {
        Self {
            charge_line_id: line.charge_line_id,
            line_version_id: line.line_version_id,
            plan_revision: line.plan_revision,
            lifecycle_state: line.lifecycle_state.as_str().to_owned(),
            row_version: line.row_version.get(),
            scope_key: LineScopeKeyView::from(&line.scope_key),
            structure: structure_view(line),
            prices: prices.iter().map(MarketPriceView::from).collect(),
        }
    }
}

/// The lines of one plan.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ChargeLineListView {
    /// Every line of the plan at its latest version, in line-id order.
    pub items: Vec<ChargeLineView>,
}

fn structure_view(line: &LineRecord) -> StructureView {
    let row = &line.content.row;
    let contract = line.content.proration_contract;
    StructureView {
        model_kind: row.model_kind.map(model_kind_wire).map(str::to_owned),
        tiers: (!line.tiers.is_empty()).then(|| {
            line.tiers
                .iter()
                .map(|tier| TierView {
                    from_qty: tier.from_qty,
                    to_qty: tier.to_qty.closed_at(),
                })
                .collect()
        }),
        package_size: row.package_size,
        quantity_source: row.quantity_source.map(|q| q.as_str().to_owned()),
        manual_quantity: row.manual_quantity,
        meter: row.meter.clone(),
        billing_granularity: row.billing_granularity.map(|g| g.as_str().to_owned()),
        tier_aggregation_window: row.tier_aggregation_window.map(|w| w.as_str().to_owned()),
        tier_qualification_window: row.tier_qualification_window.map(|w| w.as_str().to_owned()),
        aggregation_function: row.aggregation_function.map(|f| f.as_str().to_owned()),
        aggregation_granularity: row.aggregation_granularity.map(|g| g.as_str().to_owned()),
        max_hold_granules: row.max_hold_granules,
        included_allowance: row.included_allowance.as_ref().map(|allowance| {
            IncludedAllowanceView {
                quantity: allowance.quantity,
                rollover_policy: allowance.rollover_policy.as_str().to_owned(),
            }
        }),
        reservation_flavor: row.reservation_flavor.map(|f| f.as_str().to_owned()),
        min_qty_purchase: row.min_qty_purchase,
        min_qty_usage: row.min_qty_usage,
        min_qty_usage_fallback: row.min_qty_usage_fallback.map(|f| f.as_str().to_owned()),
        discount_ref: row.discount_ref.clone(),
        invoice_line_template: row.invoice_line_template.clone(),
        gl_code_ref: row.gl_code_ref.clone(),
        billing_timing: line.content.billing_timing.clone(),
        billing_anchor_policy: contract.map(|c| c.billing_anchor_policy.as_str().to_owned()),
        anchor_day: contract
            .and_then(|c| c.billing_anchor_policy.anchor_day())
            .map(AnchorDay::get),
        proration_basis: contract.map(|c| c.proration_basis.as_str().to_owned()),
        credit_on_downgrade: contract.map(|c| c.credit_on_downgrade),
    }
}

/// [`structure_view`] off the **domain** line version rather than the stored
/// record — the approval document's reader, which holds a [`PlanShape`] and not a
/// repository row.
///
/// Destructured with no rest pattern, so a member added to [`ChargeStructure`]
/// stops this compiling: the pin frames the structure exhaustively, and a
/// reviewer's document that quietly rendered one member fewer would be a
/// signature over content they were not shown. `charge_kind`, `sku_id` and
/// `dimension_key` are rendered by the line's key beside this view.
///
/// [`PlanShape`]: crate::domain::plan_shape::PlanShape
/// [`ChargeStructure`]: crate::domain::charge_line::ChargeStructure
pub(crate) fn structure_view_of(
    line: &crate::domain::charge_line::ChargeLineVersion,
) -> StructureView {
    let crate::domain::charge_line::ChargeStructure {
        invoice_line_template,
        gl_code_ref,
        charge_kind: _,
        model_kind,
        bands,
        package_size,
        quantity_source,
        manual_quantity,
        sku_id: _,
        meter,
        dimension_key: _,
        billing_granularity,
        tier_aggregation_window,
        tier_qualification_window,
        aggregation_function,
        aggregation_granularity,
        max_hold_granules,
        included_allowance,
        reservation_flavor,
        min_qty_purchase,
        min_qty_usage,
        min_qty_usage_fallback,
        discount_ref,
    } = &line.structure;
    let contract = line.proration_contract;
    StructureView {
        model_kind: model_kind.map(model_kind_wire).map(str::to_owned),
        tiers: (!bands.is_empty()).then(|| {
            bands
                .iter()
                .map(|tier| TierView {
                    from_qty: tier.from_qty,
                    to_qty: tier.to_qty.closed_at(),
                })
                .collect()
        }),
        package_size: *package_size,
        quantity_source: quantity_source.map(|q| q.as_str().to_owned()),
        manual_quantity: *manual_quantity,
        meter: meter.clone(),
        billing_granularity: billing_granularity.map(|g| g.as_str().to_owned()),
        tier_aggregation_window: tier_aggregation_window.map(|w| w.as_str().to_owned()),
        tier_qualification_window: tier_qualification_window.map(|w| w.as_str().to_owned()),
        aggregation_function: aggregation_function.map(|f| f.as_str().to_owned()),
        aggregation_granularity: aggregation_granularity.map(|g| g.as_str().to_owned()),
        max_hold_granules: *max_hold_granules,
        included_allowance: included_allowance
            .as_ref()
            .map(|allowance| IncludedAllowanceView {
                quantity: allowance.quantity,
                rollover_policy: allowance.rollover_policy.as_str().to_owned(),
            }),
        reservation_flavor: reservation_flavor.map(|f| f.as_str().to_owned()),
        min_qty_purchase: *min_qty_purchase,
        min_qty_usage: *min_qty_usage,
        min_qty_usage_fallback: min_qty_usage_fallback.map(|f| f.as_str().to_owned()),
        discount_ref: discount_ref.clone(),
        invoice_line_template: invoice_line_template.clone(),
        gl_code_ref: gl_code_ref.clone(),
        billing_timing: line.billing_timing.clone(),
        billing_anchor_policy: contract.map(|c| c.billing_anchor_policy.as_str().to_owned()),
        anchor_day: contract
            .and_then(|c| c.billing_anchor_policy.anchor_day())
            .map(AnchorDay::get),
        proration_basis: contract.map(|c| c.proration_basis.as_str().to_owned()),
        credit_on_downgrade: contract.map(|c| c.credit_on_downgrade),
    }
}

/// A market's money, off the domain terms.
pub(crate) fn money_view_of(money: &crate::domain::market_price::MarketPriceTerms) -> MoneyView {
    let crate::domain::market_price::MarketPriceTerms {
        amount_minor,
        unit_rate,
        tier_rates,
        package_price_minor,
        reserved_rate,
    } = money;
    MoneyView {
        amount_minor: amount_minor.map(crate::domain::money::MinorAmount::get),
        unit_rate_nano_minor: unit_rate.map(RateMinor::nano_minor),
        tier_rates_nano_minor: (!tier_rates.is_empty())
            .then(|| tier_rates.iter().map(|rate| rate.nano_minor()).collect()),
        package_price_minor: package_price_minor.map(crate::domain::money::MinorAmount::get),
        reserved_rate_nano_minor: reserved_rate.map(RateMinor::nano_minor),
    }
}

// ---------------------------------------------------------------------------
// Wire -> domain.
// ---------------------------------------------------------------------------

fn line_key_of(
    plan_id: PlanId,
    key: &LineScopeKeyRequest,
) -> Result<ChargeLineScopeKey, DomainError> {
    ChargeLineScopeKey::new(
        plan_id,
        PhaseId::new(key.phase),
        wire_token(
            "scope_key.price_eligibility",
            &key.price_eligibility,
            price_repo::PRICE_ELIGIBILITIES,
            PriceEligibility::as_str,
        )?,
        wire_token(
            "scope_key.charge_kind",
            &key.charge_kind,
            price_repo::CHARGE_KINDS,
            ChargeKind::as_str,
        )?,
        key.cohort.map_or(Cohort::None, Cohort::Generation),
        SkuId::new(key.sku_id),
    )?
    .with_dimension_key(DimensionKey::new(
        key.dimension_key.as_deref().unwrap_or_default(),
    ))
}

/// The shared half of a row's content, through the one reader every content
/// token already goes through.
///
/// A structure is rendered as the flat content it is the shared half of -- no
/// money, and each tier as a band whose rate is zero -- and handed to
/// [`content_of`], so a token, a bound or a proration contract is parsed by the
/// same code whichever door it came through. The zero rates are never stored: a
/// line writes geometry only.
fn structure_content(
    key: &ChargeLineScopeKey,
    structure: &StructureView,
) -> Result<PriceContent, DomainError> {
    if structure.meter.is_some() {
        let mut report = ValidationReport::default();
        report.violate_at_write(
            "VALIDATION",
            "structure.meter",
            "meter is derived from scope_key.sku_id and is not accepted on write (D-372)",
        );
        return Err(DomainError::ValidationFailed(report));
    }
    let flat = PriceContentView {
        invoice_line_template: structure.invoice_line_template.clone(),
        gl_code_ref: structure.gl_code_ref.clone(),
        model_kind: structure.model_kind.clone(),
        amount_minor: None,
        unit_rate_nano_minor: None,
        bands: structure.tiers.as_ref().map(|tiers| {
            tiers
                .iter()
                .map(|tier| TierBandView {
                    from_qty: tier.from_qty,
                    to_qty: tier.to_qty,
                    unit_price_nano_minor: 0,
                })
                .collect()
        }),
        package_size: structure.package_size,
        package_price_minor: None,
        quantity_source: structure.quantity_source.clone(),
        manual_quantity: structure.manual_quantity,
        meter: None,
        dimension_key: (!key.dimension_key().is_none())
            .then(|| key.dimension_key().as_str().to_owned()),
        billing_granularity: structure.billing_granularity.clone(),
        tier_aggregation_window: structure.tier_aggregation_window.clone(),
        tier_qualification_window: structure.tier_qualification_window.clone(),
        aggregation_function: structure.aggregation_function.clone(),
        aggregation_granularity: structure.aggregation_granularity.clone(),
        max_hold_granules: structure.max_hold_granules,
        included_allowance: structure.included_allowance.clone(),
        reserved_rate_nano_minor: None,
        reservation_flavor: structure.reservation_flavor.clone(),
        min_qty_purchase: structure.min_qty_purchase,
        min_qty_usage: structure.min_qty_usage,
        min_qty_usage_fallback: structure.min_qty_usage_fallback.clone(),
        discount_ref: structure.discount_ref.clone(),
        tax_inclusive: None,
        tax_category_ref: None,
        billing_timing: structure.billing_timing.clone(),
        billing_anchor_policy: structure.billing_anchor_policy.clone(),
        anchor_day: structure.anchor_day,
        proration_basis: structure.proration_basis.clone(),
        credit_on_downgrade: structure.credit_on_downgrade,
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
    };
    content_of(&flat)
}

/// A line's stored shared content, filled with one market's money.
///
/// # Errors
/// [`DomainError::ValidationFailed`] carrying
/// `MARKET_TIER_RATE_COUNT_MISMATCH` when rates are supplied and their count is
/// not the line's tier count: a rate is stored against a band by position, so a
/// surplus one has no band to be the rate of and a partial ladder would bill the
/// uncovered tiers at nothing. No rates at all is a legal unfinished draft.
pub(crate) fn market_content(
    line: &LineRecord,
    money: &MoneyView,
    policy: &MarketPolicyView,
) -> Result<PriceContent, DomainError> {
    let rates = money.tier_rates_nano_minor.as_deref().unwrap_or_default();
    if !rates.is_empty() && rates.len() != line.tiers.len() {
        let mut report = ValidationReport::default();
        report.violate_at_write(
            TIER_RATE_COUNT_MISMATCH,
            "money.tier_rates_nano_minor",
            format!(
                "the line has {} tier(s) and the market supplies {} rate(s); send one rate per \
                 tier, in the line's quantity order",
                line.tiers.len(),
                rates.len()
            ),
        );
        return Err(DomainError::ValidationFailed(report));
    }
    let bands = line
        .tiers
        .iter()
        .zip(rates)
        .map(|(tier, rate)| band_of(tier, *rate))
        .collect::<Result<Vec<_>, _>>()?;
    let mut content = line.content.clone();
    content.row.amount_minor = prices::amount("money.amount_minor", money.amount_minor)?;
    content.row.unit_rate = rate_of("money.unit_rate_nano_minor", money.unit_rate_nano_minor)?;
    content.row.bands = bands;
    content.row.package_price_minor =
        prices::amount("money.package_price_minor", money.package_price_minor)?;
    content.row.reserved_rate = rate_of(
        "money.reserved_rate_nano_minor",
        money.reserved_rate_nano_minor,
    )?;
    content.tax_inclusive = policy.tax_inclusive.unwrap_or(false);
    content
        .tax_category_ref
        .clone_from(&policy.tax_category_ref);
    content
        .rounding_policy_ref
        .clone_from(&policy.rounding_policy_ref);
    content.grandfather_until = policy.grandfather_until;
    Ok(content)
}

fn band_of(tier: &TierGeometry, rate: i64) -> Result<TierBand, DomainError> {
    Ok(TierBand {
        from_qty: tier.from_qty,
        to_qty: match tier.to_qty {
            BandTop::Open => BandTop::Open,
            BandTop::Closed(top) => BandTop::Closed(top),
        },
        unit_price_rate: RateMinor::from_nano_minor(rate).map_err(|e| {
            DomainError::InvalidRequest(format!("money.tier_rates_nano_minor: {e}"))
        })?,
    })
}

fn rate_of(field: &str, raw: Option<i64>) -> Result<Option<RateMinor>, DomainError> {
    raw.map(RateMinor::from_nano_minor)
        .transpose()
        .map_err(|e| DomainError::InvalidRequest(format!("{field}: {e}")))
}

/// Write-stage rules over a line: the contradictions between its content and its
/// own frozen key, which a draft may not carry even once.
fn require_no_line_contradiction(
    key: &ChargeLineScopeKey,
    content: &mut PriceContent,
    sku_context: crate::domain::row_sku_rules::RowSkuContext,
) -> Result<(), DomainError> {
    content.row.charge_kind = key.charge_kind();
    content.row.sku_id = key.sku_id();
    content.row.meter = if key.charge_kind().is_usage() {
        sku_context
            .index
            .get(key.sku_id())
            .and_then(|sku| sku.metering_unit.clone())
    } else {
        None
    };
    let report = crate::domain::rules::price_row_rules(sku_context).run(&content.row);
    match report.write_stage_only() {
        None => Ok(()),
        Some(write_stage) => Err(DomainError::ValidationFailed(write_stage)),
    }
}

/// The plan tag a structural write asserts.
///
/// Takes the parsed tag rather than the headers so that each handler names
/// `preconditions::if_match_revision` itself -- which is what the route census reads
/// to check a route that *reads* a precondition also *declares* one.
fn plan_tag(plan_id: PlanId, tag: preconditions::RevisionTag) -> PlanTag {
    PlanTag {
        plan_id,
        revision: tag.revision,
        version: tag.version,
    }
}

// ---------------------------------------------------------------------------
// Router.
// ---------------------------------------------------------------------------

/// Build the Axum router for the charge-line surface and register its operations.
#[allow(
    clippy::too_many_lines,
    reason = "one builder chain per operation; flat is clearer than helpers that hide which route declares which response"
)]
pub fn router(state: Arc<AuthoringState>, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router = Router::new();

    router = OperationBuilder::post(PLAN_CHARGE_LINES)
        .operation_id(CREATE_LINE_OPERATION)
        .summary("Draft a charge line: its axes and the structure every market shares")
        .description(
            "Creates a logical charge line and the `draft` version of its shared structure in \
             the plan's open draft revision, with no market price yet. Answers `201` with a \
             `Location` naming the line version and an `ETag` carrying the plan revision's \
             moved tag. `If-Match` takes the plan revision's tag and an `Idempotency-Key` is \
             required. The eight axes are the line's identity: a second line on the same axes \
             is refused `DUPLICATE_SCOPE_KEY` whatever id it would have been given, and an \
             axis is never edited in place - a different axis is a different line. A usage \
             line's `meter` is derived from its SKU and refused when sent. Money and currency \
             are not members of a structure and are refused as unknown fields; they belong to \
             the market prices filed under the line.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan the line belongs to.")
        .param(crate::api::rest::plans::if_match_param("plan revision"))
        .param(crate::api::rest::plans::idempotency_key_param())
        .json_request::<CreateChargeLineRequest>(openapi, "The line's axes and structure.")
        .handler(create_line)
        .json_response_with_schema::<ChargeLineView>(
            openapi,
            StatusCode::CREATED,
            "The newly drafted line, with no prices.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::get(PLAN_CHARGE_LINES)
        .operation_id("bss_pricing.list_charge_lines")
        .summary("List a plan's charge lines with their market prices")
        .description(
            "Returns every line of the plan at its latest version, each with the market prices \
             filed under that version. This is the authoring read (`plan` x `read`).",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan whose lines to list.")
        .handler(list_lines)
        .json_response_with_schema::<ChargeLineListView>(
            openapi,
            StatusCode::OK,
            "The plan's lines.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::get(PLAN_CHARGE_LINE)
        .operation_id("bss_pricing.get_charge_line")
        .summary("Read one charge line version with its market prices")
        .description(
            "Returns the named line version when it belongs to `{planId}`. A version under a \
             different plan than the `{planId}` segment answers `404`, exactly as an absent \
             one does.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan the line belongs to.")
        .path_param("lineVersionId", "The line version to read.")
        .handler(get_line)
        .json_response_with_schema::<ChargeLineView>(openapi, StatusCode::OK, "The line version.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::patch(PLAN_CHARGE_LINE)
        .operation_id("bss_pricing.patch_charge_line")
        .summary("Replace a draft line version's shared structure")
        .description(
            "Replaces the whole shared structure of a `draft` line version, tier geometry \
             included, under the plan revision's `If-Match`. Every market's tier rates are \
             kept where the new geometry still has a tier at that position; a market left with \
             fewer rates than tiers is an unfinished draft that publish reports. A `published` \
             version is immutable and answers the immutable-resource refusal; the axes are \
             never editable.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan the line belongs to.")
        .path_param("lineVersionId", "The draft line version to edit.")
        .param(crate::api::rest::plans::if_match_param("plan revision"))
        .json_request::<PatchChargeLineRequest>(openapi, "The whole structure.")
        .handler(patch_line)
        .json_response_with_schema::<ChargeLineView>(openapi, StatusCode::OK, "The edited line.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::delete(PLAN_CHARGE_LINE)
        .operation_id("bss_pricing.delete_charge_line")
        .summary("Delete a draft line version nothing is priced against")
        .description(
            "Deletes a `draft` line version under the plan revision's `If-Match`, and the \
             logical line with it when this was its only version. A version a market price \
             still references is refused `CHARGE_LINE_IN_USE`: delete its prices first.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan the line belongs to.")
        .path_param("lineVersionId", "The draft line version to delete.")
        .param(crate::api::rest::plans::if_match_param("plan revision"))
        .handler(delete_line)
        .no_content_response(StatusCode::NO_CONTENT, "The line version was deleted.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::post(PLAN_CHARGE_LINE_PRICES)
        .operation_id(CREATE_MARKET_PRICE_OPERATION)
        .summary("File a market price under a line version")
        .description(
            "Creates a `draft` monetary version for one currency and region of the line, \
             priced against exactly this line version. Answers `201` with a `Location` naming \
             the price row and an `ETag` carrying its own row version. An `Idempotency-Key` is \
             required. The region must be one the tenant declared (`REGION_UNKNOWN`), and a \
             market that already holds a `draft` or `published` price is refused \
             `DUPLICATE_SCOPE_KEY`. Tier rates are one per tier of the line, in its quantity \
             order (`MARKET_TIER_RATE_COUNT_MISMATCH` otherwise). Model, tier geometry and \
             package size are the line's and are refused here as unknown fields.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan the line belongs to.")
        .path_param(
            "lineVersionId",
            "The line version the money is priced against.",
        )
        .param(crate::api::rest::plans::idempotency_key_param())
        .json_request::<CreateMarketPriceRequest>(openapi, "The market and its money.")
        .handler(create_market_price)
        .json_response_with_schema::<MarketPriceView>(
            openapi,
            StatusCode::CREATED,
            "The newly created draft market price.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::get(PLAN_CHARGE_LINE_PRICES)
        .operation_id("bss_pricing.list_market_prices")
        .summary("List the market prices filed under a line version")
        .description("Returns the monetary versions priced against this line version.")
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan the line belongs to.")
        .path_param("lineVersionId", "The line version.")
        .handler(list_market_prices)
        .json_array_response_with_schema::<MarketPriceView>(
            openapi,
            StatusCode::OK,
            "The market prices, in market order.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    // The correlation middleware travels with the routes, as on every authoring
    // router: a surface reachable without it cannot build an `AuditStamp`.
    router
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

async fn create_line(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path(plan_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = write_scope(&enforcer, &ctx, plan_id.get(), tenant).await?;
    let request: CreateChargeLineRequest = preconditions::parse_body(&body)?;
    let tag = plan_tag(plan_id, preconditions::if_match_revision(&headers)?);
    let client_key = preconditions::idempotency_key(&headers)?;
    let request_hash = preconditions::request_digest(&request)?;
    let key = line_key_of(plan_id, &request.scope_key)?;
    let mut content = structure_content(&key, &request.structure)?;
    let now = OffsetDateTime::now_utc();

    // The registry is read in front of the transaction, and a recorded response is
    // answered before it is read at all -- `prices::create_price`'s note is the
    // argument, and this door takes the same position for the same reason.
    {
        let conn = state
            .db
            .conn()
            .map_err(|e| DomainError::Internal(format!("line authoring connection: {e}")))?;
        if let Some((status, body)) = state
            .idempotency
            .recorded_response(
                &conn,
                &scope,
                tenant,
                CREATE_LINE_OPERATION,
                &client_key,
                &request_hash,
                now,
            )
            .await
            .map_err(|e| repo_failure(&e))?
        {
            return Ok(replayed_line(plan_id, status, &body)?);
        }
        let sku_context = authoring_sku_context(
            &conn,
            state.catalog.as_ref(),
            &ctx,
            &scope,
            tenant,
            plan_id,
            key.sku_id().as_uuid(),
        )
        .await?;
        require_no_line_contradiction(&key, &mut content, sku_context)?;
    }

    let stamp = audit_stamp(&ctx, now, correlation);
    let scope_for_body = scope.clone();
    let outcome = idempotent::guarded(
        &state.db,
        &state.idempotency,
        &scope,
        GuardedRequest {
            operation: CREATE_LINE_OPERATION,
            client_key,
            request_hash,
            tenant_id: tenant,
            status: StatusCode::CREATED.as_u16().into(),
            now,
        },
        move |txn: &DbTx<'_>| -> TxFuture<'_, LineWrite> {
            Box::pin(async move {
                charge_line::create_line(txn, &scope_for_body, tenant, tag, &key, &content, stamp)
                    .await
            })
        },
        |write: &LineWrite| line_json(&ChargeLineView::of(&write.line, &[])),
    )
    .await
    .map_err(CanonicalError::from)?;

    Ok(match outcome {
        Guarded::Performed(write) => (
            StatusCode::CREATED,
            [
                (LOCATION, line_location(plan_id, write.line.line_version_id)),
                (ETAG, moved_tag(tag, write.plan_row_version)),
            ],
            Json(ChargeLineView::of(&write.line, &[])),
        )
            .into_response(),
        Guarded::Replayed { status, body } => replayed_line(plan_id, status, &body)?,
    })
}

async fn list_lines(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(plan_id): Path<Uuid>,
) -> Result<Json<ChargeLineListView>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = read_scope(&enforcer, &ctx, plan_id.get()).await?;
    let conn = state
        .db
        .conn()
        .map_err(|e| DomainError::Internal(format!("line read connection: {e}")))?;
    let lines = price_repo::list_line_records(&conn, &scope, tenant, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?;
    let mut items = Vec::with_capacity(lines.len());
    for line in &lines {
        let prices =
            price_repo::list_prices_of_version(&conn, &scope, tenant, line.line_version_id)
                .await
                .map_err(|e| repo_failure(&e))?;
        items.push(ChargeLineView::of(line, &prices));
    }
    Ok(Json(ChargeLineListView { items }))
}

async fn get_line(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path((plan_id, line_version_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ChargeLineView>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = read_scope(&enforcer, &ctx, plan_id.get()).await?;
    let view = line_view(&state, &scope, tenant, plan_id, line_version_id).await?;
    Ok(Json(view))
}

async fn patch_line(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path((plan_id, line_version_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = write_scope(&enforcer, &ctx, plan_id.get(), tenant).await?;

    let request: PatchChargeLineRequest = preconditions::parse_body(&body)?;
    let tag = plan_tag(plan_id, preconditions::if_match_revision(&headers)?);
    let now = OffsetDateTime::now_utc();
    let content = {
        let conn = state
            .db
            .conn()
            .map_err(|e| DomainError::Internal(format!("line authoring connection: {e}")))?;
        let stored =
            charge_line::require_line_of_plan(&conn, &scope, tenant, plan_id, line_version_id)
                .await?;
        let mut content = structure_content(&stored.scope_key, &request.structure)?;
        let mut sku_context = authoring_sku_context(
            &conn,
            state.catalog.as_ref(),
            &ctx,
            &scope,
            tenant,
            plan_id,
            stored.scope_key.sku_id().as_uuid(),
        )
        .await?;
        // An edit introduces no SKU: the line was already filed under this one.
        sku_context.introducing = false;
        require_no_line_contradiction(&stored.scope_key, &mut content, sku_context)?;
        content
    };

    let stamp = audit_stamp(&ctx, now, correlation);
    let txn_scope = scope.clone();
    let (_, outcome) = state
        .db
        .db()
        .in_transaction::<LineWrite, DomainError, _>(move |txn| {
            Box::pin(async move {
                charge_line::replace_structure(
                    txn,
                    &txn_scope,
                    tenant,
                    tag,
                    line_version_id,
                    &content,
                    stamp,
                )
                .await
            })
        })
        .await;
    let write = outcome.map_err(|err| {
        err.into_domain(|infra| {
            DomainError::Internal(format!("bss-pricing: charge line structure write: {infra}"))
        })
    })?;

    let view = line_view(&state, &scope, tenant, plan_id, write.line.line_version_id).await?;
    Ok((
        StatusCode::OK,
        [(ETAG, moved_tag(tag, write.plan_row_version))],
        Json(view),
    )
        .into_response())
}

async fn delete_line(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path((plan_id, line_version_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = write_scope(&enforcer, &ctx, plan_id.get(), tenant).await?;
    let tag = plan_tag(plan_id, preconditions::if_match_revision(&headers)?);
    let stamp = audit_stamp(&ctx, OffsetDateTime::now_utc(), correlation);

    let txn_scope = scope.clone();
    let (_, outcome) = state
        .db
        .db()
        .in_transaction::<u64, DomainError, _>(move |txn| {
            Box::pin(async move {
                charge_line::delete_version(txn, &txn_scope, tenant, tag, line_version_id, stamp)
                    .await
            })
        })
        .await;
    let plan_row_version = outcome.map_err(|err| {
        err.into_domain(|infra| {
            DomainError::Internal(format!("bss-pricing: charge line delete: {infra}"))
        })
    })?;

    Ok((
        StatusCode::NO_CONTENT,
        [(ETAG, moved_tag(tag, plan_row_version))],
    )
        .into_response())
}

async fn create_market_price(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path((plan_id, line_version_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = write_scope(&enforcer, &ctx, plan_id.get(), tenant).await?;

    let request: CreateMarketPriceRequest = preconditions::parse_body(&body)?;
    let client_key = preconditions::idempotency_key(&headers)?;
    let request_hash = preconditions::request_digest(&(line_version_id, &request))?;
    let currency = CurrencyCode::new(&request.currency)?;
    let region = Region::new(&request.region)?;
    let now = OffsetDateTime::now_utc();

    let (key, content) = {
        let conn = state
            .db
            .conn()
            .map_err(|e| DomainError::Internal(format!("price authoring connection: {e}")))?;
        if let Some((status, body)) = state
            .idempotency
            .recorded_response(
                &conn,
                &scope,
                tenant,
                CREATE_MARKET_PRICE_OPERATION,
                &client_key,
                &request_hash,
                now,
            )
            .await
            .map_err(|e| repo_failure(&e))?
        {
            return Ok(replayed_price(plan_id, status, &body)?);
        }
        let line =
            charge_line::require_line_of_plan(&conn, &scope, tenant, plan_id, line_version_id)
                .await?;
        let mut content = market_content(&line, &request.money, &request.market_policy)?;
        let key = MarketPriceScopeKey::new(line.scope_key.clone(), currency, region);
        let sku_context = authoring_sku_context(
            &conn,
            state.catalog.as_ref(),
            &ctx,
            &scope,
            tenant,
            plan_id,
            key.sku_id().as_uuid(),
        )
        .await?;
        prices::derive_meter(&mut content, &key, &sku_context.index);
        prices::require_no_key_contradiction(&key, &content, sku_context)?;
        (key, content)
    };

    let scope_for_body = scope.clone();
    let actor = ctx.subject_id();
    let outcome = idempotent::guarded(
        &state.db,
        &state.idempotency,
        &scope,
        GuardedRequest {
            operation: CREATE_MARKET_PRICE_OPERATION,
            client_key,
            request_hash,
            tenant_id: tenant,
            status: StatusCode::CREATED.as_u16().into(),
            now,
        },
        move |txn: &DbTx<'_>| -> TxFuture<'_, MarketPriceRecord> {
            Box::pin(async move {
                require_declared_region(txn, &scope_for_body, tenant, &key).await?;
                window_guard_repo::acquire(txn, &scope_for_body, tenant, plan_id.get())
                    .await
                    .map_err(|e| repo_failure(&e))?;
                let price_id = Uuid::now_v7();
                let draft = NewPriceDraft {
                    price_id,
                    // The exact-reference door: this money is priced against *this*
                    // version, and filing it must not rewrite the structure -- a
                    // sibling market's rates point into its geometry.
                    line_version_id: Some(line_version_id),
                    market_price_id: None,
                    scope_key: key,
                    content,
                    created_by: actor,
                    created_at_utc: now,
                    correlation_id: correlation,
                };
                Box::pin(price_repo::create_draft_on(
                    txn,
                    &scope_for_body,
                    tenant,
                    draft,
                ))
                .await
                .map_err(|e| repo_failure(&e))?;
                price_repo::load_market_price(txn, &scope_for_body, tenant, price_id)
                    .await
                    .map_err(|e| repo_failure(&e))?
                    .ok_or_else(|| {
                        DomainError::Internal(format!(
                            "market price {price_id} is not readable in the transaction that \
                             wrote it"
                        ))
                    })
            })
        },
        |found: &MarketPriceRecord| {
            serde_json::to_value(MarketPriceView::from(found)).map_err(|e| {
                DomainError::Internal(format!("cannot render the created market price: {e}"))
            })
        },
    )
    .await
    .map_err(CanonicalError::from)?;

    Ok(match outcome {
        Guarded::Performed(found) => (
            StatusCode::CREATED,
            [
                (
                    LOCATION,
                    prices::price_location(plan_id, found.record.price_id),
                ),
                (ETAG, preconditions::etag(found.record.row_version)),
            ],
            Json(MarketPriceView::from(&found)),
        )
            .into_response(),
        Guarded::Replayed { status, body } => replayed_price(plan_id, status, &body)?,
    })
}

async fn list_market_prices(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path((plan_id, line_version_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<MarketPriceView>>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = read_scope(&enforcer, &ctx, plan_id.get()).await?;
    let view = line_view(&state, &scope, tenant, plan_id, line_version_id).await?;
    Ok(Json(view.prices))
}

// ---------------------------------------------------------------------------
// Rendering.
// ---------------------------------------------------------------------------

async fn line_view(
    state: &AuthoringState,
    scope: &AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
    line_version_id: Uuid,
) -> Result<ChargeLineView, CanonicalError> {
    let conn = state
        .db
        .conn()
        .map_err(|e| DomainError::Internal(format!("line read connection: {e}")))?;
    let line =
        charge_line::require_line_of_plan(&conn, scope, tenant, plan_id, line_version_id).await?;
    let prices = price_repo::list_prices_of_version(&conn, scope, tenant, line_version_id)
        .await
        .map_err(|e| repo_failure(&e))?;
    Ok(ChargeLineView::of(&line, &prices))
}

fn line_json(view: &ChargeLineView) -> Result<serde_json::Value, DomainError> {
    serde_json::to_value(view)
        .map_err(|e| DomainError::Internal(format!("cannot render the charge line: {e}")))
}

fn line_location(plan_id: PlanId, line_version_id: Uuid) -> String {
    format!("/bss-pricing/v1/plans/{plan_id}/charge-lines/{line_version_id}")
}

/// The plan revision's tag after a structural write moved it.
fn moved_tag(tag: PlanTag, plan_row_version: u64) -> String {
    preconditions::revision_etag(
        tag.revision,
        crate::domain::concurrency::RowVersion::new(plan_row_version),
    )
}

fn replayed_line(
    plan_id: PlanId,
    status: i32,
    body: &serde_json::Value,
) -> Result<Response, DomainError> {
    let status = super::replayed_status(CREATE_LINE_OPERATION, status)?;
    let location = body
        .get("line_version_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .map(|id| line_location(plan_id, id));
    Ok(match location {
        Some(location) => (status, [(LOCATION, location)], Json(body.clone())).into_response(),
        None => (status, Json(body.clone())).into_response(),
    })
}

fn replayed_price(
    plan_id: PlanId,
    status: i32,
    body: &serde_json::Value,
) -> Result<Response, DomainError> {
    let status = super::replayed_status(CREATE_MARKET_PRICE_OPERATION, status)?;
    let location = body
        .get("price_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .map(|id| prices::price_location(plan_id, id));
    Ok(match location {
        Some(location) => (status, [(LOCATION, location)], Json(body.clone())).into_response(),
        None => (status, Json(body.clone())).into_response(),
    })
}

#[cfg(test)]
#[path = "charge_lines_tests.rs"]
mod charge_lines_tests;
