//! Tests for the authored price-row shape.

use std::collections::HashSet;

use super::{
    AggregationFunction, AggregationGranularity, BandTop, BillingGranularity, ModelKind, PriceRow,
    TierBand, model_kind_wire,
};
use crate::domain::money::MinorAmount;
use crate::domain::scope_key::ChargeKind;

fn minor(units: i64) -> MinorAmount {
    MinorAmount::new(units).expect("test amount is non-negative")
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
    assert!(TierBand::closed(100, 100, minor(1)).is_zero_width());
    assert!(TierBand::closed(100, 50, minor(1)).is_zero_width());
    assert!(!TierBand::closed(100, 101, minor(1)).is_zero_width());
}

#[test]
fn an_open_topped_band_is_never_zero_width() {
    // There is no upper bound to compare against, so the degenerate case cannot
    // arise however high the floor is.
    assert!(!TierBand::open(u64::MAX, minor(1)).is_zero_width());
    assert!(TierBand::open(0, minor(1)).to_qty.is_open());
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
