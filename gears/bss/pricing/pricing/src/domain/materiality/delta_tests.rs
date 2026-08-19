//! One case per clause of D-115's delta domain.
//!
//! Every case is a **pair** of rows that differ in exactly one thing, because the
//! rule under test is a comparison and a pair differing in two says nothing about
//! which one answered. The fixture is a row that already equals its own baseline,
//! so a case that changed nothing would come back a zero delta rather than a
//! refusal — which is what makes each `NotComputable` assertion evidence.

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use super::{AmountMove, MoveScale, RowDelta, row_delta};
use crate::domain::concurrency::RowVersion;
use crate::domain::contracts::{AnchorDay, BillingAnchorPolicy, ProrationBasis, ProrationContract};
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount, RateMinor};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{
    IncludedAllowance, MinQtyUsageFallback, ModelKind, PriceRow, QuantitySource, ReservationFlavor,
    RolloverPolicy, TierBand,
};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};

/// A band rate, stated in whole minor units so these cases read as they always
/// did (D-311). The stored scale is 10^-9 of one.
fn rate(minor_units: i64) -> RateMinor {
    RateMinor::from_minor_units(minor_units).expect("a non-negative rate")
}

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
        tax_category_ref: None,
        billing_timing: Some("advance".to_owned()),
        proration_contract: None,
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
            Some(top) => TierBand::closed(from, top, rate(price)),
            None => TierBand::open(from, rate(price)),
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
            scale: MoveScale::Minor,
        })
    );
    // And a `per_unit` row takes **its own** operand: since D-311 its price is a
    // rate in its own column and its own scale, so the half of the clause a
    // `flat`-only case would leave unproved is now also the half that proves the
    // two kinds no longer share `amount_minor`.
    let mut current = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    current.unit_rate = Some(rate(12));
    let mut baseline = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    baseline.unit_rate = Some(rate(10));
    assert_eq!(
        row_delta(&record(current), &record(baseline)),
        RowDelta::Amount(AmountMove {
            from_minor: 10_000_000_000,
            to_minor: 12_000_000_000,
            scale: MoveScale::NanoMinor,
        })
    );
}

/// **A `per_unit` row still carrying only `amount_minor` has no rate to compare**,
/// and says which field is missing rather than reporting a zero move.
///
/// The pre-D-311 spelling is exactly what a row written by an older writer looks
/// like, and the dangerous answer would be `Amount(0 → 0)`: a move of nothing,
/// below every bar, auto-published.
#[test]
fn a_per_unit_row_without_its_rate_is_not_computable() {
    let mut current = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    current.amount_minor = Some(minor(12));
    let mut baseline = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    baseline.amount_minor = Some(minor(10));

    assert_eq!(
        row_delta(&record(current), &record(baseline)),
        RowDelta::NotComputable("unit_rate")
    );
}

/// `graduated` / `volume` → the band-wise rate vector, on unchanged geometry.
///
/// The bands read back in their own scale (D-311) — the helper states each rate
/// in whole minor units, so `5` here is the same band price it was before rates
/// were storable below a cent.
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
                from_minor: 4_000_000_000,
                to_minor: 5_000_000_000,
                scale: MoveScale::NanoMinor,
            },
            AmountMove {
                from_minor: 3_000_000_000,
                to_minor: 3_000_000_000,
                scale: MoveScale::NanoMinor,
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
            scale: MoveScale::Minor,
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

/// `tax_category_ref` moves what a subscriber is billed and moves no amount.
///
/// **Its own case rather than a fourth line in the one above**, because the arm
/// it asserts was *missing* while that case stood green: those three were the
/// whole registered set this crate carried until Slice 4 added the column, and a
/// case that merely grew a line would have recorded the arm's arrival without
/// recording that it had been absent (`T-14`). D-48 makes `taxCategory` one of
/// the five descriptor elements Billing countersigns, so a supersession moving
/// only this field is exactly the change a second principal should see.
#[test]
fn a_tax_category_change_has_no_computable_delta() {
    let mut current = flat(1000);
    current.tax_category_ref = Some("reduced".to_owned());

    assert_eq!(
        row_delta(&current, &flat(1000)),
        RowDelta::NotComputable("tax_category_ref")
    );

    // The same move from the other side, and it is not the same fact restated:
    // `None` is *the row states none*, which D-154 resolves against the region's
    // default at publish. Dropping an authored category hands the row to a
    // default that a taxonomy edit can change afterwards — so the direction that
    // looks like "clearing a field" is the one with the wider blast radius.
    let mut cleared = flat(1000);
    cleared.tax_category_ref = None;
    let mut baseline = flat(1000);
    baseline.tax_category_ref = Some("standard".to_owned());
    assert_eq!(
        row_delta(&cleared, &baseline),
        RowDelta::NotComputable("tax_category_ref")
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
            scale: MoveScale::Minor,
        })
    );
    assert_eq!(
        row_delta(&package(100, 1000), &package(100, 1000)),
        RowDelta::PackagePrice(AmountMove {
            from_minor: 1000,
            to_minor: 1000,
            scale: MoveScale::Minor,
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
        scale: MoveScale::Minor,
    };
    let down = AmountMove {
        from_minor: 1000,
        to_minor: 500,
        scale: MoveScale::Minor,
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
        scale: MoveScale::Minor,
    };

    assert!(move_.reaches_absolute(500), "500 reaches 500");
    assert!(!move_.reaches_absolute(501), "500 is below 501");
    assert!(
        AmountMove {
            from_minor: 1000,
            to_minor: 1000,
            scale: MoveScale::Minor,
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
        scale: MoveScale::Minor,
    };
    assert_eq!(
        from_zero.reaches_percent(500),
        None,
        "there is no percentage of nothing"
    );

    let ten_percent = AmountMove {
        from_minor: 1000,
        to_minor: 1100,
        scale: MoveScale::Minor,
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

/// A zero baseline that **did not move** is below every percent bar, and a zero
/// baseline that moved is still incomputable.
///
/// The two zeroes are different facts and the case above only pins one of them.
/// `0 → 500` is an infinite rise and has no percentage; `0 → 0` is a delta of
/// zero, which is below every bar there is — and `Some(false)` is the only answer
/// that says so. Answering `None` there hands
/// [`super::super::Comparison::NotComparable`] to the comparer, which the
/// evaluator renders as `noConfiguredThreshold` about a threshold the tenant did
/// configure.
///
/// This is not a corner: since D-45 the allowance compile prepends a `[0, N) @ $0`
/// band to every allowance-bearing row (`domain::allowance`), that band never
/// moves, and `band_delta` emits it as element zero of the vector — so under a
/// percent policy the unmoved zero decided every such row.
#[test]
fn an_unmoved_zero_baseline_is_below_every_percent_bar() {
    let unmoved = AmountMove {
        from_minor: 0,
        to_minor: 0,
        scale: MoveScale::NanoMinor,
    };
    assert_eq!(
        unmoved.reaches_percent(1),
        Some(false),
        "a delta of zero is below a bar of one basis point, baseline or no baseline"
    );
    assert_eq!(
        unmoved.reaches_percent(10_000),
        Some(false),
        "and below the widest bar the store will hold"
    );
    // The zero *bar* has no site on this basis, which is why the answer above can
    // be unconditional: `m20260802_000018`'s CHECK is `percent_bp IS NULL OR
    // percent_bp > 0`, where the absolute basis' is `absolute_minor >= 0`. The
    // "a configured zero makes everything material" reading that
    // `a_move_that_reaches_the_threshold_is_not_below_it` pins is therefore
    // `reaches_absolute`'s alone, and the two bases do not disagree about an
    // input either of them can be given.

    let moved_off_zero = AmountMove {
        from_minor: 0,
        to_minor: 1,
        scale: MoveScale::NanoMinor,
    };
    assert_eq!(
        moved_off_zero.reaches_percent(10_000),
        None,
        "a baseline of nothing that moved still has no percentage; only the \
         unmoved case gains an answer"
    );
}

/// The percent comparison holds its operands where `reaches_absolute` holds its
/// own — and a rate a tenant can author is enough to make that matter.
///
/// Saturating in `i64`, `scaled = |delta| * 10_000` clamps once the magnitude
/// passes `i64::MAX / 10_000` — a rate move above `$9,223.37` per unit — and
/// `bar = |baseline| * percent_bp` clamps on the same order. When **both** clamp
/// they land on `i64::MAX` together, and `S >= S` answers `reached` about a
/// comparison that has lost both its operands.
///
/// The band below is a `$20,000`-per-unit rate rising 50% against a 100% bar: the
/// answer is `below` and the saturating form said `reached`. Fail-safe in
/// direction — an over-flagged change costs an approval, not an amount — which is
/// why this is the widening's proof rather than a wrong-money case.
#[test]
fn a_percent_bar_over_a_large_rate_is_not_decided_by_saturation() {
    // 2e15 nano-minor is 2,000,000 minor units — $20,000 per unit — and 3e15 is a
    // 50% rise off it. Both `scaled` (1e19) and `bar` (2e19) are past `i64::MAX`
    // (9.223e18), so `i64` arithmetic clamps both to the same value.
    let half_again = AmountMove {
        from_minor: 2_000_000_000_000_000,
        to_minor: 3_000_000_000_000_000,
        scale: MoveScale::NanoMinor,
    };
    assert_eq!(
        half_again.reaches_percent(10_000),
        Some(false),
        "a 50% rise is below a 100% bar, and both sides of that comparison \
         overflow an i64"
    );

    // And the bar still trips over the same overflow, so the case above is not a
    // widening that answers `below` to everything large.
    let tripled = AmountMove {
        from_minor: 2_000_000_000_000_000,
        to_minor: 6_000_000_000_000_000,
        scale: MoveScale::NanoMinor,
    };
    assert_eq!(
        tripled.reaches_percent(10_000),
        Some(true),
        "a 200% rise reaches a 100% bar"
    );
}

/// D-115's three remaining row-contract entries, each on its own — one case
/// cannot tell an implemented arm from a missing one, which is why the
/// `billingTiming` test above already splits its three.
///
/// The failure each closes is the same shape: a supersession that moves only the
/// proration contract moves **no amount**, so without an arm here it is
/// classified immaterial and publishes with no second principal ever seeing it.
/// `credit_on_downgrade` is the sharpest — it decides whether a downgrading
/// subscriber is refunded at all.
#[test]
fn each_proration_contract_field_has_no_computable_delta_on_its_own() {
    let baseline = {
        let mut row = flat(1000);
        row.proration_contract = Some(ProrationContract {
            billing_anchor_policy: BillingAnchorPolicy::CalendarMonth,
            proration_basis: ProrationBasis::CalendarDaysActual,
            credit_on_downgrade: false,
        });
        row
    };

    for (moved, field) in [
        (
            ProrationContract {
                billing_anchor_policy: BillingAnchorPolicy::SubscriptionStart,
                proration_basis: ProrationBasis::CalendarDaysActual,
                credit_on_downgrade: false,
            },
            "billing_anchor_policy",
        ),
        (
            ProrationContract {
                billing_anchor_policy: BillingAnchorPolicy::CalendarMonth,
                proration_basis: ProrationBasis::BySecond,
                credit_on_downgrade: false,
            },
            "prorationBasis",
        ),
        (
            ProrationContract {
                billing_anchor_policy: BillingAnchorPolicy::CalendarMonth,
                proration_basis: ProrationBasis::CalendarDaysActual,
                credit_on_downgrade: true,
            },
            "credit_on_downgrade",
        ),
    ] {
        let mut current = baseline.clone();
        current.proration_contract = Some(moved);
        assert_eq!(
            row_delta(&current, &baseline),
            RowDelta::NotComputable(field)
        );
    }
}

/// A `fixed_day` anchor that moves only its **day** is a different anchor. The
/// policy token is equal on both sides, so an arm comparing tokens rather than
/// the whole policy would call this immaterial and publish a moved cycle clock.
#[test]
fn a_fixed_day_anchor_moving_only_its_day_is_still_a_contract_change() {
    let anchored = |day: u8| {
        let mut row = flat(1000);
        row.proration_contract = Some(ProrationContract {
            billing_anchor_policy: BillingAnchorPolicy::FixedDay(
                AnchorDay::new(day).expect("a day of the month"),
            ),
            proration_basis: ProrationBasis::CalendarDaysActual,
            credit_on_downgrade: false,
        });
        row
    };

    assert_eq!(
        row_delta(&anchored(15), &anchored(1)),
        RowDelta::NotComputable("billing_anchor_policy")
    );
}

/// Gaining or losing the whole set is a move in either direction: the set is
/// required on a recurring row, so a row that stopped carrying it is a row whose
/// consumer contract changed.
#[test]
fn gaining_or_losing_the_contract_is_a_change_in_either_direction() {
    let with = {
        let mut row = flat(1000);
        row.proration_contract = Some(ProrationContract {
            billing_anchor_policy: BillingAnchorPolicy::CalendarMonth,
            proration_basis: ProrationBasis::CalendarDaysActual,
            credit_on_downgrade: false,
        });
        row
    };
    let without = flat(1000);

    assert_eq!(
        row_delta(&with, &without),
        RowDelta::NotComputable("billing_anchor_policy")
    );
    assert_eq!(
        row_delta(&without, &with),
        RowDelta::NotComputable("billing_anchor_policy")
    );
}

// ---------------------------------------------------------------------------
// Slice 10's primitives, which reached this domain a wave late (D-254).
// ---------------------------------------------------------------------------

/// **A reserved rate that moves is not an immaterial change**, and until D-254 it
/// was classified as one.
///
/// `reserved_rate` is money — D-139 denominates it per covered granule — but
/// it is not `amount_minor`, so `amount_delta` compared two unchanged amounts and
/// answered a **zero move**. A zero move is immaterial, and an immaterial publish
/// needs one principal. A four-fold rise in the reserved rate therefore reached
/// consumers with no second approver, on a row whose on-demand price never moved.
///
/// It is `NotComputable` rather than an `Amount` move for D-115 clause (2)'s
/// stated reason: no effective-price delta is computable catalog-side — that would
/// need the covered-granule count, which is Rating's runtime fact — so the G1
/// fail-safe applies and the operator gets a second signature instead of a number.
#[test]
fn a_reserved_rate_move_is_not_computable_rather_than_a_zero_delta() {
    let mut current = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    current.amount_minor = Some(minor(10));
    current.reserved_rate = Some(RateMinor::from_minor_units(4000).expect("a non-negative rate"));
    let mut baseline = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    baseline.amount_minor = Some(minor(10));
    baseline.reserved_rate = Some(RateMinor::from_minor_units(1000).expect("a non-negative rate"));

    assert_eq!(
        row_delta(&record(current), &record(baseline)),
        RowDelta::NotComputable("reserved_rate"),
        "the on-demand amount is identical, so nothing but this field can have answered"
    );
}

/// The three fields that decide **what quantity is billable** all fail closed.
///
/// Each is in `evaluation-policy-generation`'s roster for exactly this reason —
/// `reservation_flavor` decides whether the reserved quantity enters the on-demand
/// counter `Q` at all (`inst-rv-tier-q` / `inst-rv-level`), and the usage floor and
/// its fallback decide what a below-floor quantity bills as. A field the roster
/// calls quantity-determining and this domain calls immaterial is the same
/// contradiction `includedAllowance.quantity` was added here to close.
#[test]
fn the_quantity_determining_primitives_fail_closed() {
    let base = || {
        let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
        // **The rate, not an amount.** This fixture carried
        // `amount_minor = Some(10)` on a `per_unit` row until 2026-08-18 — the
        // shape D-311's placement matrix forbids and publish refuses. It went
        // unnoticed because the three assertions below name fields `row_delta`
        // reaches *before* the money check; adding a fourth surfaced it, as
        // `NotComputable("unit_rate")` where the new field's name belonged.
        //
        // Fixed here rather than asserted around: a probe whose base row cannot be
        // published proves nothing about a domain that only ever sees published
        // rows.
        row.unit_rate = Some(RateMinor::from_minor_units(10).expect("a non-negative rate"));
        row.reserved_rate = Some(RateMinor::from_minor_units(1000).expect("a non-negative rate"));
        row.reservation_flavor = Some(ReservationFlavor::Capacity);
        row.min_qty_usage = Some(5);
        row.min_qty_usage_fallback = Some(MinQtyUsageFallback::Exception);
        row
    };

    let mut flavor = base();
    flavor.reservation_flavor = Some(ReservationFlavor::Consumption);
    assert_eq!(
        row_delta(&record(flavor), &record(base())),
        RowDelta::NotComputable("reservation_flavor")
    );

    let mut floor = base();
    floor.min_qty_usage = Some(50);
    assert_eq!(
        row_delta(&record(floor), &record(base())),
        RowDelta::NotComputable("min_qty_usage")
    );

    let mut fallback = base();
    fallback.min_qty_usage_fallback = None;
    assert_eq!(
        row_delta(&record(fallback), &record(base())),
        RowDelta::NotComputable("min_qty_usage_fallback")
    );

    // The fourth, and it was missing from this list as well as from the domain
    // (review M-3). `LevelMaxHold` says what the field decides: past the bound a
    // held level **reads 0**, so moving it changes what every sampling gap bills.
    // A successor identical but for `1` -> `8760` produced an all-zero band delta
    // and committed on one principal, while a gauge that stopped reporting billed
    // its last level for a year instead of an hour.
    //
    // `NotComputable("max_hold_granules")` and not merely "material": a stub
    // answering `NotComputable("")` would satisfy the weaker assertion, and the
    // name is what an operator reads to learn which field forced the review.
    let mut hold = base();
    hold.max_hold_granules = Some(8760);
    assert_eq!(
        row_delta(&record(hold), &record(base())),
        RowDelta::NotComputable("max_hold_granules")
    );
}
