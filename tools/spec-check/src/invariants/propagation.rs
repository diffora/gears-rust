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
    use std::path::PathBuf;

    fn pricing() -> Corpus {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../gears/bss/pricing/docs");
        Corpus::load(&root).expect("pricing corpus loads")
    }

    #[test]
    fn current_pricing_register_is_fully_propagated() {
        let findings = check(&pricing());
        let missing: Vec<_> = findings
            .iter()
            .filter(|f| f.invariant == "P1/propagation-missing")
            .collect();
        assert!(missing.is_empty(), "unexpected: {missing:#?}");
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
