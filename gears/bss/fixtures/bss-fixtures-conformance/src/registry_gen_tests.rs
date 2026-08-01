use super::*;
use bss_fixtures::{Corpus, ModelKind};

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
    // guarantees each one is gated, so each must also earn its flag.
    for kind in ModelKind::ALL {
        let v = registry
            .variants
            .iter()
            .find(|v| v.kind == kind)
            .unwrap_or_else(|| panic!("{kind:?} must be registered"));
        assert!(v.oracle, "{kind:?} must have earned its oracle flag");
    }
}

#[test]
fn the_gate_opens_only_where_both_halves_are_earned() {
    // `FixtureGate` reads `oracle && publish`. Opening the gate for a kind
    // therefore means two separate runs have passed for it: the reference
    // oracle reproduced its evaluation cases, and pricing's `PublishValidator`
    // reproduced its publish cases. Neither half alone is publishable, and the
    // `rating` half is deliberately not consulted -- requiring it would block
    // every publish at launch, since the rating gear has no code.
    let oracle_only = build(&corpus(), &[]).expect("registry builds");

    for v in &oracle_only.variants {
        assert!(v.oracle, "{:?} must have earned its oracle flag", v.kind);
        assert!(!v.publish, "{:?} was handed no publish half", v.kind);
        assert!(!v.rating, "{:?} must not claim a rating half yet", v.kind);
        assert!(
            !oracle_only.gate_open_for(v.kind),
            "{:?} must not open the gate on the oracle flag alone",
            v.kind
        );
    }
}

#[test]
fn the_publish_half_is_recorded_exactly_as_handed_in() {
    // The generator does not know who ran what, and must not decide it either:
    // it records the set it is given, kind for kind. A kind absent from the
    // earned set stays shut even though its oracle half is green.
    let earned = build(&corpus(), &[ModelKind::Volume]).expect("registry builds");

    assert!(
        earned.gate_open_for(ModelKind::Volume),
        "volume earned both halves and must open"
    );
    for kind in ModelKind::ALL
        .into_iter()
        .filter(|k| *k != ModelKind::Volume)
    {
        assert!(
            !earned.gate_open_for(kind),
            "{kind:?} earned no publish half and must stay shut"
        );
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
