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
    /// A catalog `modelKind` no family gates. `inst-fx-gate` blocks publish of
    /// any kind without a green fixture, so an ungated kind is unpublishable.
    ModelKindUngated { kind: ModelKind },
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
