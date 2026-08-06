//! `TaxonomyValidator`'s two owned rules, and the vocabulary the four taxonomies
//! are addressed by (`design/04-currency-tax.md` §3, §5, §6).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use super::{
    REGION_UNKNOWN, RegionsDeclared, TAXONOMY_VALUE_IN_USE, TaxonomyClass, TaxonomyState,
    ValueReferences, check_retirable,
};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::overlay::{ScopeClass, ScopeValue};
use crate::domain::plan_shape::PlanShape;
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::PriceRow;
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use crate::domain::validation::{ValidationReport, ValidationRule};

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0)
        .single()
        .expect("the fixed instant is unambiguous")
}

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x91a4))
}

fn value(raw: &str) -> ScopeValue {
    ScopeValue::new(raw).expect("a non-blank fixture value")
}

fn region(raw: &str) -> Region {
    Region::new(raw).expect("a non-blank fixture region")
}

/// One candidate row on a named region.
fn row_in(price_id: u128, in_region: &str) -> PriceRecord {
    let scope_key = ScopeKey::new(
        plan(),
        CurrencyCode::new("EUR").expect("three letters"),
        region(in_region),
        PhaseId::new(Uuid::from_u128(0xf1)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("all_subscriptions pairs with cohort none");

    let mut row = PriceRow::new(ChargeKind::Recurring, None);
    row.amount_minor = Some(MinorAmount::new(1000).expect("non-negative"));

    PriceRecord {
        price_id: Uuid::from_u128(price_id),
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

/// A plan shape carrying exactly the named regions, one row each.
fn shape_over(regions: &[&str]) -> PlanShape {
    let mut shape = PlanShape::new(plan(), 1, now());
    shape.rows = regions
        .iter()
        .enumerate()
        .map(|(n, r)| row_in(0xb000 + n as u128, r))
        .collect();
    shape
}

fn declared(regions: &[&str]) -> RegionsDeclared {
    RegionsDeclared {
        declared: regions.iter().map(|r| region(r)).collect(),
    }
}

fn codes(report: &ValidationReport) -> Vec<String> {
    report.violations.iter().map(|v| v.code.clone()).collect()
}

fn subjects(report: &ValidationReport) -> Vec<String> {
    report
        .violations
        .iter()
        .map(|v| v.subject.clone())
        .collect()
}

fn run(rule: &RegionsDeclared, shape: &PlanShape) -> ValidationReport {
    let mut report = ValidationReport::default();
    rule.evaluate(shape, &mut report);
    report
}

// ---------------------------------------------------------------------------
// The codes, transcribed from section 5 rather than from the identifiers.
// ---------------------------------------------------------------------------

/// A constant whose value drifted from its name would still compile and would
/// still be wrong on the wire.
#[test]
fn the_codes_are_spelled_as_section_5_spells_them() {
    assert_eq!(REGION_UNKNOWN, "REGION_UNKNOWN");
    assert_eq!(TAXONOMY_VALUE_IN_USE, "TAXONOMY_VALUE_IN_USE");
}

// ---------------------------------------------------------------------------
// The classes.
// ---------------------------------------------------------------------------

/// §5 addresses exactly four, and `orgTier` is spelled as §5 spells it.
///
/// The camelCase segment is a **recorded divergence** (register `T-4`), not an
/// accident: every other wire token for this class is `org_tier`. Pinned here so
/// that changing it is a decision someone has to make on purpose.
#[test]
fn the_four_path_segments_are_section_5s_own_spelling() {
    let segments: Vec<&str> = TaxonomyClass::ALL
        .iter()
        .map(|c| c.path_segment())
        .collect();
    assert_eq!(segments, ["region", "brand", "partner", "orgTier"]);
}

/// Every segment parses back to the class that rendered it, and nothing else
/// parses at all.
#[test]
fn a_segment_round_trips_and_an_unknown_one_does_not_parse() {
    for &class in TaxonomyClass::ALL {
        assert_eq!(
            TaxonomyClass::parse_segment(class.path_segment()),
            Some(class),
            "{class} must parse back from its own segment"
        );
    }
    // The two classes that are deliberately not addressable: `global` has no
    // value universe, and `customerGroup`'s table belongs to Slice 9's
    // membership half and does not exist (D-223).
    assert_eq!(TaxonomyClass::parse_segment("global"), None);
    assert_eq!(TaxonomyClass::parse_segment("customer_group"), None);
    // And the spelling that is *not* section 5's.
    assert_eq!(TaxonomyClass::parse_segment("org_tier"), None);
}

/// The table map is `ScopeClass`', not a second copy of it.
///
/// This is the assertion that keeps one answer to *which universe declares this
/// class*. A fork would pass its own tests and disagree with `overlay_repo`.
#[test]
fn each_class_names_the_table_its_scope_class_names() {
    for &class in TaxonomyClass::ALL {
        let via_scope_class = class
            .scope_class()
            .taxonomy_table()
            .expect("all four narrow classes declare a table");
        assert_eq!(class.table(), via_scope_class, "{class} forked the map");
    }
    assert_eq!(
        TaxonomyClass::Region.scope_class(),
        ScopeClass::Region,
        "the narrowing must not renumber the classes"
    );
    assert_eq!(TaxonomyClass::OrgTier.scope_class(), ScopeClass::OrgTier);
}

/// D-01's markers are the region's alone — the schema says so and so does §6.
#[test]
fn only_the_region_class_carries_the_tax_markers() {
    assert!(TaxonomyClass::Region.carries_tax_markers());
    for &class in TaxonomyClass::ALL
        .iter()
        .filter(|c| **c != TaxonomyClass::Region)
    {
        assert!(
            !class.carries_tax_markers(),
            "{class} must not claim columns the schema does not give it"
        );
    }
}

/// `active | retired`, both directions, and nothing else.
#[test]
fn the_state_pair_round_trips_and_admits_no_third_token() {
    for &state in TaxonomyState::ALL {
        assert_eq!(TaxonomyState::parse(state.as_str()), Some(state));
    }
    assert_eq!(TaxonomyState::parse("deprecated"), None);
    assert_eq!(
        TaxonomyState::default(),
        TaxonomyState::Active,
        "a value an operator declares is declared, not withdrawn"
    );
}

// ---------------------------------------------------------------------------
// `inst-tx-region` — price-row membership.
// ---------------------------------------------------------------------------

#[test]
fn a_row_in_a_declared_region_passes() {
    let report = run(&declared(&["eu", "us"]), &shape_over(&["eu"]));
    assert!(report.is_publishable(), "a declared region blocks nothing");
    assert_eq!(codes(&report), Vec::<String>::new());
}

#[test]
fn a_row_in_an_undeclared_region_fails_naming_the_value() {
    let report = run(&declared(&["eu"]), &shape_over(&["ap-south"]));

    assert_eq!(codes(&report), [REGION_UNKNOWN]);
    assert_eq!(
        subjects(&report),
        ["ap-south"],
        "the refusal names the value the operator must declare"
    );
    let detail = &report.violations[0].detail;
    assert!(
        detail.contains("ap-south"),
        "the detail names the value too: {detail}"
    );
    assert!(
        detail.contains("config/taxonomies/region"),
        "and tells the operator where to declare it: {detail}"
    );
}

/// One violation per undeclared **value**, not per row.
///
/// A plan with three rows in one bad region is one authoring mistake. Three
/// copies of it is a report an operator scrolls past, and it is how an aggregate
/// report stops being readable as the row count grows — the 20-currency floor of
/// C1 makes that a real number, not a hypothetical.
#[test]
fn one_undeclared_region_across_many_rows_is_reported_once() {
    let report = run(
        &declared(&["eu"]),
        &shape_over(&["ap-south", "ap-south", "ap-south"]),
    );

    assert_eq!(codes(&report), [REGION_UNKNOWN]);
}

/// Two different undeclared regions are two different facts.
#[test]
fn two_undeclared_regions_are_reported_separately() {
    let report = run(&declared(&["eu"]), &shape_over(&["ap-south", "sa-east"]));

    assert_eq!(codes(&report), [REGION_UNKNOWN, REGION_UNKNOWN]);
    assert_eq!(subjects(&report), ["ap-south", "sa-east"]);
}

/// A tenant who has declared nothing fails every row, and that is C2 rather than
/// an edge case.
///
/// *"membership is validated at save/publish (unknown value fails before
/// publish)"* carves out no tenant. The opposite reading — an empty universe
/// permits everything — is the shape D-211 was written against, and it is the
/// difference between a fail-closed rule and a rule that is off until someone
/// configures it on.
#[test]
fn a_tenant_with_no_declared_region_fails_closed_rather_than_open() {
    let report = run(&declared(&[]), &shape_over(&["eu"]));

    assert_eq!(codes(&report), [REGION_UNKNOWN]);
}

/// The rule is registered under the instruction id it implements.
#[test]
fn the_rule_names_the_instruction_it_implements() {
    assert_eq!(declared(&[]).name(), "inst-tx-region");
}

// ---------------------------------------------------------------------------
// `inst-tx-mutation` — the retire guard.
// ---------------------------------------------------------------------------

#[test]
fn an_unreferenced_value_retires_cleanly() {
    let report = check_retirable(
        TaxonomyClass::Brand,
        &value("acme"),
        ValueReferences::default(),
    );
    assert!(report.is_publishable());
}

#[test]
fn a_value_a_published_row_names_cannot_retire() {
    let report = check_retirable(
        TaxonomyClass::Region,
        &value("eu"),
        ValueReferences {
            published_price_rows: 4,
            active_overlay_scopes: 0,
        },
    );

    assert_eq!(codes(&report), [TAXONOMY_VALUE_IN_USE]);
    assert_eq!(subjects(&report), ["eu"]);
}

/// D-120's widening, which is the whole reason the guard is written over a class.
///
/// Before it, *"a region value retired cleanly while region-scoped overlays still
/// named it"*. The overlay plane is checked for **every** class, including the
/// three that are not price-row axes at all.
#[test]
fn a_value_only_an_overlay_scope_names_cannot_retire_either_in_any_class() {
    for &class in TaxonomyClass::ALL {
        let report = check_retirable(
            class,
            &value("acme"),
            ValueReferences {
                published_price_rows: 0,
                active_overlay_scopes: 1,
            },
        );
        assert_eq!(
            codes(&report),
            [TAXONOMY_VALUE_IN_USE],
            "{class}: an overlay scope is a reference on every taxonomy-backed class (D-120)"
        );
    }
}

/// The refusal names **both** planes' counts.
///
/// An operator holding a value referenced by nine overlays and one row has two
/// jobs; a refusal naming one of them sends them back a second time.
#[test]
fn the_refusal_names_both_referencing_planes() {
    let report = check_retirable(
        TaxonomyClass::Region,
        &value("eu"),
        ValueReferences {
            published_price_rows: 1,
            active_overlay_scopes: 9,
        },
    );

    let detail = &report.violations[0].detail;
    assert!(detail.contains('1'), "the row count is named: {detail}");
    assert!(detail.contains('9'), "the overlay count is named: {detail}");
    assert!(
        detail.contains("region"),
        "and the class, so the operator knows which universe: {detail}"
    );
}
