//! What the aggregate run promises: one report, every finding, the same answer
//! twice.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use super::{PublishRuleParams, ROUNDING_POLICY_UNRESOLVED, run_publish_rules};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::plan_rules::{
    CustomIntervalBounds, DescriptorSetComplete, HYBRID_INCOMPLETE, PHASE_GRAPH_INVALID,
    PLANTIER_MISSING,
};
use crate::domain::plan_shape::{BillingCycle, PlanShape};
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
