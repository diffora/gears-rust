//! What a shared structure's cutover must look like across a line's markets.
//!
//! Every instant here is fixed (`t(n)` is a Unix second), so no case can start
//! passing or failing because today's date moved.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    STRUCTURE_CUTOVER_MISMATCH, STRUCTURE_MARKET_BINDING_MISSING, StructureBinding,
    validate_structure_schedule,
};
use crate::domain::error::DomainError;
use crate::domain::instant::from_unix;
use crate::domain::money::CurrencyCode;
use crate::domain::scope_key::{
    ChargeKind, ChargeLineScopeKey, Cohort, MarketPriceScopeKey, PhaseId, PlanId, PriceEligibility,
    Region, SkuId,
};
use crate::domain::validation::ValidationReport;
use crate::domain::window::WINDOW_OVERLAP;

fn line() -> ChargeLineScopeKey {
    ChargeLineScopeKey::new(
        PlanId::new(Uuid::from_u128(1)),
        PhaseId::new(Uuid::from_u128(2)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
        SkuId::new(Uuid::from_u128(3)),
    )
    .expect("all_subscriptions pairs with cohort none")
}

fn market(currency: &str, region: &str) -> MarketPriceScopeKey {
    MarketPriceScopeKey::new(
        line(),
        CurrencyCode::new(currency).expect("iso currency"),
        Region::new(region).expect("non-blank region"),
    )
}

fn t(seconds: i64) -> OffsetDateTime {
    from_unix(1_800_000_000 + seconds, 0).expect("a real instant")
}

fn v(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn bind(
    market: &MarketPriceScopeKey,
    line_version_id: Uuid,
    from: OffsetDateTime,
    to: Option<OffsetDateTime>,
) -> StructureBinding {
    StructureBinding {
        market: market.clone(),
        line_version_id,
        effective_from: from,
        effective_to: to,
    }
}

fn report_of(error: DomainError) -> ValidationReport {
    match error {
        DomainError::ValidationFailed(report) => report,
        other => panic!("unexpected refusal: {other}"),
    }
}

fn codes(report: &ValidationReport) -> Vec<String> {
    report
        .violations
        .iter()
        .map(|violation| violation.code.clone())
        .collect()
}

/// The plan's own case: two markets, structure `v1` then `v2`, and the USD
/// cutover a hundred seconds later than the EUR one.
#[test]
fn structure_cutover_requires_the_same_boundary_in_every_market() {
    let eur = market("EUR", "EU");
    let usd = market("USD", "US");
    let required = BTreeSet::from([eur.clone(), usd.clone()]);
    let mut bindings = vec![
        bind(&eur, v(11), t(0), Some(t(100))),
        bind(&eur, v(12), t(100), None),
        bind(&usd, v(11), t(0), Some(t(200))),
        bind(&usd, v(12), t(200), None),
    ];

    let report = report_of(
        validate_structure_schedule(&required, &bindings, t(0), None)
            .expect_err("the markets change structure on different days"),
    );
    assert_eq!(codes(&report), vec![STRUCTURE_CUTOVER_MISMATCH.to_owned()]);
    assert_eq!(report.violations[0].subject, line().to_string());
    let detail = &report.violations[0].detail;
    assert!(
        detail.contains("EUR/EU => 00000000-0000-0000-0000-00000000000c")
            && detail.contains("USD/US => 00000000-0000-0000-0000-00000000000b"),
        "the report names each market's version at the boundary: {detail}"
    );

    bindings[2].effective_to = Some(t(100));
    bindings[3].effective_from = t(100);
    assert!(
        validate_structure_schedule(&required, &bindings, t(0), None).is_ok(),
        "one boundary in every market publishes"
    );
}

/// **`global` is a market like any other here.** A region's override and the
/// currency-wide price behind it are two markets of one line, so they are bound
/// to one structure at every instant — an override that moved to `v2` while
/// `global` stayed on `v1` is the same fault as any two markets disagreeing. The
/// fallback makes this matter more, not less: a `de` buyer crosses from one row
/// to the other with no act, and would cross a structure boundary doing it.
#[test]
fn an_override_and_the_currency_wide_price_are_bound_to_one_structure() {
    let global = market("EUR", "global");
    let de = market("EUR", "DE");
    let required = BTreeSet::from([global.clone(), de.clone()]);
    let mut bindings = vec![
        bind(&global, v(11), t(0), None),
        bind(&de, v(11), t(0), Some(t(100))),
        bind(&de, v(12), t(100), None),
    ];

    let report = report_of(
        validate_structure_schedule(&required, &bindings, t(0), None)
            .expect_err("the override changed structure and the currency-wide price did not"),
    );
    assert_eq!(codes(&report), vec![STRUCTURE_CUTOVER_MISMATCH.to_owned()]);

    bindings[0].effective_to = Some(t(100));
    bindings.push(bind(&global, v(12), t(100), None));
    assert!(
        validate_structure_schedule(&required, &bindings, t(0), None).is_ok(),
        "one boundary in both publishes"
    );
}

/// The world in which the case above is observable: a line whose markets never
/// change structure at all passes, so the rule is not simply refusing everything.
#[test]
fn a_line_whose_structure_never_changes_passes() {
    let eur = market("EUR", "EU");
    let usd = market("USD", "US");
    let required = BTreeSet::from([eur.clone(), usd.clone()]);
    let bindings = vec![bind(&eur, v(11), t(0), None), bind(&usd, v(11), t(0), None)];

    assert!(validate_structure_schedule(&required, &bindings, t(0), None).is_ok());
}

/// Money is free to differ, and to change, while the structure does not: the
/// rule reads `line_version_id` and nothing about the amount. Two markets whose
/// monetary versions change on different days — each staying on `v1` — publish.
#[test]
fn independent_monetary_versions_of_one_structure_are_legal() {
    let eur = market("EUR", "EU");
    let usd = market("USD", "US");
    let required = BTreeSet::from([eur.clone(), usd.clone()]);
    let bindings = vec![
        // EUR reprices at t100, USD at t200; both stay on structure v1.
        bind(&eur, v(11), t(0), Some(t(100))),
        bind(&eur, v(11), t(100), None),
        bind(&usd, v(11), t(0), Some(t(200))),
        bind(&usd, v(11), t(200), None),
    ];

    assert!(validate_structure_schedule(&required, &bindings, t(0), None).is_ok());
}

/// A market whose coverage **ends** exactly where its sibling moves to a new
/// structure is named, and named as a *missing binding* rather than as a
/// mismatch: the edit is to give that market the new interval, not to move
/// somebody else's boundary.
#[test]
fn a_market_dropped_at_a_siblings_cutover_is_named() {
    let eur = market("EUR", "EU");
    let usd = market("USD", "US");
    let required = BTreeSet::from([eur.clone(), usd.clone()]);
    let bindings = vec![
        bind(&eur, v(11), t(0), Some(t(100))),
        bind(&eur, v(12), t(100), None),
        // USD's coverage stops at t100 and nothing follows it.
        bind(&usd, v(11), t(0), Some(t(100))),
    ];

    let report = report_of(
        validate_structure_schedule(&required, &bindings, t(0), None)
            .expect_err("USD is unbound from t100 onwards"),
    );
    assert_eq!(
        codes(&report),
        vec![STRUCTURE_MARKET_BINDING_MISSING.to_owned()],
        "no mismatch is reported beside it: one market answered, so there is nothing to compare"
    );
    assert_eq!(report.violations[0].subject, usd.to_string());
    assert!(
        report.violations[0]
            .detail
            .contains(&crate::domain::instant::format_rfc3339(t(100))),
        "the report names the boundary it failed at: {}",
        report.violations[0].detail
    );
}

/// A line whose coverage opens **after** the horizon does is not this rule's
/// business, and the boundary at the horizon's own start says nothing.
///
/// `inst-wc-required` counts a *scheduled* window as coverage, so a plan
/// published ahead of its start date is legal — and an earlier draft of this
/// rule refused every one of them, sixty-four fixtures' worth, by treating "no
/// market is bound here" as a fault instead of as an instant it has no question
/// about.
#[test]
fn a_line_whose_coverage_opens_after_the_horizon_is_not_judged_at_its_start() {
    let eur = market("EUR", "EU");
    let required = BTreeSet::from([eur.clone()]);
    let bindings = vec![bind(&eur, v(11), t(100), None)];

    assert!(validate_structure_schedule(&required, &bindings, t(0), None).is_ok());
}

/// Two markets of one line may launch on different days. Nothing about their
/// structure disagrees — neither has changed version — so a staggered launch is
/// a coverage question and not this one.
#[test]
fn a_staggered_launch_is_not_a_structure_fault() {
    let eur = market("EUR", "EU");
    let usd = market("USD", "US");
    let required = BTreeSet::from([eur.clone(), usd.clone()]);
    let bindings = vec![
        bind(&eur, v(11), t(0), None),
        bind(&usd, v(11), t(100), None),
    ];

    assert!(validate_structure_schedule(&required, &bindings, t(0), None).is_ok());
}

/// And a market with no window at all is `inst-wc-required`'s finding, not this
/// rule's: nothing about it changes structure, so this sweep stays silent and
/// the author is told the one thing that is wrong.
#[test]
fn a_market_with_no_binding_at_all_is_left_to_the_coverage_rule() {
    let eur = market("EUR", "EU");
    let usd = market("USD", "US");
    let required = BTreeSet::from([eur.clone(), usd]);
    let bindings = vec![bind(&eur, v(11), t(0), None)];

    assert!(validate_structure_schedule(&required, &bindings, t(0), None).is_ok());
}

/// Two bindings active on one market at one instant are the existing overlap
/// refusal, not a new code: the fault is two structures in one market, which is
/// what `WINDOW_OVERLAP` already says about the plane this list is read off.
#[test]
fn two_structures_in_one_market_at_one_instant_are_an_overlap() {
    let eur = market("EUR", "EU");
    let required = BTreeSet::from([eur.clone()]);
    let bindings = vec![
        bind(&eur, v(11), t(0), Some(t(200))),
        bind(&eur, v(12), t(100), None),
    ];

    let report = report_of(
        validate_structure_schedule(&required, &bindings, t(0), None)
            .expect_err("one market cannot hold two structures at once"),
    );
    assert!(
        codes(&report).contains(&WINDOW_OVERLAP.to_owned()),
        "the overlap code is reused: {:?}",
        codes(&report)
    );
}

/// A bounded horizon judges only inside itself. The same mismatched pair passes
/// when the horizon closes before either cutover.
#[test]
fn a_bounded_horizon_judges_only_the_instants_inside_it() {
    let eur = market("EUR", "EU");
    let usd = market("USD", "US");
    let required = BTreeSet::from([eur.clone(), usd.clone()]);
    let bindings = vec![
        bind(&eur, v(11), t(0), Some(t(100))),
        bind(&eur, v(12), t(100), None),
        bind(&usd, v(11), t(0), Some(t(200))),
        bind(&usd, v(12), t(200), None),
    ];

    assert!(
        validate_structure_schedule(&required, &bindings, t(0), Some(t(50))).is_ok(),
        "the horizon closes before either market cuts over"
    );
    assert!(
        validate_structure_schedule(&required, &bindings, t(0), Some(t(150))).is_err(),
        "widening it past the first cutover reaches the mismatch"
    );
}

/// A binding on a market the caller did not declare is the caller's fault, not
/// the author's: it is refused as a malformed request rather than reported as a
/// publish violation, exactly as `compose_windows` refuses a price row outside
/// its candidate set.
#[test]
fn a_binding_outside_the_required_set_is_a_malformed_request() {
    let eur = market("EUR", "EU");
    let usd = market("USD", "US");
    let required = BTreeSet::from([eur.clone()]);
    let bindings = vec![bind(&eur, v(11), t(0), None), bind(&usd, v(11), t(0), None)];

    assert!(matches!(
        validate_structure_schedule(&required, &bindings, t(0), None),
        Err(DomainError::InvalidRequest(_))
    ));
}

#[test]
fn an_empty_binding_interval_is_a_malformed_request() {
    let eur = market("EUR", "EU");
    let required = BTreeSet::from([eur.clone()]);
    let bindings = vec![bind(&eur, v(11), t(100), Some(t(100)))];

    assert!(matches!(
        validate_structure_schedule(&required, &bindings, t(0), None),
        Err(DomainError::InvalidRequest(_))
    ));
}

/// A line with no required market is not a line this publish sells, and the
/// sweep says nothing about it — including when bindings exist.
#[test]
fn a_line_with_no_required_market_is_not_judged() {
    let eur = market("EUR", "EU");
    let bindings = vec![bind(&eur, v(11), t(100), None)];

    assert!(validate_structure_schedule(&BTreeSet::new(), &bindings, t(0), None).is_ok());
}
