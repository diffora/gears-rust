//! Charge-kind cardinality and the phase charge-shape rules.
//!
//! Production change that would make [`removed_setup_kind_is_not_accepted`] pass:
//! drop `ChargeKind::OneTimeSetup` from `parse` / `as_str`. The REST request
//! deserializer is `ScopeKeyRequest` then `scope_key_of`, not a test-only parser.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use uuid::Uuid;

use super::{charge_kinds, requires_frequency};
use crate::api::rest::prices::{ScopeKeyRequest, scope_key_of};
use crate::domain::charge_line::{ChargeLineVersion, ChargeStructure};
use crate::domain::instant::utc_ymd_hms;
use crate::domain::market_price::{MarketPriceTerms, MarketPriceVersion};
use crate::domain::money::{CurrencyCode, MinorAmount, RateMinor};
use crate::domain::plan_rules::{
    AVAILABLE_FROM_IN_PAST, INVALID_CUSTOM_INTERVAL, LINE_MARKET_PRICE_MISSING,
    PHASE_CHARGE_LINES_EMPTY, PHASE_USAGE_INCOMPATIBLE, PURCHASE_QTY_RANGE_INVALID,
    RECURRING_FREQUENCY_REQUIRED,
};
use crate::domain::plan_shape::{
    CustomIntervalUnit, Frequency, PhaseGraph, PhaseKind, PlanPhase, PlanShape, PublishedBaseline,
};
use crate::domain::price_row::{
    AggregationFunction, BillingGranularity, IncludedAllowance, ModelKind, RolloverPolicy,
};
use crate::domain::scope_key::{
    ChargeKind, ChargeLineScopeKey, Cohort, MarketPriceScopeKey, PhaseId, PlanId, PriceEligibility,
    Region, SkuId,
};
use crate::domain::validation::{ValidationReport, ValidationRule};
use time::{Duration, OffsetDateTime};

const MAX_DAYS: u32 = 366;
const MAX_MONTHS: u32 = 24;

const TERMINAL: u128 = 0x7e_11;
const TRIAL: u128 = 0x77_1a;

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x91a4))
}

fn phase_id(seed: u128) -> PhaseId {
    PhaseId::new(Uuid::from_u128(seed))
}

fn sku() -> SkuId {
    SkuId::new(Uuid::from_u128(5))
}

fn now() -> OffsetDateTime {
    utc_ymd_hms(2026, 8, 3, 12, 0, 0)
}

fn currency(code: &str) -> CurrencyCode {
    CurrencyCode::new(code).expect("test currency is three letters")
}

fn region(value: &str) -> Region {
    Region::new(value).expect("test region is non-blank")
}

fn line_key(kind: ChargeKind, on_phase: PhaseId) -> ChargeLineScopeKey {
    ChargeLineScopeKey::new(
        plan(),
        on_phase,
        PriceEligibility::AllSubscriptions,
        kind,
        Cohort::None,
        sku(),
    )
    .expect("all_subscriptions pairs with cohort none")
}

fn line(seed: u128, kind: ChargeKind, on_phase: PhaseId) -> ChargeLineVersion {
    let mut structure = ChargeStructure::new(
        kind,
        Some(if kind == ChargeKind::Usage {
            ModelKind::PerUnit
        } else {
            ModelKind::Flat
        }),
    );
    if kind == ChargeKind::Usage {
        structure.meter = Some("egress.gb".to_owned());
        structure.sku_id = sku();
    }
    ChargeLineVersion {
        charge_line_id: Uuid::from_u128(seed),
        line_version_id: Uuid::from_u128(seed + 0x100),
        scope_key: line_key(kind, on_phase),
        structure,
        billing_timing: None,
        proration_contract: None,
    }
}

fn market(
    line: &ChargeLineVersion,
    code: &str,
    market: &str,
    money: MarketPriceTerms,
) -> MarketPriceVersion {
    MarketPriceVersion {
        market_price_id: Uuid::from_u128(line.charge_line_id.as_u128() + 0x200),
        price_id: Uuid::from_u128(line.charge_line_id.as_u128() + 0x300),
        line_version_id: line.line_version_id,
        scope_key: MarketPriceScopeKey::new(line.scope_key.clone(), currency(code), region(market)),
        money,
    }
}

fn zero_flat() -> MarketPriceTerms {
    MarketPriceTerms {
        amount_minor: Some(MinorAmount::new(0).expect("zero is an explicit operand")),
        ..MarketPriceTerms::default()
    }
}

fn priced_flat() -> MarketPriceTerms {
    MarketPriceTerms {
        amount_minor: Some(MinorAmount::new(2_500).expect("test amount")),
        ..MarketPriceTerms::default()
    }
}

fn priced_rate() -> MarketPriceTerms {
    MarketPriceTerms {
        unit_rate: Some(RateMinor::from_minor_units(3).expect("test rate")),
        ..MarketPriceTerms::default()
    }
}

fn terminal_graph() -> PhaseGraph {
    PhaseGraph::new(vec![PlanPhase {
        phase_id: phase_id(TERMINAL),
        kind: PhaseKind::Evergreen,
        display_name: None,
        ordinal: 1,
        converts_to_phase_id: None,
        phase_duration_days: None,
        display_trial_days: None,
    }])
}

fn trial_then_terminal_graph() -> PhaseGraph {
    PhaseGraph::new(vec![
        PlanPhase {
            phase_id: phase_id(TRIAL),
            kind: PhaseKind::Trial,
            display_name: None,
            ordinal: 0,
            converts_to_phase_id: Some(phase_id(TERMINAL)),
            phase_duration_days: Some(14),
            display_trial_days: Some(14),
        },
        PlanPhase {
            phase_id: phase_id(TERMINAL),
            kind: PhaseKind::Evergreen,
            display_name: None,
            ordinal: 1,
            converts_to_phase_id: None,
            phase_duration_days: None,
            display_trial_days: None,
        },
    ])
}

fn shape_with(lines: Vec<ChargeLineVersion>, prices: Vec<MarketPriceVersion>) -> PlanShape {
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.phases = terminal_graph();
    subject.charge_lines = lines;
    subject.market_prices = prices;
    subject
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

fn details(report: &ValidationReport) -> String {
    report
        .violations
        .iter()
        .map(|violation| format!("{}|{}", violation.subject, violation.detail))
        .collect::<Vec<_>>()
        .join("\n")
}

fn custom(n: u32, unit: CustomIntervalUnit) -> PlanShape {
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.frequency = Some(Frequency::CustomEveryN { n, unit });
    subject
}

fn bounds() -> super::CustomIntervalBounds {
    super::CustomIntervalBounds::new(MAX_DAYS, MAX_MONTHS)
}

fn setup_json(charge_kind: &str) -> String {
    format!(
        r#"{{
            "sku_id": "00000000-0000-0000-0000-000000000005",
            "currency": "usd",
            "region": "US",
            "phase": "00000000-0000-0000-0000-000000000022",
            "price_eligibility": "all_subscriptions",
            "charge_kind": "{charge_kind}",
            "cohort": null
        }}"#
    )
}

#[test]
fn removed_setup_kind_is_not_accepted() {
    assert!(ChargeKind::parse("one_time_setup").is_none());
    assert_eq!(ChargeKind::parse("one_time"), Some(ChargeKind::OneTime));
}

#[test]
fn charge_kind_cardinality_is_three_live_tokens() {
    assert_eq!(ChargeKind::ALL.len(), 3, "one member per live variant");
    let tokens: BTreeSet<&str> = ChargeKind::ALL.iter().map(|kind| kind.as_str()).collect();
    assert_eq!(tokens, BTreeSet::from(["recurring", "usage", "one_time"]));
    for kind in ChargeKind::ALL {
        assert_eq!(ChargeKind::parse(kind.as_str()), Some(*kind));
    }
}

#[test]
fn rest_request_deserializer_rejects_setup_and_accepts_one_time() {
    let setup: ScopeKeyRequest =
        serde_json::from_str(&setup_json("one_time_setup")).expect("wire DTO is a string");
    assert!(ChargeKind::parse(&setup.charge_kind).is_none());
    assert!(
        scope_key_of(plan(), &setup).is_err(),
        "REST scope_key_of must refuse the removed kind, not a test-only parser"
    );

    let one_time: ScopeKeyRequest =
        serde_json::from_str(&setup_json("one_time")).expect("wire DTO is a string");
    assert_eq!(
        ChargeKind::parse(&one_time.charge_kind),
        Some(ChargeKind::OneTime)
    );
    assert!(scope_key_of(plan(), &one_time).is_ok());
}

#[test]
fn charge_kinds_collects_presence_from_lines() {
    let one_time = line(1, ChargeKind::OneTime, phase_id(TERMINAL));
    let usage = line(2, ChargeKind::Usage, phase_id(TERMINAL));
    let recurring = line(3, ChargeKind::Recurring, phase_id(TERMINAL));
    assert_eq!(
        charge_kinds(std::slice::from_ref(&one_time)),
        BTreeSet::from([ChargeKind::OneTime])
    );
    assert_eq!(
        charge_kinds(std::slice::from_ref(&usage)),
        BTreeSet::from([ChargeKind::Usage])
    );
    assert_eq!(
        charge_kinds(std::slice::from_ref(&recurring)),
        BTreeSet::from([ChargeKind::Recurring])
    );
    assert_eq!(
        charge_kinds(&[one_time, usage, recurring]),
        BTreeSet::from([
            ChargeKind::OneTime,
            ChargeKind::Usage,
            ChargeKind::Recurring
        ])
    );
}

#[test]
fn frequency_is_required_only_when_a_recurring_line_exists() {
    assert!(!requires_frequency(&[
        line(1, ChargeKind::OneTime, phase_id(TERMINAL)),
        line(2, ChargeKind::Usage, phase_id(TERMINAL)),
    ]));
    assert!(requires_frequency(&[line(
        3,
        ChargeKind::Recurring,
        phase_id(TERMINAL)
    )]));
}

#[test]
fn a_one_time_only_plan_does_not_owe_frequency() {
    let one_time = line(1, ChargeKind::OneTime, phase_id(TERMINAL));
    let subject = shape_with(
        vec![one_time.clone()],
        vec![market(&one_time, "usd", "US", priced_flat())],
    );
    assert!(findings(&super::RecurringFrequencyRequired, &subject).is_publishable());
}

#[test]
fn a_usage_only_plan_does_not_owe_frequency() {
    let usage = line(1, ChargeKind::Usage, phase_id(TERMINAL));
    let subject = shape_with(
        vec![usage.clone()],
        vec![market(&usage, "usd", "US", priced_rate())],
    );
    assert!(findings(&super::RecurringFrequencyRequired, &subject).is_publishable());
}

#[test]
fn a_recurring_only_plan_without_frequency_is_refused() {
    let recurring = line(1, ChargeKind::Recurring, phase_id(TERMINAL));
    let subject = shape_with(
        vec![recurring.clone()],
        vec![market(&recurring, "usd", "US", priced_flat())],
    );
    let report = findings(&super::RecurringFrequencyRequired, &subject);
    assert_eq!(codes(&report), vec![RECURRING_FREQUENCY_REQUIRED]);
}

#[test]
fn a_recurring_only_plan_with_frequency_publishes_that_arm() {
    let recurring = line(1, ChargeKind::Recurring, phase_id(TERMINAL));
    let mut subject = shape_with(
        vec![recurring.clone()],
        vec![market(&recurring, "usd", "US", priced_flat())],
    );
    subject.frequency = Some(Frequency::Monthly);
    assert!(findings(&super::RecurringFrequencyRequired, &subject).is_publishable());
}

#[test]
fn mixed_kinds_in_one_phase_are_legal() {
    let recurring = line(1, ChargeKind::Recurring, phase_id(TERMINAL));
    let usage = line(2, ChargeKind::Usage, phase_id(TERMINAL));
    let one_time = line(3, ChargeKind::OneTime, phase_id(TERMINAL));
    let mut subject = shape_with(
        vec![recurring.clone(), usage.clone(), one_time.clone()],
        vec![
            market(&recurring, "usd", "US", priced_flat()),
            market(&usage, "usd", "US", priced_rate()),
            market(&one_time, "usd", "US", priced_flat()),
        ],
    );
    subject.frequency = Some(Frequency::Monthly);
    assert!(findings(&super::PhaseChargeLinesPresent, &subject).is_publishable());
    assert!(findings(&super::LineMarketPricePresent, &subject).is_publishable());
    assert!(findings(&super::RecurringFrequencyRequired, &subject).is_publishable());
}

#[test]
fn an_empty_draft_without_phases_is_not_missing_charge_lines() {
    let subject = PlanShape::new(plan(), 0, now());
    assert!(findings(&super::PhaseChargeLinesPresent, &subject).is_publishable());
}

#[test]
fn an_ordinary_phase_with_no_logical_lines_fails_at_publish() {
    let subject = shape_with(Vec::new(), Vec::new());
    let report = findings(&super::PhaseChargeLinesPresent, &subject);
    assert_eq!(codes(&report), vec![PHASE_CHARGE_LINES_EMPTY]);
    assert!(
        report.violations[0]
            .subject
            .contains(&phase_id(TERMINAL).to_string()),
        "the empty phase is named: {}",
        report.violations[0].subject
    );
}

#[test]
fn two_currency_rows_of_one_line_are_one_logical_line() {
    let recurring = line(1, ChargeKind::Recurring, phase_id(TERMINAL));
    let mut subject = shape_with(
        vec![recurring.clone()],
        vec![market(&recurring, "usd", "US", priced_flat()), {
            let mut eur = market(&recurring, "eur", "DE", priced_flat());
            eur.market_price_id = Uuid::from_u128(0x51);
            eur.price_id = Uuid::from_u128(0x52);
            eur
        }],
    );
    subject.frequency = Some(Frequency::Monthly);
    assert!(findings(&super::PhaseChargeLinesPresent, &subject).is_publishable());
    assert_eq!(subject.charge_lines.len(), 1);
    assert_eq!(subject.market_prices.len(), 2);
}

#[test]
fn a_missing_market_variant_is_named_with_phase_line_and_market() {
    let recurring = line(1, ChargeKind::Recurring, phase_id(TERMINAL));
    let mut subject = shape_with(
        vec![recurring.clone()],
        vec![market(&recurring, "usd", "US", priced_flat()), {
            let other = line(2, ChargeKind::OneTime, phase_id(TERMINAL));
            market(&other, "eur", "DE", priced_flat())
        }],
    );
    subject.frequency = Some(Frequency::Monthly);
    subject.charge_lines = vec![recurring];
    let report = findings(&super::LineMarketPricePresent, &subject);
    assert_eq!(codes(&report), vec![LINE_MARKET_PRICE_MISSING]);
    let subject_key = &report.violations[0].subject;
    assert!(
        subject_key.contains(&phase_id(TERMINAL).to_string()),
        "{subject_key}"
    );
    assert!(
        subject_key.contains("EUR") || subject_key.contains("eur"),
        "{subject_key}"
    );
    assert!(subject_key.contains("DE"), "{subject_key}");
}

#[test]
fn an_explicit_zero_operand_counts_as_a_binding() {
    let usage = line(1, ChargeKind::Usage, phase_id(TERMINAL));
    let subject = shape_with(
        vec![usage.clone()],
        vec![market(&usage, "usd", "US", zero_flat())],
    );
    assert!(findings(&super::LineMarketPricePresent, &subject).is_publishable());
}

#[test]
fn absent_operands_are_not_a_binding() {
    let usage = line(1, ChargeKind::Usage, phase_id(TERMINAL));
    let subject = shape_with(
        vec![usage.clone()],
        vec![market(&usage, "usd", "US", MarketPriceTerms::default())],
    );
    let report = findings(&super::LineMarketPricePresent, &subject);
    assert_eq!(codes(&report), vec![LINE_MARKET_PRICE_MISSING]);
}

#[test]
fn usage_present_only_in_another_phase_is_incompatible() {
    let trial_usage = line(1, ChargeKind::Usage, phase_id(TRIAL));
    let evergreen_recurring = line(2, ChargeKind::Recurring, phase_id(TERMINAL));
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.phases = trial_then_terminal_graph();
    subject.frequency = Some(Frequency::Monthly);
    subject.charge_lines = vec![trial_usage.clone(), evergreen_recurring.clone()];
    subject.market_prices = vec![
        market(&trial_usage, "usd", "US", priced_rate()),
        market(&evergreen_recurring, "usd", "US", priced_flat()),
    ];
    let report = findings(&super::PhaseUsageCompatible, &subject);
    assert_eq!(codes(&report), vec![PHASE_USAGE_INCOMPATIBLE]);
}

#[test]
fn consecutive_phase_usage_may_change_price_but_not_counter_fields() {
    let trial_usage = line(1, ChargeKind::Usage, phase_id(TRIAL));
    let mut evergreen_usage = line(2, ChargeKind::Usage, phase_id(TERMINAL));
    evergreen_usage.structure.billing_granularity = Some(BillingGranularity::PerHour);
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.phases = trial_then_terminal_graph();
    subject.charge_lines = vec![trial_usage.clone(), evergreen_usage.clone()];
    subject.market_prices = vec![
        market(&trial_usage, "usd", "US", priced_rate()),
        market(&evergreen_usage, "usd", "US", priced_rate()),
    ];
    let report = findings(&super::PhaseUsageCompatible, &subject);
    assert_eq!(codes(&report), vec![PHASE_USAGE_INCOMPATIBLE]);

    evergreen_usage.structure.billing_granularity = None;
    subject.charge_lines = vec![trial_usage.clone(), evergreen_usage.clone()];
    subject.market_prices = vec![
        market(&trial_usage, "usd", "US", priced_rate()),
        market(&evergreen_usage, "usd", "US", {
            let mut cheaper = priced_rate();
            cheaper.unit_rate = Some(RateMinor::from_minor_units(0).expect("zero rate"));
            cheaper
        }),
    ];
    assert!(findings(&super::PhaseUsageCompatible, &subject).is_publishable());
}

#[test]
fn allowance_quantity_on_a_compatible_ladder_does_not_fail_continuation() {
    let mut trial_usage = line(1, ChargeKind::Usage, phase_id(TRIAL));
    trial_usage.structure.included_allowance = Some(IncludedAllowance {
        quantity: 10,
        rollover_policy: RolloverPolicy::None,
    });
    let mut evergreen_usage = line(2, ChargeKind::Usage, phase_id(TERMINAL));
    evergreen_usage.structure.included_allowance = Some(IncludedAllowance {
        quantity: 50,
        rollover_policy: RolloverPolicy::None,
    });
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.phases = trial_then_terminal_graph();
    subject.charge_lines = vec![trial_usage.clone(), evergreen_usage.clone()];
    subject.market_prices = vec![
        market(&trial_usage, "usd", "US", priced_rate()),
        market(&evergreen_usage, "usd", "US", priced_rate()),
    ];
    assert!(findings(&super::PhaseUsageCompatible, &subject).is_publishable());
}

#[test]
fn authored_default_aggregation_matches_unauthored_on_continuation() {
    let trial_usage = line(1, ChargeKind::Usage, phase_id(TRIAL));
    let mut evergreen_usage = line(2, ChargeKind::Usage, phase_id(TERMINAL));
    evergreen_usage.structure.aggregation_function = Some(AggregationFunction::Sum);
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.phases = trial_then_terminal_graph();
    subject.charge_lines = vec![trial_usage.clone(), evergreen_usage.clone()];
    subject.market_prices = vec![
        market(&trial_usage, "usd", "US", priced_rate()),
        market(&evergreen_usage, "usd", "US", priced_rate()),
    ];
    assert!(findings(&super::PhaseUsageCompatible, &subject).is_publishable());
}

#[test]
fn grandfathered_only_markets_are_not_required_sold_markets() {
    let selling = line(1, ChargeKind::Usage, phase_id(TERMINAL));
    let mut grandfathered = line(2, ChargeKind::Usage, phase_id(TERMINAL));
    grandfathered.scope_key = ChargeLineScopeKey::new(
        plan(),
        phase_id(TERMINAL),
        PriceEligibility::ExistingGrandfathered,
        ChargeKind::Usage,
        Cohort::Generation(now()),
        sku(),
    )
    .expect("grandfathered pairs with a generation");
    let subject = shape_with(
        vec![selling.clone(), grandfathered.clone()],
        vec![
            market(&selling, "usd", "US", priced_rate()),
            market(&grandfathered, "eur", "DE", priced_rate()),
        ],
    );
    assert!(findings(&super::LineMarketPricePresent, &subject).is_publishable());
}

#[test]
fn a_non_positive_custom_interval_fails() {
    let report = findings(&bounds(), &custom(0, CustomIntervalUnit::Days));

    assert_eq!(codes(&report), vec![INVALID_CUSTOM_INTERVAL]);
    assert!(
        details(&report).contains("days(0)"),
        "the report names n and its unit"
    );
}

#[test]
fn the_days_cap_is_inclusive_and_an_over_cap_interval_is_rejected_not_clamped() {
    let at_cap = findings(&bounds(), &custom(MAX_DAYS, CustomIntervalUnit::Days));
    assert!(at_cap.is_publishable());

    let over_cap = findings(&bounds(), &custom(MAX_DAYS + 1, CustomIntervalUnit::Days));
    assert_eq!(codes(&over_cap), vec![INVALID_CUSTOM_INTERVAL]);
    let detail = details(&over_cap);
    assert!(
        detail.contains("days(367)"),
        "the report names n and the unit"
    );
    assert!(
        detail.contains("366"),
        "the report names the cap it applied"
    );
    assert!(
        detail.contains("never clamped"),
        "the refusal states that the value was not substituted"
    );
}

#[test]
fn the_months_cap_is_its_own_bound() {
    let at_cap = findings(&bounds(), &custom(MAX_MONTHS, CustomIntervalUnit::Months));
    assert!(at_cap.is_publishable());

    let over_cap = findings(
        &bounds(),
        &custom(MAX_MONTHS + 1, CustomIntervalUnit::Months),
    );
    assert_eq!(codes(&over_cap), vec![INVALID_CUSTOM_INTERVAL]);
    assert!(details(&over_cap).contains("months(25)"));

    let same_n_in_days = findings(&bounds(), &custom(MAX_MONTHS + 1, CustomIntervalUnit::Days));
    assert!(
        same_n_in_days.is_publishable(),
        "25 is over the months cap and well inside the days cap"
    );
}

#[test]
fn a_fixed_frequency_and_an_unauthored_one_are_not_this_rules_business() {
    let mut fixed = PlanShape::new(plan(), 3, now());
    fixed.frequency = Some(Frequency::Quarterly);
    assert!(findings(&bounds(), &fixed).is_publishable());

    let unauthored = PlanShape::new(plan(), 3, now());
    assert!(findings(&bounds(), &unauthored).is_publishable());
}

#[test]
fn an_inverted_purchase_quantity_window_fails() {
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.purchase_min_qty = Some(10);
    subject.purchase_max_qty = Some(2);

    let report = findings(&super::PurchaseQtyRange, &subject);

    assert_eq!(codes(&report), vec![PURCHASE_QTY_RANGE_INVALID]);
    let detail = details(&report);
    assert!(detail.contains("purchase_min_qty 10"));
    assert!(detail.contains("purchase_max_qty 2"));
}

#[test]
fn an_equal_a_wider_and_a_half_authored_window_all_pass() {
    let mut equal = PlanShape::new(plan(), 3, now());
    equal.purchase_min_qty = Some(5);
    equal.purchase_max_qty = Some(5);
    assert!(findings(&super::PurchaseQtyRange, &equal).is_publishable());

    let mut wider = PlanShape::new(plan(), 3, now());
    wider.purchase_min_qty = Some(1);
    wider.purchase_max_qty = Some(100);
    assert!(findings(&super::PurchaseQtyRange, &wider).is_publishable());

    let mut floor_only = PlanShape::new(plan(), 3, now());
    floor_only.purchase_min_qty = Some(99);
    assert!(findings(&super::PurchaseQtyRange, &floor_only).is_publishable());

    let mut ceiling_only = PlanShape::new(plan(), 3, now());
    ceiling_only.purchase_max_qty = Some(1);
    assert!(findings(&super::PurchaseQtyRange, &ceiling_only).is_publishable());
}

#[test]
fn a_newly_set_past_available_from_fails() {
    let backdated = now() - Duration::days(30);
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.available_from = Some(backdated);
    let report = findings(&super::AvailableFromNotBackdated, &subject);
    assert_eq!(codes(&report), vec![AVAILABLE_FROM_IN_PAST]);
}

#[test]
fn a_re_published_unchanged_available_from_that_has_since_passed_passes() {
    let authored = now() - Duration::days(30);
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.available_from = Some(authored);
    subject.baseline = Some(PublishedBaseline {
        terminal_phase_id: phase_id(TERMINAL),
        phase_ids_in_use: [phase_id(TERMINAL)].into(),
        available_from: Some(authored),
        available_to: None,
    });

    assert!(findings(&super::AvailableFromNotBackdated, &subject).is_publishable());
}

#[test]
fn a_changed_past_available_from_still_fails_against_a_baseline() {
    let published = now() - Duration::days(30);
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.available_from = Some(now() - Duration::days(60));
    subject.baseline = Some(PublishedBaseline {
        terminal_phase_id: phase_id(TERMINAL),
        phase_ids_in_use: [phase_id(TERMINAL)].into(),
        available_from: Some(published),
        available_to: None,
    });

    let report = findings(&super::AvailableFromNotBackdated, &subject);

    assert_eq!(codes(&report), vec![AVAILABLE_FROM_IN_PAST]);
}

#[test]
fn a_first_publish_setting_a_past_date_fails_and_a_future_date_passes() {
    let mut backdated = PlanShape::new(plan(), 3, now());
    backdated.available_from = Some(now() - Duration::seconds(1));
    assert!(
        backdated.baseline.is_none(),
        "a first publish has no baseline"
    );
    assert_eq!(
        codes(&findings(&super::AvailableFromNotBackdated, &backdated)),
        vec![AVAILABLE_FROM_IN_PAST]
    );

    let mut dated_forward = PlanShape::new(plan(), 3, now());
    dated_forward.available_from = Some(now() + Duration::days(7));
    assert!(findings(&super::AvailableFromNotBackdated, &dated_forward).is_publishable());

    let mut at_the_instant = PlanShape::new(plan(), 3, now());
    at_the_instant.available_from = Some(now());
    assert!(findings(&super::AvailableFromNotBackdated, &at_the_instant).is_publishable());

    let unauthored = PlanShape::new(plan(), 3, now());
    assert!(findings(&super::AvailableFromNotBackdated, &unauthored).is_publishable());
}

#[test]
fn a_baseline_that_carried_no_date_makes_a_past_one_newly_set() {
    let mut subject = PlanShape::new(plan(), 3, now());
    subject.available_from = Some(now() - Duration::days(1));
    subject.baseline = Some(PublishedBaseline {
        terminal_phase_id: phase_id(TERMINAL),
        phase_ids_in_use: [phase_id(TERMINAL)].into(),
        available_from: None,
        available_to: None,
    });

    assert_eq!(
        codes(&findings(&super::AvailableFromNotBackdated, &subject)),
        vec![AVAILABLE_FROM_IN_PAST]
    );
}
