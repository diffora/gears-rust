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
use crate::domain::price_row::{ModelKind, PriceRow};
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
    report.rows.iter().map(|outcome| outcome.row).collect()
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
    for outcome in &report.rows {
        assert_eq!(outcome.violations.len(), 1);
        assert_eq!(outcome.violations[0].code, DUPLICATE_SCOPE_KEY);
    }
    assert!(
        report.rows[0].violations[0].detail.contains("row(s) 1"),
        "row 0 must name row 1: {}",
        report.rows[0].violations[0].detail
    );
    assert!(
        report.rows[1].violations[0].detail.contains("row(s) 0"),
        "row 1 must name row 0: {}",
        report.rows[1].violations[0].detail
    );
    assert!(report.blocks_the_batch());
}

#[test]
fn three_rows_on_one_key_each_name_the_other_two() {
    let report = classify(&[row(base()), row(base()), row(base())]);
    assert_eq!(failed_rows(&report), vec![0, 1, 2]);
    assert!(report.rows[1].violations[0].detail.contains("row(s) 0, 2"));
}

#[test]
fn two_rows_differing_only_in_their_usage_line_are_the_same_draft_row() {
    // **The case an equality over the whole `ScopeKey` gets wrong.** The draft
    // plane's partial `UNIQUE` does not include `meter` or `dimension_key`, so
    // these two collide — and letting them through would move the collision to
    // commit, which is per-row and cannot report it as a batch fault (D-148).
    let usage = key("eu", PriceEligibility::AllSubscriptions, ChargeKind::Usage);
    let metered = usage
        .clone()
        .with_usage_line(
            Some(Meter::new("api-calls").expect("a meter")),
            DimensionKey::new("region=eu"),
        )
        .expect("a usage line on a recurring key");
    let other_meter = usage
        .with_usage_line(
            Some(Meter::new("storage-gb").expect("a meter")),
            DimensionKey::new("region=eu"),
        )
        .expect("a usage line on a recurring key");
    assert_ne!(metered, other_meter, "the keys themselves differ");

    let report = classify(&[row(metered), row(other_meter)]);
    assert_eq!(
        failed_rows(&report),
        vec![0, 1],
        "two usage lines on one canonical scope key are one draft row"
    );
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
fn an_edit_and_a_create_aimed_at_one_draft_row_still_collide() {
    // `if_match` distinguishes an edit from a create and changes nothing here:
    // two rows aimed at one draft row collide whether or not one of them claims
    // to own it already.
    let mut edit = row(base());
    edit.if_match = Some(crate::domain::concurrency::RowVersion::new(3));
    let report = classify(&[edit, row(base())]);
    assert_eq!(failed_rows(&report), vec![0, 1]);
}

#[test]
fn an_empty_batch_blocks_nothing() {
    let report = classify(&[]);
    assert!(!report.blocks_the_batch());
    assert_eq!(failed_rows(&report), Vec::<usize>::new());
}
