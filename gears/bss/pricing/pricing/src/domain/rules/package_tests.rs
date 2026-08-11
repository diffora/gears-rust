//! Tests for the package (block) pricing rules.

use bss_fixtures::ModelKind;

use super::{PackageFields, PackageWindow};
use crate::domain::money::{MinorAmount, RateMinor};
use crate::domain::price_row::{BillingGranularity, PriceRow, TierAggregationWindow, TierBand};
use crate::domain::rules::{EVAL_POLICY_MISSING, PACKAGE_FIELDS_INVALID};
use crate::domain::scope_key::ChargeKind;
use crate::domain::validation::{ValidationReport, ValidationRule};

fn minor(units: i64) -> MinorAmount {
    MinorAmount::new(units).expect("test amount is non-negative")
}

/// A band rate, stated in whole minor units so these cases read as they
/// always did (D-311). The stored scale is 10^-9 of one.
fn rate(minor_units: i64) -> RateMinor {
    RateMinor::from_minor_units(minor_units).expect("test rate is non-negative")
}

fn findings(rule: &impl ValidationRule<PriceRow>, row: &PriceRow) -> ValidationReport {
    let mut report = ValidationReport::default();
    rule.evaluate(row, &mut report);
    report
}

fn codes(report: &ValidationReport) -> Vec<&str> {
    report
        .violations
        .iter()
        .map(|violation| violation.code.as_str())
        .collect()
}

/// A publishable `package` usage row: 100 SMS per block, 5.00 per block,
/// accumulating over the invoice period.
fn package_usage() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Package));
    row.meter = Some("sms".to_owned());
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
    row.package_size = Some(100);
    row.package_price_minor = Some(minor(500));
    row
}

#[test]
fn a_well_formed_package_row_passes() {
    let row = package_usage();

    assert!(findings(&PackageFields, &row).violations.is_empty());
    assert!(findings(&PackageWindow, &row).violations.is_empty());
}

#[test]
fn a_zero_package_size_fails_publish() {
    let mut row = package_usage();
    row.package_size = Some(0);

    assert_eq!(
        codes(&findings(&PackageFields, &row)),
        vec![PACKAGE_FIELDS_INVALID]
    );
}

#[test]
fn a_package_row_carrying_tier_bands_fails_publish() {
    // Block pricing and tier bands are two formulas for one price; a row holding
    // both leaves Tariffs to choose.
    let mut row = package_usage();
    row.bands = vec![TierBand::open(0, rate(5))];

    assert_eq!(
        codes(&findings(&PackageFields, &row)),
        vec![PACKAGE_FIELDS_INVALID]
    );
}

#[test]
fn block_fields_on_a_row_that_does_not_price_in_blocks_fail_publish() {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.package_size = Some(100);

    assert_eq!(
        codes(&findings(&PackageFields, &row)),
        vec![PACKAGE_FIELDS_INVALID]
    );
}

#[test]
fn a_package_row_without_an_accumulation_window_fails_publish() {
    let mut row = package_usage();
    row.tier_aggregation_window = None;

    assert_eq!(
        codes(&findings(&PackageWindow, &row)),
        vec![EVAL_POLICY_MISSING]
    );
}

#[test]
fn billing_granularity_does_not_substitute_for_the_accumulation_window() {
    // The subtlety D-58 exists for: `billingGranularity` quantizes the quantity,
    // it does not bound a period, and block math is non-linear in the period.
    // 150 units is 2 blocks per invoice period and 30 blocks under a daily fold.
    let mut row = package_usage();
    row.tier_aggregation_window = None;
    row.billing_granularity = Some(BillingGranularity::PerDay);

    assert_eq!(
        codes(&findings(&PackageWindow, &row)),
        vec![EVAL_POLICY_MISSING]
    );
}

#[test]
fn the_window_rule_judges_only_package_rows() {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.tier_aggregation_window = None;

    assert!(findings(&PackageWindow, &row).violations.is_empty());
}
