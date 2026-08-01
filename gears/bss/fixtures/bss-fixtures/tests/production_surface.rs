//! Guards the surface a **gear** compiles.
//!
//! Pricing's `FixtureGate` takes this crate with `default-features = false`: it
//! asks the registry whether a kind is green and needs nothing else. That
//! narrow build is easy to break without noticing, because every other build in
//! the workspace turns the `corpus` feature on — so this test compiles the
//! narrow surface on purpose and exercises exactly what the gate does.
//!
//! Run: `cargo test -p bss-fixtures --no-default-features --test production_surface`

use bss_fixtures::{ModelKind, Registry};

const SAMPLE: &str = r#"
[[variants]]
kind    = "graduated"
oracle  = true
publish = true
rating  = false
"#;

#[test]
fn the_gate_answers_from_the_registry_alone() {
    let reg: Registry = toml::from_str(SAMPLE).expect("registry parses");

    assert!(reg.gate_open_for(ModelKind::Graduated));
    assert!(!reg.gate_open_for(ModelKind::Package));
}

#[test]
fn an_empty_registry_opens_nothing() {
    let reg = Registry::default();

    for kind in ModelKind::ALL {
        assert!(!reg.gate_open_for(kind), "{kind:?} must not be open");
    }
}
