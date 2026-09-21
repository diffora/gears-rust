//! Charge-shape rules: plan behaviour is derived from actual phase charge lines.
//!
//! Interval, purchase-quantity and availability rules that used to live beside
//! cycle dispatch stay here; they do not read a plan type.

use std::collections::{BTreeMap, BTreeSet};

use toolkit_macros::domain_model;

use crate::domain::charge_line::{ChargeLineVersion, ChargeStructure};
use crate::domain::currency_binding;
use crate::domain::instant::format_rfc3339;
use crate::domain::market_price::{MarketPriceTerms, MarketPriceVersion};
use crate::domain::money::{CurrencyCode, RateMinor};
use crate::domain::plan_rules::{
    AVAILABLE_FROM_IN_PAST, INVALID_CUSTOM_INTERVAL, LINE_MARKET_PRICE_MISSING,
    PHASE_CHARGE_LINES_EMPTY, PHASE_USAGE_INCOMPATIBLE, PURCHASE_QTY_RANGE_INVALID,
    RECURRING_FREQUENCY_REQUIRED,
};
use crate::domain::plan_shape::{CustomIntervalUnit, Frequency, PlanShape};
use crate::domain::price_row::{PriceRow, unit_determining_mismatch};
use crate::domain::scope_key::{
    ChargeKind, ChargeLineScopeKey, Cohort, PhaseId, PriceEligibility, PriceOverlay, Region, SkuId,
};
use crate::domain::validation::{ValidationReport, ValidationRule};

/// Distinct charge kinds present on the effective lines.
#[must_use]
pub fn charge_kinds(lines: &[ChargeLineVersion]) -> BTreeSet<ChargeKind> {
    lines
        .iter()
        .map(|line| line.scope_key.charge_kind())
        .collect()
}

/// Frequency is required only when a recurring line exists.
#[must_use]
pub fn requires_frequency(lines: &[ChargeLineVersion]) -> bool {
    lines
        .iter()
        .any(|line| line.scope_key.charge_kind() == ChargeKind::Recurring)
}

/// `inst-cs-customfreq` — a custom interval is positive and within its unit's
/// cap. Unrelated to plan-type dispatch; retained from the cycle-shape module.
#[domain_model]
#[derive(Clone, Copy, Debug)]
pub struct CustomIntervalBounds {
    max_custom_interval_days: u32,
    max_custom_interval_months: u32,
}

impl CustomIntervalBounds {
    /// The rule bound to one tenant's caps.
    #[must_use]
    pub const fn new(max_custom_interval_days: u32, max_custom_interval_months: u32) -> Self {
        Self {
            max_custom_interval_days,
            max_custom_interval_months,
        }
    }

    const fn cap_for(self, unit: CustomIntervalUnit) -> u32 {
        match unit {
            CustomIntervalUnit::Days => self.max_custom_interval_days,
            CustomIntervalUnit::Months => self.max_custom_interval_months,
        }
    }
}

impl ValidationRule<PlanShape> for CustomIntervalBounds {
    fn name(&self) -> &'static str {
        "inst-cs-customfreq"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        let Some(Frequency::CustomEveryN { n, unit }) = subject.frequency else {
            return;
        };
        let cap = self.cap_for(unit);
        if n == 0 {
            report.violate(
                INVALID_CUSTOM_INTERVAL,
                subject.subject(),
                format!(
                    "customEveryN {unit}({n}) counts no {unit} at all: a custom frequency \
                     recurs every n of its unit, and n must be positive"
                ),
            );
            return;
        }
        if n > cap {
            report.violate(
                INVALID_CUSTOM_INTERVAL,
                subject.subject(),
                format!(
                    "customEveryN {unit}({n}) is above the configured cap of {cap} {unit}; the \
                     interval is rejected here and never clamped, because a clamped interval is \
                     a billing period the operator did not author and would never see"
                ),
            );
        }
    }
}

/// Ordinary phases must carry logical charge lines at publish.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct PhaseChargeLinesPresent;

impl ValidationRule<PlanShape> for PhaseChargeLinesPresent {
    fn name(&self) -> &'static str {
        "inst-cs-charge-lines"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        let lines = effective_lines(subject);
        for phase in subject.phases.in_ordinal_order() {
            let present = lines
                .iter()
                .any(|line| line.scope_key.phase() == phase.phase_id);
            if present {
                continue;
            }
            report.violate(
                PHASE_CHARGE_LINES_EMPTY,
                phase_subject(subject, phase.phase_id),
                "this ordinary phase has no logical charge lines; emptiness is counted on \
                 lines, not on flattened currency rows. An unfinished draft with no phases \
                 yet is not this fault"
                    .to_owned(),
            );
        }
    }
}

/// Frequency is required when effective recurring lines exist.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct RecurringFrequencyRequired;

impl ValidationRule<PlanShape> for RecurringFrequencyRequired {
    fn name(&self) -> &'static str {
        "inst-cs-frequency"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        if !requires_frequency(&effective_lines(subject)) {
            return;
        }
        if subject.frequency.is_some() {
            return;
        }
        report.violate(
            RECURRING_FREQUENCY_REQUIRED,
            subject.subject(),
            "frequency is unset while this revision carries a recurring charge line: a plan \
             that recurs and does not say when it charges has told a subscriber nothing about \
             the interval it is billed over"
                .to_owned(),
        );
    }
}

/// Every applicable line owes a monetary binding in every sold market.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct LineMarketPricePresent;

impl ValidationRule<PlanShape> for LineMarketPricePresent {
    fn name(&self) -> &'static str {
        "inst-cs-market-price"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        let lines = effective_lines(subject);
        let variants = effective_variants(subject);
        let markets = sold_markets(subject, &variants);
        for line in &lines {
            if line.scope_key.price_eligibility() == PriceEligibility::ExistingGrandfathered {
                continue;
            }
            if !subject
                .phases
                .in_ordinal_order()
                .iter()
                .any(|phase| phase.phase_id == line.scope_key.phase())
            {
                continue;
            }
            for (currency, region) in &markets {
                if has_binding(&variants, line, currency, region) {
                    continue;
                }
                report.violate(
                    LINE_MARKET_PRICE_MISSING,
                    format!(
                        "{}|{}|{currency}|{region}",
                        phase_subject(subject, line.scope_key.phase()),
                        line_label(line)
                    ),
                    format!(
                        "line {} has no monetary binding in {currency}/{region}, a market this \
                         revision sells in. An explicit zero operand counts; an absent operand \
                         or absent variant does not",
                        line_label(line)
                    ),
                );
            }
        }
    }
}

/// Per-phase usage lookup: no inheritance, consecutive counter-compatibility.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct PhaseUsageCompatible;

impl ValidationRule<PlanShape> for PhaseUsageCompatible {
    fn name(&self) -> &'static str {
        "inst-ph-usage-compatible"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        let lines = effective_lines(subject);
        let phases = subject.phases.in_ordinal_order();
        for window in phases.windows(2) {
            let prev = window[0];
            let next = window[1];
            let prev_usage = usage_by_identity(&lines, prev.phase_id);
            let next_usage = usage_by_identity(&lines, next.phase_id);
            let mut keys = BTreeSet::new();
            keys.extend(prev_usage.keys().copied());
            keys.extend(next_usage.keys().copied());
            for key in keys {
                match (prev_usage.get(&key), next_usage.get(&key)) {
                    (Some(left), Some(right)) => {
                        let changed = unit_determining_mismatch(
                            &assembled_row(&left.structure),
                            &assembled_row(&right.structure),
                        );
                        if changed.is_empty() {
                            continue;
                        }
                        report.violate(
                            PHASE_USAGE_INCOMPATIBLE,
                            format!(
                                "{}/{}",
                                phase_subject(subject, next.phase_id),
                                line_label(right)
                            ),
                            format!(
                                "this phase-local usage definition changes {} against the \
                                 previous phase's definition of the same applicable line; the \
                                 tier counter is phase-blind, so conversion never resets it. \
                                 Price amount, phase id and line id alone must not fail \
                                 continuation",
                                changed.join(", ")
                            ),
                        );
                    }
                    (Some(only), None) => {
                        report.violate(
                            PHASE_USAGE_INCOMPATIBLE,
                            format!(
                                "{}/{}",
                                phase_subject(subject, next.phase_id),
                                line_label(only)
                            ),
                            format!(
                                "usage line {} is present on phase {} and missing on {}; \
                                 lookup is phase-local and must not select another phase's \
                                 tariff",
                                line_label(only),
                                prev.phase_id,
                                next.phase_id
                            ),
                        );
                    }
                    (None, Some(only)) => {
                        report.violate(
                            PHASE_USAGE_INCOMPATIBLE,
                            format!(
                                "{}/{}",
                                phase_subject(subject, prev.phase_id),
                                line_label(only)
                            ),
                            format!(
                                "usage line {} is present on phase {} and missing on {}; \
                                 lookup is phase-local and must not select another phase's \
                                 tariff",
                                line_label(only),
                                next.phase_id,
                                prev.phase_id
                            ),
                        );
                    }
                    (None, None) => {}
                }
            }
        }
    }
}

/// `inst-cs-onetime` — the purchase-quantity window admits a quantity.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct PurchaseQtyRange;

impl ValidationRule<PlanShape> for PurchaseQtyRange {
    fn name(&self) -> &'static str {
        "inst-cs-onetime"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        let (Some(min), Some(max)) = (subject.purchase_min_qty, subject.purchase_max_qty) else {
            return;
        };
        if min <= max {
            return;
        }
        report.violate(
            PURCHASE_QTY_RANGE_INVALID,
            subject.subject(),
            format!(
                "purchase_min_qty {min} is above purchase_max_qty {max}: the window admits no \
                 quantity, so the plan is publishable and unbuyable"
            ),
        );
    }
}

/// `inst-cs-availability` — a past `availableFrom` is backdating, unless it is
/// the date the plan already published with.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct AvailableFromNotBackdated;

impl ValidationRule<PlanShape> for AvailableFromNotBackdated {
    fn name(&self) -> &'static str {
        "inst-cs-availability"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        let Some(available_from) = subject.available_from else {
            return;
        };
        if available_from >= subject.evaluated_at {
            return;
        }
        let published = subject
            .baseline
            .as_ref()
            .and_then(|baseline| baseline.available_from);
        if published == Some(available_from) {
            return;
        }
        report.violate(
            AVAILABLE_FROM_IN_PAST,
            subject.subject(),
            format!(
                "availableFrom {} is newly set or changed and is before the evaluation instant \
                 {}: a plan may not be made available in the past. The historical-import path is \
                 the only sanctioned backdating; re-publishing the date this plan already \
                 published with is not backdating and is allowed",
                format_rfc3339(available_from),
                format_rfc3339(subject.evaluated_at)
            ),
        );
    }
}

fn effective_lines(subject: &PlanShape) -> Vec<ChargeLineVersion> {
    if !subject.charge_lines.is_empty() {
        return subject.charge_lines.clone();
    }
    let mut by_line: BTreeMap<ChargeLineScopeKey, ChargeLineVersion> = BTreeMap::new();
    for record in &subject.rows {
        by_line
            .entry(record.scope_key.line().clone())
            .or_insert_with(|| {
                let (structure, _) = crate::domain::market_price::split_row(record.row.clone());
                ChargeLineVersion {
                    charge_line_id: record.price_id,
                    line_version_id: record.price_id,
                    scope_key: record.scope_key.line().clone(),
                    structure,
                    billing_timing: record.billing_timing.clone(),
                    proration_contract: record.proration_contract,
                }
            });
    }
    by_line.into_values().collect()
}

fn effective_variants(subject: &PlanShape) -> Vec<MarketPriceVersion> {
    if !subject.market_prices.is_empty() {
        return subject.market_prices.clone();
    }
    subject
        .rows
        .iter()
        .map(|record| {
            let (_, money) = crate::domain::market_price::split_row(record.row.clone());
            MarketPriceVersion {
                market_price_id: record.price_id,
                price_id: record.price_id,
                line_version_id: record.price_id,
                scope_key: record.scope_key.clone(),
                money,
            }
        })
        .collect()
}

fn sold_markets(
    subject: &PlanShape,
    variants: &[MarketPriceVersion],
) -> BTreeSet<(CurrencyCode, Region)> {
    if !subject.rows.is_empty() {
        return currency_binding::sold_markets(subject);
    }
    variants
        .iter()
        .filter(|price| {
            price.scope_key.price_eligibility() != PriceEligibility::ExistingGrandfathered
        })
        .map(|price| {
            (
                price.scope_key.currency().clone(),
                price.scope_key.region().clone(),
            )
        })
        .collect()
}

fn has_money(money: &MarketPriceTerms) -> bool {
    money.amount_minor.is_some()
        || money.unit_rate.is_some()
        || !money.tiers.is_empty()
        || money.package_price_minor.is_some()
        || money.reserved_rate.is_some()
}

fn has_binding(
    variants: &[MarketPriceVersion],
    line: &ChargeLineVersion,
    currency: &CurrencyCode,
    region: &Region,
) -> bool {
    variants.iter().any(|price| {
        same_line_scope(price, line)
            && price.scope_key.currency() == currency
            && price.scope_key.region() == region
            && has_money(&price.money)
    })
}

fn same_line_scope(price: &MarketPriceVersion, line: &ChargeLineVersion) -> bool {
    price.scope_key.price_eligibility() == line.scope_key.price_eligibility()
        && price.scope_key.cohort() == line.scope_key.cohort()
        && price.scope_key.price_overlay() == line.scope_key.price_overlay()
        && (price.line_version_id == line.line_version_id
            || price.scope_key.line() == &line.scope_key)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct UsageIdentity<'a> {
    sku_id: SkuId,
    dimension_key: &'a str,
    meter: Option<&'a str>,
    price_eligibility: PriceEligibility,
    cohort: Cohort,
    overlay: PriceOverlay,
}

fn usage_by_identity(
    lines: &[ChargeLineVersion],
    phase: PhaseId,
) -> BTreeMap<UsageIdentity<'_>, &ChargeLineVersion> {
    let mut map = BTreeMap::new();
    for line in lines {
        if line.scope_key.phase() != phase {
            continue;
        }
        if line.scope_key.charge_kind() != ChargeKind::Usage {
            continue;
        }
        let key = UsageIdentity {
            sku_id: line.scope_key.sku_id(),
            dimension_key: line.structure.dimension_key.as_str(),
            meter: line.structure.meter.as_deref(),
            price_eligibility: line.scope_key.price_eligibility(),
            cohort: line.scope_key.cohort(),
            overlay: line.scope_key.price_overlay(),
        };
        map.entry(key).or_insert(line);
    }
    map
}

/// Inverse of [`crate::domain::market_price::split_row`]'s structure half.
///
/// Dummy money is enough for [`unit_determining_mismatch`]: amounts are not on
/// that list, and a zero `unit_rate` lets `presented_model_kind` compile an
/// allowance ladder so quantity on a compatible ladder stays a price lever
/// (D-317).
fn assembled_row(structure: &ChargeStructure) -> PriceRow {
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

    PriceRow {
        invoice_line_template: invoice_line_template.clone(),
        gl_code_ref: gl_code_ref.clone(),
        charge_kind: *charge_kind,
        model_kind: *model_kind,
        amount_minor: None,
        unit_rate: Some(RateMinor::ZERO),
        // A ladder is a market's, and none of D-82's unit-determining fields
        // reads one: break-points are a lever a monetary successor may move.
        bands: Vec::new(),
        package_size: *package_size,
        package_price_minor: None,
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
        reserved_rate: None,
        reservation_flavor: *reservation_flavor,
        min_qty_purchase: *min_qty_purchase,
        min_qty_usage: *min_qty_usage,
        min_qty_usage_fallback: *min_qty_usage_fallback,
        discount_ref: discount_ref.clone(),
    }
}

fn phase_subject(shape: &PlanShape, phase_id: PhaseId) -> String {
    format!("{}/phase/{phase_id}", shape.subject())
}

fn line_label(line: &ChargeLineVersion) -> String {
    format!(
        "{}|{}|{}|{}",
        line.scope_key.charge_kind(),
        line.scope_key.sku_id().as_uuid(),
        line.scope_key.dimension_key().as_str(),
        line.line_version_id
    )
}

#[cfg(test)]
#[path = "charge_shape_tests.rs"]
mod charge_shape_tests;
