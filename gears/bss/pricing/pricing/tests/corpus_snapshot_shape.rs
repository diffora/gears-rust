//! **Every snapshot in the corpus describes a row that would actually publish.**
//!
//! The corpus is the conformance contract between this catalog and Rating, and
//! a case pins what a row costs — or what publish does with it — by describing
//! the row. So a snapshot the catalog would refuse is a specification of an
//! **impossible row**: it teaches both gears the arithmetic of something that
//! can never be published, and it teaches it in the one artifact whose whole
//! purpose is to be the agreed reading.
//!
//! It also breaks the cases quietly rather than loudly. A publish pair authored
//! short stops at the first row-shape rejection and the guard the case exists to
//! test is never reached — that is exactly how five `supersession-continuity`
//! pairs sat green-looking against `EVAL_POLICY_MISSING`. An evaluation case
//! cannot fail that way at all, because no evaluator runs the publish rules:
//! nothing anywhere was checking the rows those cases describe. This test is
//! that check.
//!
//! It lives gear-side because only the gear has the rules
//! (`price_row_rules`, `design/03-price-structure.md`). `bss-fixtures` cannot
//! run them — it is the crate a gear takes as a production dependency, and the
//! corpus's standing invariant is that no evaluator, and no gear's rule set,
//! reaches it.
//!
//! ## Why the shape rules and not the whole validator
//!
//! The row-shape pipeline is asked of **one row**; the publish validator is
//! asked of a **pair**, and half the corpus's snapshots have no pair. Running
//! the shape rules directly is therefore the only form of the question every
//! snapshot can answer, and it is the one the pair guard presupposes anyway:
//! the validator runs shape first precisely because a comparison between two
//! rows only means something once both would have been accepted on their own.
//!
//! The projection used here is [`validator::slice3_row`], which does **not**
//! apply the validator's unrepresentable-field gate. That gate declines a
//! snapshot carrying a field the gear cannot hold — `proration_basis`, and the
//! Slice-10 reservation pair — and declining is right when the question is
//! "what does publish do with this row". It is wrong here: those fields belong
//! to other slices, the Slice-3 rules have nothing to say about them, and the
//! Slice-3 *part* of a `proration` or `reserved` snapshot is still a row whose
//! shape must hold. Excusing those two families would exempt eight of the
//! corpus's twenty-seven cases from the rule this test exists to enforce.

#![allow(clippy::expect_used, clippy::unwrap_used)]

// Only the snapshot -> row projection is used here; the pair guard and the
// report helpers are `corpus_publish.rs`'s business. Re-authoring the
// projection in this file would create a second place for the corpus and the
// gear to disagree about what a row is, which is the failure the corpus exists
// to prevent.
#[allow(dead_code)]
#[path = "../examples/regen_registry/validator.rs"]
mod validator;

use bss_fixtures::{Case, Corpus, Snapshot};
use bss_pricing::domain::rules::price_row_rules;

/// The snapshots a case carries, each labelled as it reads in a failure line.
///
/// A publish case is checked on **both** sides. The successor is the row under
/// test, but a predecessor that would not publish is a predecessor that was
/// never in the catalog — so the pair it forms is a supersession of something
/// that never existed, and whatever the case then asserts about the guard is
/// asserted about a hypothetical.
fn snapshots(case: &Case) -> Vec<(&'static str, &Snapshot)> {
    match case {
        Case::Evaluation(c) => vec![("snapshot", &c.snapshot)],
        Case::Publish(c) => vec![("predecessor", &c.predecessor), ("successor", &c.successor)],
    }
}

#[test]
fn every_snapshot_in_the_corpus_describes_a_publishable_row() {
    let corpus = Corpus::load(&Corpus::corpus_root()).expect("corpus loads");
    let rules = price_row_rules();
    let mut findings: Vec<String> = Vec::new();

    for case in &corpus.cases {
        for (side, snapshot) in snapshots(case) {
            let row = match validator::slice3_row(snapshot) {
                Ok(row) => row,
                Err(error) => {
                    findings.push(format!(
                        "{id} [{side}]: the gear's row shape cannot hold this snapshot: {error}",
                        id = case.id()
                    ));
                    continue;
                }
            };
            for violation in rules.run(&row).violations {
                findings.push(format!(
                    "{id} [{side}]: {code} -- {detail}",
                    id = case.id(),
                    code = violation.code,
                    detail = violation.detail
                ));
            }
        }
    }

    assert!(
        findings.is_empty(),
        "the corpus describes {n} row(s) the catalog would refuse to publish. A case pins what a \
         row costs, so a snapshot that could not publish specifies an impossible row -- and a \
         publish pair authored short stops at the row-shape rejection below, never reaching the \
         guard it exists to test.\n\nAuthor the WHOLE row, not the delta \
         (gears/bss/fixtures/README.md, `Adding a case`), and never move an expected value to \
         make one pass:\n\n  {findings}\n",
        n = findings.len(),
        findings = findings.join("\n  ")
    );
}
