//! Market-specific monetary terms of a charge, and the converter that joins
//! them back onto shared structure.
//!
//! [`PriceRow`] remains the internal resolved value. [`split_row`] /
//! [`resolve_row`] are converters for that value and for fixture builders —
//! never a public write adapter over the old joined row.

use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::charge_line::ChargeStructure;
use crate::domain::error::DomainError;
use crate::domain::money::{MinorAmount, RateMinor};
use crate::domain::price_row::{PriceRow, TierBand};
use crate::domain::rules::row_local_rules;
use crate::domain::scope_key::MarketPriceScopeKey;

/// Market money operands of one price.
///
/// Tax display, rounding, grandfathering, supersession and the currency/region
/// axes stay on the records and keys that already own them. This value is only
/// the amounts and rates.
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarketPriceTerms {
    /// See [`PriceRow::amount_minor`].
    pub amount_minor: Option<MinorAmount>,
    /// See [`PriceRow::unit_rate`].
    pub unit_rate: Option<RateMinor>,
    /// This market's ladder: each band's bounds beside the rate that prices it.
    /// Empty on a non-tiered line. Two markets of one line may differ in the
    /// number of bands, their break-points and their rates — only the line's
    /// `model_kind` is shared. See [`PriceRow::bands`].
    pub tiers: Vec<TierBand>,
    /// See [`PriceRow::package_price_minor`].
    pub package_price_minor: Option<MinorAmount>,
    /// See [`PriceRow::reserved_rate`].
    pub reserved_rate: Option<RateMinor>,
}

/// One market price version: identity, the line version it prices, the market
/// key, and the money operands.
///
/// Revision, lifecycle and audit stay on the existing owning records.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketPriceVersion {
    /// Stable identity of this market price across versions.
    pub market_price_id: Uuid,
    /// The price-row id the store already uses (`PriceRecord::price_id`).
    pub price_id: Uuid,
    /// The [`crate::domain::charge_line::ChargeLineVersion::line_version_id`]
    /// this money is bound to.
    pub line_version_id: Uuid,
    /// Ten-axis market selection, including `currency` and `region`.
    pub scope_key: MarketPriceScopeKey,
    /// Market money operands.
    pub money: MarketPriceTerms,
}

/// Split a resolved [`PriceRow`] into shared structure and market money.
///
/// Exhaustive on [`PriceRow`]: a new field is a compile-time decision about
/// which half owns it. This is not a write adapter.
#[must_use]
pub fn split_row(row: PriceRow) -> (ChargeStructure, MarketPriceTerms) {
    let PriceRow {
        invoice_line_template,
        gl_code_ref,
        charge_kind,
        model_kind,
        amount_minor,
        unit_rate,
        bands,
        package_size,
        package_price_minor,
        quantity_source,
        manual_quantity,
        sku_id,
        meter,
        dimension_key,
        billing_granularity,
        tier_aggregation_window,
        tier_qualification_window,
        aggregation_function,
        aggregation_granularity,
        max_hold_granules,
        included_allowance,
        reserved_rate,
        reservation_flavor,
        min_qty_purchase,
        min_qty_usage,
        min_qty_usage_fallback,
        discount_ref,
    } = row;

    (
        ChargeStructure {
            invoice_line_template,
            gl_code_ref,
            charge_kind,
            model_kind,
            package_size,
            quantity_source,
            manual_quantity,
            sku_id,
            meter,
            dimension_key,
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
        },
        MarketPriceTerms {
            amount_minor,
            unit_rate,
            tiers: bands,
            package_price_minor,
            reserved_rate,
        },
    )
}

/// Join shared structure and market money back into a resolved [`PriceRow`].
///
/// The market's ladder arrives whole — a rate sits beside the bound it prices —
/// so there is no count to reconcile and no partial row to refuse. The assembled
/// row then runs the existing row-local model/operand rules: a draft may still
/// be stored incomplete as a [`PriceRow`], but this converter refuses a join
/// that those rules already reject (a `flat` line with a ladder, a competing
/// amount column, and so on). Publication remains the authority that rejects an
/// incomplete operand set on a real write.
///
/// # Errors
///
/// [`DomainError::ValidationFailed`] carrying the existing row-local codes when
/// the assembled row is not publishable.
pub fn resolve_row(
    structure: &ChargeStructure,
    money: &MarketPriceTerms,
) -> Result<PriceRow, DomainError> {
    let ChargeStructure {
        invoice_line_template,
        gl_code_ref,
        charge_kind,
        model_kind,
        package_size,
        quantity_source,
        manual_quantity,
        sku_id,
        meter,
        dimension_key,
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
    } = structure;
    let MarketPriceTerms {
        amount_minor,
        unit_rate,
        tiers,
        package_price_minor,
        reserved_rate,
    } = money;

    let row = PriceRow {
        invoice_line_template: invoice_line_template.clone(),
        gl_code_ref: gl_code_ref.clone(),
        charge_kind: *charge_kind,
        model_kind: *model_kind,
        amount_minor: *amount_minor,
        unit_rate: *unit_rate,
        bands: tiers.clone(),
        package_size: *package_size,
        package_price_minor: *package_price_minor,
        quantity_source: *quantity_source,
        manual_quantity: *manual_quantity,
        sku_id: *sku_id,
        meter: meter.clone(),
        dimension_key: dimension_key.clone(),
        billing_granularity: *billing_granularity,
        tier_aggregation_window: *tier_aggregation_window,
        tier_qualification_window: *tier_qualification_window,
        aggregation_function: *aggregation_function,
        aggregation_granularity: *aggregation_granularity,
        max_hold_granules: *max_hold_granules,
        included_allowance: *included_allowance,
        reserved_rate: *reserved_rate,
        reservation_flavor: *reservation_flavor,
        min_qty_purchase: *min_qty_purchase,
        min_qty_usage: *min_qty_usage,
        min_qty_usage_fallback: *min_qty_usage_fallback,
        discount_ref: discount_ref.clone(),
    };

    let report = row_local_rules().run(&row);
    if report.is_publishable() {
        Ok(row)
    } else {
        Err(DomainError::ValidationFailed(report))
    }
}

#[cfg(test)]
#[path = "market_price_tests.rs"]
mod market_price_tests;
