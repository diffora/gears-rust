//! Phase 1's batch-only rules, executed.
//!
//! The rules here need no store, so these cases build no world — which is the
//! reason the batch-only half was separated in the first place. What they must
//! not do is assert a subset that survives the bug: every case names both the
//! rows it expects to fail **and** the rows it expects to pass, because a report
//! that failed everything and a report that failed the right thing are otherwise
//! the same green.

use super::{BatchReport, DUPLICATE_SCOPE_KEY, ImportRow, classify};
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::price_record::PriceContent;
use crate::domain::price_row::{
    IncludedAllowance, ModelKind, PriceRow, RolloverPolicy, TierQualificationWindow,
};
use crate::domain::publish::rules::PRIMITIVE_RULES_UNBUILT;
use crate::domain::scope_key::{
    ChargeKind, Cohort, DimensionKey, Meter, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use uuid::Uuid;

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x9_1a))
}

fn phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(0xfa_5e))
}

fn key(region: &str, eligibility: PriceEligibility, charge: ChargeKind) -> ScopeKey {
    ScopeKey::new(
        plan(),
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new(region).expect("a non-blank region"),
        phase(),
        eligibility,
        charge,
        Cohort::None,
    )
    .expect("the class pairs with the cohort")
}

fn base() -> ScopeKey {
    key(
        "eu",
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
    )
}

fn content() -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    PriceContent {
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: Some("advance".to_owned()),
        proration_contract: None,
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

fn row(scope_key: ScopeKey) -> ImportRow {
    ImportRow {
        scope_key,
        content: content(),
        if_match: None,
    }
}

fn failed_rows(report: &BatchReport) -> Vec<usize> {
    report.rows().iter().map(|outcome| outcome.row).collect()
}

#[test]
fn a_batch_of_distinct_keys_passes_and_does_not_block() {
    let report = classify(&[
        row(base()),
        row(key(
            "us",
            PriceEligibility::AllSubscriptions,
            ChargeKind::Recurring,
        )),
        row(key(
            "eu",
            PriceEligibility::AllSubscriptions,
            ChargeKind::OneTime,
        )),
        row(key(
            "eu",
            PriceEligibility::NewSubscriptionsOnly,
            ChargeKind::Recurring,
        )),
    ]);
    assert_eq!(failed_rows(&report), Vec::<usize>::new());
    assert!(!report.blocks_the_batch());
}

#[test]
fn two_rows_on_one_key_both_fail_and_each_names_the_other() {
    let report = classify(&[row(base()), row(base())]);

    assert_eq!(
        failed_rows(&report),
        vec![0, 1],
        "a collision has two rows in it and neither is more at fault"
    );
    for outcome in report.rows() {
        assert_eq!(outcome.violations.len(), 1);
        assert_eq!(outcome.violations[0].code, DUPLICATE_SCOPE_KEY);
    }
    assert!(
        report.rows()[0].violations[0].detail.contains("row(s) 1"),
        "row 0 must name row 1: {}",
        report.rows()[0].violations[0].detail
    );
    assert!(
        report.rows()[1].violations[0].detail.contains("row(s) 0"),
        "row 1 must name row 0: {}",
        report.rows()[1].violations[0].detail
    );
    assert!(report.blocks_the_batch());
}

#[test]
fn three_rows_on_one_key_each_name_the_other_two() {
    let report = classify(&[row(base()), row(base()), row(base())]);
    assert_eq!(failed_rows(&report), vec![0, 1, 2]);
    assert!(
        report.rows()[1].violations[0]
            .detail
            .contains("row(s) 0, 2")
    );
}

#[test]
fn two_rows_differing_only_in_their_usage_line_are_two_keys_and_both_author() {
    // **D-103's confirmed worked example**, and the case a first build got
    // backwards (D-283). `m20260802_000023` widened the draft plane's partial
    // `UNIQUE` to include `COALESCE(meter, '')` and `dimension_key`, and D-196
    // made the usage pair normative axes of the canonical key — so a PaaS plan
    // pricing cloudlets, storage and egress is one plan, and these two rows are
    // two keys. `tests/sqlite_price_repo.rs` proves the store admits them both;
    // this proves Phase 1 does not refuse them first.
    let usage = key("eu", PriceEligibility::AllSubscriptions, ChargeKind::Usage);
    let metered = usage
        .clone()
        .with_usage_line(
            Some(Meter::new("api-calls").expect("a meter")),
            DimensionKey::new("region=eu"),
        )
        .expect("a usage line on a usage key");
    let other_meter = usage
        .with_usage_line(
            Some(Meter::new("storage-gb").expect("a meter")),
            DimensionKey::new("region=eu"),
        )
        .expect("a usage line on a usage key");

    let report = classify(&[row(metered.clone()), row(other_meter)]);
    assert_eq!(
        failed_rows(&report),
        Vec::<usize>::new(),
        "two meters are two keys, and Phase 1 must not refuse what the store admits"
    );

    // And the same line twice IS a duplicate — otherwise this case would pass
    // for a `classify` that never reports anything at all.
    let doubled = classify(&[row(metered.clone()), row(metered)]);
    assert_eq!(failed_rows(&doubled), vec![0, 1]);
}

#[test]
fn two_rows_differing_only_in_their_dimension_key_are_also_two_keys() {
    // The other axis the widened index carries. Untested, it is an axis the
    // duplicate rule could stop comparing with nothing noticing.
    let usage = key("eu", PriceEligibility::AllSubscriptions, ChargeKind::Usage);
    let eu = usage
        .clone()
        .with_usage_line(
            Some(Meter::new("api-calls").expect("a meter")),
            DimensionKey::new("region=eu"),
        )
        .expect("a usage line on a usage key");
    let us = usage
        .with_usage_line(
            Some(Meter::new("api-calls").expect("a meter")),
            DimensionKey::new("region=us"),
        )
        .expect("a usage line on a usage key");

    let report = classify(&[row(eu.clone()), row(us)]);
    assert_eq!(failed_rows(&report), Vec::<usize>::new());

    // The same companion its sibling carries, and for the same reason: without
    // it this passes for a `classify` that never reports anything at all.
    let doubled = classify(&[row(eu.clone()), row(eu)]);
    assert_eq!(failed_rows(&doubled), vec![0, 1]);
}

#[test]
fn a_duplicate_pair_does_not_drag_the_rest_of_the_batch_down() {
    // The other half of "one invalid row blocks the batch": the *batch* is
    // blocked, but the report must still say which rows are actually wrong.
    // A report that failed all four would block the same batch and tell the
    // operator nothing.
    let report = classify(&[
        row(base()),
        row(key(
            "us",
            PriceEligibility::AllSubscriptions,
            ChargeKind::Recurring,
        )),
        row(base()),
        row(key(
            "eu",
            PriceEligibility::AllSubscriptions,
            ChargeKind::OneTime,
        )),
    ]);
    assert_eq!(failed_rows(&report), vec![0, 2]);
    assert!(report.blocks_the_batch());
}

#[test]
fn a_row_carrying_an_unbuilt_primitive_fails_and_inherits_the_publish_code() {
    // **The refusal moves earlier, it does not move** (D-177/D-179): publish
    // still refuses these fields on its own authority. What this arm changes is
    // that the operator hears it while the batch can still be fixed.
    let mut row = row(base());
    row.content.row.included_allowance = Some(IncludedAllowance {
        quantity: 100,
        rollover_policy: RolloverPolicy::Carry,
    });

    let report = classify(&[row]);
    assert_eq!(failed_rows(&report), vec![0]);
    assert_eq!(report.rows()[0].violations[0].code, PRIMITIVE_RULES_UNBUILT);
    assert!(
        report.rows()[0].violations[0]
            .detail
            .contains("includedAllowance"),
        "the sentence names the field the operator has to remove: {}",
        report.rows()[0].violations[0].detail
    );
    assert!(report.blocks_the_batch());
}

#[test]
fn a_row_carrying_both_unbuilt_primitives_is_told_about_both() {
    // The all-or-nothing posture only pays for itself if the report is complete
    // — a row fixed one field at a time is a second batch for nothing.
    let mut row = row(base());
    row.content.row.included_allowance = Some(IncludedAllowance {
        quantity: 100,
        rollover_policy: RolloverPolicy::Carry,
    });
    row.content.row.tier_qualification_window = Some(TierQualificationWindow::TrailingPeriod);

    let report = classify(&[row]);
    assert_eq!(report.rows()[0].violations.len(), 2, "both, not the first");
    let details: Vec<&str> = report.rows()[0]
        .violations
        .iter()
        .map(|violation| violation.detail.as_str())
        .collect();
    assert!(details.iter().any(|d| d.contains("includedAllowance")));
    assert!(
        details
            .iter()
            .any(|d| d.contains("tierQualificationWindow"))
    );
}

#[test]
fn a_row_can_carry_two_different_faults_and_hears_about_both() {
    // The two rules are independent and both answer for every row. A report that
    // stopped at the first would send the operator round twice.
    let mut first = row(base());
    first.content.row.included_allowance = Some(IncludedAllowance {
        quantity: 100,
        rollover_policy: RolloverPolicy::Carry,
    });
    let report = classify(&[first, row(base())]);

    assert_eq!(failed_rows(&report), vec![0, 1]);
    let codes: Vec<&str> = report.rows()[0]
        .violations
        .iter()
        .map(|violation| violation.code.as_str())
        .collect();
    assert_eq!(
        codes,
        vec![DUPLICATE_SCOPE_KEY, PRIMITIVE_RULES_UNBUILT],
        "row 0 is both a duplicate and unjudged, and the order is stable"
    );
    assert_eq!(
        report.rows()[1]
            .violations
            .iter()
            .map(|violation| violation.code.as_str())
            .collect::<Vec<_>>(),
        vec![DUPLICATE_SCOPE_KEY],
        "row 1 carries no primitive and must not inherit its neighbour's fault"
    );
}

#[test]
fn an_empty_batch_blocks_nothing() {
    let report = classify(&[]);
    assert!(!report.blocks_the_batch());
    assert_eq!(failed_rows(&report), Vec::<usize>::new());
}
