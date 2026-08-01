use super::*;
use crate::corpus::{Corpus, FamilyMeta, GateRole};
use crate::model::{
    Assertion, Case, CaseKind, ChargeExpect, EvaluationCase, Expect, Given, Runtime, Snapshot,
};

fn snapshot(kind: ModelKind) -> Snapshot {
    Snapshot {
        model_kind: kind,
        currency: "USD".into(),
        bands: Vec::new(),
        amount_minor: Some(100),
        package_size: None,
        package_price_minor: None,
        quantity_source: None,
        tier_aggregation_window: None,
        billing_granularity: None,
        proration_basis: None,
        meter: None,
        dimension_key: None,
        aggregation_function: None,
        aggregation_granularity: None,
        max_hold_granules: None,
        tier_qualification_window: None,
        included_allowance: None,
        reserved_rate_minor: None,
        reservation_flavor: None,
    }
}

fn case(id: &str, family: Family, kind: ModelKind, provenance: Vec<String>) -> Case {
    Case::Evaluation(Box::new(EvaluationCase {
        family,
        id: id.into(),
        kind: CaseKind::Evaluation,
        provenance,
        snapshot: snapshot(kind),
        runtime: Runtime::default(),
        assert: vec![Assertion {
            given: Given {
                q: 1,
                ..Given::default()
            },
            expect: Expect::Charge(ChargeExpect { charge_minor: 100 }),
            why: None,
        }],
    }))
}

#[test]
fn a_gated_kind_without_a_case_is_a_violation() {
    let corpus = Corpus {
        cases: vec![case(
            "only-graduated",
            Family::TierBoundary,
            ModelKind::Graduated,
            vec!["AC#60".into()],
        )],
        families: vec![FamilyMeta {
            family: Family::TierBoundary,
            role: GateRole::Publish,
            gates: vec![ModelKind::Graduated, ModelKind::Volume],
            provenance: vec!["AC#60".into()],
        }],
    };

    let violations = check_integrity(&corpus);

    assert!(
        matches!(
            violations.as_slice(),
            [IntegrityViolation::GatedKindUncovered {
                kind: ModelKind::Volume,
                ..
            }]
        ),
        "got: {violations:?}"
    );
}

#[test]
fn empty_provenance_is_a_violation() {
    let corpus = Corpus {
        cases: vec![case(
            "no-provenance",
            Family::PerUnit,
            ModelKind::PerUnit,
            Vec::new(),
        )],
        families: vec![FamilyMeta {
            family: Family::PerUnit,
            role: GateRole::Publish,
            gates: vec![ModelKind::PerUnit],
            provenance: vec!["AC#60".into()],
        }],
    };

    let violations = check_integrity(&corpus);

    assert!(
        violations
            .iter()
            .any(|v| matches!(v, IntegrityViolation::MissingProvenance { .. }))
    );
}

#[test]
fn a_family_directory_without_a_declaration_is_a_violation() {
    let corpus = Corpus {
        cases: vec![case(
            "orphan",
            Family::Package,
            ModelKind::Package,
            vec!["AC#60".into()],
        )],
        families: Vec::new(),
    };

    let violations = check_integrity(&corpus);

    assert!(
        violations
            .iter()
            .any(|v| matches!(v, IntegrityViolation::UndeclaredFamily { .. }))
    );
}

#[test]
fn a_case_with_no_assertions_is_a_violation() {
    let mut c = case(
        "asserts-nothing",
        Family::PerUnit,
        ModelKind::PerUnit,
        vec!["AC#60".into()],
    );
    if let Case::Evaluation(e) = &mut c {
        e.assert.clear();
    }

    let corpus = Corpus {
        cases: vec![c],
        families: vec![FamilyMeta {
            family: Family::PerUnit,
            role: GateRole::Publish,
            gates: vec![ModelKind::PerUnit],
            provenance: vec!["AC#60".into()],
        }],
    };

    let violations = check_integrity(&corpus);

    assert!(
        violations
            .iter()
            .any(|v| matches!(v, IntegrityViolation::CaseAssertsNothing { .. }))
    );
}

#[test]
fn a_conformance_family_that_gates_is_a_violation() {
    // `proration` is AC #61 and blocks no publish. Claiming the conformance role
    // while listing gated kinds is a contradiction, not a shorthand.
    let corpus = Corpus {
        cases: Vec::new(),
        families: vec![FamilyMeta {
            family: Family::Proration,
            role: GateRole::Conformance,
            gates: vec![ModelKind::Flat],
            provenance: vec!["AC#61".into()],
        }],
    };

    let violations = check_integrity(&corpus);

    assert!(
        violations
            .iter()
            .any(|v| matches!(v, IntegrityViolation::ConformanceFamilyGates { .. })),
        "got: {violations:?}"
    );
}

#[test]
fn a_conformance_family_gating_nothing_is_fine() {
    // The mirror: gating nothing is correct for this role, so it must not trip
    // `FamilyGatesNothing`.
    let corpus = Corpus {
        cases: Vec::new(),
        families: vec![FamilyMeta {
            family: Family::Proration,
            role: GateRole::Conformance,
            gates: Vec::new(),
            provenance: vec!["AC#61".into()],
        }],
    };

    assert!(check_integrity(&corpus).is_empty());
}

#[test]
fn a_rejection_without_an_error_code_is_a_violation() {
    // "publish fails" without saying how is not reviewable, and the codes are
    // themselves part of the published contract.
    use crate::model::{PublishAssertion, PublishCase, PublishVerdict};

    let corpus = Corpus {
        cases: vec![Case::Publish(Box::new(PublishCase {
            family: Family::SupersessionContinuity,
            id: "nameless-rejection".into(),
            kind: CaseKind::Publish,
            provenance: vec!["D-82".into()],
            predecessor: snapshot(ModelKind::Graduated),
            successor: snapshot(ModelKind::Graduated),
            assert: vec![PublishAssertion {
                expect: PublishVerdict::Rejected {
                    error_code: "  ".into(),
                },
                why: None,
            }],
        }))],
        families: vec![FamilyMeta {
            family: Family::SupersessionContinuity,
            role: GateRole::Conformance,
            gates: Vec::new(),
            provenance: vec!["D-82".into()],
        }],
    };

    let violations = check_integrity(&corpus);

    assert!(
        violations
            .iter()
            .any(|v| matches!(v, IntegrityViolation::RejectionWithoutCode { .. })),
        "got: {violations:?}"
    );
}

#[test]
fn an_ungated_model_kind_is_a_violation() {
    // Only `flat` is gated here, so the other four kinds are unpublishable.
    let corpus = Corpus {
        cases: Vec::new(),
        families: vec![FamilyMeta {
            family: Family::Flat,
            role: GateRole::Publish,
            gates: vec![ModelKind::Flat],
            provenance: vec!["AC#60".into()],
        }],
    };

    let violations = check_kind_coverage(&corpus);

    assert_eq!(violations.len(), 4, "got: {violations:?}");
    assert!(violations.contains(&IntegrityViolation::ModelKindUngated {
        kind: ModelKind::Graduated
    }));
}

#[test]
fn kind_coverage_is_separate_from_internal_consistency() {
    // A partial corpus is legitimately incomplete, so `check_integrity` must not
    // fail it for being partial — that is why the two checks are separate.
    let corpus = Corpus {
        cases: vec![case(
            "only-flat",
            Family::Flat,
            ModelKind::Flat,
            vec!["AC#60".into()],
        )],
        families: vec![FamilyMeta {
            family: Family::Flat,
            role: GateRole::Publish,
            gates: vec![ModelKind::Flat],
            provenance: vec!["AC#60".into()],
        }],
    };

    assert!(check_integrity(&corpus).is_empty());
    assert!(!check_kind_coverage(&corpus).is_empty());
}

#[test]
fn the_committed_corpus_gates_every_model_kind() {
    let corpus = Corpus::load(&Corpus::corpus_root()).expect("corpus loads");

    let ungated = check_kind_coverage(&corpus);

    assert!(
        ungated.is_empty(),
        "every catalog modelKind must be gated by some family: {ungated:#?}"
    );
}

#[test]
fn the_committed_corpus_is_clean() {
    let corpus = Corpus::load(&Corpus::corpus_root()).expect("corpus loads");

    let violations = check_integrity(&corpus);

    assert!(
        violations.is_empty(),
        "corpus integrity violations: {violations:#?}"
    );
}
