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

/// Records the score. Update the number deliberately when an invariant lands —
/// a change here is a claim about effectiveness and belongs in the commit message.
#[test]
fn records_the_step1_backtest_score() {
    let corpus = historical();
    let seams = SeamIndex::build(std::slice::from_ref(&corpus));
    let declared = DeclaredInstructions::build(std::slice::from_ref(&corpus));
    let total = invariants::propagation::check(&corpus, &seams).len()
        + invariants::fr_coverage::check(&corpus).len()
        + invariants::closure::check(&corpus, &declared).len();
    println!("step-1 backtest: {total} finding(s) against 10073c36");
    assert!(
        total > 0,
        "a checker that finds nothing in a known-bad tree is broken"
    );
}
