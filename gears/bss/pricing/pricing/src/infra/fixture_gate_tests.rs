//! Tests for [`crate::infra::fixture_gate`].
//!
//! The interesting cases are all the *closed* ones: this gate has exactly one
//! job, which is to refuse when it has no positive evidence — and, since the
//! registry gained its variant axis, to refuse with the **name** of the evidence
//! it is missing.

use std::path::{Path, PathBuf};

use super::{FixtureGate, Reservation, required_variants};
use crate::domain::error::DomainError;
use crate::domain::money::MinorAmount;
use crate::domain::price_row::{
    AggregationFunction, AggregationGranularity, BandTop, BillingGranularity, PriceRow,
    QuantitySource, TierAggregationWindow, TierBand,
};
use crate::domain::scope_key::ChargeKind;
use bss_fixtures::{ModelKind, Variant};

/// The committed corpus registry, resolved from this crate's manifest so the
/// test does not depend on the working directory `cargo test` was invoked from.
fn committed_registry_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/registry.toml")
}

fn minor(units: i64) -> MinorAmount {
    MinorAmount::new(units).expect("a non-negative amount")
}

/// A publishable row of `kind`, in the shape the corpus authors it.
///
/// Whole rows rather than deltas, for the same reason the corpus authors whole
/// rows: the gate is asked about a row, and `is_level` / `is_tiered` /
/// `is_usage` are read off the fields, so a row assembled short would be asked a
/// different question than the one the test names.
fn row(kind: ModelKind) -> PriceRow {
    let mut row = match kind {
        ModelKind::Flat => {
            let mut r = PriceRow::new(ChargeKind::Recurring, Some(kind));
            r.amount_minor = Some(minor(9900));
            r
        }
        ModelKind::PerUnit => {
            let mut r = PriceRow::new(ChargeKind::Recurring, Some(kind));
            r.amount_minor = Some(minor(1500));
            r.quantity_source = Some(QuantitySource::SubscriptionSeatCount);
            r
        }
        ModelKind::Graduated | ModelKind::Volume => {
            let mut r = PriceRow::new(ChargeKind::Usage, Some(kind));
            r.meter = Some("compute.seconds".to_owned());
            r.billing_granularity = Some(BillingGranularity::PerHour);
            r.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
            r.bands = vec![TierBand {
                from_qty: 0,
                to_qty: BandTop::Open,
                unit_price_minor: minor(5),
            }];
            r
        }
        ModelKind::Package => {
            let mut r = PriceRow::new(ChargeKind::Usage, Some(kind));
            r.meter = Some("api.calls".to_owned());
            r.billing_granularity = Some(BillingGranularity::WholeUnit);
            r.tier_aggregation_window = Some(TierAggregationWindow::InvoicePeriod);
            r.package_size = Some(1000);
            r.package_price_minor = Some(minor(2500));
            r
        }
    };
    row.dimension_key = String::new();
    row
}

/// The same row with a granule fold on it: a non-`sum` (level) row.
fn level_row(kind: ModelKind) -> PriceRow {
    let mut row = row(kind);
    row.aggregation_function = Some(AggregationFunction::Peak);
    row.aggregation_granularity = Some(AggregationGranularity::Hour);
    row.max_hold_granules = Some(1);
    row
}

// ---------------------------------------------------------------------------
// The mapping: which variants a row requires
// ---------------------------------------------------------------------------

#[test]
fn every_row_requires_its_own_model_kind_variant() {
    for kind in ModelKind::ALL {
        assert!(
            required_variants(&row(kind), Reservation::Unreserved).contains(&Variant::ModelKind),
            "{kind:?} must require its own fixture: inst-fx-gate blocks any kind without one"
        );
    }
}

#[test]
fn a_sum_row_requires_no_level_aggregation_variant_and_a_non_sum_row_does() {
    // `inst-la-fixture`: publish of any non-`sum` row without the green joint
    // fixture is blocked, exactly like a `modelKind` variant. The `sum` half is
    // the load-bearing one -- if a plain row demanded the level fixture, the
    // fixture would be gating rows it says nothing about.
    let plain = required_variants(&row(ModelKind::Graduated), Reservation::Unreserved);
    assert!(!plain.contains(&Variant::LevelAggregation));

    let level = required_variants(&level_row(ModelKind::Graduated), Reservation::Unreserved);
    assert!(
        level.contains(&Variant::LevelAggregation),
        "a peak row folds granules and needs the fixture that pins the fold"
    );
}

#[test]
fn only_a_tiered_usage_row_requires_the_continuity_variant() {
    // D-22: the continuity fixture gates the first publish of any **tiered
    // usage** kind, alongside that kind's own fixture. Both halves matter --
    // the fixture is about a tier counter, and a non-usage row has none.
    for kind in [ModelKind::Graduated, ModelKind::Volume] {
        assert!(
            required_variants(&row(kind), Reservation::Unreserved)
                .contains(&Variant::SupersessionContinuity),
            "{kind:?} is a tiered usage kind (D-22)"
        );
    }

    // `package` is a usage kind and is not tiered: its money is in the block
    // fields, and `inst-mk-forbidden`/`inst-pk-fields` keep bands off it.
    assert!(
        !required_variants(&row(ModelKind::Package), Reservation::Unreserved)
            .contains(&Variant::SupersessionContinuity)
    );

    // A `graduated` recurring row is not authorable at all
    // (`inst-mk-chargekind`), but the mapping must not be the place that decides
    // so: it reads `is_usage`, and a non-usage row simply has no counter.
    let mut recurring = row(ModelKind::Graduated);
    recurring.charge_kind = ChargeKind::Recurring;
    assert!(
        !required_variants(&recurring, Reservation::Unreserved)
            .contains(&Variant::SupersessionContinuity)
    );

    for kind in [ModelKind::Flat, ModelKind::PerUnit] {
        assert!(
            !required_variants(&row(kind), Reservation::Unreserved)
                .contains(&Variant::SupersessionContinuity)
        );
    }
}

// ---------------------------------------------------------------------------
// The named Slice-10 hole
// ---------------------------------------------------------------------------

#[test]
fn slice_3_cannot_see_a_reservation_and_the_hole_is_named_rather_than_silent() {
    // `reservedRateMinor` / `reservationFlavor` are Slice-10 fields and
    // `PriceRow` is the Slice-3 shape, so there is nothing to read. The gear
    // states that in one place -- `Reservation::of_slice3_row` -- instead of
    // never asking the question. When Slice 10 lands, that function reads the
    // row and this assertion is the one that has to change.
    for kind in ModelKind::ALL {
        assert_eq!(
            Reservation::of_slice3_row(&row(kind)),
            Reservation::Unreserved,
            "no Slice-3 row can report a reservation; if this fails, Slice 10 landed and \
             `of_slice3_row` owes it a real read"
        );
        assert!(
            !required_variants(&row(kind), Reservation::of_slice3_row(&row(kind)))
                .contains(&Variant::Reserved),
            "a row nobody can author as reserved must not be gated on the reservation fixture"
        );
    }
}

#[test]
fn a_reserved_row_requires_the_reservation_variant_the_moment_a_caller_states_one() {
    // The other side of the hole: the rule is implemented and tested now, so the
    // day the field exists the gate is already correct. `inst-fx-kinds` -- "the
    // reservation variant of a usage row requires its own fixture (Slice 10
    // registers it into this gate)".
    let required = required_variants(&row(ModelKind::Graduated), Reservation::Reserved);
    assert!(required.contains(&Variant::Reserved));
    assert!(
        required.contains(&Variant::ModelKind),
        "a reserved row still needs its kind's own fixture: the variants accumulate"
    );
}

#[test]
fn a_reserved_row_is_gated_on_its_own_variant_against_the_committed_registry() {
    // `reserved` carries cases for `graduated` only, so the committed registry
    // registers `(graduated, reserved)` and nothing else. A reserved `volume`
    // row is therefore refused -- and refused by name.
    let gate = FixtureGate::load(&committed_registry_path());

    gate.check(&row(ModelKind::Graduated), Reservation::Reserved)
        .expect("the corpus carries reserved fixtures for graduated");

    let err = gate
        .check(&row(ModelKind::Volume), Reservation::Reserved)
        .expect_err("no reserved fixture exists for volume");
    assert!(
        err.to_string().contains("reserved"),
        "the refusal must name the missing variant: {err}"
    );
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn a_closed_gate_refuses_every_kind() {
    let gate = FixtureGate::closed();

    for kind in ModelKind::ALL {
        let err = gate
            .check(&row(kind), Reservation::Unreserved)
            .expect_err("a closed gate must refuse every kind");
        assert!(
            matches!(err, DomainError::FixtureMissing(_)),
            "{kind:?} must be refused as FIXTURE_MISSING, got {err:?}"
        );
    }
}

#[test]
fn the_refusal_names_the_kind_as_the_corpus_spells_it_and_the_variant_that_is_missing() {
    // The operator's next move is to grep `registry.toml` for the row, and the
    // row is `(kind, variant)` -- so a refusal naming only the kind sends them
    // to five lines, one of which is green and none of which is the one that
    // blocked the publish.
    let err = FixtureGate::closed()
        .check(&row(ModelKind::PerUnit), Reservation::Unreserved)
        .expect_err("closed");

    let text = err.to_string();
    assert!(
        text.contains("per_unit"),
        "the refusal must name the kind in the corpus spelling: {text}"
    );
    assert!(
        text.contains("model_kind"),
        "the refusal must name the variant in the corpus spelling: {text}"
    );
}

#[test]
fn a_missing_registry_yields_a_closed_gate_rather_than_a_panic() {
    // Fail-closed at the load boundary: a deployment without the fixtures
    // artifact still boots and still serves reads, and every publish stops.
    let gate = FixtureGate::load(Path::new(
        "/nonexistent/bss-pricing/does-not-exist/registry.toml",
    ));

    for kind in ModelKind::ALL {
        assert!(
            gate.check(&row(kind), Reservation::Unreserved).is_err(),
            "an unreadable registry must leave {kind:?} closed, never open"
        );
    }
    assert!(gate.open_variants().is_empty());
}

#[test]
fn a_malformed_registry_yields_a_closed_gate() {
    // Not the same failure as a missing file, and it must not be a different
    // answer: a registry the gear cannot parse is a registry it cannot trust.
    let dir = std::env::temp_dir().join("bss-pricing-fixture-gate-tests");
    std::fs::create_dir_all(&dir).expect("create the scratch directory");
    let path = dir.join("malformed-registry.toml");
    std::fs::write(&path, "this is not = = valid toml").expect("write the malformed registry");

    let gate = FixtureGate::load(&path);

    for kind in ModelKind::ALL {
        assert!(
            gate.check(&row(kind), Reservation::Unreserved).is_err(),
            "an unparseable registry must leave {kind:?} closed"
        );
    }
    std::fs::remove_file(&path).expect("clean up the scratch file");
}

#[test]
fn a_row_without_a_model_kind_resolves_no_fixture_and_is_refused() {
    // The kind is the registry's first key. `inst-mk-explicit` has already
    // refused this row; the gate must not be the one place that reads an absent
    // kind as an admissible one.
    let gate = FixtureGate::load(&committed_registry_path());
    let mut kindless = row(ModelKind::Flat);
    kindless.model_kind = None;

    let err = gate
        .check(&kindless, Reservation::Unreserved)
        .expect_err("a row with no kind has no fixture");
    assert!(matches!(err, DomainError::FixtureMissing(_)), "{err:?}");
}

// ---------------------------------------------------------------------------
// Where the committed corpus actually stands
// ---------------------------------------------------------------------------

#[test]
fn the_committed_corpus_is_currently_open_for_every_plain_row() {
    // The honest statement of where the corpus stands, re-stated (not deleted)
    // when the gate changed what it admits -- which is what the previous two
    // versions of this test asked their readers to do.
    //
    // It used to read `closed for every kind`: every row carried `oracle = true`
    // and `publish = false`, because the `publish` half is earned by THIS gear's
    // validator reproducing the corpus's publish cases and that validator did not
    // exist. It now exists, it reproduces every publish case, and the corpus
    // carries at least one answerable publish case per kind -- and, since the
    // coverage check was tightened, at least one expecting `accepted`.
    //
    // "Plain" is the word that changed with the variant axis: a `sum`,
    // unreserved row needs its `modelKind` fixture, plus the continuity fixture
    // if it is a tiered usage row. Both are green for all five kinds.
    //
    // The same instruction applies to this version: the day a row closes again,
    // this test fails, and that failure is the signal to read why before editing
    // it.
    let path = committed_registry_path();
    assert!(
        path.exists(),
        "the committed corpus registry must be present at {}",
        path.display()
    );
    let gate = FixtureGate::load(&path);

    for kind in ModelKind::ALL {
        gate.check(&row(kind), Reservation::Unreserved)
            .unwrap_or_else(|e| panic!("{kind:?} must pass the gate on a plain row: {e}"));
    }
}

#[test]
fn a_level_row_publishes_only_where_a_level_fixture_exists() {
    // The finding, stated as a test. `level-aggregation` carries four cases and
    // every one of them is a `graduated` usage row, so that is the only kind the
    // variant is registered for. Before the variant axis, a `peak` `volume` row
    // published on `volume`'s own fixture -- which pins band arithmetic over a
    // quantity nothing in the corpus folds.
    let gate = FixtureGate::load(&committed_registry_path());

    gate.check(&level_row(ModelKind::Graduated), Reservation::Unreserved)
        .expect("the corpus folds granules on a graduated row");

    let err = gate
        .check(&level_row(ModelKind::Volume), Reservation::Unreserved)
        .expect_err("no case folds a level on a volume row");
    assert!(
        err.to_string().contains("level_aggregation"),
        "the refusal must name the missing variant: {err}"
    );
}

#[test]
fn the_boot_line_shows_the_variants_and_not_only_the_kinds() {
    // `open_kinds` is a floor, not the answer: it would tell an operator that
    // `volume` is open while `volume/level_aggregation` is not registered at
    // all, and the first refusal of a level `volume` row would then read as a
    // bug rather than as the state of the corpus.
    let gate = FixtureGate::load(&committed_registry_path());

    assert_eq!(gate.open_kinds().len(), ModelKind::ALL.len());

    let open = gate.open_variants();
    assert!(
        open.contains(&"graduated/level_aggregation".to_owned()),
        "{open:?}"
    );
    assert!(
        !open.contains(&"volume/level_aggregation".to_owned()),
        "{open:?}"
    );
}
