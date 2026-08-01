use super::*;
use crate::kinds::ModelKind;

const SAMPLE: &str = r#"
[[variants]]
kind    = "graduated"
oracle  = true
publish = true
rating  = false

[[variants]]
kind    = "volume"
oracle  = true
publish = false
rating  = false
"#;

#[test]
fn the_gate_opens_only_on_oracle_and_publish() {
    let reg: Registry = toml::from_str(SAMPLE).expect("registry parses");

    // The `rating` flag is not part of the gate: rating has no code yet, and
    // requiring it would block every publish at launch.
    assert!(reg.gate_open_for(ModelKind::Graduated));
    // Publish half not yet earned.
    assert!(!reg.gate_open_for(ModelKind::Volume));
    // Never registered at all — an absent kind is never open.
    assert!(!reg.gate_open_for(ModelKind::Package));
}

#[test]
fn an_empty_registry_opens_nothing() {
    let reg = Registry::default();

    for kind in [
        ModelKind::Flat,
        ModelKind::PerUnit,
        ModelKind::Graduated,
        ModelKind::Volume,
        ModelKind::Package,
    ] {
        assert!(!reg.gate_open_for(kind), "{kind:?} must not be open");
    }
}

// Reading the committed file needs the corpus layout, which is the feature's
// business; the gate itself never does this.
#[cfg(feature = "corpus")]
#[test]
fn the_committed_registry_parses() {
    Registry::load(&crate::Corpus::corpus_root().join("registry.toml"))
        .expect("committed registry must parse");
}
