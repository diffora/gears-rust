//! Slice 6's consumer-contract rules — `design/06-consumer-contracts.md` §3, §5.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use super::{
    AnchorDay, BILLING_TIMING_MISSING, BillingAnchorPolicy, BillingTimingPresent,
    PRORATION_CONTRACT_MIXED_MARKET, PRORATION_INPUTS_CONTRADICTORY, PRORATION_INPUTS_MISSING,
    ProrationBasis, ProrationContract, ProrationContractMarketUniform, ProrationCreditHasBasis,
    ProrationInputsPresent,
};
use super::{
    CHANGE_TARGET_UNPUBLISHED, COMPARABILITY_RANK_REQUIRED, COMPARABILITY_RANK_REVOKED,
    ChangeGraphAuthorable, ChangeTargetIndex, PlanChangeContract, UsageCounterOnPlanChange,
};
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
    proration_contract: Option<ProrationContract>,
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
        proration_contract,
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
            None,
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
            None,
        ),
        row(
            0xb6,
            ChargeKind::Recurring,
            "USD",
            "us",
            PriceEligibility::AllSubscriptions,
            None,
            None,
        ),
    ]);

    assert_eq!(codes(&run(&BillingTimingPresent, &shape)).len(), 2);
}

// ---------------------------------------------------------------------------
// `inst-pi-required` / `inst-pi-credit-none` / `inst-pi-uniform`
// (§3 Proration Input Contract, §5, D-123 scoped by D-132).
// ---------------------------------------------------------------------------

/// One contract, named by its three members.
fn contract(
    policy: BillingAnchorPolicy,
    basis: ProrationBasis,
    credit_on_downgrade: bool,
) -> ProrationContract {
    ProrationContract {
        billing_anchor_policy: policy,
        proration_basis: basis,
        credit_on_downgrade,
    }
}

/// The contract every uniformity case moves exactly one member away from.
fn baseline_contract() -> ProrationContract {
    contract(
        BillingAnchorPolicy::CalendarMonth,
        ProrationBasis::CalendarDaysActual,
        false,
    )
}

/// A recurring row on `currency`/`region`, in `eligibility`, carrying `c`.
fn priced(
    price_id: u128,
    currency: &str,
    region: &str,
    eligibility: PriceEligibility,
    c: Option<ProrationContract>,
) -> PriceRecord {
    row(
        price_id,
        ChargeKind::Recurring,
        currency,
        region,
        eligibility,
        Some("advance"),
        c,
    )
}

#[test]
fn the_three_codes_are_the_designs_verbatim() {
    assert_eq!(PRORATION_INPUTS_MISSING, "PRORATION_INPUTS_MISSING");
    assert_eq!(
        PRORATION_INPUTS_CONTRADICTORY,
        "PRORATION_INPUTS_CONTRADICTORY"
    );
    assert_eq!(
        PRORATION_CONTRACT_MIXED_MARKET,
        "PRORATION_CONTRACT_MIXED_MARKET"
    );
}

#[test]
fn a_recurring_row_without_the_proration_inputs_fails_publish_naming_the_row() {
    let shape = shape_of(vec![priced(
        0xc1,
        "EUR",
        "eu",
        PriceEligibility::AllSubscriptions,
        None,
    )]);

    let report = run(&ProrationInputsPresent, &shape);

    assert_eq!(codes(&report), [PRORATION_INPUTS_MISSING]);
    assert_eq!(
        report.violations[0].subject,
        Uuid::from_u128(0xc1).to_string()
    );
}

#[test]
fn a_recurring_row_stating_the_contract_passes() {
    let shape = shape_of(vec![priced(
        0xc2,
        "EUR",
        "eu",
        PriceEligibility::AllSubscriptions,
        Some(baseline_contract()),
    )]);

    assert!(codes(&run(&ProrationInputsPresent, &shape)).is_empty());
}

/// The contract attaches to recurring rows. A usage or one-time row authors no
/// proration inputs, and demanding them would reject a row whose values are not
/// the author's to give — the same boundary `inst-bt-required` draws.
#[test]
fn a_non_recurring_row_owes_no_proration_contract() {
    for kind in [
        ChargeKind::Usage,
        ChargeKind::OneTime,
        ChargeKind::OneTimeSetup,
    ] {
        let shape = shape_of(vec![row(
            0xc3,
            kind,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            None,
            None,
        )]);

        assert!(
            codes(&run(&ProrationInputsPresent, &shape)).is_empty(),
            "{kind} authors no proration contract"
        );
    }
}

/// A grandfathered row is held to **presence** — D-132 excludes it from the
/// uniformity row set, which is a different rule.
#[test]
fn a_grandfathered_recurring_row_still_owes_its_own_contract() {
    let shape = shape_of(vec![priced(
        0xc4,
        "EUR",
        "eu",
        PriceEligibility::ExistingGrandfathered,
        None,
    )]);

    assert_eq!(
        codes(&run(&ProrationInputsPresent, &shape)),
        [PRORATION_INPUTS_MISSING]
    );
}

#[test]
fn crediting_a_downgrade_with_no_basis_to_size_it_fails_publish() {
    let shape = shape_of(vec![priced(
        0xc5,
        "EUR",
        "eu",
        PriceEligibility::AllSubscriptions,
        Some(contract(
            BillingAnchorPolicy::CalendarMonth,
            ProrationBasis::None,
            true,
        )),
    )]);

    let report = run(&ProrationCreditHasBasis, &shape);

    assert_eq!(codes(&report), [PRORATION_INPUTS_CONTRADICTORY]);
    assert_eq!(
        report.violations[0].subject,
        Uuid::from_u128(0xc5).to_string()
    );
}

/// Both halves are individually legal: a plan that never prorates is a real
/// plan, and a credited downgrade is a real policy. Only the pair contradicts.
#[test]
fn neither_half_of_the_contradiction_is_a_violation_on_its_own() {
    for c in [
        contract(
            BillingAnchorPolicy::CalendarMonth,
            ProrationBasis::None,
            false,
        ),
        contract(
            BillingAnchorPolicy::CalendarMonth,
            ProrationBasis::BySecond,
            true,
        ),
    ] {
        let shape = shape_of(vec![priced(
            0xc6,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            Some(c),
        )]);
        assert!(
            codes(&run(&ProrationCreditHasBasis, &shape)).is_empty(),
            "{c:?} is publishable on its own"
        );
    }
}

/// D-123's own scenario: an intro-pricing plan anchoring `subscription_start`
/// on the intro row and `fixed_day(1)` on the terminal row, both on one market.
#[test]
fn two_anchors_on_one_market_fail_publish_naming_the_divergent_rows() {
    let shape = shape_of(vec![
        priced(
            0xd1,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            Some(contract(
                BillingAnchorPolicy::SubscriptionStart,
                ProrationBasis::CalendarDaysActual,
                false,
            )),
        ),
        priced(
            0xd2,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            Some(contract(
                BillingAnchorPolicy::FixedDay(AnchorDay::new(1).expect("a day of the month")),
                ProrationBasis::CalendarDaysActual,
                false,
            )),
        ),
    ]);

    let report = run(&ProrationContractMarketUniform, &shape);

    assert_eq!(codes(&report), [PRORATION_CONTRACT_MIXED_MARKET]);
    let detail = &report.violations[0].detail;
    for id in [0xd1_u128, 0xd2] {
        assert!(
            detail.contains(&Uuid::from_u128(id).to_string()),
            "the design requires the divergent rows to be named: {detail}"
        );
    }
    assert!(
        detail.contains("billingAnchorPolicy"),
        "and the divergent field: {detail}"
    );
}

/// Two `fixed_day` anchors on different days are two anchors, not one. A
/// comparison on the token alone would call them equal and publish a market
/// with two cycle clocks.
#[test]
fn two_fixed_days_on_different_days_are_a_divergence() {
    let shape = shape_of(vec![
        priced(
            0xd3,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            Some(contract(
                BillingAnchorPolicy::FixedDay(AnchorDay::new(1).expect("a day")),
                ProrationBasis::CalendarDaysActual,
                false,
            )),
        ),
        priced(
            0xd4,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            Some(contract(
                BillingAnchorPolicy::FixedDay(AnchorDay::new(15).expect("a day")),
                ProrationBasis::CalendarDaysActual,
                false,
            )),
        ),
    ]);

    assert_eq!(
        codes(&run(&ProrationContractMarketUniform, &shape)),
        [PRORATION_CONTRACT_MIXED_MARKET]
    );
}

/// Per market, not per plan: the D-110 shape. EU on the 1st and US on signup
/// day is exactly what this rule leaves legal.
#[test]
fn the_same_split_across_two_markets_publishes() {
    let shape = shape_of(vec![
        priced(
            0xd5,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            Some(contract(
                BillingAnchorPolicy::FixedDay(AnchorDay::new(1).expect("a day")),
                ProrationBasis::CalendarDaysActual,
                false,
            )),
        ),
        priced(
            0xd6,
            "USD",
            "us",
            PriceEligibility::AllSubscriptions,
            Some(contract(
                BillingAnchorPolicy::SubscriptionStart,
                ProrationBasis::CalendarDaysActual,
                false,
            )),
        ),
    ]);

    assert!(codes(&run(&ProrationContractMarketUniform, &shape)).is_empty());
}

/// D-132: an immutable generation is not in the uniformity row set. Without
/// this, one cutover would permanently freeze the market's cycle clock and
/// every later publish would fail on a row nobody can fix.
#[test]
fn a_grandfathered_generation_diverging_from_the_current_rows_publishes() {
    let shape = shape_of(vec![
        priced(
            0xd7,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            Some(baseline_contract()),
        ),
        priced(
            0xd8,
            "EUR",
            "eu",
            PriceEligibility::ExistingGrandfathered,
            Some(contract(
                BillingAnchorPolicy::SubscriptionStart,
                ProrationBasis::BySecond,
                true,
            )),
        ),
    ]);

    assert!(codes(&run(&ProrationContractMarketUniform, &shape)).is_empty());
}

/// `new_subscriptions_only` **is** in the row set — D-123 names the two
/// eligibility classes it covers, and this is the second of them.
#[test]
fn a_new_subscriptions_only_row_is_inside_the_uniformity_set() {
    let shape = shape_of(vec![
        priced(
            0xd9,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            Some(baseline_contract()),
        ),
        priced(
            0xda,
            "EUR",
            "eu",
            PriceEligibility::NewSubscriptionsOnly,
            Some(contract(
                BillingAnchorPolicy::SubscriptionStart,
                ProrationBasis::CalendarDaysActual,
                false,
            )),
        ),
    ]);

    assert_eq!(
        codes(&run(&ProrationContractMarketUniform, &shape)),
        [PRORATION_CONTRACT_MIXED_MARKET]
    );
}

/// All three members are compared, and the finding names which one diverged.
#[test]
fn the_basis_and_the_credit_flag_are_compared_too_and_the_finding_names_the_field() {
    for (moved, field) in [
        (
            contract(
                BillingAnchorPolicy::CalendarMonth,
                ProrationBasis::BySecond,
                false,
            ),
            "prorationBasis",
        ),
        (
            contract(
                BillingAnchorPolicy::CalendarMonth,
                ProrationBasis::CalendarDaysActual,
                true,
            ),
            "creditOnDowngrade",
        ),
    ] {
        let shape = shape_of(vec![
            priced(
                0xe1,
                "EUR",
                "eu",
                PriceEligibility::AllSubscriptions,
                Some(baseline_contract()),
            ),
            priced(
                0xe2,
                "EUR",
                "eu",
                PriceEligibility::AllSubscriptions,
                Some(moved),
            ),
        ]);

        let report = run(&ProrationContractMarketUniform, &shape);
        assert_eq!(codes(&report), [PRORATION_CONTRACT_MIXED_MARKET]);
        assert!(
            report.violations[0].detail.contains(field),
            "{field} diverged: {}",
            report.violations[0].detail
        );
    }
}

/// A row with no contract at all is `inst-pi-required`'s finding, not this
/// one's: reporting the same row twice under two codes would make an author
/// remediate one omission in two places.
#[test]
fn an_absent_contract_is_not_a_divergence() {
    let shape = shape_of(vec![
        priced(
            0xe3,
            "EUR",
            "eu",
            PriceEligibility::AllSubscriptions,
            Some(baseline_contract()),
        ),
        priced(0xe4, "EUR", "eu", PriceEligibility::AllSubscriptions, None),
    ]);

    assert!(codes(&run(&ProrationContractMarketUniform, &shape)).is_empty());
}

// ---------------------------------------------------------------------------
// `inst-pc-targets` / `inst-pc-mutual` / `inst-pc-rank` / `inst-pc-failsafe`
// (§3 Plan-Change Contract, §5, K4, D-23, D-24, D-54).
// ---------------------------------------------------------------------------

fn other(n: u128) -> Uuid {
    Uuid::from_u128(0xf000 + n)
}

/// A plan shape carrying `contract` and no rows.
fn shape_with(contract: PlanChangeContract) -> PlanShape {
    let mut shape = PlanShape::new(plan(), 1, now());
    shape.change_contract = contract;
    shape
}

/// An index in which each `(plan, rank)` is published, and `inbound` point here.
fn index(published: &[(Uuid, Option<i32>)], inbound: &[Uuid]) -> ChangeTargetIndex {
    ChangeTargetIndex::new(
        published.iter().copied().collect(),
        inbound.iter().copied().collect(),
    )
}

fn graph(index: ChangeTargetIndex) -> ChangeGraphAuthorable {
    ChangeGraphAuthorable { index }
}

#[test]
fn the_change_graph_codes_are_the_designs_verbatim() {
    assert_eq!(CHANGE_TARGET_UNPUBLISHED, "CHANGE_TARGET_UNPUBLISHED");
    assert_eq!(COMPARABILITY_RANK_REQUIRED, "COMPARABILITY_RANK_REQUIRED");
    assert_eq!(COMPARABILITY_RANK_REVOKED, "COMPARABILITY_RANK_REVOKED");
}

/// `inst-pc-failsafe`: absence is a value with a reading, and it is the one
/// state this rule has nothing to say about.
#[test]
fn a_plan_offering_no_self_service_change_is_publishable_with_no_rank() {
    let shape = shape_with(PlanChangeContract::default());

    assert!(codes(&run(&graph(index(&[], &[])), &shape)).is_empty());
}

/// An author who clears their last edge is leaving self-service change, and K4
/// must not refuse the publish that does it.
#[test]
fn an_empty_edge_list_is_leaving_self_service_change_not_entering_it() {
    let shape = shape_with(PlanChangeContract {
        allowed_change_targets: Some(Vec::new()),
        comparability_rank: None,
        usage_counter_on_plan_change: UsageCounterOnPlanChange::Reset,
    });

    assert!(codes(&run(&graph(index(&[], &[])), &shape)).is_empty());
}

#[test]
fn a_dangling_target_fails_publish_naming_the_target() {
    let shape = shape_with(PlanChangeContract {
        allowed_change_targets: Some(vec![other(1)]),
        comparability_rank: Some(10),
        usage_counter_on_plan_change: UsageCounterOnPlanChange::Reset,
    });

    let report = run(&graph(index(&[], &[])), &shape);

    assert_eq!(codes(&report), [CHANGE_TARGET_UNPUBLISHED]);
    assert!(
        report.violations[0].detail.contains(&other(1).to_string()),
        "{}",
        report.violations[0].detail
    );
}

/// K4, the source half: a plan that names edges owes a rank.
#[test]
fn a_source_plan_with_edges_and_no_rank_fails_publish() {
    let shape = shape_with(PlanChangeContract {
        allowed_change_targets: Some(vec![other(1)]),
        comparability_rank: None,
        usage_counter_on_plan_change: UsageCounterOnPlanChange::Reset,
    });

    assert_eq!(
        codes(&run(&graph(index(&[(other(1), Some(5))], &[])), &shape)),
        [COMPARABILITY_RANK_REQUIRED]
    );
}

/// `inst-pc-mutual`, the target half: without the target's rank the runtime
/// classification A→B is uncomputable, so publish refuses the edge.
#[test]
fn a_published_target_without_a_rank_fails_publish() {
    let shape = shape_with(PlanChangeContract {
        allowed_change_targets: Some(vec![other(1)]),
        comparability_rank: Some(10),
        usage_counter_on_plan_change: UsageCounterOnPlanChange::Reset,
    });

    let report = run(&graph(index(&[(other(1), None)], &[])), &shape);

    assert_eq!(codes(&report), [COMPARABILITY_RANK_REQUIRED]);
    assert!(
        report.violations[0].detail.contains(&other(1).to_string()),
        "the target is named: {}",
        report.violations[0].detail
    );
}

#[test]
fn a_whole_edge_publishes_when_both_ends_carry_a_rank() {
    let shape = shape_with(PlanChangeContract {
        allowed_change_targets: Some(vec![other(1), other(2)]),
        comparability_rank: Some(10),
        usage_counter_on_plan_change: UsageCounterOnPlanChange::Carry,
    });

    let idx = index(&[(other(1), Some(5)), (other(2), Some(20))], &[]);
    assert!(codes(&run(&graph(idx), &shape)).is_empty());
}

/// D-54's reverse guard. Without it a rank-less re-publish leaves
/// already-published inbound edges unclassifiable at read time — the same
/// read-time drift D-23 cut rule-based targets to avoid.
#[test]
fn dropping_the_rank_while_inbound_edges_reference_the_plan_is_refused() {
    let shape = shape_with(PlanChangeContract::default());

    let report = run(&graph(index(&[], &[other(7), other(8)])), &shape);

    assert_eq!(codes(&report), [COMPARABILITY_RANK_REVOKED]);
    let detail = &report.violations[0].detail;
    for id in [other(7), other(8)] {
        assert!(
            detail.contains(&id.to_string()),
            "the referencing plans are enumerated: {detail}"
        );
    }
}

/// The legitimate route D-54 leaves open: remove the inbound edges first, and
/// the rank may then go.
#[test]
fn a_rank_less_republish_with_no_inbound_edges_publishes() {
    let shape = shape_with(PlanChangeContract::default());

    assert!(codes(&run(&graph(index(&[], &[])), &shape)).is_empty());
}

/// A plan that still carries its rank may be pointed at by anyone.
#[test]
fn keeping_the_rank_satisfies_the_reverse_guard() {
    let shape = shape_with(PlanChangeContract {
        allowed_change_targets: None,
        comparability_rank: Some(3),
        usage_counter_on_plan_change: UsageCounterOnPlanChange::Reset,
    });

    assert!(codes(&run(&graph(index(&[], &[other(7)])), &shape)).is_empty());
}

/// Every offending edge is reported, not the first.
#[test]
fn every_dangling_target_is_named() {
    let shape = shape_with(PlanChangeContract {
        allowed_change_targets: Some(vec![other(1), other(2)]),
        comparability_rank: Some(10),
        usage_counter_on_plan_change: UsageCounterOnPlanChange::Reset,
    });

    // The **codes**, not merely the count. A count assertion cannot tell
    // `CHANGE_TARGET_UNPUBLISHED` from the `COMPARABILITY_RANK_REQUIRED` an
    // unpublished target also produces -- measured by probing the dangling arm
    // and watching this case stay green while its sibling reddened.
    assert_eq!(
        codes(&run(&graph(index(&[], &[])), &shape)),
        [CHANGE_TARGET_UNPUBLISHED, CHANGE_TARGET_UNPUBLISHED],
        "two dangling edges are two findings, both of them this code"
    );
}
