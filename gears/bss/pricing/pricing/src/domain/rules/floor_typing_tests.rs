//! `inst-ft-fallback` and `inst-ft-warn`, over a bare row.

use super::{
    FLOOR_FALLBACK_MISSING, FLOOR_FALLBACK_WITHOUT_FLOOR, FLOOR_INSIDE_PRICED_BAND,
    FloorFallbackDeclared, FloorOutsideBands,
};
use crate::domain::money::RateMinor;
use crate::domain::price_row::{
    BandTop, BillingGranularity, MinQtyUsageFallback, ModelKind, PriceRow, TierAggregationWindow,
    TierBand,
};
use crate::domain::scope_key::ChargeKind;
use crate::domain::validation::{ValidationPipeline, ValidationReport};

/// A band rate in whole minor units, scaled to the stored rate scale
/// (D-311) so these cases price what they always priced.
fn rate(minor_units: i64) -> RateMinor {
    RateMinor::from_minor_units(minor_units).expect("a non-negative rate")
}

fn run(row: &PriceRow) -> ValidationReport {
    ValidationPipeline::new()
        .with_rule(Box::new(FloorFallbackDeclared))
        .with_rule(Box::new(FloorOutsideBands))
        .run(row)
}

fn violations(report: &ValidationReport) -> Vec<String> {
    report.violations.iter().map(|v| v.code.clone()).collect()
}

fn warnings(report: &ValidationReport) -> Vec<String> {
    report.warnings.iter().map(|w| w.code.clone()).collect()
}

/// A tiered usage row banded `[0, 1000)` then `[1000, open)`.
fn banded_row() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("storage.gb".to_owned());
    row.billing_granularity = Some(BillingGranularity::PerHour);
    row.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
    row.bands = vec![
        TierBand {
            from_qty: 0,
            to_qty: BandTop::Closed(1_000),
            unit_price_rate: rate(10),
        },
        TierBand {
            from_qty: 1_000,
            to_qty: BandTop::Open,
            unit_price_rate: rate(6),
        },
    ];
    row
}

/// The world in which every case below is observable.
#[test]
fn a_row_with_no_floor_at_all_is_clean() {
    let report = run(&banded_row());
    assert!(report.violations.is_empty(), "{:?}", violations(&report));
    assert!(report.warnings.is_empty(), "{:?}", warnings(&report));
}

/// `inst-ft-fallback`: the floor without the behaviour beneath it.
#[test]
fn a_usage_floor_without_a_fallback_is_refused() {
    let mut row = banded_row();
    row.min_qty_usage = Some(1_000);
    assert!(
        violations(&run(&row)).contains(&FLOOR_FALLBACK_MISSING.to_owned()),
        "{:?}",
        violations(&run(&row))
    );
}

/// And the complete pair is clean -- so the case above is evidence about the
/// missing fallback rather than about setting a usage floor at all.
#[test]
fn a_usage_floor_with_its_fallback_is_clean() {
    let mut row = banded_row();
    row.min_qty_usage = Some(1_000);
    row.min_qty_usage_fallback = Some(MinQtyUsageFallback::Exception);
    assert!(
        run(&row).violations.is_empty(),
        "{:?}",
        violations(&run(&row))
    );
}

/// A purchase floor needs no fallback: Subscriptions rejects the order outright,
/// so there is no below-floor line to decide the fate of.
#[test]
fn a_purchase_floor_needs_no_fallback() {
    let mut row = banded_row();
    row.min_qty_purchase = Some(1_000);
    assert!(
        run(&row).violations.is_empty(),
        "{:?}",
        violations(&run(&row))
    );
}

/// The inert half: a fallback with nothing to fall back from warns, never fails.
#[test]
fn a_fallback_without_a_floor_warns_and_does_not_refuse() {
    let mut row = banded_row();
    row.min_qty_usage_fallback = Some(MinQtyUsageFallback::Exception);
    let report = run(&row);
    assert!(report.violations.is_empty(), "{:?}", violations(&report));
    assert!(
        warnings(&report).contains(&FLOOR_FALLBACK_WITHOUT_FLOOR.to_owned()),
        "{:?}",
        warnings(&report)
    );
}

/// `inst-ft-warn`: a floor strictly inside a band hides quantity that band prices.
#[test]
fn a_floor_inside_a_band_warns_and_publishes() {
    let mut row = banded_row();
    row.min_qty_purchase = Some(500);
    let report = run(&row);
    assert!(
        report.violations.is_empty(),
        "the finding is advisory; the plan publishes: {:?}",
        violations(&report)
    );
    assert!(
        warnings(&report).contains(&FLOOR_INSIDE_PRICED_BAND.to_owned()),
        "{:?}",
        warnings(&report)
    );
}

/// The boundary the rule is deliberately strict about: a floor **at** a band's
/// lower bound hides nothing, and is the authoring an operator who noticed the
/// overlap would produce. Warning there would train operators to ignore it.
#[test]
fn a_floor_exactly_at_a_band_boundary_does_not_warn() {
    for at in [0_u64, 1_000] {
        let mut row = banded_row();
        row.min_qty_purchase = Some(at);
        assert!(
            !warnings(&run(&row)).contains(&FLOOR_INSIDE_PRICED_BAND.to_owned()),
            "a floor at {at} sits on a band bound and hides nothing"
        );
    }
}

/// A floor above every band's start, inside the open top band, still warns --
/// the open top has no ceiling to be outside of.
#[test]
fn a_floor_inside_the_open_top_band_warns() {
    let mut row = banded_row();
    row.min_qty_usage = Some(5_000);
    row.min_qty_usage_fallback = Some(MinQtyUsageFallback::Exception);
    assert!(
        warnings(&run(&row)).contains(&FLOOR_INSIDE_PRICED_BAND.to_owned()),
        "{:?}",
        warnings(&run(&row))
    );
}
