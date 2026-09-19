//! Round-trip tests for shared charge structure versus market money.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use crate::domain::charge_line::ChargeLineVersion;
use crate::domain::concurrency::RowVersion;
use crate::domain::contracts::{BillingAnchorPolicy, ProrationBasis, ProrationContract};
use crate::domain::error::DomainError;
use crate::domain::instant::utc_ymd_hms;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::market_price::{
    MARKET_TIER_RATE_COUNT_MISMATCH, MarketPriceTerms, MarketPriceVersion, resolve_row, split_row,
};
use crate::domain::money::{CurrencyCode, MinorAmount, RateMinor};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{
    BillingGranularity, IncludedAllowance, MinQtyUsageFallback, ModelKind, PriceRow,
    QuantitySource, ReservationFlavor, RolloverPolicy, TierAggregationWindow, TierBand,
};
use crate::domain::rules::{AMOUNT_PLACEMENT_INVALID, EVAL_POLICY_MISPLACED};
use crate::domain::scope_key::{
    ChargeKind, ChargeLineScopeKey, Cohort, MarketPriceScopeKey, PhaseId, PlanId, PriceEligibility,
    Region, SkuId,
};
use uuid::Uuid;

fn minor(units: i64) -> MinorAmount {
    MinorAmount::new(units).expect("test amount is non-negative")
}

fn rate(minor_units: i64) -> RateMinor {
    RateMinor::from_minor_units(minor_units).expect("test rate is non-negative")
}

fn sub_minor(nano: i64) -> RateMinor {
    RateMinor::from_nano_minor(nano).expect("test rate is non-negative")
}

fn round_trip(row: &PriceRow) {
    let (structure, money) = split_row(row.clone());
    assert_eq!(
        resolve_row(&structure, &money).expect("resolved join is publishable"),
        *row
    );
}

fn codes(err: DomainError) -> Vec<String> {
    match err {
        DomainError::ValidationFailed(report) => report
            .violations
            .iter()
            .map(|violation| violation.code.clone())
            .collect(),
        other => panic!("expected ValidationFailed, got {other:?}"),
    }
}

fn line() -> ChargeLineScopeKey {
    ChargeLineScopeKey::new(
        PlanId::new(Uuid::from_u128(0x9_1a4)),
        PhaseId::new(Uuid::from_u128(0xfa_5e)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
        SkuId::new(Uuid::from_u128(5)),
    )
    .expect("all_subscriptions pairs with cohort none")
}

fn market(region: &str) -> MarketPriceScopeKey {
    MarketPriceScopeKey::new(
        line(),
        CurrencyCode::new("usd").expect("USD is three letters"),
        Region::new(region).expect("a non-blank region"),
    )
}

fn timing() -> ProrationContract {
    ProrationContract {
        billing_anchor_policy: BillingAnchorPolicy::SubscriptionStart,
        proration_basis: ProrationBasis::CalendarDaysActual,
        credit_on_downgrade: true,
    }
}

#[test]
fn changing_market_money_does_not_change_structure() {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(1500).unwrap());
    let (structure, mut money) = split_row(row.clone());
    assert_eq!(resolve_row(&structure, &money).unwrap(), row);
    money.amount_minor = Some(MinorAmount::new(1700).unwrap());
    let changed = resolve_row(&structure, &money).unwrap();
    assert_eq!(changed.amount_minor, money.amount_minor);
    assert_eq!(split_row(changed).0, structure);
}

#[test]
fn per_unit_round_trip_keeps_sub_minor_rate_and_descriptors() {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::PerUnit));
    row.unit_rate = Some(sub_minor(15_000_000));
    row.quantity_source = Some(QuantitySource::SubscriptionSeatCount);
    row.invoice_line_template = Some("{sku} - {period}".to_owned());
    row.gl_code_ref = Some("4000".to_owned());
    row.discount_ref = Some("promo-seat".to_owned());
    row.min_qty_purchase = Some(1);
    round_trip(&row);

    let (structure, mut money) = split_row(row);
    money.unit_rate = Some(sub_minor(23_000_000));
    let changed = resolve_row(&structure, &money).expect("rate change stays publishable");
    assert_eq!(changed.unit_rate, money.unit_rate);
    assert_eq!(split_row(changed).0, structure);
}

#[test]
fn graduated_round_trip_keeps_sub_minor_tier_rates() {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("egress_bytes".to_owned());
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = vec![
        TierBand::closed(0, 1_000, sub_minor(15_000_000)),
        TierBand::open(1_000, sub_minor(11_000_000)),
    ];
    row.reserved_rate = Some(sub_minor(1_666_667));
    row.reservation_flavor = Some(ReservationFlavor::Capacity);
    row.min_qty_usage = Some(1);
    row.min_qty_usage_fallback = Some(MinQtyUsageFallback::Exception);
    round_trip(&row);
}

#[test]
fn volume_round_trip_preserves_zero_opening_rate() {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Volume));
    row.meter = Some("api_calls".to_owned());
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
    row.bands = vec![
        TierBand::closed(0, 100, RateMinor::ZERO),
        TierBand::open(100, rate(4)),
    ];
    round_trip(&row);
}

#[test]
fn package_round_trip_allows_a_zero_block_price() {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Package));
    row.meter = Some("objects".to_owned());
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
    row.package_size = Some(100);
    row.package_price_minor = Some(minor(0));
    round_trip(&row);
}

#[test]
fn per_unit_usage_round_trip_carries_allowance() {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    row.meter = Some("egress.gb".to_owned());
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
    row.unit_rate = Some(rate(2));
    row.included_allowance = Some(IncludedAllowance {
        quantity: 50,
        rollover_policy: RolloverPolicy::None,
    });
    round_trip(&row);
}

#[test]
fn missing_tier_rate_is_a_count_mismatch_not_a_partial_row() {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("egress_bytes".to_owned());
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = vec![
        TierBand::closed(0, 1_000, rate(10)),
        TierBand::open(1_000, rate(6)),
    ];
    let (structure, mut money) = split_row(row);
    money.tier_rates.pop();
    let err = resolve_row(&structure, &money).expect_err("unequal counts must not join");
    assert_eq!(codes(err), vec![MARKET_TIER_RATE_COUNT_MISMATCH]);
}

#[test]
fn extra_tier_rate_is_a_count_mismatch() {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("egress_bytes".to_owned());
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = vec![
        TierBand::closed(0, 1_000, rate(10)),
        TierBand::open(1_000, rate(6)),
    ];
    let (structure, mut money) = split_row(row);
    money.tier_rates.push(rate(1));
    let err = resolve_row(&structure, &money).expect_err("unequal counts must not join");
    assert_eq!(codes(err), vec![MARKET_TIER_RATE_COUNT_MISMATCH]);
}

#[test]
fn a_flat_row_with_a_tier_rate_is_invalid() {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(minor(1_500));
    let (structure, mut money) = split_row(row);
    money.tier_rates = vec![rate(9)];
    let err = resolve_row(&structure, &money).expect_err("flat + tier rate must not join");
    let found = codes(err);
    assert!(
        found.contains(&MARKET_TIER_RATE_COUNT_MISMATCH.to_owned())
            || found
                .iter()
                .any(|code| code == EVAL_POLICY_MISPLACED || code == AMOUNT_PLACEMENT_INVALID),
        "expected a count mismatch or an existing operand rule, got {found:?}"
    );
}

#[test]
fn tax_and_rounding_stay_on_the_market_record() {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(minor(0));
    let record = PriceRecord {
        resolved_invoice_line_template: None,
        resolved_gl_code: None,
        price_id: Uuid::from_u128(0xb_10),
        scope_key: market("US"),
        row,
        tax_inclusive: true,
        tax_category_ref: Some("standard-vat".to_owned()),
        billing_timing: Some("advance".to_owned()),
        proration_contract: Some(timing()),
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: Some(utc_ymd_hms(2027, 1, 1, 0, 0, 0)),
        supersedes_price_id: Some(Uuid::from_u128(0xb_0f)),
        lifecycle_state: LifecycleState::Draft,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: utc_ymd_hms(2026, 8, 2, 10, 0, 0),
        row_version: RowVersion::new(3),
    };

    let (structure, money) = split_row(record.row.clone());
    round_trip(&record.row);
    assert!(money.amount_minor.is_some());
    assert!(structure.bands.is_empty());
    assert!(record.tax_inclusive);
    assert_eq!(record.tax_category_ref.as_deref(), Some("standard-vat"));
    assert_eq!(record.rounding_policy_ref.as_deref(), Some("half_up"));
    assert!(record.grandfather_until.is_some());
    assert!(record.supersedes_price_id.is_some());
    assert_eq!(record.billing_timing.as_deref(), Some("advance"));

    let line_version = ChargeLineVersion {
        charge_line_id: Uuid::from_u128(0xc_01),
        line_version_id: Uuid::from_u128(0xc_02),
        scope_key: line(),
        structure,
        billing_timing: record.billing_timing.clone(),
        proration_contract: record.proration_contract,
    };
    let us = MarketPriceVersion {
        market_price_id: Uuid::from_u128(0xd_01),
        price_id: record.price_id,
        line_version_id: line_version.line_version_id,
        scope_key: market("US"),
        money: money.clone(),
    };
    let mut ca_money = money;
    ca_money.amount_minor = Some(minor(1_900));
    let ca = MarketPriceVersion {
        market_price_id: Uuid::from_u128(0xd_02),
        price_id: Uuid::from_u128(0xb_11),
        line_version_id: line_version.line_version_id,
        scope_key: market("CA"),
        money: ca_money,
    };

    assert_eq!(us.line_version_id, ca.line_version_id);
    assert_eq!(us.scope_key.line(), ca.scope_key.line());
    assert_ne!(us.scope_key, ca.scope_key);
    assert_eq!(
        resolve_row(&line_version.structure, &us.money)
            .expect("US money joins")
            .amount_minor,
        Some(minor(0))
    );
    assert_eq!(
        resolve_row(&line_version.structure, &ca.money)
            .expect("CA money joins")
            .amount_minor,
        Some(minor(1_900))
    );
}

#[test]
fn market_terms_default_is_an_empty_operand_set() {
    assert_eq!(
        MarketPriceTerms::default(),
        MarketPriceTerms {
            amount_minor: None,
            unit_rate: None,
            tier_rates: Vec::new(),
            package_price_minor: None,
            reserved_rate: None,
        }
    );
}
