//! Tests for D-372's row-SKU rules (I3-I6).
//!
//! Every case here is read through [`price_row_rules`] rather than through a
//! rule built by hand, because the registration order *is* part of the contract:
//! `RowSkuPublished` runs first so that a row naming a SKU nobody can read is
//! reported as that, once, rather than as three consequences of it. A test that
//! instantiated one rule would pass with the four registered in any order, or in
//! none.
//!
//! The row fixtures are deliberately **publishable** under the row-local roster
//! that `price_row_rules` appends: the negative cases assert the *first* code,
//! and the two positive cases assert there is no code at all — which a row that
//! merely fails `inst-mk-explicit` would satisfy vacuously in the first form and
//! contradict in the second.

use std::sync::Arc;

use bss_fixtures::ModelKind;
use uuid::Uuid;

use super::RowSkuContext;
use crate::domain::money::{MinorAmount, RateMinor};
use crate::domain::ports::CatalogSku;
use crate::domain::price_row::{BillingGranularity, PriceRow, TierAggregationWindow, TierBand};
use crate::domain::registry_view::SkuIndex;
use crate::domain::rules::{
    FEE_ROW_SKU_METERED, METER_SKU_MISMATCH, ROW_SKU_SELLABLE, SKU_NOT_PUBLISHED,
    USAGE_ROW_SKU_UNMETERED, price_row_rules,
};
use crate::domain::scope_key::{ChargeKind, SkuId};

fn sku(id: u128, unit: Option<&str>, sellable: bool) -> CatalogSku {
    CatalogSku {
        sku_id: Uuid::from_u128(id),
        sku_code: format!("SKU-{id}"),
        name: "fixture".to_owned(),
        metering_unit: unit.map(str::to_owned),
        status: "published".to_owned(),
        plan_tier: None,
        sku_type: "service".to_owned(),
        sellable,
        usage_type_ref: unit.map(|unit| format!("gts.cf.usage.{unit}.v1~")),
    }
}

/// The registry read model these rules judge against, and the plan's own SKU.
fn ctx() -> RowSkuContext {
    let index = SkuIndex::from_listing(vec![
        sku(0x1, None, true),             // the plan's own SKU
        sku(0x2, Some("GB-hour"), false), // a resource
        sku(0x3, Some("GB-hour"), true),  // sellable elsewhere -- refused as a row SKU
        sku(0x4, None, false),            // a fee SKU without a meter
    ]);
    RowSkuContext {
        plan_sku: SkuId::new(Uuid::from_u128(0x1)),
        index: Arc::new(index),
    }
}

/// A band rate in whole minor units, as `rules_tests` states them (D-311).
fn rate(minor_units: i64) -> RateMinor {
    RateMinor::from_minor_units(minor_units).expect("test rate is non-negative")
}

/// A usage row the row-local roster publishes, copied from `rules_tests`.
fn graduated_usage() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("api_calls".to_owned());
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = vec![
        TierBand::closed(0, 1_000, rate(10)),
        TierBand::open(1_000, rate(6)),
    ];
    row
}

/// A recurring row the row-local roster publishes: the plan fee.
fn flat_recurring() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(2_500).expect("test amount is non-negative"));
    row
}

/// A row on `sku`, whose row-local shape is the publishable one for `kind`.
fn row(sku: u128, kind: ChargeKind, meter: Option<&str>) -> PriceRow {
    let mut row = if kind.is_usage() {
        graduated_usage()
    } else {
        flat_recurring()
    };
    row.sku_id = SkuId::new(Uuid::from_u128(sku));
    row.charge_kind = kind;
    row.meter = meter.map(str::to_owned);
    row
}

/// Every code the row reports against `ctx`, in report order.
fn codes_against(ctx: RowSkuContext, row: &PriceRow) -> Vec<String> {
    price_row_rules(ctx)
        .run(row)
        .violations
        .into_iter()
        .map(|violation| violation.code)
        .collect()
}

/// Every code the row reports, in report order -- the diagnostic the `None`
/// cases print when they fail.
fn codes(row: &PriceRow) -> Vec<String> {
    codes_against(ctx(), row)
}

fn first_code(row: &PriceRow) -> Option<String> {
    codes(row).into_iter().next()
}

#[test]
fn a_usage_row_on_an_unmetered_sku_is_refused() {
    assert_eq!(
        first_code(&row(0x4, ChargeKind::Usage, None)).as_deref(),
        Some(USAGE_ROW_SKU_UNMETERED)
    );
}

#[test]
fn a_fee_row_on_a_metered_sku_is_refused() {
    assert_eq!(
        first_code(&row(0x2, ChargeKind::Recurring, None)).as_deref(),
        Some(FEE_ROW_SKU_METERED)
    );
}

#[test]
fn a_stored_meter_that_disagrees_with_the_sku_is_refused() {
    assert_eq!(
        first_code(&row(0x2, ChargeKind::Usage, Some("vCPU-hour"))).as_deref(),
        Some(METER_SKU_MISMATCH)
    );
}

#[test]
fn a_foreign_sellable_sku_is_refused_but_the_plans_own_is_admitted() {
    assert_eq!(
        first_code(&row(0x3, ChargeKind::Usage, Some("GB-hour"))).as_deref(),
        Some(ROW_SKU_SELLABLE)
    );

    let own = row(0x1, ChargeKind::Recurring, None);
    assert_eq!(
        first_code(&own),
        None,
        "the plan fee on the plan's own SKU: {:?}",
        codes(&own)
    );

    let resource = row(0x2, ChargeKind::Usage, Some("GB-hour"));
    assert_eq!(
        first_code(&resource),
        None,
        "a sellable=false resource: {:?}",
        codes(&resource)
    );
}

#[test]
fn an_unknown_or_unpublished_sku_is_refused_sku_not_published() {
    assert_eq!(
        first_code(&row(0x9, ChargeKind::Usage, None)).as_deref(),
        Some(SKU_NOT_PUBLISHED)
    );
    assert_eq!(SKU_NOT_PUBLISHED, "SKU_NOT_PUBLISHED");
}

/// A SKU the registry holds but has **not** published is the same refusal, and
/// it is the half that carries the code's name.
///
/// Both spellings matter: the unknown-SKU arm above would be satisfied by a rule
/// that only checked the lookup, and this gear's registry read model carries
/// `draft` and `deprecated` rows too -- `status` is verbatim registry vocabulary
/// (`CatalogSku::status`), not an enum this gear narrows.
#[test]
fn a_deprecated_sku_is_refused_by_the_same_rule() {
    let mut deprecated = sku(0x7, Some("GB-hour"), false);
    deprecated.status = "deprecated".to_owned();
    let ctx = RowSkuContext {
        plan_sku: SkuId::new(Uuid::from_u128(0x1)),
        index: Arc::new(SkuIndex::from_listing(vec![deprecated])),
    };
    // Otherwise a well-formed row on that SKU: the only fault is the status.
    let subject = row(0x7, ChargeKind::Usage, Some("GB-hour"));

    assert_eq!(codes_against(ctx, &subject), vec![SKU_NOT_PUBLISHED]);
}

/// The other three rules decline to judge a row whose SKU they cannot read.
///
/// The row below carries a meter that agrees with no declaration at all, so a
/// `MeterMatchesSku` that judged an absent SKU would report `METER_SKU_MISMATCH`
/// beside the refusal -- an author told to fix a derivation against a SKU that
/// does not exist. Set equality rather than a first-code check, because that is
/// the half a first-code check cannot see.
#[test]
fn the_rules_after_the_lookup_decline_when_the_sku_is_absent() {
    let subject = row(0x9, ChargeKind::Usage, Some("vCPU-hour"));

    assert_eq!(codes(&subject), vec![SKU_NOT_PUBLISHED]);
}
