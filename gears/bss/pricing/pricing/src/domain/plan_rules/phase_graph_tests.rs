//! Tests for the phase-schedule rules.
//!
//! Every rule is judged **alone**, through [`judge`], because the ten of them
//! overlap by design: a two-terminal graph is both an integrity fault and a
//! nonlinear chain, and a row on an unattached phase is both `PHASE_ROW_ORPHANED`
//! and — through the phase it leaves uncovered — `PHASE_UNCOVERED`. A suite that
//! ran the whole pipeline could not say which rule caught what. Each arm below is
//! written so that deleting the arm from the rule turns the test red - a rule with
//! many arms and a suite proving only the happy path proves nothing at all.
//!
//! The fixtures are deliberately *clean* in every dimension a test is not
//! attacking: `linear_chain` is duration-correct, ordinal-correct and
//! evergreen-terminal, so a violation a test asserts can only have come from the
//! one thing it broke.

use bss_fixtures::ModelKind;
use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use super::{
    DisplayTrialDaysOnTrialPhase, PhaseChainLinear, PhaseCoverage, PhaseDuration,
    PhaseGraphIntegrity, PhaseOverrideBase, PhaseOverrideUnits, RowPhaseAttached,
    TerminalPhaseKind, TerminalPhaseStable,
};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount, RateMinor};
use crate::domain::plan_rules::{
    DISPLAY_TRIAL_DAYS_INVALID, PHASE_CHAIN_NONLINEAR, PHASE_DURATION_INVALID, PHASE_GRAPH_INVALID,
    PHASE_IN_USE, PHASE_OVERRIDE_ORPHANED, PHASE_OVERRIDE_UNIT_MISMATCH, PHASE_ROW_ORPHANED,
    PHASE_UNCOVERED, TERMINAL_PHASE_CHANGED, TERMINAL_PHASE_KIND_INVALID,
};
use crate::domain::plan_shape::{
    BillingCycle, PhaseGraph, PhaseKind, PlanPhase, PlanShape, PublishedBaseline,
};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{
    AggregationFunction, AggregationGranularity, BillingGranularity, PriceRow,
    TierAggregationWindow, TierBand, TierQualificationWindow,
};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use crate::domain::validation::{ValidationReport, ValidationRule, Violation};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const TRIAL: u128 = 0x10;
const INTRO: u128 = 0x20;
const EVERGREEN: u128 = 0x30;
/// A phase id no fixture ever attaches — the id the stand's rows already named
/// (D-337).
const GHOST: u128 = 0x60;
const METER: &str = "egress_bytes";

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x91a4))
}

fn phase_id(seed: u128) -> PhaseId {
    PhaseId::new(Uuid::from_u128(seed))
}

fn currency(code: &str) -> CurrencyCode {
    CurrencyCode::new(code).expect("test currency is three letters")
}

fn region(value: &str) -> Region {
    Region::new(value).expect("test region is non-blank")
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 3, 12, 0, 0)
        .single()
        .expect("the fixed instant is unambiguous")
}

fn minor(units: i64) -> MinorAmount {
    MinorAmount::new(units).expect("test amount is non-negative")
}

/// A band rate, stated in whole minor units so these cases read as they
/// always did (D-311). The stored scale is 10^-9 of one.
fn rate(minor_units: i64) -> RateMinor {
    RateMinor::from_minor_units(minor_units).expect("test rate is non-negative")
}

/// A phase. A non-terminal one carries the duration `inst-ph-duration` requires,
/// so every graph fixture is duration-clean until a test breaks it on purpose.
fn phase(seed: u128, kind: PhaseKind, ordinal: i32, converts_to: Option<PhaseId>) -> PlanPhase {
    PlanPhase {
        phase_id: phase_id(seed),
        kind,
        ordinal,
        converts_to_phase_id: converts_to,
        phase_duration_days: converts_to.is_some().then_some(14),
        display_trial_days: None,
    }
}

/// `trial -> intro -> evergreen`, the shape the slice is written about.
fn linear_chain() -> Vec<PlanPhase> {
    vec![
        phase(TRIAL, PhaseKind::Trial, 0, Some(phase_id(INTRO))),
        phase(INTRO, PhaseKind::Intro, 1, Some(phase_id(EVERGREEN))),
        phase(EVERGREEN, PhaseKind::Evergreen, 2, None),
    ]
}

fn shape_of(phases: Vec<PlanPhase>) -> PlanShape {
    let mut shape = PlanShape::new(plan(), 3, now());
    shape.phases = PhaseGraph::new(phases);
    shape
}

fn phased() -> PlanShape {
    shape_of(linear_chain())
}

fn record(
    charge_kind: ChargeKind,
    code: &str,
    market: &str,
    on_phase: PhaseId,
    row: PriceRow,
) -> PriceRecord {
    let scope_key = ScopeKey::new(
        plan(),
        currency(code),
        region(market),
        on_phase,
        PriceEligibility::AllSubscriptions,
        charge_kind,
        Cohort::None,
    )
    .expect("all_subscriptions pairs with cohort none");

    PriceRecord {
        price_id: Uuid::from_u128(0xb_1000),
        scope_key,
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: None,
        proration_contract: None,
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
        lifecycle_state: LifecycleState::Draft,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: now(),
        row_version: RowVersion::new(0),
    }
}

fn recurring(code: &str, market: &str, on_phase: PhaseId) -> PriceRecord {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(minor(2_500));
    record(ChargeKind::Recurring, code, market, on_phase, row)
}

/// A graduated metered row on `meter`, denominated `per_hour` over the calendar
/// month - the denomination the override tests move against.
fn usage_tuned(
    code: &str,
    market: &str,
    on_phase: PhaseId,
    meter: &str,
    tune: impl FnOnce(&mut PriceRow),
) -> PriceRecord {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some(meter.to_owned());
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = vec![
        TierBand::closed(0, 1_000, rate(10)),
        TierBand::open(1_000, rate(6)),
    ];
    tune(&mut row);
    record(ChargeKind::Usage, code, market, on_phase, row)
}

fn usage(code: &str, market: &str, on_phase: PhaseId, meter: &str) -> PriceRecord {
    usage_tuned(code, market, on_phase, meter, |_| {})
}

fn judge(rule: &dyn ValidationRule<PlanShape>, shape: &PlanShape) -> ValidationReport {
    let mut report = ValidationReport::default();
    rule.evaluate(shape, &mut report);
    report
}

/// The single violation a report was expected to carry.
fn only(report: &ValidationReport) -> &Violation {
    assert_eq!(
        report.violations.len(),
        1,
        "expected exactly one violation, got {:?}",
        report.violations
    );
    &report.violations[0]
}

// ---------------------------------------------------------------------------
// The clean chain
// ---------------------------------------------------------------------------

#[test]
fn a_clean_trial_intro_evergreen_chain_passes_every_structural_rule() {
    let subject = phased();

    for rule in [
        &PhaseGraphIntegrity as &dyn ValidationRule<PlanShape>,
        &PhaseChainLinear,
        &TerminalPhaseKind,
        &PhaseDuration,
    ] {
        let report = judge(rule, &subject);
        assert!(
            report.is_publishable(),
            "{} rejected the clean chain: {:?}",
            rule.name(),
            report.violations
        );
    }
}

#[test]
fn every_rule_names_the_instruction_it_implements() {
    // The name is what the pipeline attributes a failing rule by, so it is part
    // of the contract and not a debug string.
    // In the order `plan_shape_rules` registers them, and **every** rule of the
    // module. `DisplayTrialDaysOnTrialPhase` was missing from this list while its
    // own cases below exercised it: a census that skips a rule cannot tell anyone
    // that the rule renamed itself, which is the one thing it is here for.
    let named: Vec<&str> = [
        &PhaseGraphIntegrity as &dyn ValidationRule<PlanShape>,
        &PhaseChainLinear,
        &TerminalPhaseKind,
        &PhaseDuration,
        &DisplayTrialDaysOnTrialPhase,
        &RowPhaseAttached,
        &PhaseCoverage,
        &PhaseOverrideBase,
        &PhaseOverrideUnits,
        &TerminalPhaseStable,
    ]
    .iter()
    .map(|rule| rule.name())
    .collect();

    assert_eq!(
        named,
        vec![
            "inst-ph-graph",
            "inst-ph-graph/linear",
            "inst-ph-graph/terminal-kind",
            "inst-ph-duration",
            "inst-ph-trial",
            // Immediately before `inst-ph-coverage`, and the adjacency is the
            // contract (D-337): the two are exact inverses over one relation, so
            // a report carrying both names them together.
            "inst-ph-row-attached",
            "inst-ph-coverage",
            "inst-ph-usage-invariant",
            "inst-ph-override-units",
            "inst-ph-terminal-stable",
        ]
    );
}

// ---------------------------------------------------------------------------
// PhaseGraphIntegrity
// ---------------------------------------------------------------------------

#[test]
fn a_dangling_conversion_target_fails() {
    let mut phases = linear_chain();
    phases[1].converts_to_phase_id = Some(phase_id(0x99));
    let subject = shape_of(phases);

    let report = judge(&PhaseGraphIntegrity, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_GRAPH_INVALID);
    assert!(violation.subject.contains(&phase_id(INTRO).to_string()));
    assert!(violation.detail.contains(&phase_id(0x99).to_string()));
}

#[test]
fn a_two_cycle_fails_and_names_both_of_its_phases() {
    // The evergreen phase keeps the graph at exactly one terminal, so the only
    // thing this shape breaks is acyclicity.
    let subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Trial, 0, Some(phase_id(INTRO))),
        phase(INTRO, PhaseKind::Intro, 1, Some(phase_id(TRIAL))),
        phase(EVERGREEN, PhaseKind::Evergreen, 2, None),
    ]);

    let report = judge(&PhaseGraphIntegrity, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_GRAPH_INVALID);
    for on_cycle in [phase_id(TRIAL), phase_id(INTRO)] {
        assert!(
            violation.detail.contains(&on_cycle.to_string()),
            "the cycle members are what the author has to break: {}",
            violation.detail
        );
    }
    assert!(
        !violation.detail.contains(&phase_id(EVERGREEN).to_string()),
        "the terminal phase is not on the cycle and must not be named"
    );
}

#[test]
fn a_phase_set_with_no_terminal_phase_fails() {
    // An empty graph is the shape that isolates the arm: no phase, so no cycle
    // either, and the count is the only thing left to report. Zero terminals is
    // not a state an index can refuse - the partial UNIQUE holds the
    // at-most-one half and the pipeline runs before the write.
    let report = judge(&PhaseGraphIntegrity, &shape_of(Vec::new()));
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_GRAPH_INVALID);
    assert!(violation.detail.contains("no phase"));
}

#[test]
fn a_chain_that_only_cycles_reports_both_the_cycle_and_the_missing_terminal() {
    let subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Trial, 0, Some(phase_id(INTRO))),
        phase(INTRO, PhaseKind::Intro, 1, Some(phase_id(TRIAL))),
    ]);

    let report = judge(&PhaseGraphIntegrity, &subject);

    assert_eq!(report.violations.len(), 2, "{:?}", report.violations);
    assert!(
        report
            .violations
            .iter()
            .all(|violation| violation.code == PHASE_GRAPH_INVALID)
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.detail.contains("no phase")),
        "a cycle with no terminal phase is two faults, not one"
    );
}

#[test]
fn two_terminal_phases_fail_and_are_both_named() {
    let subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Trial, 0, Some(phase_id(INTRO))),
        phase(INTRO, PhaseKind::Evergreen, 1, None),
        phase(EVERGREEN, PhaseKind::Evergreen, 2, None),
    ]);

    let report = judge(&PhaseGraphIntegrity, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_GRAPH_INVALID);
    for terminal in [phase_id(INTRO), phase_id(EVERGREEN)] {
        assert!(violation.detail.contains(&terminal.to_string()));
    }
}

// ---------------------------------------------------------------------------
// PhaseChainLinear
// ---------------------------------------------------------------------------

#[test]
fn a_chain_that_skips_a_phase_in_the_ordinal_order_fails() {
    // trial converts straight to evergreen, over the top of intro. Acyclic,
    // one terminal, every duration authored - and the plan still demands
    // coverage rows for a phase no subscription ever enters.
    let subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Trial, 0, Some(phase_id(EVERGREEN))),
        phase(INTRO, PhaseKind::Intro, 1, Some(phase_id(EVERGREEN))),
        phase(EVERGREEN, PhaseKind::Evergreen, 2, None),
    ]);

    let report = judge(&PhaseChainLinear, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_CHAIN_NONLINEAR);
    assert!(violation.subject.contains(&phase_id(TRIAL).to_string()));
    assert!(violation.detail.contains(&phase_id(INTRO).to_string()));
}

#[test]
fn a_chain_that_branches_over_the_ordinal_order_fails() {
    // Four phases, two of them converting into the last: the second one is the
    // branch, and it is the edge that strands the third.
    let subject = shape_of(vec![
        phase(0x1, PhaseKind::Trial, 0, Some(phase_id(0x2))),
        phase(0x2, PhaseKind::Intro, 1, Some(phase_id(0x4))),
        phase(0x3, PhaseKind::Intro, 2, Some(phase_id(0x4))),
        phase(0x4, PhaseKind::Evergreen, 3, None),
    ]);

    let report = judge(&PhaseChainLinear, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_CHAIN_NONLINEAR);
    assert!(violation.subject.contains(&phase_id(0x2).to_string()));
    assert!(
        violation.detail.contains(&phase_id(0x3).to_string()),
        "the report has to name the phase the edge should have gone to: {}",
        violation.detail
    );
}

#[test]
fn a_dead_phase_after_the_terminal_one_fails() {
    // The unreachable phase the 2026-07-31 fix names. `EVERGREEN` here is a
    // phase nothing converts into, which still demands coverage rows; it is
    // reported through the edge that strands it.
    let subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Trial, 0, Some(phase_id(INTRO))),
        phase(INTRO, PhaseKind::Evergreen, 1, None),
        phase(EVERGREEN, PhaseKind::Evergreen, 2, None),
    ]);

    let report = judge(&PhaseChainLinear, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_CHAIN_NONLINEAR);
    assert!(violation.subject.contains(&phase_id(INTRO).to_string()));
    assert!(violation.detail.contains(&phase_id(EVERGREEN).to_string()));
}

#[test]
fn a_non_terminal_phase_at_the_end_of_the_ordinal_order_fails() {
    let mut phases = linear_chain();
    phases[2].converts_to_phase_id = Some(phase_id(TRIAL));
    let subject = shape_of(phases);

    let report = judge(&PhaseChainLinear, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_CHAIN_NONLINEAR);
    assert!(violation.subject.contains(&phase_id(EVERGREEN).to_string()));
    assert!(violation.detail.contains("terminal"));
}

#[test]
fn a_duplicate_ordinal_fails() {
    // Not a topology fault at all: the edges are a perfect chain. The order is
    // what breaks - two phases share the lowest ordinal, so "the entry phase"
    // names both and D-39's first non-trial phase is undefined again.
    let subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Trial, 0, Some(phase_id(INTRO))),
        phase(INTRO, PhaseKind::Intro, 0, Some(phase_id(EVERGREEN))),
        phase(EVERGREEN, PhaseKind::Evergreen, 2, None),
    ]);

    let report = judge(&PhaseChainLinear, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_CHAIN_NONLINEAR);
    assert!(violation.subject.contains(&phase_id(INTRO).to_string()));
    assert!(violation.detail.contains(&phase_id(TRIAL).to_string()));
}

#[test]
fn the_chain_is_read_in_ordinal_order_and_not_in_authored_order() {
    // The same three phases, authored backwards. Nothing records authoring
    // order, so a rule that judged "first authored" would reject a plan the
    // design set calls correct - and would put setup timing and migration
    // entry on different phases.
    let mut phases = linear_chain();
    phases.reverse();
    let subject = shape_of(phases);

    assert!(judge(&PhaseChainLinear, &subject).is_publishable());
    assert_eq!(
        subject.phases.entry().map(|entry| entry.phase_id),
        Some(phase_id(TRIAL)),
        "the entry phase is the lowest ordinal, which is the last one authored here"
    );
}

// ---------------------------------------------------------------------------
// TerminalPhaseKind
// ---------------------------------------------------------------------------

#[test]
fn a_trial_terminal_phase_fails_and_says_what_to_author_instead() {
    let subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Intro, 0, Some(phase_id(EVERGREEN))),
        phase(EVERGREEN, PhaseKind::Trial, 1, None),
    ]);

    let report = judge(&TerminalPhaseKind, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, TERMINAL_PHASE_KIND_INVALID);
    assert!(violation.subject.contains(&phase_id(EVERGREEN).to_string()));
    assert!(violation.detail.contains("trial"));
    assert!(
        violation.detail.contains("evergreen terminal phase"),
        "the rule owes the authoring answer, not only the refusal: {}",
        violation.detail
    );
}

#[test]
fn an_intro_terminal_phase_fails() {
    // Intro pricing forever is an evergreen terminal phase at the intro price.
    let subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Trial, 0, Some(phase_id(EVERGREEN))),
        phase(EVERGREEN, PhaseKind::Intro, 1, None),
    ]);

    let report = judge(&TerminalPhaseKind, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, TERMINAL_PHASE_KIND_INVALID);
    assert!(violation.detail.contains("intro"));
}

#[test]
fn a_non_evergreen_kind_before_the_terminal_phase_is_legitimate() {
    // The kind column is not terminality: `trial` and `intro` are exactly what
    // the non-terminal phases of a phased plan carry.
    assert!(judge(&TerminalPhaseKind, &phased()).is_publishable());
}

// ---------------------------------------------------------------------------
// PhaseDuration
// ---------------------------------------------------------------------------

#[test]
fn a_non_terminal_phase_without_a_duration_fails() {
    let mut phases = linear_chain();
    phases[0].phase_duration_days = None;
    let subject = shape_of(phases);

    let report = judge(&PhaseDuration, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_DURATION_INVALID);
    assert!(violation.subject.contains(&phase_id(TRIAL).to_string()));
}

#[test]
fn a_zero_day_phase_fails() {
    let mut phases = linear_chain();
    phases[1].phase_duration_days = Some(0);
    let subject = shape_of(phases);

    let report = judge(&PhaseDuration, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_DURATION_INVALID);
    assert!(violation.subject.contains(&phase_id(INTRO).to_string()));
    assert!(violation.detail.contains("zero"));
}

#[test]
fn a_terminal_phase_carrying_a_duration_fails() {
    let mut phases = linear_chain();
    phases[2].phase_duration_days = Some(30);
    let subject = shape_of(phases);

    let report = judge(&PhaseDuration, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_DURATION_INVALID);
    assert!(violation.subject.contains(&phase_id(EVERGREEN).to_string()));
    assert!(violation.detail.contains("30"));
}

// ---------------------------------------------------------------------------
// PhaseCoverage
// ---------------------------------------------------------------------------

#[test]
fn a_phase_with_no_recurring_row_fails_naming_the_phase_and_the_market() {
    let mut subject = phased();
    subject.billing_cycle = Some(BillingCycle::Recurring);
    subject.rows = vec![
        recurring("usd", "US", phase_id(TRIAL)),
        recurring("usd", "US", phase_id(EVERGREEN)),
    ];

    let report = judge(&PhaseCoverage, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_UNCOVERED);
    assert!(violation.subject.contains(&phase_id(INTRO).to_string()));
    assert!(violation.subject.contains("USD"));
    assert!(violation.subject.contains("US"));
}

#[test]
fn the_same_uncovered_phase_on_a_usage_plan_is_outside_the_rule() {
    // The 2026-07-28 scoping fix. A usage-only plan has no recurring row that
    // could ever cover a phase, so the literal reading would block it through
    // its implicit terminal phase - a rule it can never satisfy.
    let mut subject = phased();
    subject.billing_cycle = Some(BillingCycle::Usage);
    subject.rows = vec![usage("usd", "US", phase_id(EVERGREEN), METER)];

    assert!(judge(&PhaseCoverage, &subject).is_publishable());
}

#[test]
fn a_one_time_plan_is_outside_the_rule_and_an_unauthored_cycle_is_not_judged() {
    let mut subject = phased();
    subject.rows = vec![recurring("usd", "US", phase_id(TRIAL))];

    assert!(
        judge(&PhaseCoverage, &subject).is_publishable(),
        "a draft that has not declared its cycle yet is not a coverage fault"
    );

    subject.billing_cycle = Some(BillingCycle::OneTime);
    assert!(judge(&PhaseCoverage, &subject).is_publishable());

    subject.billing_cycle = Some(BillingCycle::Hybrid);
    assert!(
        !judge(&PhaseCoverage, &subject).is_publishable(),
        "hybrid carries a recurring part and is inside the rule"
    );
}

#[test]
fn coverage_is_required_in_every_sold_market_independently() {
    // The plan sells EUR because a usage row prices it there. Covering the
    // phases in USD alone leaves a EUR subscriber's conversion resolving to
    // nothing.
    let mut subject = phased();
    subject.billing_cycle = Some(BillingCycle::Hybrid);
    subject.rows = vec![
        recurring("usd", "US", phase_id(TRIAL)),
        recurring("usd", "US", phase_id(INTRO)),
        recurring("usd", "US", phase_id(EVERGREEN)),
        usage("eur", "EU", phase_id(EVERGREEN), METER),
    ];

    let report = judge(&PhaseCoverage, &subject);

    assert_eq!(report.violations.len(), 3, "{:?}", report.violations);
    assert!(
        report
            .violations
            .iter()
            .all(|violation| violation.code == PHASE_UNCOVERED
                && violation.subject.contains("EUR"))
    );
}

// ---------------------------------------------------------------------------
// RowPhaseAttached (D-337)
// ---------------------------------------------------------------------------

#[test]
fn a_row_on_an_unattached_phase_is_reported_with_the_row_and_the_phase_named() {
    // The stand's shape, in a test: the row keys on a phase nobody attached, and
    // before this rule the plan was refused only for having no terminal phase.
    //
    // Armed against the **detail**, not the count. `PHASE_GRAPH_INVALID` fires on
    // several of the same shapes, so a probe that counted violations would pass
    // with this rule deleted; and the phase id is the operand the author acts on,
    // because a scope key is the row's identity and the only remediations are
    // attaching *that* id or deleting the row.
    let mut subject = phased();
    subject.rows = vec![recurring("usd", "US", phase_id(GHOST))];

    let report = judge(&RowPhaseAttached, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_ROW_ORPHANED);
    assert!(
        violation.subject.contains(&phase_id(GHOST).to_string()),
        "the finding groups under the phase one act would attach: {}",
        violation.subject
    );
    assert!(
        violation
            .detail
            .contains(&Uuid::from_u128(0xb_1000).to_string()),
        "the row is named, because the other remediation is deleting it: {}",
        violation.detail
    );
    assert!(
        violation.detail.contains("does not attach"),
        "the reason is stated rather than left to be inferred: {}",
        violation.detail
    );
}

#[test]
fn a_row_on_an_attached_phase_is_not_reported() {
    // The positive control, without which the refusal above proves only that the
    // rule fires. Every phase of `linear_chain` is exercised: a rule that read
    // the terminal phase alone would pass the first and fail the other two, which
    // is `inst-ph-terminal-stable`'s question and not this one.
    let mut subject = phased();
    for attached in [TRIAL, INTRO, EVERGREEN] {
        subject.rows = vec![recurring("usd", "US", phase_id(attached))];

        assert!(
            judge(&RowPhaseAttached, &subject).violations.is_empty(),
            "a row on the attached phase {attached:#x} is not a finding"
        );
    }
}

#[test]
fn every_stranded_row_is_reported_and_not_only_the_first() {
    // The remediation is per row — attach the phase, or delete *this* row — so a
    // report naming one of several sends the author back once per row. The shape
    // that motivated the rule carried four.
    let mut second = recurring("usd", "US", phase_id(GHOST));
    second.price_id = Uuid::from_u128(0xb_2000);
    let mut subject = phased();
    subject.rows = vec![recurring("usd", "US", phase_id(GHOST)), second];

    let report = judge(&RowPhaseAttached, &subject);

    assert_eq!(report.violations.len(), 2, "{:?}", report.violations);
    assert!(
        report
            .violations
            .iter()
            .all(|violation| violation.code == PHASE_ROW_ORPHANED)
    );
    // Each names its own row, so the two findings are actionable separately.
    assert!(
        report.violations[1]
            .detail
            .contains(&Uuid::from_u128(0xb_2000).to_string()),
        "{}",
        report.violations[1].detail
    );
}

#[test]
fn a_row_is_judged_against_the_phase_set_and_not_against_the_baseline() {
    // Why this is a rule of its own rather than a widening of
    // `inst-ph-terminal-stable` arm (b): that rule's first act is
    // `let Some(baseline) = … else { return }`, so on a plan that has never
    // published it says nothing at all. This subject is exactly that plan.
    let mut subject = phased();
    subject.rows = vec![recurring("usd", "US", phase_id(GHOST))];
    subject.baseline = None;

    assert!(
        judge(&TerminalPhaseStable, &subject).violations.is_empty(),
        "the rule that used to be the only guard is silent on a first publish"
    );
    assert_eq!(
        only(&judge(&RowPhaseAttached, &subject)).code,
        PHASE_ROW_ORPHANED
    );
}

// ---------------------------------------------------------------------------
// PhaseOverrideBase
// ---------------------------------------------------------------------------

#[test]
fn one_phase_invariant_usage_row_satisfies_every_phase() {
    // D-15: the terminal-phase row covers all phases, so a plan with no
    // override at all has nothing for either override rule to say.
    let mut subject = phased();
    subject.rows = vec![usage("usd", "US", phase_id(EVERGREEN), METER)];

    assert!(judge(&PhaseOverrideBase, &subject).is_publishable());
    assert!(judge(&PhaseOverrideUnits, &subject).is_publishable());
}

#[test]
fn an_override_whose_line_has_no_terminal_phase_row_is_orphaned() {
    // The terminal phase prices a different meter, so the trial row overrides
    // nothing: after conversion its line resolves to nothing at all.
    let mut subject = phased();
    subject.rows = vec![
        usage("usd", "US", phase_id(EVERGREEN), "storage_bytes"),
        usage("usd", "US", phase_id(TRIAL), METER),
    ];

    let report = judge(&PhaseOverrideBase, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_OVERRIDE_ORPHANED);
    assert!(violation.subject.contains(&phase_id(TRIAL).to_string()));
    assert!(
        violation.subject.contains(METER),
        "the line is half of what the author has to locate: {}",
        violation.subject
    );
}

#[test]
fn a_dimension_key_makes_a_different_line() {
    // The line is `(meter, dimensionKey)`, so a dimensioned override is not
    // based on the undimensioned row of the same meter.
    let mut subject = phased();
    subject.rows = vec![
        usage("usd", "US", phase_id(EVERGREEN), METER),
        usage_tuned("usd", "US", phase_id(TRIAL), METER, |row| {
            row.dimension_key = "region".to_owned();
        }),
    ];

    assert_eq!(
        only(&judge(&PhaseOverrideBase, &subject)).code,
        PHASE_OVERRIDE_ORPHANED
    );
}

#[test]
fn an_ambiguous_terminal_phase_leaves_both_override_rules_quiet() {
    // With two terminals "the phase-invariant row" is undefined.
    // `PHASE_GRAPH_INVALID` is the fault the author fixes first, and reporting
    // its consequences as further faults costs a remediation round trip.
    let mut subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Trial, 0, Some(phase_id(INTRO))),
        phase(INTRO, PhaseKind::Evergreen, 1, None),
        phase(EVERGREEN, PhaseKind::Evergreen, 2, None),
    ]);
    subject.rows = vec![usage_tuned("usd", "US", phase_id(TRIAL), METER, |row| {
        row.billing_granularity = Some(BillingGranularity::PerDay);
    })];

    assert!(judge(&PhaseOverrideBase, &subject).is_publishable());
    assert!(judge(&PhaseOverrideUnits, &subject).is_publishable());
}

// ---------------------------------------------------------------------------
// PhaseOverrideUnits
// ---------------------------------------------------------------------------

#[test]
fn a_free_trial_override_at_the_same_denomination_publishes() {
    // Free trial usage is an explicit trial-phase row at zero, never a silent
    // default - and it stays fully expressible, because the guard binds the
    // denomination and not the money.
    let mut subject = phased();
    subject.rows = vec![
        usage("usd", "US", phase_id(EVERGREEN), METER),
        usage_tuned("usd", "US", phase_id(TRIAL), METER, |row| {
            row.bands = vec![TierBand::open(0, rate(0))];
        }),
    ];

    assert!(judge(&PhaseOverrideBase, &subject).is_publishable());
    assert!(judge(&PhaseOverrideUnits, &subject).is_publishable());
}

#[test]
fn an_override_at_another_billing_granularity_fails_naming_the_field() {
    // The D-77 factor-of-24 class through the phase axis: an hours-denominated
    // counter continues into day-denominated bands.
    let mut subject = phased();
    subject.rows = vec![
        usage("usd", "US", phase_id(EVERGREEN), METER),
        usage_tuned("usd", "US", phase_id(TRIAL), METER, |row| {
            row.billing_granularity = Some(BillingGranularity::PerDay);
        }),
    ];

    let report = judge(&PhaseOverrideUnits, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_OVERRIDE_UNIT_MISMATCH);
    assert!(violation.detail.contains("billingGranularity"));
    assert!(violation.subject.contains(&phase_id(TRIAL).to_string()));
}

#[test]
fn a_resized_package_override_fails_naming_package_size() {
    // D-122: rating counts blocks by a cumulative ceil-diff that presupposes
    // one block size per window, and the window does not end at the phase.
    let package = |size: u64| {
        move |row: &mut PriceRow| {
            row.model_kind = Some(ModelKind::Package);
            row.bands = Vec::new();
            row.package_size = Some(size);
            row.package_price_minor = Some(minor(500));
        }
    };
    let mut subject = phased();
    subject.rows = vec![
        usage_tuned("usd", "US", phase_id(EVERGREEN), METER, package(100)),
        usage_tuned("usd", "US", phase_id(TRIAL), METER, package(250)),
    ];

    let report = judge(&PhaseOverrideUnits, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_OVERRIDE_UNIT_MISMATCH);
    assert_eq!(
        violation.detail.matches("package_size").count(),
        1,
        "only the resized field is named: {}",
        violation.detail
    );
}

#[test]
fn spelling_out_the_defaults_is_not_a_denomination_change() {
    // "Authored nothing" and "authored the default" are the same row. Reading
    // the raw Options would fail an override that changed nothing at all.
    let mut subject = phased();
    subject.rows = vec![
        usage("usd", "US", phase_id(EVERGREEN), METER),
        usage_tuned("usd", "US", phase_id(TRIAL), METER, |row| {
            row.aggregation_function = Some(AggregationFunction::Sum);
            row.aggregation_granularity = Some(AggregationGranularity::Hour);
            row.tier_qualification_window = Some(TierQualificationWindow::Current);
        }),
    ];

    assert!(judge(&PhaseOverrideUnits, &subject).is_publishable());
}

#[test]
fn the_violation_names_every_one_of_the_seven_shared_fields() {
    // The pin on this side of the shared comparison: an override that moves all
    // seven is reported as all seven. Add a field to
    // `unit_determining_mismatch` and this list is short; remove one and it is
    // long.
    let mut subject = phased();
    subject.rows = vec![
        usage("usd", "US", phase_id(EVERGREEN), METER),
        usage_tuned("usd", "US", phase_id(TRIAL), METER, |row| {
            row.model_kind = Some(ModelKind::Package);
            row.bands = Vec::new();
            row.package_size = Some(100);
            row.package_price_minor = Some(minor(500));
            row.billing_granularity = Some(BillingGranularity::PerDay);
            row.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
            row.tier_qualification_window = Some(TierQualificationWindow::TrailingPeriod);
            row.aggregation_function = Some(AggregationFunction::Peak);
            row.aggregation_granularity = Some(AggregationGranularity::Day);
        }),
    ];

    let report = judge(&PhaseOverrideUnits, &subject);
    let violation = only(&report);
    let named = violation
        .detail
        .split_once("changes ")
        .and_then(|(_, rest)| rest.split_once(" against"))
        .map(|(list, _)| list)
        .expect("the detail names the offending fields between `changes` and `against`");

    assert_eq!(
        named.split(", ").collect::<Vec<_>>(),
        vec![
            "model_kind",
            "billingGranularity",
            "aggregationFunction",
            "aggregationGranularity",
            "tierAggregationWindow",
            "tierQualificationWindow",
            "package_size",
        ]
    );
}

#[test]
fn an_override_in_a_market_its_line_does_not_price_is_not_compared() {
    // The referent is the terminal-phase row a subscriber in *this* market
    // converts into - the two keys differ in the phase axis alone. Borrowing
    // another market's row would name fields no counter ever crosses; the
    // missing base is `USAGE_MARKET_INCOMPLETE`'s to report.
    let mut subject = phased();
    subject.rows = vec![
        usage("usd", "US", phase_id(EVERGREEN), METER),
        usage_tuned("eur", "EU", phase_id(TRIAL), METER, |row| {
            row.billing_granularity = Some(BillingGranularity::PerDay);
        }),
    ];

    assert!(judge(&PhaseOverrideUnits, &subject).is_publishable());
    assert!(
        judge(&PhaseOverrideBase, &subject).is_publishable(),
        "the line does have a phase-invariant home, which is all D-117 asks"
    );
}

// ---------------------------------------------------------------------------
// TerminalPhaseStable
// ---------------------------------------------------------------------------

#[test]
fn a_revision_moving_the_terminal_phase_fails() {
    // The D-64 scenario: a usage-only plan published non-phased on the implicit
    // terminal T0, revised to add a trial plus a new evergreen phase. T0 stops
    // being terminal, its metered row stops being phase-invariant, and a
    // subscription in the new phase resolves no usage row at all.
    let t0 = phase_id(0x40);
    let mut subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Trial, 0, Some(t0)),
        PlanPhase {
            phase_id: t0,
            kind: PhaseKind::Evergreen,
            ordinal: 1,
            converts_to_phase_id: Some(phase_id(EVERGREEN)),
            phase_duration_days: Some(30),
            display_trial_days: None,
        },
        phase(EVERGREEN, PhaseKind::Evergreen, 2, None),
    ]);
    subject.baseline = Some(PublishedBaseline {
        terminal_phase_id: t0,
        phase_ids_in_use: [t0].into(),
        available_from: None,
        available_to: None,
    });

    let report = judge(&TerminalPhaseStable, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, TERMINAL_PHASE_CHANGED);
    assert!(violation.detail.contains(&t0.to_string()));
    assert!(violation.subject.contains(&phase_id(EVERGREEN).to_string()));
}

#[test]
fn a_revision_dropping_a_phase_still_in_use_fails() {
    let dropped = phase_id(0x99);
    let mut subject = phased();
    subject.baseline = Some(PublishedBaseline {
        terminal_phase_id: phase_id(EVERGREEN),
        phase_ids_in_use: [phase_id(EVERGREEN), dropped].into(),
        available_from: None,
        available_to: None,
    });

    let report = judge(&TerminalPhaseStable, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_IN_USE);
    assert!(violation.subject.contains(&dropped.to_string()));
}

#[test]
fn an_unchanged_terminal_phase_with_every_phase_re_attached_publishes() {
    let mut subject = phased();
    subject.baseline = Some(PublishedBaseline {
        terminal_phase_id: phase_id(EVERGREEN),
        phase_ids_in_use: [phase_id(TRIAL), phase_id(EVERGREEN)].into(),
        available_from: None,
        available_to: None,
    });

    assert!(judge(&TerminalPhaseStable, &subject).is_publishable());
}

#[test]
fn a_first_publish_has_nothing_to_be_stable_against() {
    // Both arms are comparisons, and a comparison with no referent either
    // passes vacuously or blocks every publish. `None` is the first publish.
    let mut subject = shape_of(vec![phase(0x40, PhaseKind::Evergreen, 0, None)]);
    subject.baseline = None;

    assert!(judge(&TerminalPhaseStable, &subject).is_publishable());
}

#[test]
fn an_ambiguous_terminal_quiets_the_immutability_arm_and_not_the_in_use_arm() {
    // Arm (a) needs a single terminal phase to compare against; arm (b) needs
    // only the phase set, so a broken chain never hides a dropped in-use phase.
    let dropped = phase_id(0x99);
    let mut subject = shape_of(vec![
        phase(TRIAL, PhaseKind::Evergreen, 0, None),
        phase(EVERGREEN, PhaseKind::Evergreen, 1, None),
    ]);
    subject.baseline = Some(PublishedBaseline {
        terminal_phase_id: phase_id(0x40),
        phase_ids_in_use: [dropped].into(),
        available_from: None,
        available_to: None,
    });

    let report = judge(&TerminalPhaseStable, &subject);
    let violation = only(&report);

    assert_eq!(violation.code, PHASE_IN_USE);
}

// ---------------------------------------------------------------------------
// inst-ph-trial (D-151)
// ---------------------------------------------------------------------------

#[test]
fn an_evergreen_terminal_phase_publishing_a_trial_length_is_refused() {
    // The case nothing caught, and the reason it is worth a rule of its own.
    // A plan whose ONLY phase is the evergreen terminal one, carrying
    // `displayTrialDays`: `PhaseDuration` is correct to find no duration on a
    // terminal phase, `TerminalPhaseKind` is correct to find `evergreen`, and
    // section 6's CHECK is SATISFIED because comparing to NULL is NULL. Three
    // guards, each right, and the shape they exist to forbid passed all of them.
    let mut terminal = phase(0x7e_11, PhaseKind::Evergreen, 1, None);
    terminal.display_trial_days = Some(14);
    let subject = shape_of(vec![terminal]);

    // The three that legitimately pass.
    assert!(judge(&PhaseDuration, &subject).violations.is_empty());
    assert!(judge(&TerminalPhaseKind, &subject).violations.is_empty());
    assert!(judge(&PhaseGraphIntegrity, &subject).violations.is_empty());

    // The one that does not.
    let report = judge(&DisplayTrialDaysOnTrialPhase, &subject);
    let violation = only(&report);
    assert_eq!(violation.code, DISPLAY_TRIAL_DAYS_INVALID);
    assert!(
        violation.subject.contains(&phase_id(0x7e_11).to_string()),
        "the phase is named: {}",
        violation.subject
    );
    assert!(
        violation.detail.contains("evergreen"),
        "{}",
        violation.detail
    );
}

#[test]
fn an_intro_phase_carrying_a_trial_length_is_refused_too() {
    // The projection binds to the phase `kind`, and `intro` is the other kind a
    // non-terminal phase can be.
    let mut intro = phase(0x11_10, PhaseKind::Intro, 0, Some(phase_id(0x7e_11)));
    intro.display_trial_days = Some(30);
    let subject = shape_of(vec![intro, phase(0x7e_11, PhaseKind::Evergreen, 1, None)]);

    let report = judge(&DisplayTrialDaysOnTrialPhase, &subject);
    let violation = only(&report);
    assert_eq!(violation.code, DISPLAY_TRIAL_DAYS_INVALID);
    assert!(violation.detail.contains("intro"), "{}", violation.detail);
}

#[test]
fn a_trial_phase_with_no_duration_to_project_is_refused_naming_the_null_check() {
    // The half section 6's CHECK cannot carry at all: NULL propagation makes it
    // satisfied whenever `phase_duration_days` is NULL while
    // `display_trial_days` is set, on both engines.
    let mut trial = phase(0x77_1a, PhaseKind::Trial, 0, Some(phase_id(0x7e_11)));
    trial.phase_duration_days = None;
    trial.display_trial_days = Some(14);
    let subject = shape_of(vec![trial, phase(0x7e_11, PhaseKind::Evergreen, 1, None)]);

    let report = judge(&DisplayTrialDaysOnTrialPhase, &subject);
    let violation = only(&report);
    assert_eq!(violation.code, DISPLAY_TRIAL_DAYS_INVALID);
    assert!(
        violation.detail.contains("phaseDurationDays"),
        "{}",
        violation.detail
    );
}

#[test]
fn a_trial_phase_projecting_the_duration_it_has_is_silent() {
    // One value, two projections - the shape `inst-ph-trial` exists to permit.
    let mut trial = phase(0x77_1a, PhaseKind::Trial, 0, Some(phase_id(0x7e_11)));
    trial.phase_duration_days = Some(14);
    trial.display_trial_days = Some(14);
    let subject = shape_of(vec![trial, phase(0x7e_11, PhaseKind::Evergreen, 1, None)]);

    assert!(
        judge(&DisplayTrialDaysOnTrialPhase, &subject)
            .violations
            .is_empty()
    );
}

#[test]
fn every_offending_phase_is_reported_and_not_only_the_first() {
    let mut intro = phase(0x11_10, PhaseKind::Intro, 0, Some(phase_id(0x7e_11)));
    intro.display_trial_days = Some(30);
    let mut terminal = phase(0x7e_11, PhaseKind::Evergreen, 1, None);
    terminal.display_trial_days = Some(14);

    let report = judge(
        &DisplayTrialDaysOnTrialPhase,
        &shape_of(vec![intro, terminal]),
    );

    assert_eq!(report.violations.len(), 2, "{:?}", report.violations);
    assert!(
        report
            .violations
            .iter()
            .all(|v| v.code == DISPLAY_TRIAL_DAYS_INVALID)
    );
}
