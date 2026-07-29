use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

use crate::Corpus;
use crate::finding::{Finding, Severity};

/// P2 — every `…-fr-…` id defined in the PRD is claimed by at least one slice's
/// `**Traces to**:` block, and no slice traces to an id the PRD never defines.
pub fn check(corpus: &Corpus) -> Vec<Finding> {
    let defined = defined_requirements(corpus);
    let (claimed, mut findings) = claimed_requirements(corpus);

    for id in &defined {
        if !claimed.contains_key(id) {
            findings.push(Finding {
                invariant: "P2/fr-unclaimed".to_string(),
                severity: Severity::Low,
                file: "PRD.md".to_string(),
                line: None,
                message: format!("{id} is claimed by no slice's Traces-to line"),
            });
        }
    }

    for (id, files) in &claimed {
        if !defined.contains(id) {
            for f in files {
                findings.push(Finding {
                    invariant: "P2/fr-dangling".to_string(),
                    severity: Severity::Medium,
                    file: f.clone(),
                    line: None,
                    message: format!("Traces to {id}, which the PRD does not define"),
                });
            }
        }
    }

    findings
}

fn defined_requirements(corpus: &Corpus) -> BTreeSet<String> {
    let id =
        Regex::new(r"\*\*ID\*\*:\s*`(cpt-cf-[a-z0-9-]*-fr-[a-z0-9-]+)`").expect("valid id regex");
    corpus
        .text("PRD.md")
        .map(|t| id.captures_iter(t).map(|c| c[1].to_string()).collect())
        .unwrap_or_default()
}

/// Collects ids claimed by slice documents, recognising the two conventions the
/// design set actually uses. Eleven of the twelve pricing slices use
/// `**Traces to**:`; the twelfth (`design/01-foundation.md`) predates that
/// convention and instead lists requirements under a `This slice directly
/// addresses:` heading. Both are equally valid ownership claims — the P2
/// question is "is this requirement claimed by a slice", not "claimed in a
/// particular markdown shape" — so both are read for claims. A document using
/// the second shape still gets a `P2/traceability-convention-divergent`
/// finding of its own: the checker must not silently absorb a shape split from
/// the rest of the set.
fn claimed_requirements(corpus: &Corpus) -> (BTreeMap<String, BTreeSet<String>>, Vec<Finding>) {
    let id = Regex::new(r"`(cpt-cf-[a-z0-9-]*-fr-[a-z0-9-]+)`").expect("valid id regex");
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut findings = Vec::new();

    for (path, text) in corpus.files() {
        if path == "PRD.md" {
            continue;
        }
        collect_traces_to(path, text, &id, &mut out);
        if collect_directly_addresses(path, text, &id, &mut out) {
            findings.push(Finding {
                invariant: "P2/traceability-convention-divergent".to_string(),
                severity: Severity::Low,
                file: path.to_string(),
                line: None,
                message: format!(
                    "{path} claims requirements via `This slice directly addresses:`; \
                     the rest of the design set uses `**Traces to**:`"
                ),
            });
        }
    }
    (out, findings)
}

/// Ids inside a `**Traces to**:` block, which runs until the first blank line.
/// A `BTreeSet` per id, not a `Vec`: a copy/paste or line-wrap slip can repeat
/// the same id in one block, and that is one dangling defect, not one per
/// repetition — deduplicating here (rather than in the dangling loop) keeps
/// both collectors and their caller honest about "which documents claim this
/// id", with no separate dedup step to forget.
fn collect_traces_to(
    path: &str,
    text: &str,
    id: &Regex,
    out: &mut BTreeMap<String, BTreeSet<String>>,
) {
    let mut in_block = false;
    for line in text.lines() {
        if line.contains("**Traces to**:") {
            in_block = true;
        } else if in_block && line.trim().is_empty() {
            in_block = false;
        }
        if in_block {
            for c in id.captures_iter(line) {
                out.entry(c[1].to_string())
                    .or_default()
                    .insert(path.to_string());
            }
        }
    }
}

/// Ids inside a `This slice directly addresses:` block. Unlike `**Traces
/// to**:`, the marker is followed by a blank line before the bullets start
/// (`design/01-foundation.md`'s only occurrence: marker, blank line, six
/// bullets, then end of file, no trailing heading) — so "stop at the first
/// blank line" would collect nothing here. Instead: skip blank lines after the
/// marker, collect ids from bullet lines (optional leading whitespace then
/// `-`), and stop at the next heading (`^#`) or the first non-blank,
/// non-bullet line — falling off the end of the document closes the block
/// too. Returns whether the marker was seen at all, so the caller can raise
/// the convention-divergence finding.
fn collect_directly_addresses(
    path: &str,
    text: &str,
    id: &Regex,
    out: &mut BTreeMap<String, BTreeSet<String>>,
) -> bool {
    const MARKER: &str = "This slice directly addresses:";
    let mut in_block = false;
    let mut seen = false;
    for line in text.lines() {
        if line.contains(MARKER) {
            in_block = true;
            seen = true;
            continue;
        }
        if !in_block {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue; // blank line between the marker and the bullets: stay open
        }
        if line.starts_with('#') || !trimmed.starts_with('-') {
            in_block = false;
            continue;
        }
        for c in id.captures_iter(line) {
            out.entry(c[1].to_string())
                .or_default()
                .insert(path.to_string());
        }
    }
    seen
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
    fn every_pricing_fr_is_claimed_by_a_slice() {
        let findings = check(&pricing());
        let orphans: Vec<_> = findings
            .iter()
            .filter(|f| f.invariant == "P2/fr-unclaimed")
            .collect();
        assert!(orphans.is_empty(), "unexpected: {orphans:#?}");
    }

    #[test]
    fn flags_an_fr_no_slice_traces_to() {
        let corpus = Corpus::from_parts(
            "synthetic",
            [
                (
                    "PRD.md",
                    "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-lonely`\n- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-other`\n",
                ),
                ("design/01-a.md", "**Traces to**: `cpt-cf-bss-x-fr-other`\n"),
            ],
        );
        let findings = check(&corpus);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].invariant, "P2/fr-unclaimed");
        assert!(findings[0].message.contains("cpt-cf-bss-x-fr-lonely"));
    }

    #[test]
    fn reads_traces_to_lines_that_wrap() {
        let corpus = Corpus::from_parts(
            "synthetic",
            [
                (
                    "PRD.md",
                    "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-first`\n- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-wrapped`\n",
                ),
                (
                    "design/01-a.md",
                    "**Traces to**: `cpt-cf-bss-x-fr-first`,\n`cpt-cf-bss-x-fr-wrapped`\n\nNext paragraph.\n",
                ),
            ],
        );
        assert!(check(&corpus).is_empty());
    }

    #[test]
    fn recognises_this_slice_directly_addresses_as_an_alternate_claim_convention() {
        // Mirrors design/01-foundation.md's exact shape: marker line, then a
        // *blank* line, then bullets, then end of file with no trailing
        // heading. A fixture whose bullets started immediately after the
        // marker would pass even with a wrong "stop at the first blank line"
        // rule (the `**Traces to**:` rule) — the blank line here is what
        // proves the stop condition is right, since that rule would collect
        // nothing at all from this shape.
        //
        // Two ids, not one: this is what makes "exactly one divergence
        // finding" a real assertion about the finding's cardinality (raised
        // once per document) rather than one that would also pass a latent
        // bug where the push lived inside the per-id capture loop instead of
        // after it (which would emit one divergence finding per id here).
        let corpus = Corpus::from_parts(
            "synthetic",
            [
                (
                    "PRD.md",
                    "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-foundational`\n- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-alsofoundational`\n",
                ),
                (
                    "design/01-a.md",
                    "This slice directly addresses:\n\n- `cpt-cf-bss-x-fr-foundational` / `cpt-cf-bss-x-fr-alsofoundational` — does the thing\n",
                ),
            ],
        );
        let findings = check(&corpus);
        assert!(
            !findings.iter().any(|f| f.invariant == "P2/fr-unclaimed"),
            "requirements should be treated as claimed: {findings:#?}"
        );
        let divergences: Vec<_> = findings
            .iter()
            .filter(|f| f.invariant == "P2/traceability-convention-divergent")
            .collect();
        assert_eq!(divergences.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(divergences[0].severity, Severity::Low);
        assert_eq!(divergences[0].file, "design/01-a.md");
    }

    #[test]
    fn flags_a_slice_tracing_to_a_requirement_that_does_not_exist() {
        let corpus = Corpus::from_parts(
            "synthetic",
            [
                ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-x-fr-real`\n"),
                (
                    "design/01-a.md",
                    "**Traces to**: `cpt-cf-bss-x-fr-real`, `cpt-cf-bss-x-fr-ghost`\n",
                ),
            ],
        );
        let findings = check(&corpus);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].invariant, "P2/fr-dangling");
        assert_eq!(findings[0].file, "design/01-a.md");
    }

    #[test]
    fn deduplicates_a_dangling_id_repeated_in_one_document() {
        // A copy/paste or line-wrap slip can list the same undefined id twice in
        // one Traces-to block. That is one defect, not two — repeated mentions of
        // the same (file, id) pair must not inflate the finding count.
        let corpus = Corpus::from_parts(
            "synthetic",
            [
                ("PRD.md", ""),
                (
                    "design/01-a.md",
                    "**Traces to**: `cpt-cf-bss-x-fr-ghost`, `cpt-cf-bss-x-fr-ghost`\n",
                ),
            ],
        );
        let findings = check(&corpus);
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P2/fr-dangling");
        assert_eq!(findings[0].file, "design/01-a.md");
    }
}
