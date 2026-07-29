use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

use crate::Corpus;
use crate::finding::{Finding, Severity};

/// P3 — declared-and-referenced closure for instruction ids and error codes.
pub fn check(corpus: &Corpus) -> Vec<Finding> {
    let mut findings = Vec::new();

    let declared_inst = Regex::new(r"- `(inst-[a-z0-9-]+)`\s*$").expect("valid decl regex");
    let any_inst = Regex::new(r"`(inst-[a-z0-9-]+)`").expect("valid ref regex");

    let mut declared: BTreeSet<String> = BTreeSet::new();
    let mut referenced: BTreeMap<String, String> = BTreeMap::new();

    for (path, text) in corpus.files() {
        for line in text.lines() {
            if let Some(c) = declared_inst.captures(line.trim_end()) {
                declared.insert(c[1].to_string());
            }
            for c in any_inst.captures_iter(line) {
                referenced
                    .entry(c[1].to_string())
                    .or_insert_with(|| path.to_string());
            }
        }
    }

    for (id, path) in &referenced {
        if !declared.contains(id) {
            findings.push(Finding {
                invariant: "P3/inst-dangling".to_string(),
                severity: Severity::Medium,
                file: path.clone(),
                line: None,
                message: format!("`{id}` is referenced but declared by no instruction line"),
            });
        }
    }

    findings.extend(check_error_codes(corpus));
    findings
}

/// Error codes declared inside a `**Problem responses (RFC 9457):**` block (which runs
/// until the first blank line) and never mentioned again anywhere else in the corpus, plus
/// documents that declare codes without ever opening such a block at all. The latter mirrors
/// `fr_coverage.rs`'s `collect_directly_addresses`: `design/01-foundation.md` names its
/// Foundation-owned codes in prose rather than a Problem-responses block, so without this
/// second check those codes — and any future document doing the same — would be invisible
/// to the "declared" side of the closure rule, silently narrowing what P3 covers rather than
/// surfacing the gap. One finding per document, not per code, since the defect is "this
/// document uses a different convention," not "this code is unreachable."
fn check_error_codes(corpus: &Corpus) -> Vec<Finding> {
    let code = Regex::new(r"`([A-Z][A-Z0-9_]{4,})`").expect("valid code regex");
    let mut declared: BTreeMap<String, String> = BTreeMap::new();
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    let mut findings = Vec::new();

    for (path, text) in corpus.files() {
        let mut in_block = false;
        let mut saw_block = false;
        let mut saw_code = false;
        for line in text.lines() {
            if line.contains("**Problem responses (RFC 9457):**") {
                in_block = true;
                saw_block = true;
            } else if in_block && line.trim().is_empty() {
                in_block = false;
            }
            for c in code.captures_iter(line) {
                saw_code = true;
                if in_block {
                    declared
                        .entry(c[1].to_string())
                        .or_insert_with(|| path.to_string());
                } else {
                    referenced.insert(c[1].to_string());
                }
            }
        }
        if saw_code && !saw_block {
            findings.push(Finding {
                invariant: "P3/code-convention-divergent".to_string(),
                severity: Severity::Low,
                file: path.to_string(),
                line: None,
                message: format!(
                    "{path} declares error codes without a `**Problem responses (RFC 9457):**` \
                     block; the rest of the design set uses that convention"
                ),
            });
        }
    }

    findings.extend(
        declared
            .iter()
            .filter(|(codeid, _)| !referenced.contains(*codeid))
            .map(|(codeid, path)| Finding {
                invariant: "P3/code-unreferenced".to_string(),
                severity: Severity::Low,
                file: path.clone(),
                line: None,
                message: format!(
                    "`{codeid}` is declared in a Problem-responses block but referenced by no rule"
                ),
            }),
    );
    findings
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
    fn flags_an_instruction_id_referenced_but_never_declared() {
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "design/01-a.md",
                "1. Do the thing per `inst-xx-ghost` - `inst-xx-real`\n",
            )],
        );
        let findings = check(&corpus);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].invariant, "P3/inst-dangling");
        assert!(findings[0].message.contains("inst-xx-ghost"));
    }

    #[test]
    fn flags_an_error_code_declared_but_never_referenced() {
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "design/01-a.md",
                "**Problem responses (RFC 9457):** `USED_CODE` (422),\n`ORPHAN_CODE` (409)\n\nRule: fails with `USED_CODE`.\n",
            )],
        );
        let findings = check(&corpus);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].invariant, "P3/code-unreferenced");
        assert!(findings[0].message.contains("ORPHAN_CODE"));
    }

    #[test]
    fn flags_a_document_that_declares_codes_outside_any_problem_responses_block() {
        // Mirrors design/01-foundation.md's shape: codes named in ordinary prose, never
        // inside a `**Problem responses (RFC 9457):**` block. Two codes, not one — this is
        // what makes "exactly one divergence finding" a real assertion about the finding's
        // cardinality (one per document) rather than one that would also pass a latent bug
        // where the push lived inside the per-code capture loop instead of after it (which
        // would emit one divergence finding per code here).
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "design/01-a.md",
                "Foundation-owned failure modes, referenced (never redefined) by slices: \
                 `FIRST_CODE` (409), `SECOND_CODE` (422).\n",
            )],
        );
        let findings = check(&corpus);
        let divergences: Vec<_> = findings
            .iter()
            .filter(|f| f.invariant == "P3/code-convention-divergent")
            .collect();
        assert_eq!(divergences.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(divergences[0].severity, Severity::Low);
        assert_eq!(divergences[0].file, "design/01-a.md");
    }

    #[test]
    fn pricing_corpus_has_no_dangling_instruction_ids() {
        let dangling: Vec<_> = check(&pricing())
            .into_iter()
            .filter(|f| f.invariant == "P3/inst-dangling")
            .collect();
        assert!(dangling.is_empty(), "unexpected: {dangling:#?}");
    }

    /// Pinned baseline of `P3/code-unreferenced` findings against the live pricing corpus,
    /// hand-derived from the failure output of this test on 2026-07-29 (not by running the
    /// checker and trusting whatever it produces — a self-derived baseline asserts
    /// nothing). These 51 `(code, declaring file)` pairs are **debt, not correctness**:
    /// confirmed real — each rule describes its failure mode in prose (e.g. "any overlap
    /// fails") without naming the specific code its slice's Problem-responses catalogue
    /// defines for that failure. Fixing them is a separate docs round (owed alongside
    /// **D-69**), not this task loop's job. Pinned as an exact set so a *new*
    /// unreferenced code fails this test immediately, and so a *fixed* one fails it too —
    /// the list must be updated deliberately when the docs improve, never left to quietly
    /// become a floor.
    const PINNED_UNREFERENCED_CODES_2026_07_29: &[(&str, &str)] = &[
        ("ADDON_CYCLE", "design/02-plan-definition.md"),
        ("ADDON_INCOMPATIBLE", "design/02-plan-definition.md"),
        ("ADDON_OVERRIDE_UNRESOLVED", "design/02-plan-definition.md"),
        ("APPROVAL_ROLE_REQUIRED", "design/05-governance.md"),
        (
            "AVAILABILITY_OUTSIDE_COVERAGE",
            "design/07-pricewindow-linkage.md",
        ),
        ("BACKDATE_GRANT_REQUIRED", "design/05-governance.md"),
        ("BASIS_MISSING", "design/08-bundles.md"),
        ("BILLING_TIMING_MISSING", "design/06-consumer-contracts.md"),
        ("BRAND_UNKNOWN", "design/04-currency-tax.md"),
        ("BULK_ROW_CONFLICT", "design/12-operator-efficiency.md"),
        (
            "CHANGE_TARGET_UNPUBLISHED",
            "design/06-consumer-contracts.md",
        ),
        ("CLONE_SOURCE_NOT_FOUND", "design/12-operator-efficiency.md"),
        ("COMPONENT_UNPUBLISHED", "design/08-bundles.md"),
        (
            "COMPOSITE_CONSTITUENT_UNPUBLISHED",
            "design/10-advanced-primitives.md",
        ),
        (
            "COMPOSITE_SELF_REFERENCE",
            "design/10-advanced-primitives.md",
        ),
        (
            "COMPOSITE_TOO_FEW_CONSTITUENTS",
            "design/10-advanced-primitives.md",
        ),
        (
            "CREDIT_UNIT_UNPUBLISHED",
            "design/10-advanced-primitives.md",
        ),
        ("DESCRIPTOR_INCOMPLETE", "design/02-plan-definition.md"),
        ("EVAL_POLICY_MISPLACED", "design/03-price-structure.md"),
        ("FLOOR_FALLBACK_MISSING", "design/10-advanced-primitives.md"),
        (
            "FLOOR_INSIDE_PRICED_BAND",
            "design/10-advanced-primitives.md",
        ),
        ("FLOOR_TYPE_MISSING", "design/10-advanced-primitives.md"),
        (
            "GRANDFATHERED_ROW_IMMUTABLE",
            "design/07-pricewindow-linkage.md",
        ),
        (
            "GRANDFATHER_LOOSEN_FORBIDDEN",
            "design/07-pricewindow-linkage.md",
        ),
        (
            "GRANT_APPLICABILITY_INELIGIBLE",
            "design/10-advanced-primitives.md",
        ),
        (
            "GRANT_APPLICABILITY_UNIT_MISMATCH",
            "design/10-advanced-primitives.md",
        ),
        (
            "GRANT_APPLICABILITY_UNPUBLISHED",
            "design/10-advanced-primitives.md",
        ),
        ("GRANT_EXPIRY_MISSING", "design/10-advanced-primitives.md"),
        ("GRANT_PRICE_UNSCOPED", "design/10-advanced-primitives.md"),
        ("GRANT_REF_UNDEFINED", "design/06-consumer-contracts.md"),
        ("GROUP_UNKNOWN", "design/09-price-overlays.md"),
        ("HYBRID_INCOMPLETE", "design/02-plan-definition.md"),
        ("METER_AMBIGUOUS", "design/02-plan-definition.md"),
        ("PACKAGE_FIELDS_INVALID", "design/03-price-structure.md"),
        ("PHASE_DURATION_INVALID", "design/02-plan-definition.md"),
        ("PHASE_GRAPH_INVALID", "design/02-plan-definition.md"),
        ("PLANTIER_DIVERGENT", "design/02-plan-definition.md"),
        ("PLANTIER_MISSING", "design/02-plan-definition.md"),
        (
            "PRORATION_INPUTS_MISSING",
            "design/06-consumer-contracts.md",
        ),
        ("PURCHASE_QTY_RANGE_INVALID", "design/02-plan-definition.md"),
        ("QUANTITY_SOURCE_MISSING", "design/03-price-structure.md"),
        ("REASON_REQUIRED", "design/05-governance.md"),
        ("REGION_SCOPE_DENIED", "design/05-governance.md"),
        (
            "RESERVATION_ON_NON_USAGE",
            "design/10-advanced-primitives.md",
        ),
        ("RUN_SELECTOR_EMPTY", "design/12-operator-efficiency.md"),
        ("SETUP_ROW_INVALID", "design/02-plan-definition.md"),
        ("TAXONOMY_VALUE_IN_USE", "design/04-currency-tax.md"),
        ("TAX_BASIS_INCOMPLETE", "design/04-currency-tax.md"),
        ("TIER_BANDS_GAP", "design/03-price-structure.md"),
        ("TIER_BANDS_OVERLAP", "design/03-price-structure.md"),
        ("WINDOW_GAP", "design/07-pricewindow-linkage.md"),
    ];

    /// Parses `P3/code-unreferenced` findings back into `(code, file)` pairs. `file` is
    /// already a structured `Finding` field; `code` is pulled from `check_error_codes`'s own
    /// fixed message template (`` `{codeid}` is declared in a Problem-responses block but
    /// referenced by no rule ``). Test-only, same reasoning as `propagation.rs`'s
    /// `missing_pairs`: production `Finding` has no separate symbol field, and this round
    /// changes test expression, not that shape.
    fn unreferenced_pairs(findings: &[Finding]) -> Vec<(String, String)> {
        let shape = Regex::new(
            r"^`([A-Z][A-Z0-9_]+)` is declared in a Problem-responses block but referenced by no rule$",
        )
        .expect("valid message-shape regex");
        findings
            .iter()
            .filter(|f| f.invariant == "P3/code-unreferenced")
            .map(|f| {
                let caps = shape.captures(&f.message).unwrap_or_else(|| {
                    panic!(
                        "P3/code-unreferenced message doesn't match the expected shape: {}",
                        f.message
                    )
                });
                (caps[1].to_string(), f.file.clone())
            })
            .collect()
    }

    #[test]
    fn code_unreferenced_findings_match_the_pinned_2026_07_29_baseline() {
        // NOT a green invariant, deliberately: see PINNED_UNREFERENCED_CODES_2026_07_29's
        // doc comment. This test exists to make debt visible and stable, not to assert the
        // corpus is clean — it currently is not, and pretending otherwise (by asserting
        // emptiness) would hide exactly the kind of gap P3 exists to catch.
        let actual: BTreeSet<(String, String)> =
            unreferenced_pairs(&check(&pricing())).into_iter().collect();
        let expected: BTreeSet<(String, String)> = PINNED_UNREFERENCED_CODES_2026_07_29
            .iter()
            .map(|(code, path)| (code.to_string(), path.to_string()))
            .collect();

        let appeared: Vec<_> = actual.difference(&expected).collect();
        let disappeared: Vec<_> = expected.difference(&actual).collect();
        assert!(
            appeared.is_empty() && disappeared.is_empty(),
            "code-unreferenced baseline drifted from the pinned 2026-07-29 set — \
             newly appeared (not in the pin): {appeared:#?}; \
             no longer reproduced (pin needs updating — did someone fix these?): {disappeared:#?}"
        );
    }
}
