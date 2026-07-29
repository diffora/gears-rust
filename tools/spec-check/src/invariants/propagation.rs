use regex::Regex;

use crate::decisions::Decision;
use crate::finding::{Finding, Severity};
use crate::{Corpus, decisions, targets};

/// P1 — for every decision that records a propagation surface, each named target
/// document must cite the decision id. `seams` is the cross-gear seam-ownership index
/// (see `targets::SeamIndex`) built from every corpus the CLI loaded, so a `SEAMS <id>`
/// propagation target can be checked against who actually owns `id` instead of a
/// prefix guess. `loaded` is every corpus the CLI loaded, so a resolved *cross-gear*
/// target (`../../<gear>/docs/SEAMS.md`) is checked against the sibling document for real
/// rather than dropped (see `targets::text_at`); a caller with no siblings passes `&[]`.
///
/// This check states its own coverage. A `DECISIONS.md` that exists but yields zero parsed
/// entries produces one `P1/decision-register-unparsed` naming the gear, because a run that
/// parsed nothing must never be reported the same way as a run that verified everything.
pub fn check(corpus: &Corpus, seams: &targets::SeamIndex, loaded: &[Corpus]) -> Vec<Finding> {
    let Some(register) = corpus.text("DECISIONS.md") else {
        return Vec::new();
    };

    let lines: Vec<&str> = register.lines().collect();
    let all = decisions::parse(register);

    let mut findings = Vec::new();

    if all.is_empty() {
        // The more important half of the id-shape widening (2026-07-29 final review, item
        // 1): before it, `^#### (D-\d+)\b` matched pricing's register only, so
        // subscriptions' 19 populated `**Propagated**:` claims went unchecked and rating's
        // table-shaped register went unchecked, and the CLI printed a finding count that
        // read exactly like a clean verdict for both. Modelled on
        // `fr_coverage`'s `P2/traceability-convention-unknown`, including its honesty about
        // the cost: the message states how much propagation surface went unverified.
        //
        // A register with a genuinely empty propagation surface (rating's `T-D-NN` table
        // has no `**Propagated**:` field at all) reports `0`, which is the correct, honest
        // outcome — the gear has nothing here for P1 to check — not a defect to engineer
        // away. A register whose headings P1 cannot read while carrying real claims reports
        // a nonzero count, which is loud.
        findings.push(Finding {
            invariant: "P1/decision-register-unparsed".to_string(),
            severity: Severity::Low,
            file: "DECISIONS.md".to_string(),
            line: None,
            message: format!(
                "P1 cannot verify propagation for {}: DECISIONS.md yielded zero decision \
                 entries — no `#### <id> …` heading matched the recognised id shape (`D-NN`, \
                 optionally gear-prefixed, e.g. `SUB-D-01`) — {} `**Propagated`-shaped \
                 field(s) in that register went unchecked as a result",
                corpus.root().display(),
                propagated_label_count(register),
            ),
        });
        return findings;
    }

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

        // A citation the resolver understands *nothing* in used to yield zero findings:
        // `resolve` populates `unresolved` only for tokens it recognised but could not map,
        // so an all-empty `Resolved` made every loop below push nothing (2026-07-29 final
        // review, item 4 — measured on D-49 `§15 rows ×5.` and D-66 `rating ×4 files (6
        // sites); subscriptions ×2 files (3 sites).`, the latter a real cross-gear claim).
        // That is the plan's Global Constraint verbatim: an unresolvable propagation target
        // is a `Finding`, never a silent skip.
        //
        // Guarded on the *whole* `Resolved` being empty, not just `paths`: a citation whose
        // tokens were recognised but unmappable already reports `P1/propagation-unresolvable`
        // below, and reporting both for one citation would double-count the same defect.
        // Low, the same severity as its sibling `propagation-unresolvable`, because both make
        // the same kind of statement — the resolver could not read this, so a human must —
        // rather than proving a document is wrong.
        if resolved.is_empty() && !raw.is_empty() {
            findings.push(Finding {
                invariant: "P1/propagation-uninterpretable".to_string(),
                severity: Severity::Low,
                file: "DECISIONS.md".to_string(),
                line: Some(d.line),
                message: format!(
                    "{}: propagation citation `{raw}` names nothing the resolver recognises, \
                     so the claim was not verified at all",
                    d.id
                ),
            });
        }

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

        // Low, not Medium (2026-07-29 final review, item 7): `targets::resolve`'s seam-id
        // shape is deliberately gear-agnostic (`[A-Z][A-Z0-9]*(-[A-Z0-9]+)*`, matching real
        // families like `K1`, `ASC`, `M12`, `SUB-P7`), so it also matches any all-caps word
        // written after `SEAMS` — `**Propagated**: SEAMS TBD` yields the id `TBD` and `SEAMS
        // N/A` yields `N`. Tightening the *shape* is not available: `ASC` is a real seam id
        // with no digit, so no digit-or-length rule separates ids from placeholders, and
        // enumerating the real families would put gear-specific convention back into
        // resolution. The remaining choice is severity, and at Medium this single most
        // likely false positive failed the default `--max-severity medium` gate. The design
        // spec budgets <= 2 false positives per run, and one that breaks the build is
        // qualitatively worse than one that prints a Low line a reader can dismiss; a
        // genuinely dangling seam id is still reported, just without blocking every run.
        for id in &resolved.seam_undefined {
            findings.push(Finding {
                invariant: "P1/seam-undefined".to_string(),
                severity: Severity::Low,
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
            // Cross-gear targets (`../../rating/docs/SEAMS.md`) are outside this corpus, and
            // used to be dropped by a bare `else { continue }` — four of pricing's decisions
            // (D-44, D-46, D-60, D-65) had their only cross-gear claim silently unverified
            // that way (2026-07-29 final review, item 5). `main` already loads every corpus,
            // so the join is available: `text_at` looks the target up across all of them and
            // it is then checked exactly like an in-corpus one.
            //
            // A target no loaded corpus provides is *reported*, never skipped — that is the
            // Global Constraint. Low, because it is a statement about this run's inputs (a
            // sibling gear was not passed as `--gear`) rather than a defect in a document.
            let Some(text) = targets::text_at(corpus, path, loaded) else {
                findings.push(Finding {
                    invariant: "P1/propagation-target-not-loaded".to_string(),
                    severity: Severity::Low,
                    file: "DECISIONS.md".to_string(),
                    line: Some(d.line),
                    message: format!(
                        "{} claims propagation into {path}, which no loaded gear corpus \
                         provides, so the claim was not verified — pass that gear's docs \
                         directory as another `--gear` to check it",
                        d.id
                    ),
                });
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

/// Any label shaped like a `**Propagated…**` bold field, however punctuated. Deliberately
/// loose (2026-07-29 final review, item 2): a backstop as strict as the parser it backstops
/// is not a backstop.
///
/// The parser's anchor and the old fallback *both* required the colon to sit **outside** the
/// bold span, so `- **Propagated:** PRD §1.` matched neither — and colon-inside-bold is
/// house style in these very documents (`**Produced:**`, `**Consumed:**`, `**Decision
/// (2026-07-10):**`, `**Problem responses (RFC 9457):**`; 267 occurrences across the three
/// gears, of which 66 are bullet-anchored field labels). One author writing
/// `**Propagated:**` would have silently reintroduced exactly the D-42 false negative
/// Ruling 4 was raised to eliminate. Dropping the colon requirement also catches
/// `**Propagated** — PRD §1.` and `**Propagated**  : PRD §1.`
///
/// Looseness is safe *because* this only runs when the primary parser already returned
/// `None`: anything it can read never reaches here. The literal `Propagated` keeps it from
/// firing on the neighbouring `**Propagation status:**` label, which is a different field.
fn propagated_label() -> Regex {
    Regex::new(r"\*\*Propagated[^*]*\*\*(?:\s*:)?").expect("valid label regex")
}

/// How many `**Propagated…**`-shaped field labels the whole register carries. Used only to
/// state the cost of a register P1 could not parse at all (see `check`'s
/// `P1/decision-register-unparsed`): this is the propagation surface that went unverified.
fn propagated_label_count(register: &str) -> usize {
    propagated_label().find_iter(register).count()
}

/// Body text of decision `all[i]` (its heading line through the line before the
/// next entry's heading, or through EOF for the last entry) still contains a
/// `**Propagated…**`-shaped bold field label that `decisions::parse`'s exact
/// `**Propagated**:` anchor did not capture. Returns the literal label text
/// (asterisks and any trailing colon included) when one is found, so the finding
/// can name it verbatim.
fn unparsed_propagated_label(lines: &[&str], all: &[Decision], i: usize) -> Option<String> {
    let start = all[i].line - 1;
    let end = all
        .get(i + 1)
        .map(|next| next.line - 1)
        .unwrap_or(lines.len());
    let body = lines[start..end].join("\n");
    propagated_label()
        .find(&body)
        .map(|m| m.as_str().to_string())
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
///
/// Every entry is also an **in-corpus** target, and that is all this snapshot ever could
/// have covered: when it was taken, a resolved cross-gear target (`../../<gear>/docs/SEAMS.md`)
/// was dropped by `check` without being verified, so no such gap could appear. Since the
/// 2026-07-29 final-review fix wave (item 5) those targets are checked for real, and the one
/// gap that surfaced (D-46 into rating's `SEAMS.md`) is deliberately **not** listed here: this
/// register is *accepted* debt, whose contents are a human decision, and an entry here also
/// stops the CLI failing on it. It stays a live finding until someone rules on it in the D-69
/// round. `cross_gear_propagation_gaps_are_newly_visible_unaccepted_debt` keeps that set
/// stable meanwhile, and the drift test below compares only in-corpus targets against this
/// register so the two sets never blur into each other.
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
    // The id shape matches `decisions::DECISION_ID`, not a bare `D-\d+`: since the parser
    // learned to read gear-prefixed ids (`SUB-D-01`), this must read its own messages back
    // for them too, or a subscriptions propagation gap would be unparseable by the very
    // helper the known-debt decision and the drift test are built on. Widening it changes
    // nothing about what is *suppressed* — `is_pinned_baseline` still requires an exact
    // `(gear, id, path)` hit against a register whose every entry is pricing's.
    let shape = Regex::new(&format!(
        r"^({id}) claims propagation into (.+), but that document never cites {id}$",
        id = decisions::DECISION_ID
    ))
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

    /// Every live BSS gear's corpus, in the order `make spec-check` passes them — pricing
    /// first, so `live_corpora()[0]` is the corpus the pinned baseline was taken from.
    /// Naming gears here names real trees on disk for a test to load; it is not resolution
    /// logic branching on gear identity.
    ///
    /// The whole live set, rather than the pricing corpus alone: since item 5 a cross-gear
    /// propagation target is verified against the sibling document, so a test that loaded one
    /// gear would silently check something a real `make spec-check` run does not.
    fn live_corpora() -> Vec<Corpus> {
        ["pricing", "rating", "subscriptions"]
            .iter()
            .map(|gear| {
                let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../../gears/bss/{gear}/docs"));
                Corpus::load(&root).expect("gear corpus loads")
            })
            .collect()
    }

    #[test]
    fn propagation_gaps_match_the_pinned_2026_07_29_baseline() {
        // NOT a green invariant, deliberately: see PINNED_PROPAGATION_GAPS_2026_07_29's
        // doc comment (a module-level `pub const` now, brought into scope here by `use
        // super::*` above). This test exists to make debt visible and stable, not to
        // assert the register is clean — it currently is not, and pretending otherwise
        // (by asserting emptiness, as this test previously did) hides exactly the kind
        // of gap P1 exists to catch.
        // Checked exactly as `make spec-check` checks it: every gear loaded, the seam index
        // and the cross-gear document join both built over all of them (2026-07-29 final
        // review, item 5 — before it, cross-gear targets were dropped unverified, so the
        // pricing corpus alone produced the same set).
        //
        // Scoped to *in-corpus* targets, which is what this pin has always been and all it
        // ever could have been: every entry in it names a pricing-relative path, taken at a
        // time when a `../../<gear>/docs/…` target was unverifiable by construction. The
        // cross-gear gaps item 5 made visible are asserted by
        // `cross_gear_propagation_gaps_are_newly_visible_unaccepted_debt` below, deliberately
        // *not* folded into this register — the register is accepted debt (D-69), whose
        // contents are a human decision, and absorbing a brand-new finding into it would
        // suppress it from the CLI's exit code, which is the one place someone has to look.
        let corpora = live_corpora();
        let seams = targets::SeamIndex::build(&corpora);
        let actual: BTreeSet<(String, String)> = check(&corpora[0], &seams, &corpora)
            .iter()
            .filter_map(missing_pair)
            .filter(|(_, path)| !is_cross_gear(path))
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

    /// A resolved target outside the citing corpus — the `../../<gear>/docs/SEAMS.md` shape
    /// `targets::resolve` mints for a cross-gear `SEAMS <id>` citation.
    fn is_cross_gear(path: &str) -> bool {
        path.starts_with("../")
    }

    #[test]
    fn cross_gear_propagation_gaps_are_newly_visible_unaccepted_debt() {
        // What item 5 bought, asserted as an exact set so it drifts loudly in both
        // directions. Four of pricing's decisions claim a cross-gear propagation surface
        // (D-44 `SEAMS M10`, D-46 `SEAMS RG3`, D-60 `SEAMS M12`, D-65 `SEAMS SUB-P7`); all
        // four used to be dropped by a bare `else { continue }`. Checked for real, three are
        // clean — rating's SEAMS.md cites D-44 and D-60, subscriptions' cites D-65 — and one
        // is a genuine gap: rating's `RG3` row records the reconciliation D-46 drove but
        // never cites D-46.
        //
        // Deliberately NOT added to `PINNED_PROPAGATION_GAPS_2026_07_29`: that register is
        // *accepted* debt whose contents are a human decision, and an entry there would stop
        // the CLI failing on this, which is exactly how a newly-found defect gets buried. It
        // stays a live Medium finding that reddens `make spec-check` until someone rules on
        // it in the D-69 docs round. This test's job is only to keep the set stable
        // meanwhile: a second cross-gear gap, or this one being fixed, must both fail here.
        let corpora = live_corpora();
        let seams = targets::SeamIndex::build(&corpora);
        let actual: BTreeSet<(String, String)> = check(&corpora[0], &seams, &corpora)
            .iter()
            .filter_map(missing_pair)
            .filter(|(_, path)| is_cross_gear(path))
            .collect();
        let expected: BTreeSet<(String, String)> =
            [("D-46".to_string(), "../../rating/docs/SEAMS.md".to_string())].into();
        assert_eq!(
            actual, expected,
            "the set of cross-gear propagation gaps moved — a new one is unaccepted debt to \
             take to a human, and a disappeared one means the docs round closed it and this \
             expectation needs updating (never the other way round)"
        );
    }

    #[test]
    fn a_cross_gear_target_is_verified_against_the_sibling_corpus_not_skipped() {
        // Item 5, in isolation: before this, `corpus.text(path)` returned `None` for every
        // `../../<gear>/docs/SEAMS.md` target and the loop `continue`d, so a cross-gear
        // claim could never produce a finding no matter what the sibling document said.
        // Two sibling corpora, identical except that one cites the deciding id and one does
        // not — the citing one must be silent and the other must be flagged, which no
        // skip-based implementation can do.
        let register = "#### D-90 [M] Cross-gear claim\n\n- **Propagated**: alpha SEAMS Z1.\n";
        let seam_row = |body: &str| {
            format!(
                "| # | Sev | Verdict | Seam |\n|---|-----|---------|------|\n| **Z1** | HIGH | Joint | {body} |\n"
            )
        };

        for (cites, want_findings) in [(true, 0), (false, 1)] {
            let beta = Corpus::from_parts("gears/bss/beta/docs", [("DECISIONS.md", register)]);
            let row = seam_row(if cites {
                "Adopted per D-90."
            } else {
                "Adopted, with no citation at all."
            });
            let alpha = Corpus::from_parts("gears/bss/alpha/docs", [("SEAMS.md", row.as_str())]);
            let loaded = vec![beta, alpha];
            let seams = targets::SeamIndex::build(&loaded);

            let findings = check(&loaded[0], &seams, &loaded);
            assert_eq!(
                findings.len(),
                want_findings,
                "cites={cites}: unexpected: {findings:#?}"
            );
            if want_findings == 1 {
                assert_eq!(findings[0].invariant, "P1/propagation-missing");
                assert!(findings[0].message.contains("../../alpha/docs/SEAMS.md"));
            }
        }
    }

    #[test]
    fn a_cross_gear_target_no_loaded_corpus_provides_is_reported_not_skipped() {
        // The other half of item 5's Global Constraint: when the owning gear was never
        // passed as `--gear`, the claim genuinely cannot be verified — and saying so is the
        // whole point. `SeamIndex` here knows who owns `Z1` (it was built over alpha) while
        // the document join does not (alpha is not in `loaded`), which is the only way this
        // state arises; a real `main` run builds both over the same slice.
        let beta = Corpus::from_parts(
            "gears/bss/beta/docs",
            [(
                "DECISIONS.md",
                "#### D-91 [M] Cross-gear claim\n\n- **Propagated**: alpha SEAMS Z1.\n",
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
        let seams = targets::SeamIndex::build(&[alpha]);

        let findings = check(&beta, &seams, std::slice::from_ref(&beta));
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/propagation-target-not-loaded");
        assert_eq!(findings[0].severity, Severity::Low);
        assert!(findings[0].message.contains("../../alpha/docs/SEAMS.md"));
        assert!(findings[0].message.contains("D-91"));
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
        let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
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
        let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
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
        let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/propagation-label-unparsed");
        assert_eq!(findings[0].severity, Severity::Medium);
        assert_eq!(findings[0].file, "DECISIONS.md");
        assert!(findings[0].message.contains("D-97"));
        assert!(findings[0].message.contains("Propagated pending"));
    }

    #[test]
    fn every_propagated_label_shape_the_parser_cannot_read_is_still_reported() {
        // Item 2: the fallback used to share the primary parser's exact blind spot — both
        // required the colon *outside* the bold span — so none of these shapes produced
        // anything at all. Colon-inside-bold is house style in these very documents (267
        // `**Xxx:**` occurrences across the three gears), so one author writing
        // `**Propagated:**` would have silently reintroduced the D-42 false negative that
        // Ruling 4 exists to eliminate. One case per shape, not one per function: what has
        // to hold is a property of document shapes.
        for (label, body) in [
            ("**Propagated:**", "- **Propagated:** PRD §1.\n"),
            ("**Propagated**", "- **Propagated** — PRD §1.\n"),
            ("**Propagated**  :", "- **Propagated**  : PRD §1.\n"),
            (
                "**Propagated pending**:",
                "- **Propagated pending**: PRD §1.\n",
            ),
            (
                "**Propagated (pending review)**",
                "- **Propagated (pending review)** PRD §1.\n",
            ),
        ] {
            let corpus = Corpus::from_parts(
                "synthetic",
                [
                    (
                        "DECISIONS.md",
                        format!("#### D-92 [M] Something\n\n{body}").as_str(),
                    ),
                    ("PRD.md", "Some requirement text with no citation.\n"),
                ],
            );
            let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
            assert_eq!(findings.len(), 1, "{label}: unexpected: {findings:#?}");
            assert_eq!(
                findings[0].invariant, "P1/propagation-label-unparsed",
                "{label}: a propagation label the parser cannot read must never be a silent \
                 skip: {findings:#?}"
            );
            assert!(
                findings[0].message.contains(label),
                "{label}: the finding must quote the label verbatim: {:?}",
                findings[0].message
            );
        }
    }

    #[test]
    fn the_loose_fallback_does_not_fire_on_the_neighbouring_propagation_status_label() {
        // The bound on item 2's looseness: `**Propagation status:**` is a real, different
        // field label in this corpus (2 occurrences). The fallback keys on the literal
        // `Propagated`, so a nearby label that merely shares a stem must not be mistaken for
        // an unreadable propagation field — otherwise loosening the backstop would trade one
        // false negative for a false positive.
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "DECISIONS.md",
                "#### D-89 [M] Something\n\n- **Propagation status:** tracked in §15.\n",
            )],
        );
        let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
        assert!(findings.is_empty(), "unexpected: {findings:#?}");
    }

    #[test]
    fn a_citation_the_resolver_understands_nothing_in_is_reported() {
        // Item 4: `resolve` populates `unresolved` only for tokens it *recognised* but could
        // not map, so a citation with no recognised token at all came back all-empty and
        // every loop pushed nothing — a silent skip, against the plan's Global Constraint.
        // Both shapes are live register text: D-49's `§15 rows ×5.` and D-66's, the latter a
        // real cross-gear propagation claim.
        for raw in [
            "§15 rows ×5.",
            "rating ×4 files (6 sites); subscriptions ×2 files (3 sites).",
        ] {
            let corpus = Corpus::from_parts(
                "synthetic",
                [(
                    "DECISIONS.md",
                    format!("#### D-88 [M] Something\n\n- **Propagated**: {raw}\n").as_str(),
                )],
            );
            let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
            assert_eq!(findings.len(), 1, "{raw}: unexpected: {findings:#?}");
            assert_eq!(findings[0].invariant, "P1/propagation-uninterpretable");
            assert_eq!(findings[0].severity, Severity::Low);
            assert!(
                findings[0].message.contains(raw),
                "the finding must quote the raw citation: {:?}",
                findings[0].message
            );
        }
    }

    #[test]
    fn an_uninterpretable_finding_is_not_also_raised_for_a_merely_unresolvable_token() {
        // The bound on item 4: a citation whose tokens *were* recognised but could not be
        // mapped already reports `P1/propagation-unresolvable`. Guarding on the whole
        // `Resolved` being empty rather than on `paths` alone is what keeps one defect from
        // being reported twice under two names.
        let corpus = Corpus::from_parts(
            "synthetic",
            [(
                "DECISIONS.md",
                "#### D-87 [M] Something\n\n- **Propagated**: SEAMS.\n",
            )],
        );
        let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/propagation-unresolvable");
    }

    #[test]
    fn a_register_that_yields_zero_entries_says_so_instead_of_reporting_clean() {
        // Item 1's more important half: a `DECISIONS.md` whose headings P1 cannot read used
        // to make `check` return an empty `Vec`, indistinguishable at the CLI from a register
        // verified clean. This fixture carries a real, populated propagation claim under a
        // heading shape the parser does not recognise — the worst case, and the one the
        // message must quantify.
        let corpus = Corpus::from_parts(
            "gears/bss/delta/docs",
            [(
                "DECISIONS.md",
                "### Decision 1 — not a `####` entry heading\n\n- **Propagated**: PRD §1.\n",
            )],
        );
        let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/decision-register-unparsed");
        assert_eq!(findings[0].severity, Severity::Low);
        assert!(
            findings[0].message.contains("gears/bss/delta/docs"),
            "the finding must name the gear whose register went unchecked: {:?}",
            findings[0].message
        );
        // Suppression must state its cost, exactly as `P2/traceability-convention-unknown`
        // does ("43 requirement(s) went unchecked as a result"). "1 `**Propagated`-shaped
        // field", not a bare `1` — the invariant tag and the fixture both contain digits, so
        // a bare-digit check would pass vacuously.
        assert!(
            findings[0]
                .message
                .contains("1 `**Propagated`-shaped field"),
            "the message must state how much propagation surface went unchecked: {:?}",
            findings[0].message
        );
    }

    #[test]
    fn a_corpus_with_no_decision_register_at_all_stays_silent() {
        // The bound on the zero-entries finding: "there is no register here" is not the same
        // claim as "there is a register I could not read", and only the second is a coverage
        // gap worth a finding. A gear with no `DECISIONS.md` must stay silent, or every
        // non-register corpus would start firing this.
        let corpus = Corpus::from_parts("gears/bss/delta/docs", [("PRD.md", "Requirements.\n")]);
        let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
        assert!(findings.is_empty(), "unexpected: {findings:#?}");
    }

    #[test]
    fn each_live_gear_register_is_either_parsed_or_says_it_was_not() {
        // The real-corpus regression for item 1, over all three gears at once: this is the
        // property the whole wave exists to protect, so it is asserted against the live tree
        // and not only synthetic fixtures. Pricing (`D-NN`) and subscriptions (`SUB-D-NN`)
        // must both parse — before the widening, subscriptions' 19 populated `**Propagated**:`
        // claims were checked by nothing while the CLI printed a clean-looking count. Rating's
        // register is a `T-D-NN` table with no `**Propagated**:` field anywhere, so it has no
        // propagation surface at all; the honest outcome there is the zero-entries finding and
        // nothing else, not silence.
        let corpora = live_corpora();
        let seams = targets::SeamIndex::build(&corpora);
        let mut unparsed_registers = Vec::new();
        for corpus in &corpora {
            let findings = check(corpus, &seams, &corpora);
            let root = corpus.root().display().to_string();
            let entries = decisions::parse(corpus.text("DECISIONS.md").expect("register")).len();
            let says_unparsed = findings
                .iter()
                .any(|f| f.invariant == "P1/decision-register-unparsed");
            assert_eq!(
                entries == 0,
                says_unparsed,
                "{root}: a register that parsed {entries} entries must report \
                 P1/decision-register-unparsed if and only if that count is zero: {findings:#?}"
            );
            if says_unparsed {
                unparsed_registers.push(root);
                assert_eq!(
                    findings.len(),
                    1,
                    "a register P1 could not parse has nothing else to report: {findings:#?}"
                );
            }
        }
        assert_eq!(
            unparsed_registers.len(),
            1,
            "exactly one live register (rating's `T-D-NN` table) has no P1-readable entries; \
             a second one appearing means a gear's register drifted out of the recognised \
             shape: {unparsed_registers:#?}"
        );
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
        let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
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
        let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
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
        let findings = check(&corpus, &targets::SeamIndex::default(), &[]);
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/seam-undefined");
        // Low, not Medium — see the severity rationale at the `seam_undefined` loop in
        // `check`. Asserted, not incidental: this is the one finding class whose shape
        // makes a false positive likely (`SEAMS TBD` yields the id `TBD`), so it must
        // never be the thing that fails the default `--max-severity medium` gate.
        assert_eq!(findings[0].severity, Severity::Low);
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
        let loaded = vec![alpha, beta];
        let seams = targets::SeamIndex::build(&loaded);

        let findings = check(&corpus, &seams, &loaded);
        assert_eq!(findings.len(), 1, "unexpected: {findings:#?}");
        assert_eq!(findings[0].invariant, "P1/seam-conflict");
        assert_eq!(findings[0].severity, Severity::Medium);
        assert!(findings[0].message.contains("D-94"));
        assert!(findings[0].message.contains("Z1"));
        assert!(findings[0].message.contains("alpha"));
        assert!(findings[0].message.contains("beta"));
    }
}
