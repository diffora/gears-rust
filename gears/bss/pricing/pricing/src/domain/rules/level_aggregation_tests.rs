//! Tests for the D-44 level-aggregation authoring rules.

use bss_fixtures::ModelKind;

use super::{LevelFields, LevelGranularityPairing, LevelMaxHold};
use crate::domain::money::MinorAmount;
use crate::domain::price_row::{
    AggregationFunction, AggregationGranularity, BillingGranularity, PriceRow,
    TierAggregationWindow,
};
use crate::domain::rules::{LEVEL_FIELDS_INVALID, LEVEL_GRANULARITY_MISMATCH};
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

/// A plain `sum` usage row — the launch default shape.
fn sum_usage() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    row.amount_minor = Some(minor(3));
    row.meter = Some("api_calls".to_owned());
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row
}

/// A publishable level row: cloudlet peak per hour.
fn peak_hourly() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("cloudlet".to_owned());
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.aggregation_function = Some(AggregationFunction::Peak);
    row.aggregation_granularity = Some(AggregationGranularity::Hour);
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.max_hold_granules = Some(2);
    row
}

#[test]
fn a_sum_row_owes_no_level_fields() {
    let row = sum_usage();

    assert!(findings(&LevelFields, &row).violations.is_empty());
    assert!(
        findings(&LevelGranularityPairing, &row)
            .violations
            .is_empty()
    );
    assert!(findings(&LevelMaxHold, &row).violations.is_empty());
}

#[test]
fn a_well_formed_level_row_passes_every_level_rule() {
    let row = peak_hourly();

    assert!(findings(&LevelFields, &row).violations.is_empty());
    assert!(
        findings(&LevelGranularityPairing, &row)
            .violations
            .is_empty()
    );
    assert!(findings(&LevelMaxHold, &row).violations.is_empty());
}

#[test]
fn level_fields_on_a_non_usage_row_fail_publish() {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(minor(100));
    row.aggregation_function = Some(AggregationFunction::TimeWeighted);

    assert_eq!(
        codes(&findings(&LevelFields, &row)),
        vec![LEVEL_FIELDS_INVALID]
    );
}

#[test]
fn aggregation_granularity_on_a_sum_row_fails_publish() {
    // Forbidden, not ignored: there is no granule fold to parameterize, so a
    // frozen granularity would state a policy nothing applies and an author
    // would reasonably read it as having taken effect.
    let mut row = sum_usage();
    row.aggregation_granularity = Some(AggregationGranularity::Day);

    assert_eq!(
        codes(&findings(&LevelFields, &row)),
        vec![LEVEL_FIELDS_INVALID]
    );
}

#[test]
fn a_time_weighted_hourly_row_billed_per_day_fails_publish() {
    // The D-77 factor-of-24 band-edge case: bands would be GB-hours under
    // `inst-tb-units` and GB-days under `inst-la-units`.
    let mut row = peak_hourly();
    row.aggregation_function = Some(AggregationFunction::TimeWeighted);
    row.billing_granularity = Some(BillingGranularity::PerDay);

    assert_eq!(
        codes(&findings(&LevelGranularityPairing, &row)),
        vec![LEVEL_GRANULARITY_MISMATCH]
    );
}

#[test]
fn a_day_granule_row_must_be_billed_per_day() {
    let mut row = peak_hourly();
    row.aggregation_granularity = Some(AggregationGranularity::Day);

    assert_eq!(
        codes(&findings(&LevelGranularityPairing, &row)),
        vec![LEVEL_GRANULARITY_MISMATCH]
    );

    row.billing_granularity = Some(BillingGranularity::PerDay);
    assert!(
        findings(&LevelGranularityPairing, &row)
            .violations
            .is_empty()
    );
}

#[test]
fn the_sub_hour_and_unquantized_granularities_are_illegal_on_a_level_row() {
    for granularity in [
        BillingGranularity::PerSecond,
        BillingGranularity::PerMinute,
        BillingGranularity::WholeUnit,
    ] {
        let mut row = peak_hourly();
        row.billing_granularity = Some(granularity);

        assert_eq!(
            codes(&findings(&LevelGranularityPairing, &row)),
            vec![LEVEL_GRANULARITY_MISMATCH],
            "expected {granularity} to be refused on an hourly fold"
        );
    }
}

#[test]
fn an_unauthored_granularity_on_a_level_row_pairs_with_per_hour() {
    // `hour` is the default, so a row that authors only the function is still
    // judged against `per_hour` — otherwise the pairing could be evaded by
    // omission.
    let mut row = peak_hourly();
    row.aggregation_granularity = None;

    assert!(
        findings(&LevelGranularityPairing, &row)
            .violations
            .is_empty()
    );

    row.billing_granularity = Some(BillingGranularity::PerDay);
    assert_eq!(
        codes(&findings(&LevelGranularityPairing, &row)),
        vec![LEVEL_GRANULARITY_MISMATCH]
    );
}

#[test]
fn a_missing_billing_granularity_is_not_reported_as_a_pairing_failure() {
    // It is a missing field, reported once by `inst-tb-window`. Two codes for
    // one omission would make one fault look like two.
    let mut row = peak_hourly();
    row.billing_granularity = None;

    assert!(
        findings(&LevelGranularityPairing, &row)
            .violations
            .is_empty()
    );
}

#[test]
fn a_level_row_without_a_max_hold_fails_publish() {
    let mut row = peak_hourly();
    row.max_hold_granules = None;

    assert_eq!(
        codes(&findings(&LevelMaxHold, &row)),
        vec![LEVEL_FIELDS_INVALID]
    );
}

#[test]
fn a_zero_max_hold_fails_publish() {
    // Zero granules is not the way to say "never hold": it is a bound that holds
    // nothing, and the sampling-gap policy has to be a positive statement.
    let mut row = peak_hourly();
    row.max_hold_granules = Some(0);

    assert_eq!(
        codes(&findings(&LevelMaxHold, &row)),
        vec![LEVEL_FIELDS_INVALID]
    );
}

#[test]
fn a_max_hold_on_a_sum_row_fails_publish() {
    // The other half of the rule, and the easy one to forget: a `sum` row has no
    // gap fold to bound.
    let mut row = sum_usage();
    row.max_hold_granules = Some(3);

    assert_eq!(
        codes(&findings(&LevelMaxHold, &row)),
        vec![LEVEL_FIELDS_INVALID]
    );
}

#[test]
fn a_max_hold_on_a_recurring_row_fails_publish() {
    // A non-usage row derives nothing, so it is a `sum` row by default and the
    // same half of the rule catches it.
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(minor(100));
    row.max_hold_granules = Some(3);

    assert_eq!(
        codes(&findings(&LevelMaxHold, &row)),
        vec![LEVEL_FIELDS_INVALID]
    );
}

/// D-312 — which half of this rule the authoring write can already judge.
mod write_stage {
    use super::*;
    use crate::domain::money::RateMinor;
    use crate::domain::validation::Stage;

    fn stage_of(report: &ValidationReport, code: &str) -> Stage {
        report
            .violations
            .iter()
            .find(|v| v.code == code)
            .unwrap_or_else(|| panic!("expected a {code} violation: {:?}", codes(report)))
            .stage
    }

    #[test]
    fn a_level_field_on_a_non_usage_row_is_judgeable_at_the_write() {
        // `chargeKind` is frozen by the scope key and the misplaced field is in
        // the request, so nothing a later call adds can make this row publish.
        let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
        row.amount_minor = Some(minor(2_500));
        row.aggregation_function = Some(AggregationFunction::Peak);
        let report = findings(&LevelFields, &row);
        assert_eq!(stage_of(&report, LEVEL_FIELDS_INVALID), Stage::Write);
    }

    #[test]
    fn a_granularity_on_a_sum_row_is_not_judgeable_at_the_write() {
        // The sibling fault, same code, same rule — and both its operands are
        // content: `aggregationFunction: peak` resolves it by adding intent.
        let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
        row.unit_rate = Some(RateMinor::from_minor_units(1).expect("test rate"));
        row.billing_granularity = Some(BillingGranularity::WholeUnit);
        row.aggregation_granularity = Some(AggregationGranularity::Hour);
        let report = findings(&LevelFields, &row);
        assert_eq!(stage_of(&report, LEVEL_FIELDS_INVALID), Stage::Publish);
    }
}
