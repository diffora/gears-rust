//! Backtest: run the invariants against the pricing docs as of commit 10073c36,
//! the tree before the 2026-07-29 review fixes.
//!
//! The design spec
//! (`docs/superpowers/specs/2026-07-29-bss-spec-ir-verification-design.md`)
//! gates promotion of this mechanism on rediscovery: >= 15 of the 24
//! mechanically-checkable findings after migration steps 1-3. Step 1 ships three
//! invariants, so the number here is a floor, not the final score.
//!
//! One deliberate deviation from the task brief this file was written against: its
//! sanity check anchored on D-53 — the design spec's own flagship example (its
//! Invariant-1 table row reads "would have caught: D-53; the whole D-68 class"). Measured
//! against the real historical corpus, P1 raises no finding for D-53: D-53's own
//! `**Propagated**:` field names only `S10` and `S3` — both already cite `D-53` — and
//! never names the PRD at all, in either the historical or the current `DECISIONS.md`
//! (byte-identical for D-53's own entry; the review closed the PRD gap through a *new*
//! decision, D-68, not by editing D-53's field). P1 checks "every file this decision
//! *names* contains a reference to it"; a target that was never named is invisible to it
//! by construction. So a whole-field propagation omission — the exact shape of D-53's
//! defect — is outside what this invariant can mechanically catch; only a
//! named-but-uncited target is. D-34 and D-36 below are real, rediscovered instances of
//! that second, actually-detectable shape.
//!
//! `records_the_step1_backtest_score` pins 27 P1 findings. **2 of those 27 are not
//! historical debt**: D-44 (`SEAMS M10`) and D-46 (`SEAMS RG3`) are real, defined seam ids
//! — genuinely present in rating's and subscriptions' own `SEAMS.md` — that read as
//! `P1/seam-undefined` here only because this backtest loads the pricing corpus alone, so
//! neither sibling `SEAMS.md` is loaded for `SeamIndex` to resolve against. A real
//! multi-gear run (as `main.rs` does) would not flag either. Left in the pin rather than
//! filtered out, because the pin's job is "what this exact fixture, checked exactly this
//! way, produces" — but anyone reading 27 as "27 rediscovered defects" would overcount by
//! these 2 and might go looking for a fix that doesn't exist on the pricing side.

use std::path::PathBuf;

use spec_check::invariants::closure::DeclaredInstructions;
use spec_check::targets::SeamIndex;
use spec_check::{Corpus, invariants};

fn historical() -> Corpus {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gears/bss/pricing/docs");
    Corpus::load(&root).expect("historical pricing corpus loads")
}

/// D-34 and D-36 each declare `PRD fr-scheduled-migration` in their own
/// `**Propagated**:` field (unlike D-53, which never names the PRD at all), and
/// historically the PRD contained neither `D-34` nor `D-36` anywhere — confirmed against
/// this fixture. The 2026-07-29 review closed both gaps with one new "Migration execution
/// handshake" paragraph in PRD.md's `fr-scheduled-migration` entry that cites both ids
/// (D-38 too, but D-38's own field never names the PRD either — the same blind spot as
/// D-53, so it is not asserted here). Neither id appears in the current
/// `PINNED_PROPAGATION_GAPS_2026_07_29` baseline, confirming the gap was actually closed
/// rather than merely narrowed.
#[test]
fn rediscovers_d34_and_d36_never_reaching_the_prd() {
    let corpus = historical();
    let seams = SeamIndex::build(std::slice::from_ref(&corpus));
    let findings = invariants::propagation::check(&corpus, &seams);
    for id in ["D-34", "D-36"] {
        assert!(
            findings
                .iter()
                .any(|f| f.invariant == "P1/propagation-missing"
                    && f.message.contains(id)
                    && f.message.contains("PRD.md")),
            "{id}'s propagation gap into the PRD is a real, mechanically-detected instance \
             of the same failure mode D-53 exemplifies; findings: {findings:#?}"
        );
    }
}

#[test]
fn rediscovers_the_two_unclaimed_requirements() {
    let findings = invariants::fr_coverage::check(&historical());
    let unclaimed: Vec<_> = findings
        .iter()
        .filter(|f| f.invariant == "P2/fr-unclaimed")
        .collect();
    assert_eq!(
        unclaimed.len(),
        2,
        "expected fr-level-aggregation and fr-trailing-tier-qualification; got {unclaimed:#?}"
    );
}

/// Pinned per-invariant finding counts against the frozen `10073c36` fixture
/// (2026-07-29), verified by hand against the corpus, not derived from a first run of
/// this test. Exact, not a floor — the fixture cannot change (its contents must always
/// equal what `git archive 10073c36 gears/bss/pricing/docs` produces), so these are a
/// stable pin, checked the same way the sibling pinned baselines in `propagation.rs`
/// (`PINNED_PROPAGATION_GAPS_2026_07_29`) and `closure.rs`
/// (`PINNED_UNREFERENCED_CODES_2026_07_29`) are: `records_the_step1_backtest_score` fails
/// if the real count moves in *either* direction, not just downward. 2 of `PINNED_P1`'s
/// 27 are the single-corpus seam artifact described in this file's module doc comment
/// (D-44, D-46) — not historical debt.
///
/// A failure here is a real claim about the checker's effectiveness: a change to
/// `invariants::{propagation,fr_coverage,closure}` moved how many of the historical
/// corpus's real defects it catches. Verify the new count by hand before trusting it —
/// the same way the D-53 -> D-34/D-36 substitution above was independently verified, not
/// asserted — then update these constants deliberately, in the same commit as the change
/// that moved them, with the new number and the reasoning in the commit message. Never
/// edit these to quietly re-baseline a failing run back to green.
const PINNED_P1: usize = 27;
const PINNED_P2: usize = 3;
const PINNED_P3: usize = 55;
const PINNED_TOTAL: usize = PINNED_P1 + PINNED_P2 + PINNED_P3;

/// Records the score. Fails in both directions against the pinned counts above — a
/// regression collapsing 85 to some other nonzero number, or an unnoticed explosion,
/// must be as loud as a drop to zero.
#[test]
fn records_the_step1_backtest_score() {
    let corpus = historical();
    let seams = SeamIndex::build(std::slice::from_ref(&corpus));
    let declared = DeclaredInstructions::build(std::slice::from_ref(&corpus));
    let p1 = invariants::propagation::check(&corpus, &seams).len();
    let p2 = invariants::fr_coverage::check(&corpus).len();
    let p3 = invariants::closure::check(&corpus, &declared).len();
    let total = p1 + p2 + p3;
    println!("step-1 backtest: {total} finding(s) against 10073c36 (P1 {p1}, P2 {p2}, P3 {p3})");

    assert_eq!(
        p1, PINNED_P1,
        "P1/propagation count drifted from the pinned {PINNED_P1} (got {p1}) — this is a \
         claim about the checker's effectiveness, not a number to quietly re-baseline: \
         verify the new count by hand, then update PINNED_P1 deliberately with the \
         reasoning in the commit message"
    );
    assert_eq!(
        p2, PINNED_P2,
        "P2/fr-coverage count drifted from the pinned {PINNED_P2} (got {p2}) — this is a \
         claim about the checker's effectiveness, not a number to quietly re-baseline: \
         verify the new count by hand, then update PINNED_P2 deliberately with the \
         reasoning in the commit message"
    );
    assert_eq!(
        p3, PINNED_P3,
        "P3/closure count drifted from the pinned {PINNED_P3} (got {p3}) — this is a \
         claim about the checker's effectiveness, not a number to quietly re-baseline: \
         verify the new count by hand, then update PINNED_P3 deliberately with the \
         reasoning in the commit message"
    );
    assert_eq!(
        total, PINNED_TOTAL,
        "step-1 backtest total drifted from the pinned {PINNED_TOTAL} (got {total}) — this \
         is a claim about the checker's effectiveness, not a number to quietly \
         re-baseline: verify the new total by hand, then update the pinned constants \
         deliberately with the reasoning in the commit message"
    );
}
