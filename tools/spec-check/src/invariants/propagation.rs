use regex::Regex;

use crate::decisions::Decision;
use crate::finding::{Finding, Severity};
use crate::{Corpus, decisions, targets};

/// P1 — for every decision that records a propagation surface, each named target
/// document must cite the decision id.
pub fn check(corpus: &Corpus) -> Vec<Finding> {
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
        let resolved = targets::resolve(raw, corpus);

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

    /// Pinned baseline of `P1/propagation-missing` findings against the live
    /// pricing register, hand-derived from the failure output of this test on
    /// 2026-07-29 (not by running the checker and trusting whatever it
    /// produces — a self-derived baseline asserts nothing). These 24
    /// `(decision id, target path)` pairs are **debt, not correctness**:
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
    /// become a floor.
    const PINNED_PROPAGATION_GAPS_2026_07_29: &[(&str, &str)] = &[
        ("D-01", "PRD.md"),
        (
            "D-02",
            "ADR/0001-cpt-cf-bss-pricing-adr-canonical-scope-key.md",
        ),
        ("D-02", "DESIGN.md"),
        ("D-02", "PRD.md"),
        ("D-02", "design/01-foundation.md"),
        ("D-02", "design/07-pricewindow-linkage.md"),
        ("D-04", "PRD.md"),
        ("D-05", "PRD.md"),
        ("D-06", "PRD.md"),
        ("D-07", "PRD.md"),
        ("D-13", "PRD.md"),
        ("D-15", "PRD.md"),
        ("D-16", "PRD.md"),
        ("D-19", "PRD.md"),
        ("D-20", "PRD.md"),
        ("D-24", "PRD.md"),
        ("D-25", "PRD.md"),
        ("D-28", "PRD.md"),
        ("D-32", "PRD.md"),
        ("D-35", "PRD.md"),
        ("D-39", "PRD.md"),
        ("D-40", "design/10-advanced-primitives.md"),
        ("D-41", "DESIGN.md"),
        ("D-60", "design/03-price-structure.md"),
    ];

    /// Parses `P1/propagation-missing` findings back into `(decision id, target
    /// path)` pairs by their fixed message template (`check`'s own
    /// `format!("{} claims propagation into {path}, but that document never
    /// cites {}", ...)`). Test-only: production `Finding` has no separate path
    /// field, and this round changes test expression, not that shape.
    fn missing_pairs(findings: &[Finding]) -> Vec<(String, String)> {
        let shape = Regex::new(
            r"^(D-\d+) claims propagation into (.+), but that document never cites D-\d+$",
        )
        .expect("valid message-shape regex");
        findings
            .iter()
            .filter(|f| f.invariant == "P1/propagation-missing")
            .map(|f| {
                let caps = shape.captures(&f.message).unwrap_or_else(|| {
                    panic!(
                        "P1/propagation-missing message doesn't match the expected shape: {}",
                        f.message
                    )
                });
                (caps[1].to_string(), caps[2].to_string())
            })
            .collect()
    }

    #[test]
    fn propagation_gaps_match_the_pinned_2026_07_29_baseline() {
        // NOT a green invariant, deliberately: see PINNED_PROPAGATION_GAPS_2026_07_29's
        // doc comment. This test exists to make debt visible and stable, not to
        // assert the register is clean — it currently is not, and pretending
        // otherwise (by asserting emptiness, as this test previously did) hides
        // exactly the kind of gap P1 exists to catch.
        let actual: BTreeSet<(String, String)> =
            missing_pairs(&check(&pricing())).into_iter().collect();
        let expected: BTreeSet<(String, String)> = PINNED_PROPAGATION_GAPS_2026_07_29
            .iter()
            .map(|(id, path)| (id.to_string(), path.to_string()))
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
        let findings = check(&corpus);
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
        let findings = check(&corpus);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].invariant, "P1/propagation-unresolvable");
        assert_eq!(findings[0].severity, Severity::Low);
    }

    #[test]
    fn flags_a_propagated_label_variant_the_parser_cannot_read() {
        // Mirrors the live register's D-42: a genuine propagation surface recorded
        // under `**Propagated (normative, 2026-07-28)**:` — a bold label that
        // decisions::parse's exact `**Propagated**:` anchor does not capture, so
        // `d.propagated` comes back `None` exactly as it would for a decision with
        // nothing to propagate. The two must not be conflated: this one names a
        // real, unread propagation surface.
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "DECISIONS.md",
                "#### D-97 [M] Something\n\n- **Propagated (normative, 2026-07-28)**: PRD §1.\n",
            )],
        );
        let findings = check(&corpus);
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/propagation-label-unparsed");
        assert_eq!(findings[0].severity, Severity::Medium);
        assert_eq!(findings[0].file, "DECISIONS.md");
        assert!(findings[0].message.contains("D-97"));
        assert!(
            findings[0]
                .message
                .contains("Propagated (normative, 2026-07-28)")
        );
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
        let findings = check(&corpus);
        assert!(findings.is_empty(), "unexpected: {findings:#?}");
    }
}
