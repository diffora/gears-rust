//! What the aggregate run promises: one report, every finding, the same answer
//! twice.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use super::{
    GRANDFATHER_UNTIL_FORBIDDEN, PLAN_SIZE_SOFT_CAP_EXCEEDED, PublishRuleParams,
    ROUNDING_POLICY_UNRESOLVED, SoftSizeCaps, run_publish_rules,
};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::plan_rules::{
    CustomIntervalBounds, DescriptorSetComplete, HYBRID_INCOMPLETE, INVALID_CUSTOM_INTERVAL,
    PHASE_GRAPH_INVALID, PLANTIER_MISSING,
};
use crate::domain::plan_shape::{BillingCycle, Frequency, PlanShape};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{ModelKind, PriceRow};
use crate::domain::rules::MODEL_KIND_MISSING;
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x91a4))
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 3, 12, 0, 0)
        .single()
        .expect("the fixed instant is unambiguous")
}

fn params(default_rounding_policy: Option<&str>) -> PublishRuleParams {
    PublishRuleParams::new(
        // The ratified launch caps, not zeros: a zero cap rejects every custom
        // frequency ever authored and would exercise that state instead.
        CustomIntervalBounds::new(366, 24),
        DescriptorSetComplete::default(),
        default_rounding_policy.map(ToOwned::to_owned),
        // Likewise the ratified soft caps (100 bands/row, 500 rows/plan) rather
        // than zeros, which would advise on every plan in this file.
        SoftSizeCaps::new(100, 500),
    )
}

fn record(price_id: u128, model_kind: Option<ModelKind>, rounding: Option<&str>) -> PriceRecord {
    let scope_key = ScopeKey::new(
        plan(),
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new("eu").expect("non-blank"),
        PhaseId::new(Uuid::from_u128(0xf1)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("all_subscriptions pairs with cohort none");

    let mut row = PriceRow::new(ChargeKind::Recurring, model_kind);
    row.amount_minor = Some(MinorAmount::new(1000).expect("non-negative"));

    PriceRecord {
        price_id: Uuid::from_u128(price_id),
        scope_key,
        row,
        tax_inclusive: false,
        billing_timing: None,
        rounding_policy_ref: rounding.map(ToOwned::to_owned),
        grandfather_until: None,
        supersedes_price_id: None,
        lifecycle_state: LifecycleState::Draft,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: now(),
        row_version: RowVersion::new(0),
    }
}

fn codes(report: &crate::domain::validation::ValidationReport) -> Vec<String> {
    report
        .violations
        .iter()
        .map(|violation| violation.code.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// The aggregate run.
// ---------------------------------------------------------------------------

#[test]
fn a_row_shape_fault_and_a_plan_shape_fault_appear_in_one_report_in_the_fixed_order() {
    // The two halves of the contract at once: the report is aggregate, and the
    // row findings come first because a malformed row is malformed regardless
    // of the plan it sits in.
    let mut shape = PlanShape::new(plan(), 1, now());
    shape.billing_cycle = Some(BillingCycle::Recurring);
    // `inst-cs-declared` (D-149): a plan that recurs owes a
    // frequency. These fixtures used to omit it and pass, which is
    // the vacuous pass that rule exists to close.
    shape.frequency = Some(Frequency::Monthly);
    // No model kind: a Slice-3 row fault.
    shape.rows = vec![record(0xb001, None, Some("half_up"))];
    // No plan tier, no phases, no descriptor set: Slice-2 plan faults.

    let report = run_publish_rules(&shape, &params(Some("half_up")));
    let found = codes(&report);

    assert!(!report.is_publishable());
    assert_eq!(
        found.first().map(String::as_str),
        Some(MODEL_KIND_MISSING),
        "the row half of the report comes first"
    );
    assert!(found.iter().any(|code| code == PLANTIER_MISSING));
    assert!(found.iter().any(|code| code == PHASE_GRAPH_INVALID));
}

#[test]
fn one_awful_plan_produces_every_expected_violation_rather_than_the_first() {
    // The fail-closed contract's whole point: an author who submits a plan that
    // is wrong in several ways is told about all of them and remediates in one
    // pass. A run that stopped at the first would still block the publish and
    // would still look correct from outside.
    let mut shape = PlanShape::new(plan(), 1, now());
    shape.billing_cycle = Some(BillingCycle::Hybrid);
    // `inst-cs-declared` (D-149): a plan that recurs owes a
    // frequency. These fixtures used to omit it and pass, which is
    // the vacuous pass that rule exists to close.
    shape.frequency = Some(Frequency::Monthly);
    shape.rows = vec![record(0xb001, None, None), record(0xb002, None, None)];

    let report = run_publish_rules(&shape, &params(None));
    let found = codes(&report);

    for expected in [
        MODEL_KIND_MISSING,
        ROUNDING_POLICY_UNRESOLVED,
        HYBRID_INCOMPLETE,
        PLANTIER_MISSING,
        PHASE_GRAPH_INVALID,
    ] {
        assert!(
            found.iter().any(|code| code == expected),
            "{expected} must be in the aggregate report; got {found:?}"
        );
    }
    // Both rows are reported, not just the first one found.
    assert_eq!(
        found
            .iter()
            .filter(|code| *code == MODEL_KIND_MISSING)
            .count(),
        2
    );
}

#[test]
fn a_clean_plan_yields_an_empty_report() {
    let report = run_publish_rules(&clean_plan(), &params(Some("half_up")));

    assert_eq!(codes(&report), Vec::<String>::new());
    assert!(report.is_publishable());
}

#[test]
fn the_same_subject_run_twice_yields_byte_identical_reports() {
    // The property §4.2's two runs depend on, and the reason no rule may hold
    // state: approval approves content and the commit re-validates state, and
    // the two are only comparable if the rule set is a pure function of what it
    // is handed.
    let shape = clean_plan();
    let params = params(None);

    assert_eq!(
        run_publish_rules(&shape, &params),
        run_publish_rules(&shape, &params)
    );
}

// ---------------------------------------------------------------------------
// The Foundation's own rounding rule.
// ---------------------------------------------------------------------------

#[test]
fn a_row_with_neither_its_own_policy_nor_a_tenant_default_is_unpublishable() {
    let mut shape = clean_plan();
    shape.rows = vec![record(0xb001, Some(ModelKind::Flat), None)];

    let report = run_publish_rules(&shape, &params(None));

    assert_eq!(codes(&report), [ROUNDING_POLICY_UNRESOLVED]);
    assert_eq!(
        report.violations[0].subject,
        Uuid::from_u128(0xb001).to_string(),
        "the finding names the row the author has to edit"
    );
}

#[test]
fn a_row_carrying_its_own_policy_resolves() {
    let mut shape = clean_plan();
    shape.rows = vec![record(0xb001, Some(ModelKind::Flat), Some("half_up"))];

    assert!(run_publish_rules(&shape, &params(None)).is_publishable());
}

#[test]
fn a_tenant_default_resolves_every_row_at_once() {
    let mut shape = clean_plan();
    shape.rows = vec![
        record(0xb001, Some(ModelKind::Flat), None),
        record(0xb002, Some(ModelKind::Flat), None),
    ];

    assert!(run_publish_rules(&shape, &params(Some("bankers"))).is_publishable());
}

#[test]
fn every_unresolved_row_is_reported_and_not_only_the_first() {
    let mut shape = clean_plan();
    shape.rows = vec![
        record(0xb001, Some(ModelKind::Flat), None),
        record(0xb002, Some(ModelKind::Flat), Some("half_up")),
        record(0xb003, Some(ModelKind::Flat), None),
    ];

    let report = run_publish_rules(&shape, &params(None));

    assert_eq!(
        codes(&report),
        [ROUNDING_POLICY_UNRESOLVED, ROUNDING_POLICY_UNRESOLVED]
    );
}

#[test]
fn the_rounding_code_is_spelled_as_the_design_set_spells_it() {
    // Transcribed from `01-foundation.md` §3.3, not from the identifier: a
    // constant whose value drifted from its name would still compile and would
    // still be wrong on the wire.
    assert_eq!(ROUNDING_POLICY_UNRESOLVED, "ROUNDING_POLICY_UNRESOLVED");
}

// ---------------------------------------------------------------------------
// A plan the whole set passes, so the tests above vary one thing at a time.
// ---------------------------------------------------------------------------

fn clean_plan() -> PlanShape {
    use crate::domain::plan_shape::{DescriptorSet, PhaseGraph, PhaseKind, PlanPhase};

    let terminal = PhaseId::new(Uuid::from_u128(0xf1));
    let mut shape = PlanShape::new(plan(), 1, now());
    shape.billing_cycle = Some(BillingCycle::Recurring);
    // `inst-cs-declared` (D-149): a plan that recurs owes a
    // frequency. These fixtures used to omit it and pass, which is
    // the vacuous pass that rule exists to close.
    shape.frequency = Some(Frequency::Monthly);
    shape.plan_tier = Some("standard".to_owned());
    shape.phases = PhaseGraph::new(vec![PlanPhase {
        phase_id: terminal,
        kind: PhaseKind::Evergreen,
        ordinal: 0,
        converts_to_phase_id: None,
        phase_duration_days: None,
        display_trial_days: None,
    }]);
    shape.descriptor_set = Some(DescriptorSet {
        invoice_line_template: Some("{plan}".to_owned()),
        gl_code: Some("4000".to_owned()),
        itemization_rule: Some("per_charge".to_owned()),
        additional: std::collections::BTreeMap::new(),
    });
    shape.rows = vec![record(0xb001, Some(ModelKind::Flat), Some("half_up"))];
    shape
}

// ---------------------------------------------------------------------------
// nfr-size-limits — the soft caps, advisory (D-160)
// ---------------------------------------------------------------------------

/// `params`, with the two soft caps set to whatever a case needs.
fn params_capped(bands: u32, rows: u32) -> PublishRuleParams {
    PublishRuleParams::new(
        CustomIntervalBounds::new(366, 24),
        DescriptorSetComplete::default(),
        Some("half_up".to_owned()),
        SoftSizeCaps::new(bands, rows),
    )
}

fn advisories(report: &crate::domain::validation::ValidationReport) -> Vec<String> {
    report
        .warnings
        .iter()
        .map(|advisory| advisory.code.clone())
        .collect()
}

#[test]
fn a_plan_above_the_row_cap_publishes_with_an_advisory_naming_the_cap_and_the_count() {
    // PRD section 14 ratifies both caps as a SHOULD emitting a publish warning.
    // Making them blocking so they could reuse a violation code is D-160's
    // rejected option (b) - and it would turn a raised-then-lowered cap into a
    // published plan its own tenant cannot re-publish.
    let mut shape = clean_plan();
    shape.rows = (0..3)
        .map(|n| record(0xb100 + n, Some(ModelKind::Flat), Some("half_up")))
        .collect();

    let report = run_publish_rules(&shape, &params_capped(100, 2));

    assert!(report.is_publishable(), "an advisory never blocks");
    assert_eq!(codes(&report), Vec::<String>::new());
    assert_eq!(advisories(&report), vec![PLAN_SIZE_SOFT_CAP_EXCEEDED]);
    let detail = &report.warnings[0].detail;
    assert!(detail.contains('3'), "the count is named: {detail}");
    assert!(detail.contains('2'), "and the limit: {detail}");
}

#[test]
fn a_row_above_the_band_cap_does_the_same_and_names_the_row() {
    let mut shape = clean_plan();
    let mut row = record(0xb200, Some(ModelKind::Graduated), Some("half_up"));
    row.row.bands = vec![
        crate::domain::price_row::TierBand::closed(0, 10, MinorAmount::new(500).expect("amount")),
        crate::domain::price_row::TierBand::closed(10, 20, MinorAmount::new(400).expect("amount")),
        crate::domain::price_row::TierBand::open(20, MinorAmount::new(300).expect("amount")),
    ];
    shape.rows = vec![row];

    let report = run_publish_rules(&shape, &params_capped(2, 500));

    // The row's own Slice-3 shape is not this test's subject - what matters is
    // that the size finding is an advisory and not among the violations.
    assert!(
        !codes(&report).contains(&PLAN_SIZE_SOFT_CAP_EXCEEDED.to_owned()),
        "the cap never blocks: {:?}",
        codes(&report)
    );
    assert_eq!(advisories(&report), vec![PLAN_SIZE_SOFT_CAP_EXCEEDED]);
    assert!(
        report.warnings[0]
            .subject
            .contains(&Uuid::from_u128(0xb200).to_string()),
        "the row is named: {}",
        report.warnings[0].subject
    );
}

#[test]
fn a_tenant_with_a_raised_cap_gets_no_advisory_at_the_same_size() {
    // The per-tenant read, and what D-152 bought: the value in force is the
    // authoring tenant's, so the same plan advises under one policy and not
    // under another. A rule that read the deployment default would make the
    // per-tenant carrier unobservable.
    let mut shape = clean_plan();
    shape.rows = (0..3)
        .map(|n| record(0xb100 + n, Some(ModelKind::Flat), Some("half_up")))
        .collect();

    assert_eq!(
        advisories(&run_publish_rules(&shape, &params_capped(100, 2))),
        vec![PLAN_SIZE_SOFT_CAP_EXCEEDED]
    );
    assert!(
        advisories(&run_publish_rules(&shape, &params_capped(100, 500))).is_empty(),
        "a raised cap silences it at the same size"
    );
}

#[test]
fn a_plan_at_the_cap_exactly_is_not_above_it() {
    // Both boundaries, on the side a `>` and a `>=` differ: the cap is a limit
    // the plan may reach.
    let mut shape = clean_plan();
    shape.rows = (0..2)
        .map(|n| record(0xb100 + n, Some(ModelKind::Flat), Some("half_up")))
        .collect();

    assert!(advisories(&run_publish_rules(&shape, &params_capped(100, 2))).is_empty());
}

#[test]
fn the_hard_interval_caps_still_block_and_produce_no_advisory() {
    // Section 1.2's allocation row had blurred all four caps under one heading and
    // D-160 unblurs them: the two interval caps are HARD and keep
    // `INVALID_CUSTOM_INTERVAL`. A soft advisory reaching them would tell an
    // author their over-cap interval published.
    let mut shape = clean_plan();
    shape.frequency = Some(crate::domain::plan_shape::Frequency::CustomEveryN {
        n: 400,
        unit: crate::domain::plan_shape::CustomIntervalUnit::Days,
    });

    let report = run_publish_rules(&shape, &params_capped(100, 500));

    assert!(!report.is_publishable(), "a hard cap blocks");
    assert!(
        codes(&report).contains(&INVALID_CUSTOM_INTERVAL.to_owned()),
        "{:?}",
        codes(&report)
    );
    assert!(
        advisories(&report).is_empty(),
        "and produces no advisory: {:?}",
        advisories(&report)
    );
}

// ---------------------------------------------------------------------------
// inst-el-fields — the horizon belongs to its class (D-147)
// ---------------------------------------------------------------------------

/// One record filed under `class`, carrying a grandfathering horizon.
fn horizoned(price_id: u128, class: PriceEligibility) -> PriceRecord {
    let mut record = record(price_id, Some(ModelKind::Flat), Some("half_up"));
    record.scope_key = ScopeKey::new(
        plan(),
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new("eu").expect("non-blank"),
        PhaseId::new(Uuid::from_u128(0xf1)),
        class,
        ChargeKind::Recurring,
        match class {
            // The cohort biconditional (D-147's sibling): a grandfathered class
            // pairs with a generation, and every other class with none.
            PriceEligibility::ExistingGrandfathered => Cohort::Generation(now()),
            _ => Cohort::None,
        },
    )
    .expect("the class pairs with its cohort");
    record.grandfather_until = Some(now());
    record
}

#[test]
fn a_horizon_on_a_non_grandfathered_row_is_refused_from_the_publish_path() {
    // The half D-147 left unbuilt. The repository already refuses this on both
    // authoring paths; what was missing is the report line, so a publish's 422
    // enumerates it beside every other violation instead of the first authoring
    // write happening to have caught it.
    let mut shape = clean_plan();
    shape.rows = vec![horizoned(0xb301, PriceEligibility::AllSubscriptions)];

    let report = run_publish_rules(&shape, &params(Some("half_up")));

    assert!(!report.is_publishable());
    assert!(
        codes(&report).contains(&GRANDFATHER_UNTIL_FORBIDDEN.to_owned()),
        "{:?}",
        codes(&report)
    );
    let violation = report
        .violations
        .iter()
        .find(|v| v.code == GRANDFATHER_UNTIL_FORBIDDEN)
        .expect("the finding");
    assert!(
        violation
            .subject
            .contains(&Uuid::from_u128(0xb301).to_string()),
        "the row is named: {}",
        violation.subject
    );
}

#[test]
fn a_horizon_on_a_grandfathered_row_is_exactly_what_the_field_is_for() {
    let mut shape = clean_plan();
    shape.rows = vec![horizoned(0xb302, PriceEligibility::ExistingGrandfathered)];

    let report = run_publish_rules(&shape, &params(Some("half_up")));

    assert!(
        !codes(&report).contains(&GRANDFATHER_UNTIL_FORBIDDEN.to_owned()),
        "{:?}",
        codes(&report)
    );
}

#[test]
fn the_horizon_finding_appears_in_the_same_report_as_another_rows_violation() {
    // The enumerate-all contract. Before this rule, a plan with a misfiled
    // horizon and a shape fault elsewhere reported the shape fault and left the
    // horizon to whichever authoring write reached the store first.
    let mut shape = clean_plan();
    shape.rows = vec![
        horizoned(0xb301, PriceEligibility::NewSubscriptionsOnly),
        // No model kind: a Slice-3 row fault on a different row.
        record(0xb303, None, Some("half_up")),
    ];

    let found = codes(&run_publish_rules(&shape, &params(Some("half_up"))));

    assert!(
        found.contains(&GRANDFATHER_UNTIL_FORBIDDEN.to_owned()),
        "{found:?}"
    );
    assert!(found.contains(&MODEL_KIND_MISSING.to_owned()), "{found:?}");
}

#[test]
fn the_row_type_gained_no_field_and_the_evaluation_policy_roster_did_not_move() {
    // D-162's boundary, asserted rather than trusted. `grandfather_until` lives
    // on `PriceRecord`; reaching it by widening `PriceRow` would move a field
    // across the roster boundary and imply a generation bump for something that
    // is not evaluation policy at all.
    let row = crate::domain::price_row::PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    let (rostered, outside) = crate::domain::evaluation_policy::partition_row_fields(&row);
    assert!(
        !rostered.iter().any(|field| field.contains("grandfather")),
        "the horizon is not an evaluation-policy field: {rostered:?}"
    );
    assert!(
        !outside.iter().any(|field| field.contains("grandfather")),
        "nor a PriceRow field at all: {outside:?}"
    );
    assert_eq!(
        crate::domain::evaluation_policy::EVALUATION_POLICY_GENERATION,
        "ep-1",
        "so the generation stands"
    );
}
