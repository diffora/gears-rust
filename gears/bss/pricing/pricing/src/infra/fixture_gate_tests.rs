//! Tests for [`crate::infra::fixture_gate`].
//!
//! The interesting cases are all the *closed* ones: this gate has exactly one
//! job, which is to refuse when it has no positive evidence.

use std::path::{Path, PathBuf};

use super::FixtureGate;
use crate::domain::error::DomainError;
use bss_fixtures::ModelKind;

/// The committed corpus registry, resolved from this crate's manifest so the
/// test does not depend on the working directory `cargo test` was invoked from.
fn committed_registry_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/registry.toml")
}

#[test]
fn a_closed_gate_refuses_every_kind() {
    let gate = FixtureGate::closed();

    for kind in ModelKind::ALL {
        let err = gate
            .check(kind)
            .expect_err("a closed gate must refuse every kind");
        assert!(
            matches!(err, DomainError::FixtureMissing(_)),
            "{kind:?} must be refused as FIXTURE_MISSING, got {err:?}"
        );
    }
}

#[test]
fn the_refusal_names_the_kind_as_the_corpus_spells_it() {
    // The operator's next move is to grep `registry.toml` for the kind, so the
    // message has to carry the corpus spelling and not the Rust identifier.
    let err = FixtureGate::closed()
        .check(ModelKind::PerUnit)
        .expect_err("closed");

    let text = err.to_string();
    assert!(
        text.contains("per_unit"),
        "the refusal must name the kind in the corpus spelling: {text}"
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
            gate.check(kind).is_err(),
            "an unreadable registry must leave {kind:?} closed, never open"
        );
    }
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
            gate.check(kind).is_err(),
            "an unparseable registry must leave {kind:?} closed"
        );
    }
    std::fs::remove_file(&path).expect("clean up the scratch file");
}

#[test]
fn the_committed_corpus_is_currently_open_for_every_kind() {
    // The honest statement of where the corpus stands, re-stated (not deleted)
    // when the gate changed what it admits -- which is what the previous version
    // of this test asked its reader to do.
    //
    // It used to read `closed for every kind`: every row carried `oracle = true`
    // and `publish = false`, because the `publish` half is earned by THIS gear's
    // validator reproducing the corpus's publish cases and that validator did not
    // exist. It now exists, it reproduces every publish case, and the corpus
    // carries at least one answerable publish case per kind -- `flat` and
    // `per_unit` had none at all, which is why their gates could never have
    // opened however correct the gear was. So `oracle && publish` holds for all
    // five.
    //
    // The same instruction applies to this version: the day a kind closes again,
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
        assert!(
            gate.check(kind).is_ok(),
            "{kind:?} has earned both halves of the corpus registry and must pass the gate"
        );
    }
}
