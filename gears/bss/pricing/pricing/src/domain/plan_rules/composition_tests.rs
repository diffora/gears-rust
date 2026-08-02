//! Tests for the plan-composition rules.
//!
//! Two families, and both are written against the *defect* rather than the
//! code. The injectivity family exists because D-103 restated the rule after
//! the old one was found to be enforced by nothing: so the cases pin what a
//! plan is now allowed to do (price several meters, repeat a line across
//! markets, phases, eligibility classes and grandfathered generations) as hard
//! as they pin what it may not (repeat a line inside one slice). The add-on
//! family pins the three ways a plan-authored edge set can be incoherent, and
//! one thing it must not do — fail a conflict pair that has an optional side,
//! which is the feature the field exists for.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use super::{
    AddonConflictBothRequired, AddonDependencyAcyclic, AddonEdgeMembership, MeterInjectivity,
    PlanTierDeclared,
};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::CurrencyCode;
use crate::domain::plan_rules::{
    ADDON_CYCLE, ADDON_INCOMPATIBLE, METER_AMBIGUOUS, PLANTIER_MISSING,
};
use crate::domain::plan_shape::{AddonRule, PlanShape};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{ModelKind, PriceRow};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use crate::domain::validation::{ValidationReport, ValidationRule};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x91a4))
}

fn phase_id(seed: u128) -> PhaseId {
    PhaseId::new(Uuid::from_u128(seed))
}

/// The plan's terminal phase — where a phase-invariant usage row is filed
/// (D-19).
fn terminal_phase() -> PhaseId {
    phase_id(0x7e0)
}

fn addon(seed: u128) -> Uuid {
    Uuid::from_u128(seed)
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 3, 12, 0, 0)
        .single()
        .expect("the fixed instant is unambiguous")
}

fn cutover(day: u32) -> Cohort {
    Cohort::Generation(
        Utc.with_ymd_and_hms(2026, 7, day, 0, 0, 0)
            .single()
            .expect("the fixed instant is unambiguous"),
    )
}

fn shape() -> PlanShape {
    let mut shape = PlanShape::new(plan(), 3, now());
    shape.plan_tier = Some("standard".to_owned());
    shape
}

/// One priced row, described by every axis this slice's rules read.
///
/// A struct rather than a wide constructor because the injectivity cases each
/// vary exactly one axis of a shared baseline, and a positional call would make
/// which one invisible at the call site.
#[derive(Clone)]
struct Line {
    price_id: u128,
    charge_kind: ChargeKind,
    currency: &'static str,
    region: &'static str,
    phase: PhaseId,
    eligibility: PriceEligibility,
    cohort: Cohort,
    meter: Option<&'static str>,
    dimension_key: &'static str,
}

impl Line {
    /// A metered row on `meter`, on the plan's terminal phase, in USD/EU.
    fn usage(price_id: u128, meter: &'static str) -> Self {
        Self {
            price_id,
            charge_kind: ChargeKind::Usage,
            currency: "USD",
            region: "EU",
            phase: terminal_phase(),
            eligibility: PriceEligibility::AllSubscriptions,
            cohort: Cohort::None,
            meter: Some(meter),
            dimension_key: "",
        }
    }

    /// A recurring row on the same slice, carrying no meter at all.
    fn unmetered(price_id: u128) -> Self {
        let mut line = Self::usage(price_id, "");
        line.charge_kind = ChargeKind::Recurring;
        line.meter = None;
        line
    }

    fn record(&self) -> PriceRecord {
        let scope_key = ScopeKey::new(
            plan(),
            CurrencyCode::new(self.currency).expect("test currency is three letters"),
            Region::new(self.region).expect("test region is non-blank"),
            self.phase,
            self.eligibility,
            self.charge_kind,
            self.cohort,
        )
        .expect("the test eligibility and cohort are paired");

        let mut row = PriceRow::new(self.charge_kind, Some(ModelKind::PerUnit));
        row.meter = self.meter.map(str::to_owned);
        row.dimension_key = self.dimension_key.to_owned();

        PriceRecord {
            price_id: Uuid::from_u128(self.price_id),
            scope_key,
            row,
            tax_inclusive: false,
            billing_timing: None,
            rounding_policy_ref: None,
            grandfather_until: None,
            supersedes_price_id: None,
            lifecycle_state: LifecycleState::Draft,
            created_by: Uuid::from_u128(0xac_10),
            created_at_utc: now(),
            row_version: RowVersion::new(0),
        }
    }
}

fn rule(required: bool, sku: Uuid) -> AddonRule {
    AddonRule {
        addon_sku_id: sku,
        required,
        min_qty: None,
        max_qty: Some(1),
        step_qty: None,
        price_override_ref: None,
        depends_on: Vec::new(),
        conflicts_with: Vec::new(),
    }
}

fn findings(rule: &impl ValidationRule<PlanShape>, subject: &PlanShape) -> ValidationReport {
    let mut report = ValidationReport::default();
    rule.evaluate(subject, &mut report);
    report
}

fn codes(report: &ValidationReport) -> Vec<&str> {
    report
        .violations
        .iter()
        .map(|violation| violation.code.as_str())
        .collect()
}

/// What a report found, for the message of an assertion that expected nothing.
fn reported(report: &ValidationReport) -> String {
    codes(report).join(", ")
}

/// Run the injectivity rule over a row set built from `lines`.
fn injectivity(lines: &[Line]) -> ValidationReport {
    let mut subject = shape();
    subject.rows = lines.iter().map(Line::record).collect();
    findings(&MeterInjectivity, &subject)
}

/// Run the three add-on rules over `rules`, concatenating their findings the
/// way the pipeline does.
fn composition(rules: Vec<AddonRule>) -> ValidationReport {
    let mut subject = shape();
    subject.addon_rules = rules;
    let mut report = findings(&AddonEdgeMembership, &subject);
    report.absorb(findings(&AddonDependencyAcyclic, &subject));
    report.absorb(findings(&AddonConflictBothRequired, &subject));
    report
}

// ---------------------------------------------------------------------------
// inst-cmp-plantier
// ---------------------------------------------------------------------------

#[test]
fn an_absent_plan_tier_fails_publish() {
    let mut subject = shape();
    subject.plan_tier = None;

    let report = findings(&PlanTierDeclared, &subject);

    assert_eq!(codes(&report), vec![PLANTIER_MISSING]);
    assert_eq!(report.violations[0].subject, subject.subject());
}

#[test]
fn a_blank_plan_tier_is_not_a_declaration() {
    // The equality-to-SKU half that would otherwise catch this is registry
    // work and absent, so a whitespace-only tier would publish unexamined.
    let mut subject = shape();
    subject.plan_tier = Some("   ".to_owned());

    assert_eq!(
        codes(&findings(&PlanTierDeclared, &subject)),
        vec![PLANTIER_MISSING]
    );
}

#[test]
fn a_declared_plan_tier_is_silent() {
    assert!(findings(&PlanTierDeclared, &shape()).is_publishable());
}

// ---------------------------------------------------------------------------
// inst-cmp-injective (D-103)
// ---------------------------------------------------------------------------

#[test]
fn two_rows_on_one_line_in_one_slice_are_ambiguous() {
    let first = Line::usage(0x1, "cloudlets");
    let second = Line::usage(0x2, "cloudlets");

    let report = injectivity(&[first, second]);

    assert_eq!(codes(&report), vec![METER_AMBIGUOUS]);
    let named = &report.violations[0].subject;
    assert!(named.contains("cloudlets"), "the line is named: {named}");
    assert!(named.contains("USD"), "the slice is named: {named}");
    assert!(named.contains("EU"), "the slice is named: {named}");
}

#[test]
fn a_plan_pricing_two_meters_publishes() {
    // The D-103 regression: the pre-restatement rule said a usage plan revision
    // maps exactly one meteringUnit, and a PaaS plan pricing cloudlets, storage
    // and egress is one plan rather than three.
    let report = injectivity(&[
        Line::usage(0x1, "cloudlets"),
        Line::usage(0x2, "storage"),
        Line::usage(0x3, "egress"),
    ]);

    assert!(report.is_publishable(), "unexpected: {}", reported(&report));
}

#[test]
fn one_meter_priced_over_two_dimensions_publishes() {
    let mut first = Line::usage(0x1, "egress");
    first.dimension_key = "eu-west";
    let mut second = Line::usage(0x2, "egress");
    second.dimension_key = "us-east";

    assert!(injectivity(&[first, second]).is_publishable());
}

#[test]
fn the_same_line_in_two_markets_publishes() {
    let mut other_currency = Line::usage(0x2, "cloudlets");
    other_currency.currency = "EUR";
    let mut other_region = Line::usage(0x3, "cloudlets");
    other_region.region = "US";

    let report = injectivity(&[Line::usage(0x1, "cloudlets"), other_currency, other_region]);

    assert!(report.is_publishable(), "unexpected: {}", reported(&report));
}

#[test]
fn the_same_line_in_two_phases_publishes() {
    let mut trial = Line::usage(0x2, "cloudlets");
    trial.phase = phase_id(0x7);

    assert!(injectivity(&[Line::usage(0x1, "cloudlets"), trial]).is_publishable());
}

#[test]
fn the_same_line_in_two_eligibility_classes_publishes() {
    let mut newcomers = Line::usage(0x2, "cloudlets");
    newcomers.eligibility = PriceEligibility::NewSubscriptionsOnly;

    assert!(injectivity(&[Line::usage(0x1, "cloudlets"), newcomers]).is_publishable());
}

#[test]
fn two_grandfathered_generations_of_one_line_publish() {
    // ADR-0002: the cohort axis is what lets a second cutover retain another
    // generation of the same usage line without reading as a duplicate.
    let mut july = Line::usage(0x2, "cloudlets");
    july.eligibility = PriceEligibility::ExistingGrandfathered;
    july.cohort = cutover(1);
    let mut august = Line::usage(0x3, "cloudlets");
    august.eligibility = PriceEligibility::ExistingGrandfathered;
    august.cohort = cutover(15);

    let report = injectivity(&[Line::usage(0x1, "cloudlets"), july, august]);

    assert!(report.is_publishable(), "unexpected: {}", reported(&report));
}

#[test]
fn two_rows_differing_only_in_charge_kind_still_collide() {
    // chargeKind is not a column of the index this rule restates, and the index
    // admits any row carrying a meter. A metered row mis-filed on a non-usage
    // charge component must therefore fail here rather than at the driver.
    let mut misfiled = Line::usage(0x2, "cloudlets");
    misfiled.charge_kind = ChargeKind::OneTimeSetup;

    assert_eq!(
        codes(&injectivity(&[Line::usage(0x1, "cloudlets"), misfiled])),
        vec![METER_AMBIGUOUS]
    );
}

#[test]
fn rows_without_a_meter_never_collide() {
    // NULL meters do not collide in the index, and two unmetered rows on one
    // slice are the ordinary shape of a plan, not an ambiguity.
    assert!(injectivity(&[Line::unmetered(0x1), Line::unmetered(0x2)]).is_publishable());
}

#[test]
fn a_third_row_on_a_doubled_line_is_reported_too() {
    let report = injectivity(&[
        Line::usage(0x1, "cloudlets"),
        Line::usage(0x2, "cloudlets"),
        Line::usage(0x3, "cloudlets"),
    ]);

    assert_eq!(codes(&report), vec![METER_AMBIGUOUS, METER_AMBIGUOUS]);
}

// ---------------------------------------------------------------------------
// inst-cmp-addons: edge membership
// ---------------------------------------------------------------------------

#[test]
fn a_dependency_on_a_non_member_fails() {
    let mut backup = rule(false, addon(0xa));
    backup.depends_on = vec![addon(0xff)];

    let report = composition(vec![backup, rule(false, addon(0xb))]);

    assert_eq!(codes(&report), vec![ADDON_INCOMPATIBLE]);
    assert_eq!(
        report.violations[0].subject,
        format!("{}/{}", addon(0xa), addon(0xff))
    );
}

#[test]
fn a_conflict_with_a_non_member_fails() {
    let mut backup = rule(false, addon(0xa));
    backup.conflicts_with = vec![addon(0xff)];

    assert_eq!(codes(&composition(vec![backup])), vec![ADDON_INCOMPATIBLE]);
}

#[test]
fn a_self_edge_fails() {
    let mut backup = rule(false, addon(0xa));
    backup.depends_on = vec![addon(0xa)];

    let report = composition(vec![backup]);

    // Exactly one finding: the self-edge is a loop of length one, and the cycle
    // walk deliberately leaves it to membership rather than reporting one
    // authoring mistake under two codes.
    assert_eq!(codes(&report), vec![ADDON_INCOMPATIBLE]);
}

#[test]
fn in_set_edges_are_silent() {
    let mut backup = rule(false, addon(0xa));
    backup.depends_on = vec![addon(0xb)];
    backup.conflicts_with = vec![addon(0xc)];

    let report = composition(vec![
        backup,
        rule(false, addon(0xb)),
        rule(false, addon(0xc)),
    ]);

    assert!(report.is_publishable(), "unexpected: {}", reported(&report));
}

// ---------------------------------------------------------------------------
// inst-cmp-addons: dependency cycles
// ---------------------------------------------------------------------------

#[test]
fn a_two_node_dependency_cycle_fails() {
    let mut first = rule(false, addon(0xa));
    first.depends_on = vec![addon(0xb)];
    let mut second = rule(false, addon(0xb));
    second.depends_on = vec![addon(0xa)];

    let report = composition(vec![first, second]);

    assert_eq!(codes(&report), vec![ADDON_CYCLE]);
    assert_eq!(
        report.violations[0].subject,
        format!("{},{}", addon(0xa), addon(0xb))
    );
}

#[test]
fn a_three_node_dependency_cycle_fails_with_stable_member_ordering() {
    // The case a pairwise check passes: no two of these depend on each other.
    // The members are authored out of order so the report cannot be a
    // restatement of authoring order.
    let mut third = rule(false, addon(0xc));
    third.depends_on = vec![addon(0xa)];
    let mut first = rule(false, addon(0xa));
    first.depends_on = vec![addon(0xb)];
    let mut second = rule(false, addon(0xb));
    second.depends_on = vec![addon(0xc)];

    let report = composition(vec![third, first, second]);

    assert_eq!(codes(&report), vec![ADDON_CYCLE]);
    assert_eq!(
        report.violations[0].subject,
        format!("{},{},{}", addon(0xa), addon(0xb), addon(0xc))
    );
}

#[test]
fn a_dependent_of_a_cycle_is_not_named_as_a_member() {
    // `d` reaches the loop and the loop does not reach it, so removing `d`
    // breaks nothing: naming it would send the author to the wrong rule.
    let mut first = rule(false, addon(0xa));
    first.depends_on = vec![addon(0xb)];
    let mut second = rule(false, addon(0xb));
    second.depends_on = vec![addon(0xa)];
    let mut dependent = rule(false, addon(0xd));
    dependent.depends_on = vec![addon(0xa)];

    let report = composition(vec![first, second, dependent]);

    assert_eq!(codes(&report), vec![ADDON_CYCLE]);
    assert_eq!(
        report.violations[0].subject,
        format!("{},{}", addon(0xa), addon(0xb))
    );
}

#[test]
fn two_loops_one_of_which_reaches_the_other_are_two_findings() {
    // `b` bridges into the second loop, so reachability alone folds all four
    // into one finding and names two add-ons that are not in the loop the
    // author has to break. Membership of a loop is mutual reachability, and
    // this is the shape where the two answers differ.
    let mut first = rule(false, addon(0xa));
    first.depends_on = vec![addon(0xb)];
    let mut bridge = rule(false, addon(0xb));
    bridge.depends_on = vec![addon(0xa), addon(0xc)];
    let mut third = rule(false, addon(0xc));
    third.depends_on = vec![addon(0xd)];
    let mut fourth = rule(false, addon(0xd));
    fourth.depends_on = vec![addon(0xc)];

    let report = composition(vec![first, bridge, third, fourth]);

    assert_eq!(codes(&report), vec![ADDON_CYCLE, ADDON_CYCLE]);
    assert_eq!(
        report.violations[0].subject,
        format!("{},{}", addon(0xa), addon(0xb))
    );
    assert_eq!(
        report.violations[1].subject,
        format!("{},{}", addon(0xc), addon(0xd))
    );
}

#[test]
fn a_dependency_diamond_publishes() {
    let mut top = rule(false, addon(0xa));
    top.depends_on = vec![addon(0xb), addon(0xc)];
    let mut left = rule(false, addon(0xb));
    left.depends_on = vec![addon(0xd)];
    let mut right = rule(false, addon(0xc));
    right.depends_on = vec![addon(0xd)];

    let report = composition(vec![top, left, right, rule(false, addon(0xd))]);

    assert!(report.is_publishable(), "unexpected: {}", reported(&report));
}

// ---------------------------------------------------------------------------
// inst-cmp-addons: conflicting pairs
// ---------------------------------------------------------------------------

#[test]
fn a_conflict_between_two_required_addons_fails() {
    let mut first = rule(true, addon(0xa));
    first.conflicts_with = vec![addon(0xb)];
    let mut second = rule(true, addon(0xb));
    second.conflicts_with = vec![addon(0xa)];

    // Authored high id first, so the subject pins the normalized pair rather
    // than restating whichever side the walk reached first.
    let report = composition(vec![second, first]);

    // Once, not twice: the pair is one fault however many sides authored it.
    assert_eq!(codes(&report), vec![ADDON_INCOMPATIBLE]);
    assert_eq!(
        report.violations[0].subject,
        format!("{},{}", addon(0xa), addon(0xb))
    );
}

#[test]
fn a_required_addon_conflicting_with_itself_is_reported_once() {
    // Membership owns the self-edge. Letting it through to the pair check as
    // well would report one authoring mistake under the same code twice.
    let mut lone = rule(true, addon(0xa));
    lone.conflicts_with = vec![addon(0xa)];

    assert_eq!(codes(&composition(vec![lone])), vec![ADDON_INCOMPATIBLE]);
}

#[test]
fn a_one_sided_conflict_between_two_required_addons_still_fails() {
    // Conflicts are stored normalized symmetric, so a constraint that bound
    // only its author would be one the other side could ignore.
    let mut first = rule(true, addon(0xa));
    first.conflicts_with = vec![addon(0xb)];

    let report = composition(vec![first, rule(true, addon(0xb))]);

    assert_eq!(codes(&report), vec![ADDON_INCOMPATIBLE]);
}

#[test]
fn a_conflict_with_one_optional_side_publishes() {
    // The feature the field exists for: a rule that failed every conflict pair
    // would forbid selection-time exclusivity outright.
    let mut required_side = rule(true, addon(0xa));
    required_side.conflicts_with = vec![addon(0xb)];

    let report = composition(vec![required_side, rule(false, addon(0xb))]);

    assert!(report.is_publishable(), "unexpected: {}", reported(&report));
}

#[test]
fn a_conflict_authored_by_the_optional_side_publishes_too() {
    // The same pair from the other end. Both sides have to be read before the
    // pair is judged, or which side happened to author the edge would decide
    // whether the plan publishes.
    let mut optional_side = rule(false, addon(0xa));
    optional_side.conflicts_with = vec![addon(0xb)];

    let report = composition(vec![optional_side, rule(true, addon(0xb))]);

    assert!(report.is_publishable(), "unexpected: {}", reported(&report));
}

#[test]
fn rule_names_are_the_instructions_that_own_them() {
    assert_eq!(PlanTierDeclared.name(), "inst-cmp-plantier");
    assert_eq!(MeterInjectivity.name(), "inst-cmp-injective");
    // One instruction states three separable properties of the add-on set, and
    // three rules reporting under its id is honest where three invented ids
    // would not be.
    assert_eq!(AddonEdgeMembership.name(), "inst-cmp-addons");
    assert_eq!(AddonDependencyAcyclic.name(), "inst-cmp-addons");
    assert_eq!(AddonConflictBothRequired.name(), "inst-cmp-addons");
}
