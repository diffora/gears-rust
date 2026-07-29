use regex::Regex;

use crate::decisions::Decision;
use crate::finding::{Finding, Severity};
use crate::{Corpus, decisions, targets};

/// P1 — for every decision that records a propagation surface, each named target
/// document must cite the decision id. `seams` is the cross-gear seam-ownership index
/// (see `targets::SeamIndex`) built from every corpus the CLI loaded, so a `SEAMS <id>`
/// propagation target can be checked against who actually owns `id` instead of a
/// prefix guess.
pub fn check(corpus: &Corpus, seams: &targets::SeamIndex) -> Vec<Finding> {
    let Some(register) = corpus.text("DECISIONS.md") else {
        return Vec::new();
    };

    let lines: Vec<&str> = register.lines().collect();
    let all = decisions::parse(register);

    let mut findings = Vec::new();
    for (i, d) in all.iter().enumerate() {
        let Some(raw) = d.propagated.as_deref() else {
            // `propagated: None` conflates two shapes: legitimately nothing to
            // propagate (no `**Propagated`-prefixed label at all — e.g. a decision
            // resolved by reference to another one) and a genuine propagation
            // surface recorded under a label this parser's exact `**Propagated**:`
            // anchor does not recognize (e.g. the live register's D-42:
            // `**Propagated (normative, 2026-07-28)**:`). Only the second is a
            // defect worth surfacing — widening the anchor to swallow it would
            // hide the very gap this check exists to find.
            if let Some(label) = unparsed_propagated_label(&lines, &all, i) {
                findings.push(Finding {
                    invariant: "P1/propagation-label-unparsed".to_string(),
                    severity: Severity::Medium,
                    file: "DECISIONS.md".to_string(),
                    line: Some(d.line),
                    message: format!(
                        "{} carries a propagation label this parser could not read: `{label}`",
                        d.id
                    ),
                });
            }
            continue;
        };
        let resolved = targets::resolve(raw, corpus, seams);

        for token in &resolved.unresolved {
            findings.push(Finding {
                invariant: "P1/propagation-unresolvable".to_string(),
                severity: Severity::Low,
                file: "DECISIONS.md".to_string(),
                line: Some(d.line),
                message: format!(
                    "{}: propagation target `{token}` names no document the resolver can map",
                    d.id
                ),
            });
        }

        for id in &resolved.seam_undefined {
            findings.push(Finding {
                invariant: "P1/seam-undefined".to_string(),
                severity: Severity::Medium,
                file: "DECISIONS.md".to_string(),
                line: Some(d.line),
                message: format!(
                    "{}: propagation target `SEAMS {id}` cites a seam id that no loaded \
                     gear's SEAMS.md defines",
                    d.id
                ),
            });
        }

        for (id, owners) in &resolved.seam_conflicts {
            findings.push(Finding {
                invariant: "P1/seam-conflict".to_string(),
                severity: Severity::Medium,
                file: "DECISIONS.md".to_string(),
                line: Some(d.line),
                message: format!(
                    "{}: propagation target `SEAMS {id}` is defined in more than one \
                     loaded gear's SEAMS.md: {}",
                    d.id,
                    owners.join(", ")
                ),
            });
        }

        let cite = Regex::new(&format!(r"\b{}\b", regex::escape(&d.id)))
            .expect("decision ids are regex-safe");
        for path in &resolved.paths {
            // Cross-gear targets (`../../rating/docs/SEAMS.md`) are outside this
            // corpus. Verifying them needs the cross-gear join — migration step 4 of
            // the design spec, not step 1 — so they are skipped, not reported.
            let Some(text) = corpus.text(path) else {
                continue;
            };
            if !cite.is_match(text) {
                findings.push(Finding {
                    invariant: "P1/propagation-missing".to_string(),
                    severity: Severity::Medium,
                    file: "DECISIONS.md".to_string(),
                    line: Some(d.line),
                    message: format!(
                        "{} claims propagation into {path}, but that document never cites {}",
                        d.id, d.id
                    ),
                });
            }
        }
    }
    findings
}

/// Body text of decision `all[i]` (its heading line through the line before the
/// next entry's heading, or through EOF for the last entry) still contains a
/// `**Propagated…**:`-shaped bold field label that `decisions::parse`'s exact
/// `**Propagated**:` anchor did not capture. Returns the literal label text
/// (asterisks and trailing colon included) when one is found, so the finding
/// can name it verbatim.
fn unparsed_propagated_label(lines: &[&str], all: &[Decision], i: usize) -> Option<String> {
    let start = all[i].line - 1;
    let end = all
        .get(i + 1)
        .map(|next| next.line - 1)
        .unwrap_or(lines.len());
    let body = lines[start..end].join("\n");
    let label = Regex::new(r"\*\*Propagated[^*]*\*\*:").expect("valid label regex");
    label.find(&body).map(|m| m.as_str().to_string())
}

/// Pinned baseline of `P1/propagation-missing` findings against the live
/// pricing register, hand-derived from the failure output of this test on
/// 2026-07-29 (not by running the checker and trusting whatever it
/// produces — a self-derived baseline asserts nothing). These 24
/// `(gear, decision id, target path)` triples are **debt, not correctness**:
/// pre-existing gaps left by the 2026-07-10 decision wave, confirmed real
/// by manual cross-check (PRD.md cites 34 *other* decision ids, so the
/// citation convention is genuine and broadly followed — these 24 are
/// what that wave skipped, not resolver noise). Two confirmed shapes: D-01's
/// substance is present in the PRD and only the citation is missing; D-15's
/// `PHASE_UNCOVERED` never appears there at all, a real substance gap.
/// Fixing them is a separate docs round (tracked as **D-69**), not this
/// task loop's job. Pinned as an exact set so a *new* gap fails this test
/// immediately, and so a *fixed* gap fails it too — the list must be
/// updated deliberately when the docs improve, never left to quietly
/// become a floor. Promoted to a `pub const` (2026-07-29, fix round 1) so
/// the CLI has exactly the same one definition of this debt the tests pin
/// against, rather than a second, test-only copy the CLI can't see.
///
/// Every entry names `"pricing"` (task-review Ruling 3 fix, 2026-07-29, fix round 3):
/// this baseline is a snapshot of *one specific corpus*, and `(id, path)` alone is not a
/// unique key across gears — rating and subscriptions have their own `DECISIONS.md` with
/// their own `D-NN` ids, and `PRD.md`/`design/03-price-structure.md` are common
/// filenames a sibling gear could equally have. Without the gear qualifier, a
/// same-shaped finding from a different gear would be silently swallowed as if it were
/// this pinned pricing debt. The gear name here is baseline *data*, describing which
/// corpus this specific snapshot was taken from — it must never leak into `targets.rs`'s
/// resolution path or any invariant's matching logic.
pub const PINNED_PROPAGATION_GAPS_2026_07_29: &[(&str, &str, &str)] = &[
    ("pricing", "D-01", "PRD.md"),
    (
        "pricing",
        "D-02",
        "ADR/0001-cpt-cf-bss-pricing-adr-canonical-scope-key.md",
    ),
    ("pricing", "D-02", "DESIGN.md"),
    ("pricing", "D-02", "PRD.md"),
    ("pricing", "D-02", "design/01-foundation.md"),
    ("pricing", "D-02", "design/07-pricewindow-linkage.md"),
    ("pricing", "D-04", "PRD.md"),
    ("pricing", "D-05", "PRD.md"),
    ("pricing", "D-06", "PRD.md"),
    ("pricing", "D-07", "PRD.md"),
    ("pricing", "D-13", "PRD.md"),
    ("pricing", "D-15", "PRD.md"),
    ("pricing", "D-16", "PRD.md"),
    ("pricing", "D-19", "PRD.md"),
    ("pricing", "D-20", "PRD.md"),
    ("pricing", "D-24", "PRD.md"),
    ("pricing", "D-25", "PRD.md"),
    ("pricing", "D-28", "PRD.md"),
    ("pricing", "D-32", "PRD.md"),
    ("pricing", "D-35", "PRD.md"),
    ("pricing", "D-39", "PRD.md"),
    ("pricing", "D-40", "design/10-advanced-primitives.md"),
    ("pricing", "D-41", "DESIGN.md"),
    ("pricing", "D-60", "design/03-price-structure.md"),
];

/// Parses a `P1/propagation-missing` finding's `(decision id, target path)` pair from
/// `check`'s own fixed message template. `None` for any other invariant tag or a
/// message that doesn't match the expected shape — the single production-and-test
/// definition of "how to read this finding back into pinned-baseline shape" (promoted
/// 2026-07-29, fix round 1, replacing what was a test-only `missing_pairs` helper).
///
/// Deliberately does not, and cannot, recover a gear from the `Finding` alone — a
/// `Finding` (see `finding.rs`) carries only a corpus-relative path, never a gear
/// qualifier. Callers that need to match against a gear-qualified baseline (see
/// `is_pinned_baseline`) must supply the gear themselves, from the corpus context they
/// still have at the point the finding was produced.
pub fn missing_pair(finding: &Finding) -> Option<(String, String)> {
    if finding.invariant != "P1/propagation-missing" {
        return None;
    }
    let shape =
        Regex::new(r"^(D-\d+) claims propagation into (.+), but that document never cites D-\d+$")
            .expect("valid message-shape regex");
    shape
        .captures(&finding.message)
        .map(|c| (c[1].to_string(), c[2].to_string()))
}

/// True if `finding`, attributed to `gear`, is exactly one of the pinned, accepted-debt
/// propagation gaps (tracked as D-69) rather than newly appeared drift. `gear` is not
/// read from `finding` (it can't be — see `missing_pair`) but must be supplied by the
/// caller from the corpus the finding was actually produced against (task-review Ruling
/// 3 fix, 2026-07-29, fix round 3): a same-`(id, path)` finding attributed to any other
/// gear must not match.
pub fn is_pinned_baseline(finding: &Finding, gear: &str) -> bool {
    missing_pair(finding).is_some_and(|(id, path)| {
        PINNED_PROPAGATION_GAPS_2026_07_29
            .iter()
            .any(|(pgear, pid, ppath)| *pgear == gear && *pid == id && *ppath == path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Corpus;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn pricing() -> Corpus {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../gears/bss/pricing/docs");
        Corpus::load(&root).expect("pricing corpus loads")
    }

    /// The seam index a real CLI run builds: every live BSS gear's `SEAMS.md`, loaded
    /// once. Used so this module's `check` calls see exactly what a real run would —
    /// cross-gear `SEAMS <id>` citations resolve for real instead of against an empty
    /// index (which would misreport every one of them as owned by no loaded gear).
    fn known_seams() -> targets::SeamIndex {
        let load = |gear: &str| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../gears/bss/{gear}/docs"));
            Corpus::load(&root).expect("gear corpus loads")
        };
        targets::SeamIndex::build(&[load("pricing"), load("rating"), load("subscriptions")])
    }

    #[test]
    fn propagation_gaps_match_the_pinned_2026_07_29_baseline() {
        // NOT a green invariant, deliberately: see PINNED_PROPAGATION_GAPS_2026_07_29's
        // doc comment (a module-level `pub const` now, brought into scope here by `use
        // super::*` above). This test exists to make debt visible and stable, not to
        // assert the register is clean — it currently is not, and pretending otherwise
        // (by asserting emptiness, as this test previously did) hides exactly the kind
        // of gap P1 exists to catch.
        let actual: BTreeSet<(String, String)> = check(&pricing(), &known_seams())
            .iter()
            .filter_map(missing_pair)
            .collect();
        // Raw `check()` output is only ever compared against this one corpus's own
        // pinned entries — the drift test runs pricing alone, so the (id, path)
        // projection (dropping the gear element) is the correct comparison here. Every
        // entry in the pin is `"pricing"` by construction (see the const's doc comment);
        // asserted below so a future entry added for a different gear would fail loudly
        // here rather than silently changing what this test actually checks.
        assert!(
            PINNED_PROPAGATION_GAPS_2026_07_29
                .iter()
                .all(|(gear, _, _)| *gear == "pricing"),
            "this baseline is documented as a pricing-only snapshot; a non-pricing entry \
             would invalidate this test's (id, path)-only comparison"
        );
        let expected: BTreeSet<(String, String)> = PINNED_PROPAGATION_GAPS_2026_07_29
            .iter()
            .map(|(_, id, path)| (id.to_string(), path.to_string()))
            .collect();

        let appeared: Vec<_> = actual.difference(&expected).collect();
        let disappeared: Vec<_> = expected.difference(&actual).collect();
        assert!(
            appeared.is_empty() && disappeared.is_empty(),
            "propagation-gap baseline drifted from the pinned 2026-07-29 set — \
             newly appeared (not in the pin): {appeared:#?}; \
             no longer reproduced (pin needs updating — did someone fix these?): {disappeared:#?}"
        );
    }

    #[test]
    fn is_pinned_baseline_matches_only_the_recorded_gear() {
        // Task-review Ruling 3 finding (CRITICAL): a finding whose (id, path) matches a
        // pinned pricing entry byte-for-byte must not be treated as known debt when it
        // is attributed to a different gear — the baseline is a snapshot of one specific
        // corpus, and neither `D-NN` nor the target path is unique across gears (rating
        // and subscriptions have their own DECISIONS.md with their own D-NN ids).
        let (gear, id, path) = PINNED_PROPAGATION_GAPS_2026_07_29[0];
        assert_eq!(gear, "pricing", "test assumes entry 0 is pricing's");
        let finding = Finding {
            invariant: "P1/propagation-missing".to_string(),
            severity: Severity::Medium,
            file: "DECISIONS.md".to_string(),
            line: Some(1),
            message: format!(
                "{id} claims propagation into {path}, but that document never cites {id}"
            ),
        };
        assert!(is_pinned_baseline(&finding, "pricing"));
        assert!(!is_pinned_baseline(&finding, "rating"));
        assert!(!is_pinned_baseline(&finding, "subscriptions"));
    }

    #[test]
    fn flags_a_target_that_does_not_cite_the_decision() {
        // D-99 claims it propagated into the PRD; the PRD never mentions D-99.
        let corpus = Corpus::from_parts(
            "synthetic",
            [
                (
                    "DECISIONS.md",
                    "#### D-99 [H] Invented\n\n- **Propagated**: PRD §1.\n",
                ),
                ("PRD.md", "Some requirement text with no citation.\n"),
            ],
        );
        let findings = check(&corpus, &targets::SeamIndex::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].invariant, "P1/propagation-missing");
        assert_eq!(findings[0].file, "DECISIONS.md");
        assert!(findings[0].message.contains("D-99"));
        assert!(findings[0].message.contains("PRD.md"));
    }

    #[test]
    fn reports_unresolvable_targets_separately_and_at_low_severity() {
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "DECISIONS.md",
                "#### D-98 [L] Vague\n\n- **Propagated**: SEAMS.\n",
            )],
        );
        let findings = check(&corpus, &targets::SeamIndex::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].invariant, "P1/propagation-unresolvable");
        assert_eq!(findings[0].severity, Severity::Low);
    }

    #[test]
    fn flags_a_propagated_label_shape_the_widened_parser_still_cannot_read() {
        // decisions::parse's anchor now reads a *parenthetical* qualifier
        // (`**Propagated (normative, 2026-07-28)**:`, D-42's shape until it was
        // normalised — see `resolves_a_propagated_label_with_a_parenthetical_qualifier`
        // below). A qualifier written any other way must still come back `None`,
        // so `unparsed_propagated_label`'s fallback stays reachable: per the
        // plan's Global Constraints an unresolvable propagation target must be a
        // Finding, never a silent skip, and this shape was never in scope for the
        // widening.
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "DECISIONS.md",
                "#### D-97 [M] Something\n\n- **Propagated pending**: PRD §1.\n",
            )],
        );
        let findings = check(&corpus, &targets::SeamIndex::default());
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/propagation-label-unparsed");
        assert_eq!(findings[0].severity, Severity::Medium);
        assert_eq!(findings[0].file, "DECISIONS.md");
        assert!(findings[0].message.contains("D-97"));
        assert!(findings[0].message.contains("Propagated pending"));
    }

    #[test]
    fn resolves_a_propagated_label_with_a_parenthetical_qualifier() {
        // D-42 wrote exactly this shape (until a doc edit outside this task
        // normalised it to the plain form — see the task report). The widened
        // anchor must resolve a qualified label exactly as it would the plain
        // `**Propagated**:` form: no `P1/propagation-label-unparsed`, ordinary
        // citation checking against the named target, same as
        // `flags_a_target_that_does_not_cite_the_decision` above but through the
        // qualified label.
        let corpus = Corpus::from_parts(
            "synthetic",
            [
                (
                    "DECISIONS.md",
                    "#### D-93 [M] Something\n\n- **Propagated (normative, 2026-07-28)**: PRD §1.\n",
                ),
                ("PRD.md", "Some requirement text with no citation.\n"),
            ],
        );
        let findings = check(&corpus, &targets::SeamIndex::default());
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/propagation-missing");
        assert_eq!(findings[0].file, "DECISIONS.md");
        assert!(findings[0].message.contains("D-93"));
        assert!(findings[0].message.contains("PRD.md"));
    }

    #[test]
    fn silent_when_the_entry_has_no_propagated_label_at_all() {
        // D-26's shape: legitimately nothing to propagate (resolved by reference
        // to another decision) — no `**Propagated`-prefixed label of any shape in
        // the body. Must stay silent, or every ordinarily-quiet entry in the real
        // register would start spuriously firing this finding.
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "DECISIONS.md",
                "#### D-96 [M] Resolved elsewhere\n\n- **Decision**: RESOLVED by D-03.\n",
            )],
        );
        let findings = check(&corpus, &targets::SeamIndex::default());
        assert!(findings.is_empty(), "unexpected: {findings:#?}");
    }

    #[test]
    fn flags_a_seam_citation_whose_id_no_loaded_gear_defines() {
        // "Z9" is not a row any loaded corpus's SEAMS.md defines — a dangling seam
        // reference, and a real defect distinct from an unresolvable-shorthand miss.
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "DECISIONS.md",
                "#### D-95 [M] Dangling seam reference\n\n- **Propagated**: SEAMS Z9 note.\n",
            )],
        );
        let findings = check(&corpus, &targets::SeamIndex::default());
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/seam-undefined");
        assert_eq!(findings[0].severity, Severity::Medium);
        assert_eq!(findings[0].file, "DECISIONS.md");
        assert!(findings[0].message.contains("D-95"));
        assert!(findings[0].message.contains("Z9"));
    }

    #[test]
    fn flags_a_seam_citation_whose_id_two_loaded_gears_both_define() {
        // No two real gears currently claim the same seam id (verified against the
        // live corpus), so this constructs the conflict directly: two synthetic
        // corpora, each defining a `Z1` row, both loaded alongside the citing corpus.
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "DECISIONS.md",
                "#### D-94 [M] Conflicting seam ownership\n\n- **Propagated**: SEAMS Z1 note.\n",
            )],
        );
        let alpha = Corpus::from_parts(
            "gears/bss/alpha/docs",
            [(
                "SEAMS.md",
                "| # | Sev | Verdict | Seam |\n|---|-----|---------|------|\n\
                 | **Z1** | HIGH | Joint | Alpha's definition. |\n",
            )],
        );
        let beta = Corpus::from_parts(
            "gears/bss/beta/docs",
            [(
                "SEAMS.md",
                "| # | Sev | Verdict | Seam |\n|---|-----|---------|------|\n\
                 | **Z1** | HIGH | Joint | Beta's conflicting definition. |\n",
            )],
        );
        let seams = targets::SeamIndex::build(&[alpha, beta]);

        let findings = check(&corpus, &seams);
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/seam-conflict");
        assert_eq!(findings[0].severity, Severity::Medium);
        assert!(findings[0].message.contains("D-94"));
        assert!(findings[0].message.contains("Z1"));
        assert!(findings[0].message.contains("alpha"));
        assert!(findings[0].message.contains("beta"));
    }
}
