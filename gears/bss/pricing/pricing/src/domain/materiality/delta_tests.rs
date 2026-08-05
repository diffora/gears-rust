//! One case per clause of D-115's delta domain.
//!
//! Every case is a **pair** of rows that differ in exactly one thing, because the
//! rule under test is a comparison and a pair differing in two says nothing about
//! which one answered. The fixture is a row that already equals its own baseline,
//! so a case that changed nothing would come back a zero delta rather than a
//! refusal — which is what makes each `NotComputable` assertion evidence.

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use super::{AmountMove, RowDelta, row_delta};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{
    IncludedAllowance, ModelKind, PriceRow, QuantitySource, RolloverPolicy, TierBand,
};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};

fn minor(units: i64) -> MinorAmount {
    MinorAmount::new(units).expect("a non-negative amount")
}

fn record(row: PriceRow) -> PriceRecord {
    PriceRecord {
        price_id: Uuid::from_u128(0xd_10),
        scope_key: ScopeKey::new(
            PlanId::new(Uuid::from_u128(1)),
            CurrencyCode::new("USD").expect("USD"),
            Region::new("EU").expect("a non-blank region"),
            PhaseId::new(Uuid::from_u128(2)),
            PriceEligibility::AllSubscriptions,
            ChargeKind::Recurring,
            Cohort::None,
        )
        .expect("the eight axes agree"),
        row,
        tax_inclusive: false,
        billing_timing: Some("advance".to_owned()),
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
        lifecycle_state: LifecycleState::Published,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: Utc
            .with_ymd_and_hms(2026, 8, 2, 10, 0, 0)
            .single()
            .expect("a real instant"),
        row_version: RowVersion::new(1),
    }
}

/// A `flat` row at `amount_minor`.
fn flat(amount: i64) -> PriceRecord {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(minor(amount));
    record(row)
}

/// A `graduated` row over `bands`, each `(from, to_or_open, unit_price)`.
fn graduated(bands: &[(u64, Option<u64>, i64)]) -> PriceRecord {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.bands = bands
        .iter()
        .map(|&(from, to, price)| match to {
            Some(top) => TierBand::closed(from, top, minor(price)),
            None => TierBand::open(from, minor(price)),
        })
        .collect();
    record(row)
}

/// A `package` row at `(size, price)`.
fn package(size: u64, price: i64) -> PriceRecord {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Package));
    row.package_size = Some(size);
    row.package_price_minor = Some(minor(price));
    record(row)
}

// ---------------------------------------------------------------------------
// The three per-kind operands
// ---------------------------------------------------------------------------

/// `flat` / `per_unit` → the `amount_minor` delta, with the baseline it moved
/// from.
#[test]
fn a_flat_rows_delta_is_its_amount() {
    let delta = row_delta(&flat(1500), &flat(1000));

    assert_eq!(
        delta,
        RowDelta::Amount(AmountMove {
            from_minor: 1000,
            to_minor: 1500,
        })
    );
    // And a `per_unit` row takes the same operand, which is the half of the
    // clause a `flat`-only case would leave unproved.
    let mut current = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    current.amount_minor = Some(minor(12));
    let mut baseline = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    baseline.amount_minor = Some(minor(10));
    assert_eq!(
        row_delta(&record(current), &record(baseline)),
        RowDelta::Amount(AmountMove {
            from_minor: 10,
            to_minor: 12,
        })
    );
}

/// `graduated` / `volume` → the band-wise `unit_price_minor` vector, on unchanged
/// geometry.
#[test]
fn a_graduated_rows_delta_is_the_band_vector() {
    let delta = row_delta(
        &graduated(&[(0, Some(1000), 5), (1000, None, 3)]),
        &graduated(&[(0, Some(1000), 4), (1000, None, 3)]),
    );

    assert_eq!(
        delta,
        RowDelta::BandVector(vec![
            AmountMove {
                from_minor: 4,
                to_minor: 5,
            },
            AmountMove {
                from_minor: 3,
                to_minor: 3,
            },
        ]),
        "one entry per band, in band order, including the bands that did not move"
    );
}

/// `package` → the `package_price_minor` delta, on an unchanged `package_size`.
#[test]
fn a_package_rows_delta_is_its_package_price() {
    let delta = row_delta(&package(100, 900), &package(100, 1000));

    assert_eq!(
        delta,
        RowDelta::PackagePrice(AmountMove {
            from_minor: 1000,
            to_minor: 900,
        }),
        "a cut is a move, and its magnitude is what a threshold compares"
    );
}

// ---------------------------------------------------------------------------
// The geometry clause — material regardless of thresholds
// ---------------------------------------------------------------------------

/// `[0,1000)` → `[0,10)` at identical unit prices multiplies the charge and moves
/// no unit price at all.
#[test]
fn a_band_bound_move_is_material_regardless_of_thresholds() {
    let delta = row_delta(
        &graduated(&[(0, Some(10), 5), (10, None, 3)]),
        &graduated(&[(0, Some(1000), 5), (1000, None, 3)]),
    );

    assert_eq!(delta, RowDelta::NotComputable("tier band bounds"));
}

/// A band count change: the vectors have no per-band correspondence at all, so a
/// zip would compare a band to a different band.
#[test]
fn a_band_count_change_is_material_regardless_of_thresholds() {
    let delta = row_delta(
        &graduated(&[(0, Some(1000), 5), (1000, Some(5000), 4), (5000, None, 3)]),
        &graduated(&[(0, Some(1000), 5), (1000, None, 3)]),
    );

    assert_eq!(
        delta,
        RowDelta::NotComputable("tier band count"),
        "the count is asked before the bounds, so the reason names the coarser fact"
    );
}

/// A halved `package_size` at an unchanged price is a doubling.
#[test]
fn a_package_size_change_is_material_regardless_of_thresholds() {
    let delta = row_delta(&package(50, 1000), &package(100, 1000));

    assert_eq!(delta, RowDelta::NotComputable("package_size"));
}

// ---------------------------------------------------------------------------
// The quantity clause — each at ZERO amount delta, which is the whole point
// ---------------------------------------------------------------------------

/// `manual_quantity` 10 → 1000 multiplies the charge by a hundred and moves no
/// amount.
#[test]
fn a_manual_quantity_change_is_material_at_zero_amount_delta() {
    let mut current = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    current.amount_minor = Some(minor(1000));
    current.quantity_source = Some(QuantitySource::Manual);
    current.manual_quantity = Some(1000);
    let mut baseline = current.clone();
    baseline.manual_quantity = Some(10);

    let delta = row_delta(&record(current), &record(baseline));

    assert_eq!(
        delta,
        RowDelta::NotComputable("manual_quantity"),
        "the amounts are identical, so an amount-only evaluator would see a zero delta"
    );
}

/// The included allowance is what a subscriber gets before the meter starts, so
/// halving it raises the bill with no price move anywhere.
#[test]
fn an_included_allowance_quantity_change_is_material_at_zero_amount_delta() {
    let mut current = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    current.amount_minor = Some(minor(10));
    current.included_allowance = Some(IncludedAllowance {
        quantity: 50,
        rollover_policy: RolloverPolicy::None,
    });
    let mut baseline = current.clone();
    baseline.included_allowance = Some(IncludedAllowance {
        quantity: 500,
        rollover_policy: RolloverPolicy::None,
    });

    let delta = row_delta(&record(current), &record(baseline));

    assert_eq!(delta, RowDelta::NotComputable("includedAllowance.quantity"));
}

// ---------------------------------------------------------------------------
// The contract clause — the three fields this crate carries
// ---------------------------------------------------------------------------

/// `billingTiming` is Billing's sole deferral input: `advance` → `arrears` moves
/// a whole cycle of revenue and no amount at all.
#[test]
fn a_contract_field_change_has_no_computable_delta() {
    let mut current = flat(1000);
    current.billing_timing = Some("arrears".to_owned());

    assert_eq!(
        row_delta(&current, &flat(1000)),
        RowDelta::NotComputable("billingTiming")
    );

    // The other two this crate carries, each on its own, because one case cannot
    // tell an implemented arm from a missing one.
    let mut tax = flat(1000);
    tax.tax_inclusive = true;
    assert_eq!(
        row_delta(&tax, &flat(1000)),
        RowDelta::NotComputable("tax_inclusive")
    );

    let mut sourced = flat(1000);
    sourced.row.quantity_source = Some(QuantitySource::Manual);
    assert_eq!(
        row_delta(&sourced, &flat(1000)),
        RowDelta::NotComputable("quantity_source")
    );
}

/// A `flat` row becoming `graduated` has no operand in common with its
/// predecessor: `amount_minor` is NULL by construction on the successor.
#[test]
fn a_model_kind_change_has_no_computable_delta() {
    let delta = row_delta(&graduated(&[(0, None, 5)]), &flat(1000));

    assert_eq!(delta, RowDelta::NotComputable("model_kind"));
}

/// The row that changed nothing is a delta of zero, not an incomputable one.
///
/// The fixture's own control: without it every `NotComputable` case above would
/// also pass against a `row_delta` that never computed anything.
#[test]
fn an_unchanged_row_is_a_zero_delta_and_not_an_incomputable_one() {
    assert_eq!(
        row_delta(&flat(1000), &flat(1000)),
        RowDelta::Amount(AmountMove {
            from_minor: 1000,
            to_minor: 1000,
        })
    );
    assert_eq!(
        row_delta(&package(100, 1000), &package(100, 1000)),
        RowDelta::PackagePrice(AmountMove {
            from_minor: 1000,
            to_minor: 1000,
        })
    );
}

// ---------------------------------------------------------------------------
// The comparison the two bases make
// ---------------------------------------------------------------------------

/// A cut is as material as a rise. An overlay from −10% to −90% is the D-50 hazard
/// in the other direction, and a signed comparison would wave every price cut
/// through.
#[test]
fn the_magnitude_is_unsigned_so_a_cut_is_measured_like_a_rise() {
    let up = AmountMove {
        from_minor: 1000,
        to_minor: 1500,
    };
    let down = AmountMove {
        from_minor: 1000,
        to_minor: 500,
    };

    assert_eq!(up.magnitude_minor(), 500);
    assert_eq!(down.magnitude_minor(), 500);
    // Deliberately **not** at the boundary: `400` keeps this a test of the sign
    // and leaves `>=`-versus-`>` to the case below. At `500` it was both, and
    // flipping the comparison reddened two tests for one change.
    assert!(down.reaches_absolute(400), "a 500-minor cut reaches 400");
}

/// Auto-publish is "below an explicit threshold", so reaching it is not below it —
/// and a configured threshold of `0` therefore makes everything material.
#[test]
fn a_move_that_reaches_the_threshold_is_not_below_it() {
    let move_ = AmountMove {
        from_minor: 1000,
        to_minor: 1500,
    };

    assert!(move_.reaches_absolute(500), "500 reaches 500");
    assert!(!move_.reaches_absolute(501), "500 is below 501");
    assert!(
        AmountMove {
            from_minor: 1000,
            to_minor: 1000,
        }
        .reaches_absolute(0),
        "a threshold of zero is reached by a move of nothing, so everything is material"
    );
}

/// The percent basis, cross-multiplied — and `None` on a zero baseline, which is
/// §3 step 3's *"no percentage is computable"*.
#[test]
fn a_percent_basis_against_a_zero_baseline_computes_nothing() {
    let from_zero = AmountMove {
        from_minor: 0,
        to_minor: 500,
    };
    assert_eq!(
        from_zero.reaches_percent(500),
        None,
        "there is no percentage of nothing"
    );

    let ten_percent = AmountMove {
        from_minor: 1000,
        to_minor: 1100,
    };
    assert_eq!(
        ten_percent.reaches_percent(1000),
        Some(true),
        "1000bp is 10%, and 100 of 1000 reaches it"
    );
    assert_eq!(
        ten_percent.reaches_percent(1001),
        Some(false),
        "and is below 10.01%"
    );
}
