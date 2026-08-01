//! Tests for the Slice-3 rule registration.

use bss_fixtures::ModelKind;

use super::{
    EVAL_POLICY_MISSING, MODEL_KIND_MISSING, SUPERSESSION_UNIT_MISMATCH, SupersessionPair,
    TIER_TOP_CLOSED, price_row_rules, supersession_rules,
};
use crate::domain::money::MinorAmount;
use crate::domain::price_row::{BillingGranularity, PriceRow, TierAggregationWindow, TierBand};
use crate::domain::scope_key::ChargeKind;

fn minor(units: i64) -> MinorAmount {
    MinorAmount::new(units).expect("test amount is non-negative")
}

fn graduated_usage() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("api_calls".to_owned());
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = vec![
        TierBand::closed(0, 1_000, minor(10)),
        TierBand::open(1_000, minor(6)),
    ];
    row
}

#[test]
fn the_pipeline_registers_every_slice_three_row_local_instruction() {
    // The instruction ids are the design set's own names; the registration list
    // is what makes a missing rule visible as an absent id rather than as a row
    // that quietly publishes.
    assert_eq!(
        price_row_rules().rule_names(),
        vec![
            "inst-mk-explicit",
            "inst-mk-required",
            "inst-mk-forbidden",
            "inst-mk-chargekind",
            "inst-tb-order",
            "inst-tb-first",
            "inst-tb-top",
            "inst-tb-window",
            "inst-pk-fields",
            "inst-pk-window",
            "inst-la-fields",
            "inst-la-granularity",
            "inst-la-maxhold",
        ]
    );
    assert_eq!(
        supersession_rules().rule_names(),
        vec!["inst-tb-supersession-units"]
    );
}

#[test]
fn a_well_formed_graduated_usage_row_publishes() {
    let report = price_row_rules().run(&graduated_usage());

    assert!(
        report.is_publishable(),
        "unexpected violations: {:?}",
        report
            .violations
            .iter()
            .map(|violation| violation.code.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn one_row_reports_every_fault_it_carries() {
    // The aggregate report is the point: an author remediates a plan in one
    // pass, so an unspecified kind must not mask the band and policy faults.
    let mut row = graduated_usage();
    row.model_kind = None;
    row.billing_granularity = None;
    row.bands = vec![TierBand::closed(0, 1_000, minor(10))];

    let report = price_row_rules().run(&row);
    let codes: Vec<&str> = report
        .violations
        .iter()
        .map(|violation| violation.code.as_str())
        .collect();

    assert!(codes.contains(&MODEL_KIND_MISSING));
    assert!(codes.contains(&TIER_TOP_CLOSED));
    assert!(codes.contains(&EVAL_POLICY_MISSING));
}

#[test]
fn the_supersession_pipeline_judges_a_pair_without_asking_which_mechanism_made_it() {
    // D-127: both sanctioned producers of `published -> superseded` run this
    // same pipeline over this same subject, so there is no argument by which a
    // cutover successor could be judged more leniently than an interactive one.
    let mut successor = graduated_usage();
    successor.billing_granularity = Some(BillingGranularity::PerDay);

    let report = supersession_rules().run(&SupersessionPair::new(graduated_usage(), successor));

    assert_eq!(report.violations.len(), 1);
    assert_eq!(report.violations[0].code, SUPERSESSION_UNIT_MISMATCH);
}
