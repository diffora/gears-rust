//! Tests for the model-kind rules.

use bss_fixtures::ModelKind;

use super::{ExplicitModelKind, KindChargeKindMatrix, KindForbiddenFields, KindRequiredFields};
use crate::domain::money::{MinorAmount, RateMinor};
use crate::domain::price_row::{
    BillingGranularity, PriceRow, QuantitySource, TierAggregationWindow, TierBand,
};
use crate::domain::rules::{
    AMOUNT_PLACEMENT_INVALID, EVAL_POLICY_MISPLACED, MODEL_KIND_CHARGEKIND_MISMATCH,
    MODEL_KIND_MISSING, PACKAGE_FIELDS_INVALID, QUANTITY_SOURCE_MISSING,
};
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

/// A publishable `flat` recurring row.
fn flat_recurring() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(minor(2_500));
    row
}

/// A publishable `per_unit` recurring (per-seat) row.
fn per_unit_recurring() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::PerUnit));
    row.unit_rate = Some(rate(900));
    row.quantity_source = Some(QuantitySource::SubscriptionSeatCount);
    row
}

/// A publishable `per_unit` usage row — the plain untiered metered rate.
fn per_unit_usage() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    row.unit_rate = Some(rate(3));
    row.meter = Some("api_calls".to_owned());
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row
}

/// A publishable `graduated` usage row.
fn graduated_usage() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("api_calls".to_owned());
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = vec![
        TierBand::closed(0, 1_000, rate(10)),
        TierBand::open(1_000, rate(6)),
    ];
    row
}

/// A publishable `package` usage row.
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
fn a_row_without_a_model_kind_fails_publish() {
    let row = PriceRow::new(ChargeKind::Usage, None);

    assert_eq!(
        codes(&findings(&ExplicitModelKind, &row)),
        vec![MODEL_KIND_MISSING]
    );
}

#[test]
fn a_row_with_an_explicit_kind_passes() {
    assert!(
        findings(&ExplicitModelKind, &flat_recurring())
            .violations
            .is_empty()
    );
}

#[test]
fn the_field_rules_stay_silent_while_the_kind_is_missing() {
    // Otherwise one omission produces a report full of per-kind consequences of
    // a kind nobody authored, and the fault the author must fix first is buried.
    let row = PriceRow::new(ChargeKind::Usage, None);

    assert!(findings(&KindRequiredFields, &row).violations.is_empty());
    assert!(findings(&KindForbiddenFields, &row).violations.is_empty());
    assert!(findings(&KindChargeKindMatrix, &row).violations.is_empty());
}

#[test]
fn a_flat_row_without_an_amount_fails_publish() {
    let mut row = flat_recurring();
    row.amount_minor = None;

    assert_eq!(
        codes(&findings(&KindRequiredFields, &row)),
        vec![AMOUNT_PLACEMENT_INVALID]
    );
}

/// **A `per_unit` row prices from its own rate column, and only from it** (D-311).
///
/// Both directions, because each half failed on its own for the length of one
/// commit: the rule still demanded `amount_minor` on a `per_unit` row, so a row
/// authored the new way was refused for a missing amount while a row authored
/// the old way passed with its rate column empty — the new column unauthorable
/// and the superseded spelling still accepted, which is the worst of both.
#[test]
fn a_per_unit_row_keeps_its_unit_price_in_the_rate_column() {
    assert!(
        findings(&KindRequiredFields, &per_unit_recurring())
            .violations
            .is_empty(),
        "a rate is what a per_unit row is authored with"
    );

    let mut no_rate = per_unit_recurring();
    no_rate.unit_rate = None;
    assert_eq!(
        codes(&findings(&KindRequiredFields, &no_rate)),
        vec![AMOUNT_PLACEMENT_INVALID],
        "a per_unit row with no rate has no price"
    );

    let mut old_spelling = per_unit_recurring();
    old_spelling.unit_rate = None;
    old_spelling.amount_minor = Some(minor(900));
    assert_eq!(
        codes(&findings(&KindRequiredFields, &old_spelling)),
        vec![AMOUNT_PLACEMENT_INVALID, AMOUNT_PLACEMENT_INVALID],
        "the pre-D-311 spelling is not a second way to price a per_unit row, and it \
         is two faults rather than one: an amount where none belongs and no rate \
         where one must be"
    );

    let mut both = per_unit_recurring();
    both.amount_minor = Some(minor(900));
    assert_eq!(
        codes(&findings(&KindRequiredFields, &both)),
        vec![AMOUNT_PLACEMENT_INVALID],
        "two priced columns are two competing prices, and this is the pair that \
         used to be one column"
    );
}

/// **A rate is forbidden on every kind that does not multiply by one** — the
/// other half of the matrix, and the half a rule that only checked presence
/// would leave open.
#[test]
fn a_kind_that_charges_no_per_unit_multiple_refuses_a_rate() {
    for mut row in [flat_recurring(), graduated_usage(), package_usage()] {
        let subject = row.subject();
        row.unit_rate = Some(rate(1));

        assert!(
            codes(&findings(&KindRequiredFields, &row)).contains(&AMOUNT_PLACEMENT_INVALID),
            "expected {subject} to refuse a unit rate"
        );
    }
}

#[test]
fn a_band_kind_carrying_an_amount_fails_publish() {
    // Two priced columns are two competing prices, and nothing downstream
    // adjudicates between them.
    for mut row in [graduated_usage(), package_usage()] {
        row.amount_minor = Some(minor(1));

        assert!(
            codes(&findings(&KindRequiredFields, &row)).contains(&AMOUNT_PLACEMENT_INVALID),
            "expected {} to refuse an amount_minor",
            row.subject()
        );
    }
}

#[test]
fn a_per_unit_recurring_row_must_declare_a_quantity_source() {
    let mut row = per_unit_recurring();
    row.quantity_source = None;

    assert_eq!(
        codes(&findings(&KindRequiredFields, &row)),
        vec![QUANTITY_SOURCE_MISSING]
    );
}

#[test]
fn a_per_unit_usage_row_must_not_declare_a_quantity_source() {
    // The mirror image of the rule above, and the one that is easy to get
    // backwards: a usage row's quantity comes from the meter, so an authored
    // source would be a second answer to "how much was consumed".
    let mut row = per_unit_usage();
    row.quantity_source = Some(QuantitySource::SubscriptionSeatCount);

    assert!(findings(&KindRequiredFields, &row).violations.is_empty());
    assert_eq!(
        codes(&findings(&KindForbiddenFields, &row)),
        vec![EVAL_POLICY_MISPLACED]
    );
}

#[test]
fn a_manual_quantity_source_without_a_quantity_fails_publish() {
    let mut row = per_unit_recurring();
    row.quantity_source = Some(QuantitySource::Manual);

    assert_eq!(
        codes(&findings(&KindRequiredFields, &row)),
        vec![QUANTITY_SOURCE_MISSING]
    );

    row.manual_quantity = Some(12);
    assert!(findings(&KindRequiredFields, &row).violations.is_empty());
}

#[test]
fn a_manual_quantity_without_a_manual_source_fails_publish() {
    let mut row = per_unit_recurring();
    row.manual_quantity = Some(12);

    assert_eq!(
        codes(&findings(&KindForbiddenFields, &row)),
        vec![EVAL_POLICY_MISPLACED]
    );
}

#[test]
fn a_package_row_without_its_block_fields_fails_publish() {
    let mut row = package_usage();
    row.package_size = None;

    assert_eq!(
        codes(&findings(&KindRequiredFields, &row)),
        vec![PACKAGE_FIELDS_INVALID]
    );
}

#[test]
fn tier_bands_on_a_per_unit_row_fail_publish() {
    let mut row = per_unit_recurring();
    row.bands = vec![TierBand::open(0, rate(1))];

    assert_eq!(
        codes(&findings(&KindForbiddenFields, &row)),
        vec![EVAL_POLICY_MISPLACED]
    );
}

#[test]
fn evaluation_policy_on_a_non_usage_row_fails_publish() {
    let mut row = flat_recurring();
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);

    let report = findings(&KindForbiddenFields, &row);

    assert_eq!(codes(&report), vec![EVAL_POLICY_MISPLACED]);
    let detail = &report.violations[0].detail;
    assert!(detail.contains("tierAggregationWindow"), "{detail}");
    assert!(detail.contains("billingGranularity"), "{detail}");
}

#[test]
fn a_per_unit_usage_row_carries_billing_granularity_like_every_usage_row() {
    // The 2026-07-28 correction: `billingGranularity` is usage-row-only, not
    // tiered-row-only, so the plain metered rate must not be refused for it.
    assert!(
        findings(&KindForbiddenFields, &per_unit_usage())
            .violations
            .is_empty()
    );
}

#[test]
fn flat_on_a_usage_row_fails_the_charge_kind_matrix() {
    let mut row = flat_recurring();
    row.charge_kind = ChargeKind::Usage;

    assert_eq!(
        codes(&findings(&KindChargeKindMatrix, &row)),
        vec![MODEL_KIND_CHARGEKIND_MISMATCH]
    );
}

#[test]
fn the_tier_and_block_kinds_are_illegal_on_every_non_usage_row() {
    // The tier machinery presupposes a metered quantity stream, and no `Q`
    // semantics exist for a non-usage row.
    for charge_kind in [
        ChargeKind::Recurring,
        ChargeKind::OneTime,
        ChargeKind::OneTimeSetup,
    ] {
        for model_kind in [ModelKind::Graduated, ModelKind::Volume, ModelKind::Package] {
            let row = PriceRow::new(charge_kind, Some(model_kind));

            assert_eq!(
                codes(&findings(&KindChargeKindMatrix, &row)),
                vec![MODEL_KIND_CHARGEKIND_MISMATCH],
                "expected {} to be refused",
                row.subject()
            );
        }
    }
}

#[test]
fn each_kind_is_legal_on_the_rows_the_matrix_admits() {
    for model_kind in [ModelKind::Flat, ModelKind::PerUnit] {
        let row = PriceRow::new(ChargeKind::Recurring, Some(model_kind));
        assert!(
            findings(&KindChargeKindMatrix, &row).violations.is_empty(),
            "expected {} to be legal",
            row.subject()
        );
    }
    for model_kind in [
        ModelKind::PerUnit,
        ModelKind::Graduated,
        ModelKind::Volume,
        ModelKind::Package,
    ] {
        let row = PriceRow::new(ChargeKind::Usage, Some(model_kind));
        assert!(
            findings(&KindChargeKindMatrix, &row).violations.is_empty(),
            "expected {} to be legal",
            row.subject()
        );
    }
}
