use super::*;
use bss_fixtures::{Corpus, Family, ModelKind, Variant};

fn corpus() -> Corpus {
    Corpus::load(&Corpus::corpus_root()).expect("corpus loads")
}

/// The committed file's freshness is asserted **once**, and not here.
///
/// This crate can only compute one of the two halves the file records, so an
/// expectation built here would be an expectation over a registry with the
/// publish half nailed shut. The authority is the gear, which sees both:
/// `gears/bss/pricing/pricing/tests/corpus_publish.rs`.
#[test]
fn every_gated_kind_of_a_built_family_earns_its_oracle_flag() {
    let registry = build(&corpus(), &[]).expect("registry builds");

    // Every catalog kind, not a hand-kept subset: `check_kind_coverage`
    // guarantees each one is gated on its own `model_kind` variant, so each must
    // also earn its flag there.
    for kind in ModelKind::ALL {
        let v = registry
            .variants
            .iter()
            .find(|v| v.kind == kind && v.variant == Variant::ModelKind)
            .unwrap_or_else(|| panic!("{kind:?} must be registered on its own variant"));
        assert!(v.oracle, "{kind:?} must have earned its oracle flag");
    }
}

#[test]
fn a_row_is_written_per_kind_and_variant() {
    // The registry is keyed `(kind, variant)` -- S3 design 6's `model_kind` /
    // `variant` pair -- and the cross-cutting families are what put more than
    // one row under a kind. Keyed by kind alone, `level-aggregation`,
    // `supersession-continuity` and `reserved` gated nothing at all.
    let registry = build(&corpus(), &[]).expect("registry builds");

    let has = |kind: ModelKind, variant: Variant| {
        registry
            .variants
            .iter()
            .any(|v| v.kind == kind && v.variant == variant)
    };

    assert!(has(ModelKind::Graduated, Variant::ModelKind));
    assert!(has(ModelKind::Graduated, Variant::LevelAggregation));
    assert!(has(ModelKind::Graduated, Variant::SupersessionContinuity));
    assert!(has(ModelKind::Graduated, Variant::Reserved));
    // D-22 scopes the continuity fixture to the tiered usage kinds, so `volume`
    // has one and `package` does not.
    assert!(has(ModelKind::Volume, Variant::SupersessionContinuity));
    assert!(!has(ModelKind::Package, Variant::SupersessionContinuity));
    // No case folds a level on a `volume` row, so the pair is absent -- and an
    // absent pair is never open, which is what refuses a `peak` volume row.
    assert!(!has(ModelKind::Volume, Variant::LevelAggregation));
}

#[test]
fn a_family_that_maps_to_no_variant_writes_no_row() {
    // `proration` is AC #61 and gates nothing, so it contributes no row however
    // green it is. `trailing-tier` is in the same state for a different reason
    // (Slice 10 owns `inst-tt-fixture`, and the family carries no case at all).
    assert_eq!(Family::Proration.variant(), None);
    assert_eq!(Family::TrailingTier.variant(), None);

    let registry = build(&corpus(), &[]).expect("registry builds");
    assert!(
        registry
            .variants
            .iter()
            .all(|v| Variant::ALL.contains(&v.variant))
    );
}

#[test]
fn the_gate_opens_only_where_both_halves_are_earned() {
    // `FixtureGate` reads `oracle && publish`. Opening the gate for a
    // `(kind, variant)` pair therefore means two separate runs have passed: the
    // reference oracle reproduced the family's evaluation cases, and pricing's
    // `PublishValidator` reproduced the kind's publish cases. Neither half alone
    // is publishable, and the `rating` half is deliberately not consulted --
    // requiring it would block every publish at launch, since the rating gear
    // has no code.
    let oracle_only = build(&corpus(), &[]).expect("registry builds");

    for v in &oracle_only.variants {
        assert!(v.oracle, "{:?} must have earned its oracle flag", v.kind);
        assert!(!v.publish, "{:?} was handed no publish half", v.kind);
        assert!(!v.rating, "{:?} must not claim a rating half yet", v.kind);
        assert!(
            !oracle_only.gate_open_for(v.kind, v.variant),
            "{:?}/{} must not open the gate on the oracle flag alone",
            v.kind,
            v.variant.wire()
        );
    }
}

#[test]
fn the_publish_half_is_recorded_exactly_as_handed_in() {
    // The generator does not know who ran what, and must not decide it either:
    // it records the set it is given, kind for kind. A kind absent from the
    // earned set stays shut even though its oracle half is green.
    //
    // Per **kind**, across every variant of it: an outcome is attributed to
    // `successor.model_kind` and to nothing else, so a failing publish case
    // blocks every fixture the kind has.
    let earned = build(&corpus(), &[ModelKind::Volume]).expect("registry builds");

    assert!(
        earned.gate_open_for(ModelKind::Volume, Variant::ModelKind),
        "volume earned both halves and must open"
    );
    assert!(
        earned.gate_open_for(ModelKind::Volume, Variant::SupersessionContinuity),
        "the publish half is earned per kind, so it reaches every variant of it"
    );
    for kind in ModelKind::ALL
        .into_iter()
        .filter(|k| *k != ModelKind::Volume)
    {
        for variant in Variant::ALL {
            assert!(
                !earned.gate_open_for(kind, variant),
                "{kind:?}/{} earned no publish half and must stay shut",
                variant.wire()
            );
        }
    }
}

#[test]
fn the_generated_header_names_the_command_that_regenerates_it() {
    // The header is the only instruction a reader of `registry.toml` gets, and
    // the command moved when the gear took ownership of the file. A header
    // naming a target that no longer exists is worse than none.
    let rendered = render_for(&corpus(), &[]).expect("registry renders");

    assert!(
        rendered.contains("cargo run -p bss-pricing --example regen_registry"),
        "the header must name the real regeneration command"
    );
}
