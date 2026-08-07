//! Slice 6's consumer-contract rules — `design/06-consumer-contracts.md` §3, §5.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use super::{BILLING_TIMING_MISSING, BillingTimingPresent};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::plan_shape::PlanShape;
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::PriceRow;
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use crate::domain::validation::{ValidationReport, ValidationRule};

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 7, 12, 0, 0)
        .single()
        .expect("the fixed instant is unambiguous")
}

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x0600))
}

/// One candidate row: its kind, its market, its eligibility and its timing.
fn row(
    price_id: u128,
    charge_kind: ChargeKind,
    currency: &str,
    region: &str,
    eligibility: PriceEligibility,
    billing_timing: Option<&str>,
) -> PriceRecord {
    let cohort = if eligibility == PriceEligibility::ExistingGrandfathered {
        Cohort::Generation(now())
    } else {
        Cohort::None
    };
    let scope_key = ScopeKey::new(
        plan(),
        CurrencyCode::new(currency).expect("three letters"),
        Region::new(region).expect("non-blank"),
        PhaseId::new(Uuid::from_u128(0xf1)),
        eligibility,
        charge_kind,
        cohort,
    )
    .expect("the eligibility and cohort pair");

    let mut shape = PriceRow::new(charge_kind, None);
    shape.amount_minor = Some(MinorAmount::new(1000).expect("non-negative"));

    PriceRecord {
        price_id: Uuid::from_u128(price_id),
        scope_key,
        row: shape,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: billing_timing.map(ToOwned::to_owned),
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
        lifecycle_state: LifecycleState::Draft,
        created_by: Uuid::from_u128(0x06_ac),
        created_at_utc: now(),
        row_version: RowVersion::new(0),
    }
}

fn shape_of(rows: Vec<PriceRecord>) -> PlanShape {
    let mut shape = PlanShape::new(plan(), 1, now());
    shape.rows = rows;
    shape
}

fn run(rule: &impl ValidationRule<PlanShape>, shape: &PlanShape) -> ValidationReport {
    let mut report = ValidationReport::default();
    rule.evaluate(shape, &mut report);
    report
}

fn codes(report: &ValidationReport) -> Vec<String> {
    report.violations.iter().map(|v| v.code.clone()).collect()
}

// ---------------------------------------------------------------------------
// `inst-bt-required` (§3 Billing Timing step 1, §5 `BILLING_TIMING_MISSING`).
// ---------------------------------------------------------------------------

#[test]
fn the_code_is_the_designs_verbatim() {
    assert_eq!(BILLING_TIMING_MISSING, "BILLING_TIMING_MISSING");
}

#[test]
fn a_recurring_row_without_billing_timing_fails_publish_naming_the_row() {
    let shape = shape_of(vec![row(
        0xb1,
        ChargeKind::Recurring,
        "EUR",
        "eu",
        PriceEligibility::AllSubscriptions,
        None,
    )]);

    let report = run(&BillingTimingPresent, &shape);

    assert_eq!(codes(&report), [BILLING_TIMING_MISSING]);
    let violation = &report.violations[0];
    assert_eq!(
        violation.subject,
        Uuid::from_u128(0xb1).to_string(),
        "the design's step 1 requires the row to be named"
    );
}

#[test]
fn a_recurring_row_stating_its_timing_passes() {
    for timing in ["advance", "arrears"] {
        let shape = shape_of(vec![row(
            0xb2,
            ChargeKind::Recurring,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            Some(timing),
        )]);

        assert!(
            codes(&run(&BillingTimingPresent, &shape)).is_empty(),
            "{timing} is one of the two the contract admits"
        );
    }
}

/// `inst-bt-usage`: usage and one-time rows never author the field — it is a
/// projected constant — so demanding it of them would reject a row whose value
/// is not the author's to give.
#[test]
fn a_non_recurring_row_without_billing_timing_is_not_this_rules_business() {
    for kind in [
        ChargeKind::Usage,
        ChargeKind::OneTime,
        ChargeKind::OneTimeSetup,
    ] {
        let shape = shape_of(vec![row(
            0xb3,
            kind,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            None,
        )]);

        assert!(
            codes(&run(&BillingTimingPresent, &shape)).is_empty(),
            "{kind} authors no billingTiming"
        );
    }
}

/// D-132 excludes a grandfathered generation from the *uniformity* row set —
/// an argument about comparing a frozen generation against current rows. It
/// says nothing about presence, and a grandfathered row published without the
/// field would leave Billing deriving a live subscriber's deferral by
/// heuristic.
#[test]
fn a_grandfathered_recurring_row_is_held_to_the_same_presence_rule() {
    let shape = shape_of(vec![row(
        0xb4,
        ChargeKind::Recurring,
        "EUR",
        "eu",
        PriceEligibility::ExistingGrandfathered,
        None,
    )]);

    assert_eq!(
        codes(&run(&BillingTimingPresent, &shape)),
        [BILLING_TIMING_MISSING]
    );
}

/// The report is the product: two bad rows are two findings, not the first one.
#[test]
fn every_offending_row_is_reported_not_merely_the_first() {
    let shape = shape_of(vec![
        row(
            0xb5,
            ChargeKind::Recurring,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            None,
        ),
        row(
            0xb6,
            ChargeKind::Recurring,
            "USD",
            "us",
            PriceEligibility::AllSubscriptions,
            None,
        ),
    ]);

    assert_eq!(codes(&run(&BillingTimingPresent, &shape)).len(), 2);
}
