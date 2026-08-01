use super::*;
use bss_fixtures::{Corpus, ModelKind};

fn corpus() -> Corpus {
    Corpus::load(&Corpus::corpus_root()).expect("corpus loads")
}

#[test]
fn the_committed_registry_is_fresh() {
    let expected = render_for(&corpus()).expect("registry renders");
    let committed = std::fs::read_to_string(Corpus::corpus_root().join("registry.toml"))
        .expect("committed registry exists");

    assert_eq!(
        committed, expected,
        "registry.toml is stale -- run `cargo run -p bss-fixtures-conformance \
         --bin regen_registry` and commit the regeneration on its own"
    );
}

#[test]
fn every_gated_kind_of_a_built_family_earns_its_oracle_flag() {
    let registry = build(&corpus()).expect("registry builds");

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
fn the_gate_stays_shut_until_the_publish_half_is_earned() {
    let registry = build(&corpus()).expect("registry builds");

    // Pricing's validator does not exist yet, so no variant opens the gate --
    // `oracle` alone is not enough.
    for v in &registry.variants {
        assert!(!v.publish, "{:?} must not claim a publish half yet", v.kind);
        assert!(!v.rating, "{:?} must not claim a rating half yet", v.kind);
        assert!(
            !registry.gate_open_for(v.kind),
            "{:?} must not open the gate on the oracle flag alone",
            v.kind
        );
    }
}
