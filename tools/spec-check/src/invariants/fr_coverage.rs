use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

use crate::Corpus;
use crate::finding::{Finding, Severity};
use crate::invariants::closure::is_design_slice;

/// Requirement id -> the corpus-relative paths of every slice that claims it (a
/// `BTreeSet` per id since a copy/paste slip can claim the same id twice in one file —
/// see `collect_traces_to`'s doc comment).
type ClaimsByRequirement = BTreeMap<String, BTreeSet<String>>;

/// P2 — every `…-fr-…` id defined in the PRD is claimed by at least one slice's
/// `**Traces to**:` block, and no slice traces to an id the PRD never defines.
///
/// A gear whose slices use *neither* convention P2 recognises (`**Traces to**:` nor
/// `This slice directly addresses:`) is not "fully uncovered" — it is unparsed. Rating
/// and subscriptions both carry a third, still-unrecognised convention (`## 5.
/// Traceability` sections citing short-form ids like `` `fr-overlay-stacking` `` rather
/// than the fully-qualified `cpt-cf-bss-…-fr-…` shape). Emitting a `P2/fr-unclaimed` for
/// every one of that gear's requirements would report confident-sounding gaps P2 has no
/// actual basis for — worse than silence. So: when no document in the corpus used either
/// known convention at all, this emits exactly one `P2/traceability-convention-unknown`
/// instead of the per-id sweep. `P2/fr-dangling` is unaffected either way — a slice
/// claiming (in a convention this *can* read) an id the PRD never defines is exactly as
/// real a defect regardless of how much of the corpus verifies cleanly.
pub fn check(corpus: &Corpus) -> Vec<Finding> {
    let defined = defined_requirements(corpus);
    let (claimed, mut findings, convention_seen) = claimed_requirements(corpus);

    if convention_seen {
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
    } else {
        findings.push(Finding {
            invariant: "P2/traceability-convention-unknown".to_string(),
            severity: Severity::Low,
            file: "PRD.md".to_string(),
            line: None,
            message: format!(
                "P2 cannot verify requirement coverage for {}: no slice uses a recognised \
                 traceability convention (`**Traces to**:` or `This slice directly \
                 addresses:`) — {} requirement(s) went unchecked as a result; per-id claims \
                 are not reported for this gear rather than reporting every requirement as \
                 unclaimed",
                corpus.root().display(),
                defined.len()
            ),
        });
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
///
/// The third return value is whether *either* convention was seen anywhere in the
/// corpus at all — `check` uses it to distinguish "this gear's requirements are
/// verifiably uncovered" from "this gear's convention is one P2 doesn't parse".
///
/// Scoped to `is_design_slice` documents only (shared with `closure.rs`'s identical
/// scoping of the Problem-responses convention): the brief scopes both claim-collection
/// and convention detection to *design slice* files specifically. Without this, a stray
/// `**Traces to**:` in a non-slice document — an ADR quoting the convention as an
/// example, a `DESIGN.md` illustration — would count towards `convention_seen` (wrongly
/// reviving the per-id sweep for a gear whose real slices use neither known convention)
/// and could even wrongly "claim" a requirement no real slice claims. `PRD.md` itself is
/// excluded by construction: it never starts with `design/`, so `is_design_slice` is
/// `false` for it same as before.
fn claimed_requirements(corpus: &Corpus) -> (ClaimsByRequirement, Vec<Finding>, bool) {
    let id = Regex::new(r"`(cpt-cf-[a-z0-9-]*-fr-[a-z0-9-]+)`").expect("valid id regex");
    let mut out: ClaimsByRequirement = BTreeMap::new();
    let mut findings = Vec::new();
    let mut convention_seen = false;

    for (path, text) in corpus.files() {
        if !is_design_slice(path) {
            continue;
        }
        let saw_traces_to = collect_traces_to(path, text, &id, &mut out);
        let saw_directly_addresses = collect_directly_addresses(path, text, &id, &mut out);
        if saw_directly_addresses {
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
        convention_seen = convention_seen || saw_traces_to || saw_directly_addresses;
    }
    (out, findings, convention_seen)
}

/// Ids inside a `**Traces to**:` block, which runs until the first blank line.
/// A `BTreeSet` per id, not a `Vec`: a copy/paste or line-wrap slip can repeat
/// the same id in one block, and that is one dangling defect, not one per
/// repetition — deduplicating here (rather than in the dangling loop) keeps
/// both collectors and their caller honest about "which documents claim this
/// id", with no separate dedup step to forget. Returns whether the marker was
/// seen at all, mirroring `collect_directly_addresses` — `claimed_requirements`
/// needs both collectors' "did I see my marker" signal to tell a gear using
/// neither convention from one using this convention exhaustively (thoroughly
/// claiming everything, hence never triggering a per-id finding either way).
fn collect_traces_to(path: &str, text: &str, id: &Regex, out: &mut ClaimsByRequirement) -> bool {
    let mut in_block = false;
    let mut seen = false;
    for line in text.lines() {
        if line.contains("**Traces to**:") {
            in_block = true;
            seen = true;
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
    seen
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
    out: &mut ClaimsByRequirement,
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

    #[test]
    fn flags_a_gear_using_no_known_traceability_convention_instead_of_per_id_noise() {
        // Mirrors rating's and subscriptions' real shape: the PRD defines requirements,
        // but no slice uses `**Traces to**:` or `This slice directly addresses:`
        // anywhere — instead a third, unparsed convention (`## 5. Traceability`
        // sections citing short-form ids). P2 has no way to tell "claimed" from
        // "unclaimed" for this gear's convention, so it must say so exactly once
        // rather than emit a P2/fr-unclaimed for every requirement it can't verify.
        //
        // `findings.len() == 1` below also proves P2/fr-dangling can't fire alongside
        // convention-unknown for this corpus: nothing can enter `claimed` without one of
        // the two known markers being seen, and seeing either marker is exactly what
        // "convention known" means — so a truly unrecognised-convention corpus has
        // nothing for the (untouched, still-unconditional) fr-dangling loop to iterate.
        let corpus = Corpus::from_parts(
            "gears/bss/gamma/docs",
            [
                (
                    "PRD.md",
                    "- [ ] `p1` - **ID**: `cpt-cf-bss-gamma-fr-one`\n- [ ] `p1` - **ID**: `cpt-cf-bss-gamma-fr-two`\n",
                ),
                (
                    "design/01-a.md",
                    "## 5. Traceability\n\n- **PRD**: §6.3 `fr-one`; §6.4 `fr-two`\n",
                ),
            ],
        );
        let findings = check(&corpus);
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P2/traceability-convention-unknown");
        assert_eq!(findings[0].severity, Severity::Low);
        assert!(
            !findings.iter().any(|f| f.invariant == "P2/fr-unclaimed"),
            "unexpected: {findings:#?}"
        );
        // Suppression must not be silent (task-review Ruling 2 finding): the message
        // must state the cost — how many requirements went unchecked as a result.
        // The fixture's PRD defines exactly 2 (`fr-one`, `fr-two`). Checks for "2
        // requirement" rather than a bare '2' — the invariant tag "P2" itself
        // contains a '2', so a bare-digit check would pass vacuously.
        assert!(
            findings[0].message.contains("2 requirement"),
            "message doesn't state the unchecked count: {:?}",
            findings[0].message
        );
    }

    #[test]
    fn rating_and_subscriptions_report_convention_unknown_not_per_id_noise() {
        // Real-corpus regression for the P2 fix: rating and subscriptions use a third
        // (unparsed) traceability convention — confirmed by no file in either gear
        // containing `**Traces to**:` or `This slice directly addresses:` — so each
        // must report exactly one P2/traceability-convention-unknown and zero
        // P2/fr-unclaimed. Pricing is unaffected: it uses `**Traces to**:` throughout,
        // so it keeps its normal per-id behaviour (see `every_pricing_fr_is_claimed_by_a_slice`).
        //
        // The exact unchecked-count is asserted per gear (task-review Ruling 2 finding:
        // suppression must state its cost) — 43 for rating, 47 for subscriptions,
        // hand-counted from each gear's PRD.md `**ID**:` rows.
        for (gear, unchecked) in [("rating", 43), ("subscriptions", 47)] {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../gears/bss/{gear}/docs"));
            let corpus = Corpus::load(&root).expect("gear corpus loads");
            let findings = check(&corpus);
            let unknown: Vec<_> = findings
                .iter()
                .filter(|f| f.invariant == "P2/traceability-convention-unknown")
                .collect();
            let unclaimed: Vec<_> = findings
                .iter()
                .filter(|f| f.invariant == "P2/fr-unclaimed")
                .collect();
            assert_eq!(unknown.len(), 1, "{gear}: unexpected: {findings:#?}");
            assert!(unclaimed.is_empty(), "{gear}: unexpected: {findings:#?}");
            assert!(
                unknown[0]
                    .message
                    .contains(&format!("{unchecked} requirement")),
                "{gear}: message doesn't state the unchecked count ({unchecked}): {:?}",
                unknown[0].message
            );
        }
    }

    #[test]
    fn a_traces_to_marker_outside_a_design_slice_does_not_count_as_a_known_convention() {
        // Task-review Ruling 2 finding: the brief scopes detection to *design slice*
        // files. A stray `**Traces to**:` in a non-slice document (an ADR quoting the
        // convention as an example, a DESIGN.md example, ...) must not count as this
        // gear "using a known convention" — nor may it silently absorb a claim, or a
        // requirement genuinely unclaimed by any real slice would read as claimed by
        // accident. No design slice here uses either known convention, so the only
        // correct outcome is exactly one convention-unknown finding.
        let corpus = Corpus::from_parts(
            "gears/bss/gamma/docs",
            [
                ("PRD.md", "- [ ] `p1` - **ID**: `cpt-cf-bss-gamma-fr-one`\n"),
                (
                    "ADR/0001-example.md",
                    "Design slices should write **Traces to**: `cpt-cf-bss-gamma-fr-one` \
                     per convention.\n",
                ),
            ],
        );
        let findings = check(&corpus);
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P2/traceability-convention-unknown");
        assert!(findings[0].message.contains("1 requirement"));
    }
}
