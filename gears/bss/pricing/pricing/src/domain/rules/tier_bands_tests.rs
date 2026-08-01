//! Tests for the tier-band geometry and the usage evaluation policy.

use bss_fixtures::ModelKind;

use super::{BandGeometry, BandOrigin, BandTopOpen, UsageEvaluationPolicy};
use crate::domain::money::MinorAmount;
use crate::domain::price_row::{BillingGranularity, PriceRow, TierAggregationWindow, TierBand};
use crate::domain::rules::{
    EVAL_POLICY_MISSING, TIER_BAND_EMPTY, TIER_BAND_PRICE_INCREASE, TIER_BANDS_GAP,
    TIER_BANDS_OVERLAP, TIER_TOP_CLOSED,
};
use crate::domain::scope_key::ChargeKind;
use crate::domain::validation::{ValidationReport, ValidationRule};

fn minor(units: i64) -> MinorAmount {
    MinorAmount::new(units).expect("test amount is non-negative")
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

/// A `graduated` usage row with a well-formed descending two-band ladder.
fn tiered(bands: Vec<TierBand>) -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("api_calls".to_owned());
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = bands;
    row
}

fn descending_ladder() -> Vec<TierBand> {
    vec![
        TierBand::closed(0, 1_000, minor(10)),
        TierBand::closed(1_000, 10_000, minor(6)),
        TierBand::open(10_000, minor(3)),
    ]
}

#[test]
fn a_contiguous_ladder_passes() {
    let row = tiered(descending_ladder());

    let report = findings(&BandGeometry, &row);

    assert!(report.violations.is_empty());
    assert!(report.warnings.is_empty());
    assert!(findings(&BandOrigin, &row).violations.is_empty());
    assert!(findings(&BandTopOpen, &row).violations.is_empty());
}

#[test]
fn adjacent_bands_share_their_boundary_exactly_once() {
    // `[0, 100)` then `[100, open)`: quantity 100 belongs to the second band and
    // to no other. One unit either way is a gap or an overlap.
    let row = tiered(vec![
        TierBand::closed(0, 100, minor(10)),
        TierBand::open(100, minor(5)),
    ]);

    assert!(findings(&BandGeometry, &row).violations.is_empty());
}

#[test]
fn overlapping_bands_fail_publish() {
    let row = tiered(vec![
        TierBand::closed(0, 100, minor(10)),
        TierBand::open(50, minor(5)),
    ]);

    assert_eq!(
        codes(&findings(&BandGeometry, &row)),
        vec![TIER_BANDS_OVERLAP]
    );
}

#[test]
fn a_gap_between_bands_fails_publish() {
    let row = tiered(vec![
        TierBand::closed(0, 100, minor(10)),
        TierBand::open(150, minor(5)),
    ]);

    let report = findings(&BandGeometry, &row);

    assert_eq!(codes(&report), vec![TIER_BANDS_GAP]);
    assert!(
        report.violations[0].detail.contains("[100, 150)"),
        "the finding must name the unpriced stretch: {}",
        report.violations[0].detail
    );
}

#[test]
fn a_zero_width_band_fails_publish() {
    let row = tiered(vec![
        TierBand::closed(0, 100, minor(10)),
        TierBand::closed(100, 100, minor(8)),
        TierBand::open(100, minor(5)),
    ]);

    assert!(codes(&findings(&BandGeometry, &row)).contains(&TIER_BAND_EMPTY));
}

#[test]
fn a_band_below_an_open_top_authored_last_is_not_an_overlap() {
    // This test previously pinned the opposite reading, and it was wrong: the
    // set below is a contiguous ladder written bottom-last. The persisted band
    // table is keyed (price_id, from_qty) with no ordinal, so authoring order
    // does not survive the store, and a verdict that depended on it would
    // differ between the save-time pre-check and the identical re-run inside
    // the publish commit.
    let row = tiered(vec![
        TierBand::open(1_000, minor(5)),
        TierBand::closed(0, 1_000, minor(10)),
    ]);

    assert!(findings(&BandGeometry, &row).is_publishable());
}

#[test]
fn a_band_below_an_open_top_reads_as_an_overlap() {
    // An open top covers every quantity above its floor, so anything after it in
    // the set is inside it by construction.
    let row = tiered(vec![
        TierBand::open(0, minor(10)),
        TierBand::open(1_000, minor(5)),
    ]);

    assert!(codes(&findings(&BandGeometry, &row)).contains(&TIER_BANDS_OVERLAP));
}

#[test]
fn a_rising_ladder_warns_without_blocking() {
    // The non-volume-discount pattern is unusual, not illegal: congestion and
    // penalty tiers are real. A warning that could block would be a fail-closed
    // rule hiding behind a soft word.
    let row = tiered(vec![
        TierBand::closed(0, 1_000, minor(3)),
        TierBand::open(1_000, minor(9)),
    ]);

    let report = findings(&BandGeometry, &row);

    assert!(report.is_publishable());
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].code, TIER_BAND_PRICE_INCREASE);
}

#[test]
fn a_flat_step_in_the_ladder_is_not_a_rise() {
    let row = tiered(vec![
        TierBand::closed(0, 1_000, minor(5)),
        TierBand::open(1_000, minor(5)),
    ]);

    assert!(findings(&BandGeometry, &row).warnings.is_empty());
}

#[test]
fn a_closed_top_band_fails_publish() {
    // D-17: "price undefined above X" is never the commercial intent. Capping is
    // an entitlement quota, so any quantity stays rateable.
    let row = tiered(vec![
        TierBand::closed(0, 1_000, minor(10)),
        TierBand::closed(1_000, 10_000, minor(5)),
    ]);

    assert_eq!(codes(&findings(&BandTopOpen, &row)), vec![TIER_TOP_CLOSED]);
}

#[test]
fn an_open_top_band_passes() {
    let row = tiered(descending_ladder());

    assert!(findings(&BandTopOpen, &row).violations.is_empty());
}

#[test]
fn a_tiered_row_whose_first_band_misses_the_origin_fails_publish() {
    let row = tiered(vec![
        TierBand::closed(10, 1_000, minor(10)),
        TierBand::open(1_000, minor(5)),
    ]);

    assert_eq!(codes(&findings(&BandOrigin, &row)), vec![TIER_BANDS_GAP]);
}

#[test]
fn a_zero_priced_first_band_is_valid() {
    // The exception that matters: a free opening band is how "N included" is
    // authored by hand, and it is the shape the D-45 allowance compile projects.
    let row = tiered(vec![
        TierBand::closed(0, 100, minor(0)),
        TierBand::open(100, minor(5)),
    ]);

    assert!(findings(&BandOrigin, &row).violations.is_empty());
    assert!(findings(&BandGeometry, &row).is_publishable());
}

#[test]
fn a_tiered_row_with_no_bands_at_all_fails_publish() {
    let row = tiered(Vec::new());

    assert_eq!(codes(&findings(&BandOrigin, &row)), vec![TIER_BANDS_GAP]);
}

#[test]
fn the_origin_rule_judges_only_the_kinds_whose_money_is_in_bands() {
    // A `package` or `per_unit` row owes no origin band; carrying bands at all
    // is a placement fault the kind rules own.
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Package));
    row.package_size = Some(100);

    assert!(findings(&BandOrigin, &row).violations.is_empty());
}

#[test]
fn a_usage_row_without_billing_granularity_fails_publish() {
    let mut row = tiered(descending_ladder());
    row.billing_granularity = None;

    assert_eq!(
        codes(&findings(&UsageEvaluationPolicy, &row)),
        vec![EVAL_POLICY_MISSING]
    );
}

#[test]
fn a_tiered_usage_row_without_a_reset_window_fails_publish() {
    let mut row = tiered(descending_ladder());
    row.tier_aggregation_window = None;

    assert_eq!(
        codes(&findings(&UsageEvaluationPolicy, &row)),
        vec![EVAL_POLICY_MISSING]
    );
}

#[test]
fn a_non_usage_row_owes_no_evaluation_policy() {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(minor(100));

    assert!(findings(&UsageEvaluationPolicy, &row).violations.is_empty());
}

#[test]
fn a_package_rows_missing_window_is_reported_once_by_the_package_rule() {
    // `inst-pk-window` owns the `package` case, so this rule must stay quiet
    // about it: one missing field has to read as one fault.
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Package));
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row.package_size = Some(100);
    row.package_price_minor = Some(minor(500));

    assert!(findings(&UsageEvaluationPolicy, &row).violations.is_empty());
}

#[test]
fn a_band_set_authored_out_of_order_is_judged_on_its_geometry() {
    // The persisted band table is keyed (price_id, from_qty) and carries no
    // ordinal, so authoring order does not survive the store. Judging on it
    // would let this row pass at save and fail the identical re-validation
    // inside the publish commit — a verdict that depends on how the author
    // happened to type it.
    let row = tiered(vec![
        TierBand::open(1_000, minor(6)),
        TierBand::closed(0, 1_000, minor(10)),
    ]);

    assert!(findings(&BandGeometry, &row).is_publishable());
}

#[test]
fn an_out_of_order_set_with_a_real_gap_still_fails() {
    // Sorting must not launder a broken set into a whole one.
    let row = tiered(vec![
        TierBand::open(1_500, minor(6)),
        TierBand::closed(0, 1_000, minor(10)),
    ]);

    let report = findings(&BandGeometry, &row);

    assert!(!report.is_publishable());
    assert!(codes(&report).contains(&TIER_BANDS_GAP));
}

#[test]
fn a_rise_out_of_a_free_opening_band_does_not_warn() {
    // "N included, then priced" is the canonical allowance shape and the one the
    // D-45 compile projects. Warning on it would fire on nearly every allowance
    // row, and a channel that is noisy by default is a channel authors stop
    // reading.
    let row = tiered(vec![
        TierBand::closed(0, 100, minor(0)),
        TierBand::open(100, minor(5)),
    ]);

    assert!(findings(&BandGeometry, &row).warnings.is_empty());
}

#[test]
fn a_rise_out_of_a_priced_band_still_warns() {
    // The exemption is narrow on purpose: it is about a free opening band, not
    // about every ladder whose first step is cheap.
    let row = tiered(vec![
        TierBand::closed(0, 100, minor(1)),
        TierBand::open(100, minor(5)),
    ]);

    let report = findings(&BandGeometry, &row);

    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].code, TIER_BAND_PRICE_INCREASE);
}
