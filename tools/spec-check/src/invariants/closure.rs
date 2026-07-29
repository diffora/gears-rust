use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

use crate::Corpus;
use crate::finding::{Finding, Severity};

/// The union of every `inst-*` id declared (via a `` - `inst-id` `` bullet line) across
/// every corpus the CLI loaded. `check`'s dangling-instruction rule verifies references
/// against this union rather than the referencing corpus's own declarations alone: an id
/// pricing declares and rating cites (in `SEAMS.md`, `DECISIONS.md`, an ADR, …) without a
/// local re-declaration is a legitimate cross-gear reference, not a dangling one — P3
/// must not conflate "declared in a sibling gear" with "declared nowhere".
#[derive(Debug, Default)]
pub struct DeclaredInstructions {
    ids: BTreeSet<String>,
}

impl DeclaredInstructions {
    pub fn build(corpora: &[Corpus]) -> Self {
        let declared_inst = Regex::new(r"- `(inst-[a-z0-9-]+)`\s*$").expect("valid decl regex");
        let mut ids = BTreeSet::new();
        for corpus in corpora {
            for (_, text) in corpus.files() {
                for line in text.lines() {
                    if let Some(c) = declared_inst.captures(line.trim_end()) {
                        ids.insert(c[1].to_string());
                    }
                }
            }
        }
        Self { ids }
    }

    fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }
}

/// P3 — declared-and-referenced closure for instruction ids and error codes. `declared`
/// is the cross-corpus instruction-id union (see `DeclaredInstructions`) built once from
/// every loaded gear; error-code closure (`check_error_codes`) stays scoped to `corpus`
/// alone — unlike instruction ids, error codes are not cited across gears by convention.
pub fn check(corpus: &Corpus, declared: &DeclaredInstructions) -> Vec<Finding> {
    let mut findings = Vec::new();

    let any_inst = Regex::new(r"`(inst-[a-z0-9-]+)`").expect("valid ref regex");
    let mut referenced: BTreeMap<String, String> = BTreeMap::new();

    for (path, text) in corpus.files() {
        for line in text.lines() {
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

/// True for corpus-relative paths that are numbered design slices — `design/01-foundation.md`,
/// `design/02-plan-definition.md`, and so on — the only documents expected to own an error
/// catalogue. Excludes `design/README.md` (an index, not a slice: no numeric prefix) and
/// everything outside `design/` (PRD, DESIGN, DECISIONS, ADRs), which legitimately
/// *reference* codes a slice owns without ever declaring any themselves. Path shape, not
/// "does it mention a code," is the discriminator: what makes a document own a catalogue is
/// that it is a slice.
///
/// `pub(crate)`: `fr_coverage.rs` reuses this to scope its own traceability-convention
/// detection to the same set of documents, for the same reason — a `**Traces to**:` (or
/// Problem-responses) convention lives on slices, and a non-slice document merely
/// mentioning either shape in prose must not count as the gear "using" it.
pub(crate) fn is_design_slice(path: &str) -> bool {
    path.strip_prefix("design/")
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// Error codes declared inside a `**Problem responses (RFC 9457):**` block (which runs
/// until the first blank line) and never mentioned again anywhere else in the corpus, plus
/// design slices that declare codes without ever opening such a block at all. The latter
/// mirrors `fr_coverage.rs`'s `collect_directly_addresses`: `design/01-foundation.md` names
/// its Foundation-owned codes in prose rather than a Problem-responses block, so without this
/// second check those codes — and any future slice doing the same — would be invisible to
/// the "declared" side of the closure rule, silently narrowing what P3 covers rather than
/// surfacing the gap. One finding per document, not per code, since the defect is "this
/// document uses a different convention," not "this code is unreachable." Scoped to
/// `is_design_slice` documents only: non-slice documents legitimately reference codes they
/// don't own and must not be flagged for it.
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
        if saw_code && !saw_block && is_design_slice(path) {
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

/// Pinned baseline of `P3/code-unreferenced` findings against the live pricing corpus,
/// hand-derived from the failure output of this test on 2026-07-29 (not by running the
/// checker and trusting whatever it produces — a self-derived baseline asserts
/// nothing). These 51 `(gear, code, declaring file)` triples are **debt, not
/// correctness**: confirmed real — each rule describes its failure mode in prose (e.g.
/// "any overlap fails") without naming the specific code its slice's Problem-responses
/// catalogue defines for that failure. Fixing them is a separate docs round (owed
/// alongside **D-69**), not this task loop's job. Pinned as an exact set so a *new*
/// unreferenced code fails this test immediately, and so a *fixed* one fails it too —
/// the list must be updated deliberately when the docs improve, never left to quietly
/// become a floor. Promoted to a `pub const` (2026-07-29, fix round 1) so the CLI has
/// exactly the same one definition of this debt the tests pin against, rather than a
/// second, test-only copy the CLI can't see.
///
/// Every entry names `"pricing"` (task-review Ruling 3 fix, 2026-07-29, fix round 3):
/// this baseline is a snapshot of *one specific corpus*, and an error-code token plus a
/// corpus-relative filename is not a unique key across gears — `design/03-...`-shaped
/// filenames in particular are just as likely to exist in a sibling gear's own design
/// set. Without the gear qualifier a same-shaped finding from a different gear would be
/// silently swallowed as if it were this pinned pricing debt. The gear name here is
/// baseline *data* — it must never leak into `targets.rs`'s resolution path or any
/// invariant's matching logic.
pub const PINNED_UNREFERENCED_CODES_2026_07_29: &[(&str, &str, &str)] = &[
    ("pricing", "ADDON_CYCLE", "design/02-plan-definition.md"),
    (
        "pricing",
        "ADDON_INCOMPATIBLE",
        "design/02-plan-definition.md",
    ),
    (
        "pricing",
        "ADDON_OVERRIDE_UNRESOLVED",
        "design/02-plan-definition.md",
    ),
    (
        "pricing",
        "APPROVAL_ROLE_REQUIRED",
        "design/05-governance.md",
    ),
    (
        "pricing",
        "AVAILABILITY_OUTSIDE_COVERAGE",
        "design/07-pricewindow-linkage.md",
    ),
    (
        "pricing",
        "BACKDATE_GRANT_REQUIRED",
        "design/05-governance.md",
    ),
    ("pricing", "BASIS_MISSING", "design/08-bundles.md"),
    (
        "pricing",
        "BILLING_TIMING_MISSING",
        "design/06-consumer-contracts.md",
    ),
    ("pricing", "BRAND_UNKNOWN", "design/04-currency-tax.md"),
    (
        "pricing",
        "BULK_ROW_CONFLICT",
        "design/12-operator-efficiency.md",
    ),
    (
        "pricing",
        "CHANGE_TARGET_UNPUBLISHED",
        "design/06-consumer-contracts.md",
    ),
    (
        "pricing",
        "CLONE_SOURCE_NOT_FOUND",
        "design/12-operator-efficiency.md",
    ),
    ("pricing", "COMPONENT_UNPUBLISHED", "design/08-bundles.md"),
    (
        "pricing",
        "COMPOSITE_CONSTITUENT_UNPUBLISHED",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "COMPOSITE_SELF_REFERENCE",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "COMPOSITE_TOO_FEW_CONSTITUENTS",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "CREDIT_UNIT_UNPUBLISHED",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "DESCRIPTOR_INCOMPLETE",
        "design/02-plan-definition.md",
    ),
    (
        "pricing",
        "EVAL_POLICY_MISPLACED",
        "design/03-price-structure.md",
    ),
    (
        "pricing",
        "FLOOR_FALLBACK_MISSING",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "FLOOR_INSIDE_PRICED_BAND",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "FLOOR_TYPE_MISSING",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "GRANDFATHERED_ROW_IMMUTABLE",
        "design/07-pricewindow-linkage.md",
    ),
    (
        "pricing",
        "GRANDFATHER_LOOSEN_FORBIDDEN",
        "design/07-pricewindow-linkage.md",
    ),
    (
        "pricing",
        "GRANT_APPLICABILITY_INELIGIBLE",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "GRANT_APPLICABILITY_UNIT_MISMATCH",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "GRANT_APPLICABILITY_UNPUBLISHED",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "GRANT_EXPIRY_MISSING",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "GRANT_PRICE_UNSCOPED",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "GRANT_REF_UNDEFINED",
        "design/06-consumer-contracts.md",
    ),
    ("pricing", "GROUP_UNKNOWN", "design/09-price-overlays.md"),
    (
        "pricing",
        "HYBRID_INCOMPLETE",
        "design/02-plan-definition.md",
    ),
    ("pricing", "METER_AMBIGUOUS", "design/02-plan-definition.md"),
    (
        "pricing",
        "PACKAGE_FIELDS_INVALID",
        "design/03-price-structure.md",
    ),
    (
        "pricing",
        "PHASE_DURATION_INVALID",
        "design/02-plan-definition.md",
    ),
    (
        "pricing",
        "PHASE_GRAPH_INVALID",
        "design/02-plan-definition.md",
    ),
    (
        "pricing",
        "PLANTIER_DIVERGENT",
        "design/02-plan-definition.md",
    ),
    (
        "pricing",
        "PLANTIER_MISSING",
        "design/02-plan-definition.md",
    ),
    (
        "pricing",
        "PRORATION_INPUTS_MISSING",
        "design/06-consumer-contracts.md",
    ),
    (
        "pricing",
        "PURCHASE_QTY_RANGE_INVALID",
        "design/02-plan-definition.md",
    ),
    (
        "pricing",
        "QUANTITY_SOURCE_MISSING",
        "design/03-price-structure.md",
    ),
    ("pricing", "REASON_REQUIRED", "design/05-governance.md"),
    ("pricing", "REGION_SCOPE_DENIED", "design/05-governance.md"),
    (
        "pricing",
        "RESERVATION_ON_NON_USAGE",
        "design/10-advanced-primitives.md",
    ),
    (
        "pricing",
        "RUN_SELECTOR_EMPTY",
        "design/12-operator-efficiency.md",
    ),
    (
        "pricing",
        "SETUP_ROW_INVALID",
        "design/02-plan-definition.md",
    ),
    (
        "pricing",
        "TAXONOMY_VALUE_IN_USE",
        "design/04-currency-tax.md",
    ),
    (
        "pricing",
        "TAX_BASIS_INCOMPLETE",
        "design/04-currency-tax.md",
    ),
    ("pricing", "TIER_BANDS_GAP", "design/03-price-structure.md"),
    (
        "pricing",
        "TIER_BANDS_OVERLAP",
        "design/03-price-structure.md",
    ),
    ("pricing", "WINDOW_GAP", "design/07-pricewindow-linkage.md"),
];

/// Parses a `P3/code-unreferenced` finding's `(code, declaring file)` pair from
/// `check_error_codes`'s own fixed message template. `None` for any other invariant tag
/// or a message that doesn't match the expected shape — the single production-and-test
/// definition of "how to read this finding back into pinned-baseline shape" (promoted
/// 2026-07-29, fix round 1, replacing what was a test-only `unreferenced_pairs` helper).
///
/// Deliberately does not, and cannot, recover a gear from the `Finding` alone (see
/// `finding.rs`: a `Finding` carries only a corpus-relative path). Callers matching
/// against a gear-qualified baseline (see `is_pinned_baseline`) must supply the gear
/// themselves.
pub fn unreferenced_pair(finding: &Finding) -> Option<(String, String)> {
    if finding.invariant != "P3/code-unreferenced" {
        return None;
    }
    let shape = Regex::new(
        r"^`([A-Z][A-Z0-9_]+)` is declared in a Problem-responses block but referenced by no rule$",
    )
    .expect("valid message-shape regex");
    shape
        .captures(&finding.message)
        .map(|c| (c[1].to_string(), finding.file.clone()))
}

/// True if `finding`, attributed to `gear`, is exactly one of the pinned, accepted-debt
/// unreferenced-code findings (tracked as D-69) rather than newly appeared drift. `gear`
/// must be supplied by the caller from the corpus the finding was actually produced
/// against (task-review Ruling 3 fix, 2026-07-29, fix round 3) — a same-`(code, file)`
/// finding attributed to any other gear must not match.
pub fn is_pinned_baseline(finding: &Finding, gear: &str) -> bool {
    unreferenced_pair(finding).is_some_and(|(code, file)| {
        PINNED_UNREFERENCED_CODES_2026_07_29
            .iter()
            .any(|(pgear, pcode, pfile)| *pgear == gear && *pcode == code && *pfile == file)
    })
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

    fn load_gears(names: &[&str]) -> Vec<Corpus> {
        names
            .iter()
            .map(|g| {
                let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../../gears/bss/{g}/docs"));
                Corpus::load(&root).expect("gear corpus loads")
            })
            .collect()
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
        let declared = DeclaredInstructions::build(std::slice::from_ref(&corpus));
        let findings = check(&corpus, &declared);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].invariant, "P3/inst-dangling");
        assert!(findings[0].message.contains("inst-xx-ghost"));
    }

    #[test]
    fn does_not_flag_an_instruction_id_declared_in_a_different_loaded_corpus() {
        // Mirrors the real shape this fix addresses: an id pricing declares (in a
        // design slice), cited from rating's SEAMS.md without a local re-declaration
        // there. Resolving declarations across every loaded corpus — not just the one
        // currently being checked — is exactly what makes this a legitimate reference
        // rather than a dangling one.
        let corpora = vec![
            Corpus::from_parts(
                "gears/bss/alpha/docs",
                [("design/01-a.md", "1. Some rule - `inst-shared-id`\n")],
            ),
            Corpus::from_parts(
                "gears/bss/beta/docs",
                [(
                    "SEAMS.md",
                    "Cites the joint contract `inst-shared-id` here.\n",
                )],
            ),
        ];
        let declared = DeclaredInstructions::build(&corpora);
        let findings = check(&corpora[1], &declared);
        assert!(
            !findings.iter().any(|f| f.invariant == "P3/inst-dangling"),
            "unexpected: {findings:#?}"
        );
    }

    #[test]
    fn flags_an_instruction_id_declared_in_no_loaded_corpus() {
        // Distinguishes "declared elsewhere in the loaded set" (previous test — not
        // dangling) from "declared nowhere at all" (still dangling): the cross-corpus
        // union must not become a blanket amnesty for genuinely invented ids.
        let corpora = vec![
            Corpus::from_parts(
                "gears/bss/alpha/docs",
                [("design/01-a.md", "nothing here\n")],
            ),
            Corpus::from_parts(
                "gears/bss/beta/docs",
                [("SEAMS.md", "Cites the never-declared `inst-ghost` here.\n")],
            ),
        ];
        let declared = DeclaredInstructions::build(&corpora);
        let findings = check(&corpora[1], &declared);
        let dangling: Vec<_> = findings
            .iter()
            .filter(|f| f.invariant == "P3/inst-dangling")
            .collect();
        assert_eq!(dangling.len(), 1, "unexpected: {findings:#?}");
        assert!(dangling[0].message.contains("inst-ghost"));
    }

    #[test]
    fn cross_gear_instruction_references_are_not_flagged_against_the_live_corpus() {
        // Real-corpus regression: before this fix, checking rating/subscriptions against
        // only their own declarations produced 8 P3/inst-dangling false positives for
        // ids pricing declares and they legitimately cite (inst-tt-joint,
        // inst-cmp-usagetype, inst-plv-adjustment, inst-plv-lines, inst-rv-tier-q,
        // inst-plv-class-tiebreak, inst-tb-window-continuity, inst-gs-shape). With the
        // declared set built across all three loaded gears, there must be none.
        let corpora = load_gears(&["pricing", "rating", "subscriptions"]);
        let declared = DeclaredInstructions::build(&corpora);
        let dangling: Vec<_> = corpora
            .iter()
            .flat_map(|c| check(c, &declared))
            .filter(|f| f.invariant == "P3/inst-dangling")
            .collect();
        assert!(dangling.is_empty(), "unexpected: {dangling:#?}");
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
        let declared = DeclaredInstructions::build(std::slice::from_ref(&corpus));
        let findings = check(&corpus, &declared);
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
        // would emit one divergence finding per code here). The path is deliberately
        // slice-shaped (`design/` + a numeric prefix) — `is_design_slice` excludes anything
        // else, so a non-slice-shaped fixture here would make this test pass vacuously.
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "design/01-a.md",
                "Foundation-owned failure modes, referenced (never redefined) by slices: \
                 `FIRST_CODE` (409), `SECOND_CODE` (422).\n",
            )],
        );
        let declared = DeclaredInstructions::build(std::slice::from_ref(&corpus));
        let findings = check(&corpus, &declared);
        let divergences: Vec<_> = findings
            .iter()
            .filter(|f| f.invariant == "P3/code-convention-divergent")
            .collect();
        assert_eq!(divergences.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(divergences[0].severity, Severity::Low);
        assert_eq!(divergences[0].file, "design/01-a.md");
    }

    #[test]
    fn does_not_flag_a_non_slice_document_that_references_codes() {
        // The regression this scoping guards against: PRD.md, DECISIONS.md, and the ADRs
        // reference codes that a slice owns without ever declaring any themselves — they
        // were never meant to use the Problem-responses convention, so merely mentioning a
        // code in prose (with no catalogue block) must not flag them the way it would flag
        // a diverging slice.
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "PRD.md",
                "The publish check fails with `SOME_CODE` when the row is invalid.\n",
            )],
        );
        let declared = DeclaredInstructions::build(std::slice::from_ref(&corpus));
        let findings = check(&corpus, &declared);
        assert!(
            !findings
                .iter()
                .any(|f| f.invariant == "P3/code-convention-divergent"),
            "unexpected: {findings:#?}"
        );
    }

    #[test]
    fn pricing_corpus_has_no_dangling_instruction_ids() {
        let pricing = pricing();
        let declared = DeclaredInstructions::build(std::slice::from_ref(&pricing));
        let dangling: Vec<_> = check(&pricing, &declared)
            .into_iter()
            .filter(|f| f.invariant == "P3/inst-dangling")
            .collect();
        assert!(dangling.is_empty(), "unexpected: {dangling:#?}");
    }

    #[test]
    fn code_unreferenced_findings_match_the_pinned_2026_07_29_baseline() {
        // NOT a green invariant, deliberately: see PINNED_UNREFERENCED_CODES_2026_07_29's
        // doc comment. This test exists to make debt visible and stable, not to assert the
        // corpus is clean — it currently is not, and pretending otherwise (by asserting
        // emptiness) would hide exactly the kind of gap P3 exists to catch.
        let pricing = pricing();
        let declared = DeclaredInstructions::build(std::slice::from_ref(&pricing));
        let actual: BTreeSet<(String, String)> = check(&pricing, &declared)
            .iter()
            .filter_map(unreferenced_pair)
            .collect();
        // Raw `check()` output is only ever compared against this one corpus's own
        // pinned entries here, so the (code, file) projection (dropping the gear
        // element) is the correct comparison — every entry in the pin is `"pricing"` by
        // construction (see the const's doc comment); asserted below so a future entry
        // added for a different gear would fail loudly here rather than silently
        // changing what this test actually checks.
        assert!(
            PINNED_UNREFERENCED_CODES_2026_07_29
                .iter()
                .all(|(gear, _, _)| *gear == "pricing"),
            "this baseline is documented as a pricing-only snapshot; a non-pricing entry \
             would invalidate this test's (code, file)-only comparison"
        );
        let expected: BTreeSet<(String, String)> = PINNED_UNREFERENCED_CODES_2026_07_29
            .iter()
            .map(|(_, code, path)| (code.to_string(), path.to_string()))
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

    #[test]
    fn is_pinned_baseline_matches_only_the_recorded_gear() {
        // Task-review Ruling 3 finding (CRITICAL, "do the same for the code-unreferenced
        // side"): a finding whose (code, file) matches a pinned pricing entry
        // byte-for-byte must not be treated as known debt when attributed to a
        // different gear — `design/03-...`-shaped filenames are just as plausible in a
        // sibling gear's own design set.
        let (gear, code, file) = PINNED_UNREFERENCED_CODES_2026_07_29[0];
        assert_eq!(gear, "pricing", "test assumes entry 0 is pricing's");
        let finding = Finding {
            invariant: "P3/code-unreferenced".to_string(),
            severity: Severity::Low,
            file: file.to_string(),
            line: None,
            message: format!(
                "`{code}` is declared in a Problem-responses block but referenced by no rule"
            ),
        };
        assert!(is_pinned_baseline(&finding, "pricing"));
        assert!(!is_pinned_baseline(&finding, "rating"));
        assert!(!is_pinned_baseline(&finding, "subscriptions"));
    }
}
