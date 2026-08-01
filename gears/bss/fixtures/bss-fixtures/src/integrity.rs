//! Checks the corpus holds together on its own terms.
//!
//! The load-bearing one is [`IntegrityViolation::GatedKindUncovered`]: a family
//! may declare it gates a `modelKind` and carry no case for it, which would let
//! the registry report green over absent coverage — a rule with a gate and no
//! coverage is exactly the quiet hole this corpus exists to close.
//!
//! Resolving each `provenance` id against the pricing design set is **not** done
//! here; that belongs with spec-check, which already declares and references
//! instruction ids and error codes. What is checked here is self-contained.

use crate::corpus::{Corpus, GateRole};
use crate::model::{Case, Family, ModelKind, PublishVerdict};

#[derive(Debug, PartialEq, Eq)]
pub enum IntegrityViolation {
    /// A family declares it gates this kind, but no case exercises it.
    GatedKindUncovered { family: Family, kind: ModelKind },
    /// Every case must cite the normative clause it encodes.
    MissingProvenance { case_id: String },
    /// A case exists for a family with no `_family.toml`.
    UndeclaredFamily { family: Family },
    /// A family claims the `Publish` role but lists no kinds, so it blocks
    /// nothing despite saying it does.
    FamilyGatesNothing { family: Family },
    /// A family claims the `Conformance` role yet lists gated kinds. The two
    /// readings contradict: either it gates publish or it does not.
    ConformanceFamilyGates { family: Family },
    /// A case carries no assertions, so it proves nothing.
    CaseAssertsNothing { case_id: String },
    /// A publish case expects a rejection but names no error code.
    RejectionWithoutCode { case_id: String },
    /// A publish case declares itself unanswerable but names no slice.
    ///
    /// Same discipline as [`Self::RejectionWithoutCode`]: "nothing can answer
    /// this yet" without saying what would is not reviewable, and it is the one
    /// declaration in the corpus that suspends a case's evidence.
    DeclineWithoutSlice { case_id: String },
    /// A catalog `modelKind` no family gates. `inst-fx-gate` blocks publish of
    /// any kind without a green fixture, so an ungated kind is unpublishable.
    ModelKindUngated { kind: ModelKind },
    /// A catalog `modelKind` no publish case exercises.
    ///
    /// The registry's `publish` half is earned **per kind** by a passing run, so
    /// a kind the corpus asks no publish question of can never earn it: the gate
    /// stays shut for that kind forever. Exactly the shape of the `flat` hole —
    /// a rule whose gate nothing can open reads as a rule, and is a wall.
    ModelKindWithoutPublishCase { kind: ModelKind },
}

/// Checks that every catalog `modelKind` is gated by some family.
///
/// Kept separate from [`check_integrity`] deliberately. Internal consistency
/// holds of any corpus, including the partial ones tests build; **completeness**
/// is a property of the committed corpus alone. Running them together would make
/// every partial fixture fail for being partial.
///
/// This is the check that would have caught `flat`: families and kinds are
/// different axes — nine families against five kinds, with `tier-boundary`
/// gating two — so a kind can quietly belong to no family at all.
#[must_use]
pub fn check_kind_coverage(corpus: &Corpus) -> Vec<IntegrityViolation> {
    ModelKind::ALL
        .iter()
        .filter(|kind| {
            !corpus
                .families
                .iter()
                .any(|f| f.gates.iter().any(|g| g == *kind))
        })
        .map(|kind| IntegrityViolation::ModelKindUngated { kind: *kind })
        .collect()
}

/// Checks that every catalog `modelKind` is exercised by some publish case.
///
/// The sibling of [`check_kind_coverage`], on the other half of the registry and
/// for the same reason. That one asks whether a kind is *gated*; this one asks
/// whether the gate can ever **open**. `publish` is earned per kind by a passing
/// run over that kind's publish cases, so a kind with none earns nothing — and
/// "earns nothing" is indistinguishable, in the committed file, from "failed".
/// `flat` and `per_unit` sat in exactly that state: gated by a family, green on
/// their oracle half, and permanently unpublishable because no case existed to
/// pass.
///
/// A case marked `declined_until` does **not** count. It is authored against a
/// slice nothing has built, so it cannot pass and cannot earn the flag either;
/// counting it would let a kind's only coverage be a case that can never be
/// answered — the same silence, one level down.
///
/// Kept out of [`check_integrity`] for the same reason as its sibling:
/// completeness is a property of the committed corpus, not of every partial one
/// a test builds.
#[must_use]
pub fn check_publish_case_coverage(corpus: &Corpus) -> Vec<IntegrityViolation> {
    ModelKind::ALL
        .iter()
        .filter(|kind| {
            !corpus.cases.iter().any(|case| match case {
                Case::Publish(p) => p.declined_until.is_none() && p.successor.model_kind == **kind,
                Case::Evaluation(_) => false,
            })
        })
        .map(|kind| IntegrityViolation::ModelKindWithoutPublishCase { kind: *kind })
        .collect()
}

#[must_use]
pub fn check_integrity(corpus: &Corpus) -> Vec<IntegrityViolation> {
    let mut out = Vec::new();

    for case in &corpus.cases {
        if case.provenance().is_empty() {
            out.push(IntegrityViolation::MissingProvenance {
                case_id: case.id().to_owned(),
            });
        }
        if case.assertion_count() == 0 {
            out.push(IntegrityViolation::CaseAssertsNothing {
                case_id: case.id().to_owned(),
            });
        }
        // A rejection that names no code is not reviewable, and the codes are
        // themselves part of the published contract.
        if let Case::Publish(p) = case {
            for a in &p.assert {
                if matches!(&a.expect, PublishVerdict::Rejected { error_code } if error_code.trim().is_empty())
                {
                    out.push(IntegrityViolation::RejectionWithoutCode {
                        case_id: p.id.clone(),
                    });
                }
            }
            // A decline suspends the case's evidence, so the slice that would
            // restore it has to be named.
            if p.declined_until
                .as_ref()
                .is_some_and(|slice| slice.trim().is_empty())
            {
                out.push(IntegrityViolation::DeclineWithoutSlice {
                    case_id: p.id.clone(),
                });
            }
        }
        if !corpus.families.iter().any(|f| f.family == case.family()) {
            let v = IntegrityViolation::UndeclaredFamily {
                family: case.family(),
            };
            if !out.contains(&v) {
                out.push(v);
            }
        }
    }

    for meta in &corpus.families {
        match meta.role {
            GateRole::Publish if meta.gates.is_empty() => {
                out.push(IntegrityViolation::FamilyGatesNothing {
                    family: meta.family,
                });
            }
            GateRole::Conformance if !meta.gates.is_empty() => {
                out.push(IntegrityViolation::ConformanceFamilyGates {
                    family: meta.family,
                });
            }
            _ => {}
        }
        for kind in &meta.gates {
            let covered = corpus
                .cases_for(meta.family)
                .any(|c| c.model_kind() == *kind);
            if !covered {
                out.push(IntegrityViolation::GatedKindUncovered {
                    family: meta.family,
                    kind: *kind,
                });
            }
        }
    }

    out
}

#[cfg(test)]
#[path = "integrity_tests.rs"]
mod integrity_tests;
