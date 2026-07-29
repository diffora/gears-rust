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

/// True if `finding`, attributed to `gear`, is exactly one of the pinned, accepted-debt
/// findings (a propagation gap or an unreferenced code) rather than newly appeared
/// drift. Each invariant module owns its own baseline and its own message-shape parsing
/// (see `invariants::propagation::is_pinned_baseline`,
/// `invariants::closure::is_pinned_baseline`) — this just combines them, since a
/// `Finding` only ever carries one invariant tag and so at most one of the two can ever
/// match.
///
/// `gear` is required, not optional or read off `finding` (a `Finding` has no gear field
/// — see `finding.rs`): both pinned baselines are snapshots of the *pricing* corpus
/// specifically (task-review Ruling 3 finding, 2026-07-29, fix round 3), and their keys
/// (`D-NN`, an error-code token, a corpus-relative file) are not unique across gears —
/// rating and subscriptions have their own `DECISIONS.md` with their own `D-NN` ids, and
/// share filenames like `PRD.md` or `design/03-price-structure.md`. Callers must supply
/// the gear the finding actually came from (see `main.rs`, which computes it once per
/// loaded corpus via `targets::gear_name` before this decision is made), or a same-keyed
/// finding from a different gear would be silently suppressed as pricing's pinned debt.
pub fn is_known_debt(finding: &Finding, gear: &str) -> bool {
    invariants::propagation::is_pinned_baseline(finding, gear)
        || invariants::closure::is_pinned_baseline(finding, gear)
}

/// Splits `findings` into `(live, known_debt)` — `live` is what the exit-code decision
/// and the default display are based on; `known_debt` is what `--show-known-debt`
/// reveals. Order within each group is preserved from the input. All of `findings` must
/// come from the same gear (see `is_known_debt`) — callers with more than one loaded
/// corpus call this once per corpus and accumulate the results, rather than flattening
/// findings across corpora before this decision is made.
pub fn partition_known_debt(findings: Vec<Finding>, gear: &str) -> (Vec<Finding>, Vec<Finding>) {
    findings.into_iter().partition(|f| !is_known_debt(f, gear))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Severity;
    use crate::invariants::closure::PINNED_UNREFERENCED_CODES_2026_07_29;
    use crate::invariants::propagation::PINNED_PROPAGATION_GAPS_2026_07_29;

    fn pinned_propagation_finding() -> Finding {
        let (_, id, path) = PINNED_PROPAGATION_GAPS_2026_07_29[0];
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

    fn pinned_code_unreferenced_finding() -> Finding {
        let (_, code, file) = PINNED_UNREFERENCED_CODES_2026_07_29[0];
        Finding {
            invariant: "P3/code-unreferenced".to_string(),
            severity: Severity::Low,
            file: file.to_string(),
            line: None,
            message: format!(
                "`{code}` is declared in a Problem-responses block but referenced by no rule"
            ),
        }
    }

    #[test]
    fn a_pinned_propagation_gap_is_known_debt() {
        assert!(is_known_debt(&pinned_propagation_finding(), "pricing"));
    }

    #[test]
    fn an_unpinned_finding_with_the_same_invariant_tag_is_not_known_debt() {
        assert!(!is_known_debt(&non_baseline_finding(), "pricing"));
    }

    #[test]
    fn a_pinned_code_unreferenced_finding_is_known_debt() {
        // Mirror of `a_pinned_propagation_gap_is_known_debt` for the closure side
        // (task-review finding: `closure::is_pinned_baseline` had zero test coverage —
        // `report::is_known_debt`'s second disjunct was never exercised by this module's
        // own tests before this).
        assert!(is_known_debt(
            &pinned_code_unreferenced_finding(),
            "pricing"
        ));
    }

    #[test]
    fn a_same_keyed_propagation_finding_from_a_different_gear_is_not_known_debt() {
        // The mandated cross-corpus-safety test (task-review Ruling 3, CRITICAL): a
        // `P1/propagation-missing` whose (id, path) is byte-identical to a pinned
        // pricing entry must not be known debt when attributed to a non-pricing corpus
        // — it is new drift in that gear, not the pricing debt the baseline pins.
        let (gear, _, _) = PINNED_PROPAGATION_GAPS_2026_07_29[0];
        assert_eq!(gear, "pricing", "test assumes entry 0 is pricing's");
        assert!(!is_known_debt(&pinned_propagation_finding(), "rating"));
    }

    #[test]
    fn a_same_keyed_code_unreferenced_finding_from_a_different_gear_is_not_known_debt() {
        // Same guarantee, closure side ("do the same for the code-unreferenced side").
        let (gear, _, _) = PINNED_UNREFERENCED_CODES_2026_07_29[0];
        assert_eq!(gear, "pricing", "test assumes entry 0 is pricing's");
        assert!(!is_known_debt(
            &pinned_code_unreferenced_finding(),
            "rating"
        ));
    }

    #[test]
    fn partition_separates_baseline_entries_from_new_drift() {
        let (live, known_debt) = partition_known_debt(
            vec![pinned_propagation_finding(), non_baseline_finding()],
            "pricing",
        );
        assert_eq!(live.len(), 1, "unexpected: {live:#?}");
        assert_eq!(known_debt.len(), 1, "unexpected: {known_debt:#?}");
        assert!(!is_known_debt(&live[0], "pricing"));
        assert!(is_known_debt(&known_debt[0], "pricing"));
    }

    #[test]
    fn partition_does_not_suppress_a_same_keyed_finding_attributed_to_another_gear() {
        // End-to-end version of the cross-corpus-safety guarantee at the partition
        // level: a pricing-keyed pinned finding, if it were (hypothetically) produced
        // while checking a different gear's corpus, must land in `live`, not
        // `known_debt`.
        let (live, known_debt) = partition_known_debt(vec![pinned_propagation_finding()], "rating");
        assert_eq!(live.len(), 1, "unexpected: {live:#?}");
        assert!(known_debt.is_empty(), "unexpected: {known_debt:#?}");
    }
}
