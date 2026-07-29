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

/// The whole text report, ready to print (no trailing newline — the caller's `println!`
/// supplies it).
///
/// Lives here, not inlined in `main`'s `match` arm, because the suppression *policy* is this
/// module's responsibility and so is disclosing it: the line `75 known-debt finding(s)
/// suppressed, tracked as D-69` is the only thing standing between 75 suppressed findings and
/// 75 invisible ones, and while it was declared inline in the binary no test could reach it —
/// a refactor that dropped it would have failed nothing. Both summary variants and the
/// `show_known_debt` body are asserted in this module's tests.
pub fn render_text(live: &[Finding], known_debt: &[Finding], show_known_debt: bool) -> String {
    let mut out = String::new();
    for f in live {
        out.push_str(&f.render());
        out.push('\n');
    }
    if show_known_debt && !known_debt.is_empty() {
        out.push_str(&format!(
            "\nKnown debt — accepted, tracked as {KNOWN_DEBT_TICKET}, not new drift \
             ({} finding(s)):\n",
            known_debt.len()
        ));
        for f in known_debt {
            out.push_str(&f.render());
            out.push('\n');
        }
    }
    out.push_str(&format!("\n{} finding(s)", live.len()));
    if !known_debt.is_empty() {
        out.push('\n');
        if show_known_debt {
            out.push_str(&format!(
                "{} known-debt finding(s) shown above, tracked as {KNOWN_DEBT_TICKET} \
                 (accepted, not new drift)",
                known_debt.len()
            ));
        } else {
            out.push_str(&format!(
                "{} known-debt finding(s) suppressed, tracked as {KNOWN_DEBT_TICKET} \
                 — pass --show-known-debt to see them",
                known_debt.len()
            ));
        }
    }
    out
}

/// The `--format json` envelope. A named type here rather than an anonymous struct declared
/// inside `main`'s `match` arm, for the same reason as `render_text`: the suppressed count and
/// the ticket it is tracked against are part of the reporting contract a consumer parses, so
/// they belong somewhere a test can see them.
#[derive(serde::Serialize)]
pub struct JsonReport<'a> {
    pub findings: &'a [Finding],
    pub known_debt_suppressed: usize,
    pub known_debt_tracked_as: &'static str,
    /// Present only under `--show-known-debt`, so the default envelope stays the live set
    /// plus an honest count of what was withheld.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_debt: Option<&'a [Finding]>,
}

impl<'a> JsonReport<'a> {
    pub fn new(live: &'a [Finding], known_debt: &'a [Finding], show_known_debt: bool) -> Self {
        Self {
            findings: live,
            known_debt_suppressed: known_debt.len(),
            known_debt_tracked_as: KNOWN_DEBT_TICKET,
            known_debt: show_known_debt.then_some(known_debt),
        }
    }
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

    #[test]
    fn the_default_summary_discloses_how_many_findings_were_suppressed() {
        // The one line that makes suppression honest rather than silent. Untested while it
        // lived inline in `main`'s match arm: a refactor that dropped it failed nothing, and
        // the difference between "6 findings" and "6 findings, 75 suppressed" is the
        // difference between a report and a misleading one.
        let out = render_text(
            &[non_baseline_finding()],
            &[pinned_propagation_finding()],
            false,
        );
        assert!(out.contains("\n1 finding(s)"), "unexpected: {out}");
        assert!(
            out.contains("1 known-debt finding(s) suppressed, tracked as D-69"),
            "the default summary must disclose the suppressed count: {out}"
        );
        assert!(
            out.contains("--show-known-debt"),
            "and must say how to see them: {out}"
        );
        // Suppressed means suppressed: the withheld finding's own text must not be printed,
        // or the count line would be describing something already on screen.
        assert!(
            !out.contains(&pinned_propagation_finding().message),
            "a suppressed finding must not be rendered: {out}"
        );
    }

    #[test]
    fn show_known_debt_renders_the_suppressed_findings_and_switches_the_summary() {
        let out = render_text(
            &[non_baseline_finding()],
            &[pinned_propagation_finding()],
            true,
        );
        assert!(
            out.contains("Known debt — accepted, tracked as D-69, not new drift (1 finding(s)):"),
            "unexpected: {out}"
        );
        assert!(
            out.contains(&pinned_propagation_finding().message),
            "the suppressed finding must now be rendered: {out}"
        );
        assert!(
            out.contains("1 known-debt finding(s) shown above, tracked as D-69"),
            "the summary must switch to the shown variant: {out}"
        );
        assert!(
            !out.contains("suppressed"),
            "and must not still claim suppression: {out}"
        );
    }

    #[test]
    fn a_run_with_no_known_debt_prints_no_known_debt_summary_at_all() {
        // Both variants are conditional on there being debt to talk about; a clean run must
        // not print a "0 known-debt finding(s)" line under either flag.
        for show in [false, true] {
            let out = render_text(&[non_baseline_finding()], &[], show);
            assert!(out.contains("\n1 finding(s)"), "show={show}: {out}");
            assert!(!out.contains("known-debt"), "show={show}: {out}");
            assert!(!out.contains("Known debt"), "show={show}: {out}");
        }
    }

    #[test]
    fn the_json_envelope_reports_the_suppressed_count_and_withholds_the_findings_by_default() {
        let live = [non_baseline_finding()];
        let debt = [pinned_propagation_finding()];

        let default = serde_json::to_string(&JsonReport::new(&live, &debt, false))
            .expect("the envelope serializes");
        assert!(
            default.contains(r#""known_debt_suppressed":1"#),
            "unexpected: {default}"
        );
        assert!(
            default.contains(r#""known_debt_tracked_as":"D-69""#),
            "unexpected: {default}"
        );
        assert!(
            !default.contains(r#""known_debt":"#),
            "the withheld findings must be absent, not null: {default}"
        );

        let shown = serde_json::to_string(&JsonReport::new(&live, &debt, true))
            .expect("the envelope serializes");
        assert!(
            shown.contains(r#""known_debt":["#),
            "--show-known-debt must include them: {shown}"
        );
    }
}
