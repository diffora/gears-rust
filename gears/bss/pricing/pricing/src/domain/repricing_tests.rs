//! The selector's two asymmetries — the ones a reader would get backwards —
//! and D-311's own regression: a tier band's rate must not round through a
//! coarser, minor-unit-shaped scale on its way through a reprice.

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use super::{RUN_SELECTOR_EMPTY, RunSelector, adjusts_rate, project_row};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, RateMinor};
use crate::domain::overlay::{Adjustment, AmountSet, Magnitude};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{ModelKind, PriceRow, TierBand};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};

#[test]
fn a_selector_naming_nothing_is_unconstrained_and_one_naming_any_axis_is_not() {
    assert!(RunSelector::default().is_unconstrained());

    let one_axis = RunSelector {
        currency: Some(CurrencyCode::new("EUR").expect("three letters")),
        ..RunSelector::default()
    };
    assert!(
        !one_axis.is_unconstrained(),
        "section 2's `a currency segment` is one axis, and naming it constrains the run"
    );
}

#[test]
fn the_cohort_axis_tells_unconstrained_from_the_classless_value() {
    // The distinction the wire surface cannot express and this type must: the
    // outer `None` is *the run does not constrain the axis*, the inner one is
    // *rows that retain nobody*. A reader who conflated them would build a
    // selector that quietly narrowed every run to the non-grandfathered rows and
    // would never see it, because that is also what the default exclusion does.
    let unconstrained = RunSelector::default();
    let classless = RunSelector {
        cohort: Some(Cohort::None),
        ..RunSelector::default()
    };

    assert!(unconstrained.is_unconstrained());
    assert!(!classless.is_unconstrained());
    assert_ne!(unconstrained, classless);
}

#[test]
fn only_an_explicit_grandfathered_eligibility_admits_that_class() {
    // `inst-mp-grandfathered` clause 1. The absent axis is the one place an
    // unconstrained axis narrows the run instead of widening it, so all three
    // named values are asserted rather than the one: a `matches!` written against
    // the wrong variant would still pass a test that only checked the default.
    assert!(!RunSelector::default().admits_grandfathered());

    for eligibility in [
        PriceEligibility::AllSubscriptions,
        PriceEligibility::NewSubscriptionsOnly,
    ] {
        let selector = RunSelector {
            price_eligibility: Some(eligibility),
            ..RunSelector::default()
        };
        assert!(
            !selector.admits_grandfathered(),
            "{eligibility:?} is not the retained class"
        );
    }

    let explicit = RunSelector {
        price_eligibility: Some(PriceEligibility::ExistingGrandfathered),
        ..RunSelector::default()
    };
    assert!(
        explicit.admits_grandfathered(),
        "naming the class is how an operator asks for it, and clause 2 then owes them a per-row \
         refusal rather than a set that silently shrank"
    );
}

#[test]
fn the_wire_code_is_the_one_the_design_set_declares() {
    // Spelled out rather than compared to itself: this is the one place the token
    // is written, and section 5 declares it verbatim.
    assert_eq!(RUN_SELECTOR_EMPTY, "RUN_SELECTOR_EMPTY");
}

// ---------------------------------------------------------------------------
// D-311: a tier band's rate must survive a reprice at its own scale.
// ---------------------------------------------------------------------------

/// A `graduated` row's fixture, minimal but for the two bands.
fn a_graduated_record(bands: Vec<TierBand>, currency: CurrencyCode) -> PriceRecord {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Graduated));
    row.bands = bands;
    a_record(row, currency)
}

/// A `per_unit` row's fixture, priced the way D-311 makes one: `amount_minor`
/// **NULL** and the money in `unit_rate`, which is exactly what
/// `rules::model_kind::check_amount_placement` requires of a published
/// `per_unit` row and exactly what the pre-D-311 shared `flat`/`per_unit` arm
/// could not reprice.
fn a_per_unit_record(rate: RateMinor, currency: CurrencyCode) -> PriceRecord {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::PerUnit));
    row.unit_rate = Some(rate);
    assert!(
        row.amount_minor.is_none(),
        "the fixture's own precondition, and the defect's whole mechanism: a published per_unit \
         row's amount_minor is NULL, so an arm that reprices amount_minor reprices nothing"
    );
    a_record(row, currency)
}

/// The record wrapper both fixtures above share — everything but the row.
fn a_record(row: PriceRow, currency: CurrencyCode) -> PriceRecord {
    PriceRecord {
        price_id: Uuid::from_u128(0xb_10),
        scope_key: ScopeKey::new(
            PlanId::new(Uuid::from_u128(0x9_1a4)),
            currency,
            Region::new("eu").expect("a non-blank region"),
            PhaseId::new(Uuid::from_u128(0xfa_5e)),
            PriceEligibility::AllSubscriptions,
            ChargeKind::Recurring,
            Cohort::None,
        )
        .expect("all_subscriptions pairs with cohort none"),
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: None,
        proration_contract: None,
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
        lifecycle_state: LifecycleState::Published,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: Utc.with_ymd_and_hms(2026, 8, 2, 10, 0, 0).unwrap(),
        row_version: RowVersion::new(0),
    }
}

#[test]
fn a_percentage_adjustment_keeps_two_sub_minor_unit_bands_distinct() {
    // The exact failure D-311 names: a `MinorAmount`-shaped projection
    // truncates both `0.0150` and `0.0110` to `0.01`, so a two-tariff ladder
    // becomes one after a reprice on a row that still looks well-formed.
    // `project_rate` (via `project_row`) must not repeat that at the rate's
    // own 10⁻⁹-minor-unit scale.
    let currency = CurrencyCode::new("USD").expect("three letters");
    let low = RateMinor::from_decimal(&currency, "0.0110").expect("a sub-minor-unit rate");
    let high = RateMinor::from_decimal(&currency, "0.0150").expect("a sub-minor-unit rate");
    assert_ne!(low, high, "the fixture's own precondition");

    let record = a_graduated_record(
        vec![TierBand::closed(0, 100, low), TierBand::open(100, high)],
        currency,
    );

    // +5%, `percent_bp` — the one magnitude a rate computes at all.
    let adjustment = Adjustment::Markup(Magnitude::PercentBp(500));
    let projected = project_row(record, &adjustment);

    let projected_low = projected.row.bands[0].unit_price_rate;
    let projected_high = projected.row.bands[1].unit_price_rate;
    assert_ne!(
        projected_low, projected_high,
        "a percentage adjustment must not collapse a sub-minor-unit ladder through a coarser \
         rounding scale - the exact collapse D-311 exists to prevent"
    );
    // The move is real, not merely "still different" by accident of one band
    // rounding to its neighbour's value.
    assert_ne!(projected_low, low, "the low band actually moved");
    assert_ne!(projected_high, high, "the high band actually moved");
}

#[test]
fn an_amount_or_fixed_adjustment_leaves_every_band_at_its_published_rate() {
    // `project_rate`'s other half: a currency amount has no well-defined
    // meaning as a rate mutation (see its own doc), so `project_row` leaves
    // the band exactly where it was rather than guessing.
    let currency = CurrencyCode::new("USD").expect("three letters");
    let rate = RateMinor::from_decimal(&currency, "0.0230").expect("a sub-minor-unit rate");
    let record = a_graduated_record(vec![TierBand::open(0, rate)], currency.clone());

    let amounts = AmountSet::new(vec![(currency, 50)]);
    for adjustment in [
        Adjustment::Markup(Magnitude::Amount(amounts.clone())),
        Adjustment::Discount(Magnitude::Amount(amounts.clone())),
        Adjustment::Fixed(amounts),
    ] {
        let projected = project_row(record.clone(), &adjustment);
        assert_eq!(
            projected.row.bands[0].unit_price_rate, rate,
            "an amount/fixed adjustment computes no rate mutation, so the band stays published: \
             {adjustment:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// D-311's other stranded site: `per_unit`'s own rate.
// ---------------------------------------------------------------------------

#[test]
fn a_per_unit_rows_rate_moves_under_a_percentage_adjustment() {
    // D-311 moved a `per_unit` row's money out of `amount_minor` into
    // `unit_rate` and `check_amount_placement` now **requires** `amount_minor`
    // to be NULL on one. `project_row` kept repricing `per_unit` through the
    // shared `flat` arm, whose `and_then` over that guaranteed-`None` field
    // short-circuits — so every `per_unit` reprice returned the row unchanged,
    // the apply published a byte-identical successor and materiality saw a
    // zero delta. This is that arm's own operand.
    let currency = CurrencyCode::new("USD").expect("three letters");
    // $0.0230777165 per unit — sub-minor-unit, and deliberately **not** a round
    // number of nano-minor units: 2_307_770_150 x 500 bp leaves a remainder of
    // exactly 5_000 over an **odd** quotient, so half-to-even rounds the move
    // up to 115_388_508 where truncation would have stopped at 115_388_507. The
    // pinned value below therefore separates four implementations at once —
    // the no-op this defect is, a truncating one, one routed through
    // `project_amount`'s minor-unit scale, and the correct one.
    let published = RateMinor::from_nano_minor(2_307_770_150).expect("a non-negative rate");
    let record = a_per_unit_record(published, currency);

    let projected = project_row(record, &Adjustment::Markup(Magnitude::PercentBp(500)));

    assert_eq!(
        projected.row.unit_rate,
        Some(RateMinor::from_nano_minor(2_423_158_658).expect("a non-negative rate")),
        "+5% of 2_307_770_150 nano-minor is 115_388_507.5, half-to-even 115_388_508; a rate that \
         came back at its published 2_307_770_150 is the shared-arm no-op, and 2_423_158_657 is \
         truncation"
    );
    assert!(
        projected.row.amount_minor.is_none(),
        "the projection must not resurrect the column D-311 emptied: two priced columns are two \
         competing prices (AMOUNT_PLACEMENT_INVALID)"
    );
}

#[test]
fn an_amount_or_fixed_adjustment_leaves_a_per_unit_rate_published() {
    // `project_rate`'s refusal half, on `per_unit`'s field: a currency amount
    // has no well-defined meaning as a rate mutation, so the row comes back at
    // its published rate rather than at a guess — and
    // `infra::repricing::apply_rows_in` is where that silence becomes a
    // refusal, via `adjusts_rate`, rather than a successor identical to its
    // predecessor marked `applied`.
    let currency = CurrencyCode::new("USD").expect("three letters");
    let published = RateMinor::from_nano_minor(2_307_770_150).expect("a non-negative rate");
    let record = a_per_unit_record(published, currency.clone());

    let amounts = AmountSet::new(vec![(currency, 50)]);
    for adjustment in [
        Adjustment::Markup(Magnitude::Amount(amounts.clone())),
        Adjustment::Discount(Magnitude::Amount(amounts.clone())),
        Adjustment::Fixed(amounts),
    ] {
        let projected = project_row(record.clone(), &adjustment);
        assert_eq!(
            projected.row.unit_rate,
            Some(published),
            "an amount/fixed adjustment computes no rate mutation, so the rate stays published: \
             {adjustment:?}"
        );
    }
}

#[test]
fn only_a_percentage_adjustment_is_a_rate_mutation() {
    let currency = CurrencyCode::new("USD").expect("three letters");
    let amounts = AmountSet::new(vec![(currency, 50)]);
    assert!(adjusts_rate(&Adjustment::Markup(Magnitude::PercentBp(500))));
    assert!(adjusts_rate(&Adjustment::Discount(Magnitude::PercentBp(
        500
    ))));
    assert!(!adjusts_rate(&Adjustment::Markup(Magnitude::Amount(
        amounts.clone()
    ))));
    assert!(!adjusts_rate(&Adjustment::Discount(Magnitude::Amount(
        amounts.clone()
    ))));
    assert!(!adjusts_rate(&Adjustment::Fixed(amounts)));
}
