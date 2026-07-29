//! Turns raw `Finding`s from the invariant checks into what the CLI actually shows: the
//! pinned baselines (`invariants::propagation::PINNED_PROPAGATION_GAPS_2026_07_29`,
//! `invariants::closure::PINNED_UNREFERENCED_CODES_2026_07_29`) are accepted debt, not
//! new drift — tracked as **D-69** — so a run that reproduces exactly that debt and
//! nothing else should pass, not fail forever on the same known gaps.

use crate::finding::Finding;
use crate::invariants;

/// Decision register / gear-design debt tracking ticket the pinned baselines are owed
/// against. Named once here so the CLI's summary and `--show-known-debt` output don't
/// each carry their own copy of the string.
pub const KNOWN_DEBT_TICKET: &str = "D-69";

/// True if `finding` is exactly one of the pinned, accepted-debt findings (a propagation
/// gap or an unreferenced code) rather than newly appeared drift. Each invariant module
/// owns its own baseline and its own message-shape parsing (see
/// `invariants::propagation::is_pinned_baseline`, `invariants::closure::is_pinned_baseline`)
/// — this just combines them, since a `Finding` only ever carries one invariant tag and
/// so at most one of the two can ever match.
pub fn is_known_debt(finding: &Finding) -> bool {
    invariants::propagation::is_pinned_baseline(finding)
        || invariants::closure::is_pinned_baseline(finding)
}

/// Splits `findings` into `(live, known_debt)` — `live` is what the exit-code decision
/// and the default display are based on; `known_debt` is what `--show-known-debt`
/// reveals. Order within each group is preserved from the input.
pub fn partition_known_debt(findings: Vec<Finding>) -> (Vec<Finding>, Vec<Finding>) {
    findings.into_iter().partition(|f| !is_known_debt(f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Severity;
    use crate::invariants::propagation::PINNED_PROPAGATION_GAPS_2026_07_29;

    fn pinned_propagation_finding() -> Finding {
        let (id, path) = PINNED_PROPAGATION_GAPS_2026_07_29[0];
        Finding {
            invariant: "P1/propagation-missing".to_string(),
            severity: Severity::Medium,
            file: "DECISIONS.md".to_string(),
            line: Some(1),
            message: format!(
                "{id} claims propagation into {path}, but that document never cites {id}"
            ),
        }
    }

    fn non_baseline_finding() -> Finding {
        Finding {
            invariant: "P1/propagation-missing".to_string(),
            severity: Severity::Medium,
            file: "DECISIONS.md".to_string(),
            line: Some(2),
            message: "D-999 claims propagation into PRD.md, but that document never cites D-999"
                .to_string(),
        }
    }

    #[test]
    fn a_pinned_propagation_gap_is_known_debt() {
        assert!(is_known_debt(&pinned_propagation_finding()));
    }

    #[test]
    fn an_unpinned_finding_with_the_same_invariant_tag_is_not_known_debt() {
        assert!(!is_known_debt(&non_baseline_finding()));
    }

    #[test]
    fn partition_separates_baseline_entries_from_new_drift() {
        let (live, known_debt) =
            partition_known_debt(vec![pinned_propagation_finding(), non_baseline_finding()]);
        assert_eq!(live.len(), 1, "unexpected: {live:#?}");
        assert_eq!(known_debt.len(), 1, "unexpected: {known_debt:#?}");
        assert!(!is_known_debt(&live[0]));
        assert!(is_known_debt(&known_debt[0]));
    }
}
