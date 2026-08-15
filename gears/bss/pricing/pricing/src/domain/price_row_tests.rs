//! Tests for the authored price-row shape.

use std::collections::HashSet;

use super::{
    AggregationFunction, AggregationGranularity, BandTop, BillingGranularity, IncludedAllowance,
    ModelKind, PriceRow, RolloverPolicy, TierAggregationWindow, TierBand, TierQualificationWindow,
    model_kind_wire, unit_determining_mismatch,
};
use crate::domain::money::{MinorAmount, RateMinor};
use crate::domain::scope_key::ChargeKind;

fn minor(units: i64) -> MinorAmount {
    MinorAmount::new(units).expect("test amount is non-negative")
}

/// A band rate, stated in whole minor units so these cases read as they
/// always did (D-311). The stored scale is 10^-9 of one.
fn rate(minor_units: i64) -> RateMinor {
    RateMinor::from_minor_units(minor_units).expect("test rate is non-negative")
}

/// A graduated metered row, denominated `per_hour` over the calendar month and
/// authoring none of the three fields that carry a default.
fn metered() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("egress_bytes".to_owned());
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = vec![
        TierBand::closed(0, 1_000, rate(10)),
        TierBand::open(1_000, rate(6)),
    ];
    row
}

/// The other authorable metered shape: an untiered rate, no bands, no ladder —
/// until an `includedAllowance` compiles one.
fn untiered_metered() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    row.meter = Some("egress_bytes".to_owned());
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.unit_rate = Some(rate(6));
    row
}

fn none_allowance(quantity: u64) -> IncludedAllowance {
    IncludedAllowance {
        quantity,
        rollover_policy: RolloverPolicy::None,
    }
}

/// `model_kind` is in the shared list because the formula that prices the
/// continued counter must not move. Since D-45 the formula is the **presented**
/// kind: introducing `{N, none}` on a published `per_unit` row turns it into the
/// ladder `[0, N) @ $0, [N, null) @ rate` with the authored `model_kind`
/// untouched, so the counter that was rated linearly is rated against a band set
/// mid-window — and the band it lands in depends on quantity accumulated while
/// no allowance existed.
#[test]
fn introducing_a_none_allowance_on_an_untiered_row_moves_the_presented_kind() {
    let mut after = untiered_metered();
    after.included_allowance = Some(none_allowance(100));

    assert_eq!(
        after.model_kind,
        untiered_metered().model_kind,
        "the authored kind does not move; that is what made this invisible"
    );
    assert_eq!(
        unit_determining_mismatch(&untiered_metered(), &after),
        vec!["includedAllowance (presented modelKind)"]
    );
    // Symmetric, like the rest of the list: dropping it moves the same way.
    assert_eq!(
        unit_determining_mismatch(&after, &untiered_metered()),
        vec!["includedAllowance (presented modelKind)"]
    );
}

/// The positive control, and the clause the note in
/// [`super::unit_determining_mismatch`] keeps: a `none` allowance is a price
/// lever wherever the presented kind stays put. Both of these rows publish as a
/// ladder before and after, so changing `N` changes only what the ladder costs.
#[test]
fn a_none_allowance_quantity_change_leaves_the_presented_kind_alone() {
    let mut before = untiered_metered();
    before.included_allowance = Some(none_allowance(100));
    let mut after = untiered_metered();
    after.included_allowance = Some(none_allowance(250));

    assert!(unit_determining_mismatch(&before, &after).is_empty());

    // And on a row authored as a ladder there is no presented move at all: a
    // `graduated` row presents `graduated` with or without a declaration.
    let mut carrying = metered();
    carrying.included_allowance = Some(none_allowance(100));
    assert!(unit_determining_mismatch(&metered(), &carrying).is_empty());
}

#[test]
fn a_row_that_authors_no_derivation_reads_as_a_sum_row() {
    // "Authored nothing" and "authored `sum`" have to be the same row: if they
    // validated differently, an author could turn a level rule on or off by
    // spelling the default out.
    let unauthored = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    let mut explicit = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    explicit.aggregation_function = Some(AggregationFunction::Sum);

    assert_eq!(
        unauthored.effective_aggregation_function(),
        explicit.effective_aggregation_function()
    );
    assert!(!unauthored.is_level());
    assert!(!explicit.is_level());
}

#[test]
fn a_row_authoring_a_fold_is_a_level_row_and_defaults_to_hour_granules() {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    row.aggregation_function = Some(AggregationFunction::TimeWeighted);

    assert!(row.is_level());
    assert_eq!(
        row.effective_aggregation_granularity(),
        AggregationGranularity::Hour
    );
}

#[test]
fn each_granule_has_exactly_one_billing_counterpart() {
    // D-77: the pairing is what makes the band unit derivable one way only.
    assert_eq!(
        AggregationGranularity::Hour.billing_counterpart(),
        BillingGranularity::PerHour
    );
    assert_eq!(
        AggregationGranularity::Day.billing_counterpart(),
        BillingGranularity::PerDay
    );
}

#[test]
fn a_band_covers_nothing_when_its_top_is_at_or_below_its_floor() {
    assert!(TierBand::closed(100, 100, rate(1)).is_zero_width());
    assert!(TierBand::closed(100, 50, rate(1)).is_zero_width());
    assert!(!TierBand::closed(100, 101, rate(1)).is_zero_width());
}

#[test]
fn an_open_topped_band_is_never_zero_width() {
    // There is no upper bound to compare against, so the degenerate case cannot
    // arise however high the floor is.
    assert!(!TierBand::open(u64::MAX, rate(1)).is_zero_width());
    assert!(TierBand::open(0, rate(1)).to_qty.is_open());
    assert_eq!(BandTop::Closed(42).closed_at(), Some(42));
    assert_eq!(BandTop::Open.closed_at(), None);
}

#[test]
fn a_row_with_no_kind_still_names_itself_in_a_finding() {
    // `MODEL_KIND_MISSING` is reported *about* a row, so the subject has to
    // render before the kind exists.
    let row = PriceRow::new(ChargeKind::Usage, None);

    assert_eq!(row.subject(), "usage/(no model kind)");
}

#[test]
fn every_model_kind_has_its_own_wire_spelling() {
    // The spelling is matched against the fixture registry file, so two kinds
    // sharing one token would silently open a gate for the wrong shape.
    let spellings: HashSet<&str> = ModelKind::ALL.into_iter().map(model_kind_wire).collect();

    assert_eq!(spellings.len(), ModelKind::ALL.len());
    assert!(spellings.contains("per_unit"));
}

#[test]
fn only_a_usage_charge_kind_is_a_usage_row() {
    for kind in [
        ChargeKind::Recurring,
        ChargeKind::OneTime,
        ChargeKind::OneTimeSetup,
    ] {
        assert!(!PriceRow::new(kind, Some(ModelKind::Flat)).is_usage());
    }
    assert!(PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit)).is_usage());
}

#[test]
fn only_the_band_kinds_are_tiered() {
    // `is_tiered` decides which rows owe an origin band and a reset window, so
    // it must not quietly include `package`, whose money is not in bands.
    assert!(PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated)).is_tiered());
    assert!(PriceRow::new(ChargeKind::Usage, Some(ModelKind::Volume)).is_tiered());
    assert!(!PriceRow::new(ChargeKind::Usage, Some(ModelKind::Package)).is_tiered());
    assert!(!PriceRow::new(ChargeKind::Usage, None).is_tiered());
}

// ---------------------------------------------------------------------------
// The shared unit comparison
// ---------------------------------------------------------------------------

#[test]
fn the_unit_comparison_is_exactly_seven_fields_in_the_design_sets_order() {
    // The list two guards share (`inst-tb-supersession-units` and
    // `inst-ph-override-units`). This is the definition side of that sharing:
    // add a field and this test is short, remove one and it is long, and the
    // rules over both mechanisms move together either way.
    let mut after = metered();
    after.model_kind = Some(ModelKind::Package);
    after.bands = Vec::new();
    after.package_size = Some(100);
    after.package_price_minor = Some(minor(500));
    after.billing_granularity = Some(BillingGranularity::PerDay);
    after.aggregation_function = Some(AggregationFunction::Peak);
    after.aggregation_granularity = Some(AggregationGranularity::Day);
    after.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
    after.tier_qualification_window = Some(TierQualificationWindow::TrailingPeriod);

    assert_eq!(
        unit_determining_mismatch(&metered(), &after),
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
    // Symmetric: it answers which fields moved, not which way, so a caller may
    // pass its pair in whatever order reads chronologically at its call site.
    assert_eq!(
        unit_determining_mismatch(&after, &metered()),
        unit_determining_mismatch(&metered(), &after)
    );
}

#[test]
fn the_unit_comparison_reads_the_three_defaulted_fields_through_their_defaults() {
    // "Authored nothing" and "authored the default" are the same row. Comparing
    // the raw Options here would reject a supersession, and refuse a phase
    // override, that changed nothing at all.
    let mut spelled_out = metered();
    spelled_out.aggregation_function = Some(AggregationFunction::Sum);
    spelled_out.aggregation_granularity = Some(AggregationGranularity::Hour);
    spelled_out.tier_qualification_window = Some(TierQualificationWindow::Current);

    assert!(unit_determining_mismatch(&metered(), &spelled_out).is_empty());
}

#[test]
fn the_unit_comparison_says_nothing_about_the_price_levers() {
    // The money and the band set are what repricing and free-trial authoring
    // move; `meter`, `dimensionKey` and the allowance are the supersession
    // guard's own three, deliberately not in the shared seven.
    let mut repriced = metered();
    repriced.bands = vec![TierBand::open(0, rate(0))];
    repriced.amount_minor = Some(minor(1));
    repriced.package_price_minor = Some(minor(900));
    repriced.meter = Some("ingress_bytes".to_owned());
    repriced.dimension_key = "region".to_owned();
    repriced.included_allowance = Some(IncludedAllowance {
        quantity: 100,
        rollover_policy: RolloverPolicy::Carry,
    });

    assert!(unit_determining_mismatch(&metered(), &repriced).is_empty());
}
