//! Shared charge-line structure: every non-monetary field of a
//! [`PriceRow`](crate::domain::price_row::PriceRow).
//!
//! A charge line version authors one [`ChargeStructure`] plus the billing-timing
//! and proration contract that must not vary by currency. Market money lives on
//! [`crate::domain::market_price::MarketPriceTerms`]. [`PriceRow`] remains the
//! internal resolved join; persistence and REST are unchanged in this change.

use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::contracts::ProrationContract;
use crate::domain::price_row::{
    AggregationFunction, AggregationGranularity, BillingGranularity, IncludedAllowance,
    MinQtyUsageFallback, ModelKind, QuantitySource, ReservationFlavor, TierAggregationWindow,
    TierQualificationWindow, model_kind_wire,
};
use crate::domain::scope_key::{ChargeKind, ChargeLineScopeKey, SkuId};

/// Shared, non-monetary charge shape.
///
/// Every [`crate::domain::price_row::PriceRow`] field that is not a market money
/// operand is here. **The ladder is not**: a band's bounds and its rate are one
/// market's answer to *how much*, and live together on
/// [`crate::domain::market_price::MarketPriceTerms::tiers`]. What stays shared is
/// `model_kind` — a market cannot be `graduated` where its sibling is `volume`.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChargeStructure {
    /// See [`crate::domain::price_row::PriceRow::invoice_line_template`].
    pub invoice_line_template: Option<String>,
    /// See [`crate::domain::price_row::PriceRow::gl_code_ref`].
    pub gl_code_ref: Option<String>,
    /// See [`crate::domain::price_row::PriceRow::charge_kind`].
    pub charge_kind: ChargeKind,
    /// See [`crate::domain::price_row::PriceRow::model_kind`].
    pub model_kind: Option<ModelKind>,
    /// See [`crate::domain::price_row::PriceRow::package_size`].
    pub package_size: Option<u64>,
    /// See [`crate::domain::price_row::PriceRow::quantity_source`].
    pub quantity_source: Option<QuantitySource>,
    /// See [`crate::domain::price_row::PriceRow::manual_quantity`].
    pub manual_quantity: Option<u64>,
    /// See [`crate::domain::price_row::PriceRow::sku_id`].
    pub sku_id: SkuId,
    /// See [`crate::domain::price_row::PriceRow::meter`].
    pub meter: Option<String>,
    /// See [`crate::domain::price_row::PriceRow::dimension_key`].
    pub dimension_key: String,
    /// See [`crate::domain::price_row::PriceRow::billing_granularity`].
    pub billing_granularity: Option<BillingGranularity>,
    /// See [`crate::domain::price_row::PriceRow::tier_aggregation_window`].
    pub tier_aggregation_window: Option<TierAggregationWindow>,
    /// See [`crate::domain::price_row::PriceRow::tier_qualification_window`].
    pub tier_qualification_window: Option<TierQualificationWindow>,
    /// See [`crate::domain::price_row::PriceRow::aggregation_function`].
    pub aggregation_function: Option<AggregationFunction>,
    /// See [`crate::domain::price_row::PriceRow::aggregation_granularity`].
    pub aggregation_granularity: Option<AggregationGranularity>,
    /// See [`crate::domain::price_row::PriceRow::max_hold_granules`].
    pub max_hold_granules: Option<u64>,
    /// See [`crate::domain::price_row::PriceRow::included_allowance`].
    pub included_allowance: Option<IncludedAllowance>,
    /// See [`crate::domain::price_row::PriceRow::reservation_flavor`].
    pub reservation_flavor: Option<ReservationFlavor>,
    /// See [`crate::domain::price_row::PriceRow::min_qty_purchase`].
    pub min_qty_purchase: Option<u64>,
    /// See [`crate::domain::price_row::PriceRow::min_qty_usage`].
    pub min_qty_usage: Option<u64>,
    /// See [`crate::domain::price_row::PriceRow::min_qty_usage_fallback`].
    pub min_qty_usage_fallback: Option<MinQtyUsageFallback>,
    /// See [`crate::domain::price_row::PriceRow::discount_ref`].
    pub discount_ref: Option<String>,
}

impl ChargeStructure {
    /// An otherwise-empty structure on `charge_kind` carrying `model_kind`.
    ///
    /// Matches [`crate::domain::price_row::PriceRow::new`]: optional fields start
    /// absent, `dimension_key` is the empty-tuple sentinel, and `sku_id` is the
    /// nil placeholder until the canonical line binds it.
    #[must_use]
    pub fn new(charge_kind: ChargeKind, model_kind: Option<ModelKind>) -> Self {
        Self {
            invoice_line_template: None,
            gl_code_ref: None,
            charge_kind,
            model_kind,
            package_size: None,
            quantity_source: None,
            manual_quantity: None,
            sku_id: SkuId::new(Uuid::nil()),
            meter: None,
            dimension_key: String::new(),
            billing_granularity: None,
            tier_aggregation_window: None,
            tier_qualification_window: None,
            aggregation_function: None,
            aggregation_granularity: None,
            max_hold_granules: None,
            included_allowance: None,
            reservation_flavor: None,
            min_qty_purchase: None,
            min_qty_usage: None,
            min_qty_usage_fallback: None,
            discount_ref: None,
        }
    }

    /// How a finding locates this structure for the author.
    #[must_use]
    pub fn subject(&self) -> String {
        let kind = self.model_kind.map_or("(no model kind)", model_kind_wire);
        format!("{}/{kind}", self.charge_kind)
    }
}

/// One version of a logical charge line: identity, line key, shared structure,
/// and the timing contract that every market of the line must share.
///
/// Revision, lifecycle and audit stay on the existing owning records. Do not
/// treat these fields as a second authority for those facts.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChargeLineVersion {
    /// Stable identity of the logical charge line.
    pub charge_line_id: Uuid,
    /// Identity of this structure version.
    pub line_version_id: Uuid,
    /// The eight logical axes this line occupies.
    pub scope_key: ChargeLineScopeKey,
    /// Shared non-monetary shape.
    pub structure: ChargeStructure,
    /// `advance` | `arrears`. Authored once on the line version so two currencies
    /// of the same line cannot disagree. [`crate::domain::price_record::PriceRecord`]
    /// still *presents* the resolved value.
    pub billing_timing: Option<String>,
    /// Shared proration inputs. Same ownership as [`Self::billing_timing`].
    pub proration_contract: Option<ProrationContract>,
}

#[cfg(test)]
#[path = "charge_line_tests.rs"]
mod charge_line_tests;
