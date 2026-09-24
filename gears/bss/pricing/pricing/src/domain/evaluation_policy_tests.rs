//! Field-partition and generation-shape checks for the legacy evaluation policy.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use super::{
    EVALUATION_POLICY_GENERATION, partition_plan_fields, partition_row_fields,
    partition_structure_fields,
};
use crate::domain::charge_line::ChargeStructure;
use crate::domain::contracts::{PlanChangeContract, UsageCounterOnPlanChange};
use crate::domain::price_row::PriceRow;
use crate::domain::scope_key::{ChargeKind, SkuId};
use uuid::Uuid;

/// A row whose only purpose is to give the exhaustive pattern something to
/// match — the partition is a statement about the *shape* of a price row and
/// never about a value in one.
/// Any plan-change contract: the partition reads the **shape**, never the values.
fn any_plan_contract() -> PlanChangeContract {
    PlanChangeContract {
        allowed_change_targets: None,
        comparability_rank: None,
        usage_counter_on_plan_change: UsageCounterOnPlanChange::Reset,
    }
}

fn any_row() -> PriceRow {
    PriceRow {
        invoice_line_template: None,
        gl_code_ref: None,
        charge_kind: ChargeKind::Recurring,
        model_kind: None,
        amount_minor: None,
        unit_rate: None,
        bands: Vec::new(),
        package_size: None,
        package_price_minor: None,
        quantity_source: None,
        manual_quantity: None,
        sku_id: SkuId::new(Uuid::from_u128(5)),
        meter: None,
        dimension_key: String::new(),
        billing_granularity: None,
        tier_aggregation_window: None,
        tier_qualification_window: None,
        aggregation_function: None,
        aggregation_granularity: None,
        max_hold_granules: None,
        included_allowance: None,
        reserved_rate: None,
        reservation_flavor: None,
        min_qty_purchase: None,
        min_qty_usage: None,
        min_qty_usage_fallback: None,
        discount_ref: None,
    }
}

fn any_structure() -> ChargeStructure {
    ChargeStructure {
        invoice_line_template: None,
        gl_code_ref: None,
        charge_kind: ChargeKind::Recurring,
        model_kind: None,
        package_size: None,
        quantity_source: None,
        manual_quantity: None,
        sku_id: SkuId::new(Uuid::from_u128(5)),
        meter: None,
        dimension_key: String::new(),
        billing_granularity: None,
        tier_aggregation_window: None,
        tier_qualification_window: None,
        aggregation_function: None,
        aggregation_granularity: None,
        max_hold_granules: None,
        included_allowance: None,
        reservation_flavor: None,
        min_qty_purchase: None,
        min_qty_usage: None,
        min_qty_usage_fallback: None,
        discount_ref: None,
    }
}

#[test]
fn every_field_of_the_row_is_classified_exactly_once() {
    let (roster, outside) = partition_row_fields(&any_row());

    // No stated total, and none is available: `partition_row_fields` generates its
    // destructure from the two lists, so a field the row grows and neither list
    // names is a non-exhaustive pattern rather than anything a number here could
    // catch. A literal total is the opposite check -- it fails when the lists grow
    // and the literal does not, which is bookkeeping, and it passes over the field
    // that was never classified at all.
    //
    // What is left is the half a pattern cannot see: one field written into both
    // lists, or one name written twice.
    let union: BTreeSet<&str> = roster.iter().chain(outside.iter()).copied().collect();
    assert_eq!(
        union.len(),
        roster.len() + outside.len(),
        "a field is classified twice or named twice"
    );
}

#[test]
fn every_field_of_the_plan_contract_is_classified_exactly_once() {
    let (roster, outside) = partition_plan_fields(&any_plan_contract());

    // Derived, for `every_field_of_the_row_is_classified_exactly_once`'s reason:
    // the destructure comes from these lists, so an unclassified field does not
    // compile and a total here would only restate what the pattern already refuses.
    let union: BTreeSet<&str> = roster.iter().chain(outside.iter()).copied().collect();
    assert_eq!(
        union.len(),
        roster.len() + outside.len(),
        "a field is classified twice or named twice"
    );
}

#[test]
fn the_shared_structure_roster_is_the_rows() {
    let (row_roster, _) = partition_row_fields(&any_row());
    let (structure_roster, _) = partition_structure_fields(&any_structure());
    assert_eq!(
        structure_roster, row_roster,
        "evaluation-policy fields live on the shared structure; a split that moved one \
         would be a generation bump, which this change must not do"
    );
}

#[test]
fn the_shared_structure_omits_only_market_money_columns() {
    let (_, row_outside) = partition_row_fields(&any_row());
    let (_, structure_outside) = partition_structure_fields(&any_structure());
    let money: BTreeSet<&str> = [
        "amount_minor",
        "unit_rate",
        // The ladder is a market's: a band's bounds sit beside the rate that
        // prices it, so the whole set is money and none of it is shared.
        "bands",
        "package_price_minor",
        "reserved_rate",
    ]
    .into_iter()
    .collect();
    let expected: BTreeSet<&str> = row_outside
        .into_iter()
        .filter(|field| !money.contains(field))
        .collect();
    assert_eq!(
        structure_outside.into_iter().collect::<BTreeSet<_>>(),
        expected,
        "ChargeStructure outside the roster is PriceRow's outside set minus market money"
    );
}

#[test]
fn every_field_of_the_shared_structure_is_classified_exactly_once() {
    let (roster, outside) = partition_structure_fields(&any_structure());
    let union: BTreeSet<&str> = roster.iter().chain(outside.iter()).copied().collect();
    assert_eq!(
        union.len(),
        roster.len() + outside.len(),
        "a field is classified twice or named twice"
    );
}

#[test]
fn the_generation_is_shaped_the_way_the_decision_declares_it() {
    // `ep-<n>`, n a positive integer: consumers compare it for equality and read
    // nothing out of it, so the only thing the format has to guarantee is that
    // two field sets never share a string.
    let ordinal = EVALUATION_POLICY_GENERATION
        .strip_prefix("ep-")
        .expect("the generation is `ep-<n>`");
    let ordinal: u32 = ordinal.parse().expect("`<n>` is an integer");
    assert!(ordinal >= 1);
}
