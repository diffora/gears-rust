//! Bulk import, Phase 1 — the checks that need no store
//! (`design/12-operator-efficiency.md` §3 `algo-bulk-import` `inst-bk-phase1`,
//! §4 `inst-bi-validate`; D-118, D-148).
//!
//! # Why a Phase 1 at all, and why part of it lives here
//!
//! `inst-bk-phase1` is **validate all-or-nothing**: one invalid row blocks the
//! whole batch pre-commit, with a per-row report naming every violation. That
//! posture only pays for itself if the report is complete — an operator fixing a
//! batch one refusal at a time is doing what the all-or-nothing rule exists to
//! spare them — so a rule that can answer for a row **must** answer for every row
//! rather than stopping at the first.
//!
//! Phase 1's rules split by what they need. Some need the store: whether the row
//! addresses a **published** scope key (`IMPORT_TARGETS_PUBLISHED`, D-118),
//! whether its key holds a pending approval unit (D-35), whether its `If-Match`
//! is current. Others need only the batch, and those are here. Keeping them apart
//! is not tidiness: the batch-only rules are the ones a caller can run before it
//! has opened anything, and they are the ones whose behaviour a test can state
//! without building a world.
//!
//! # The in-batch duplicate, and the key it is judged on (D-148)
//!
//! Two rows on one canonical scope key are a duplicate **inside** the batch and
//! fail Phase 1 per-row. The key is the **whole** [`ScopeKey`], usage line
//! included, which is the column list of `uq_pricing_price_scope_key_draft` as
//! `m20260802_000023` **widened** it: `COALESCE(meter, '')` and `dimension_key`
//! are in the index, so two rows differing only in their usage line are two
//! keys and both author.
//!
//! **This was got wrong once and the wrong answer is worth keeping visible**
//! (D-283). The first build read `m20260802_000002`, which created the index in
//! its original eight-axis form, and judged the duplicate on those eight — so it
//! refused a batch the store admits, and the batch it refused was D-103's
//! confirmed worked example, a `PaaS` plan pricing cloudlets, storage and egress.
//! D-196 made the usage pair normative axes of the canonical key precisely to
//! make that plan expressible. **Read the schema, not a migration**: a `CREATE
//! INDEX` is a statement about the day it ran.
//!
//! # Both sides of a duplicate are named
//!
//! A collision has two rows in it and neither is more at fault than the other:
//! the operator has to see both to decide which one they meant. Reporting only
//! the second would also make the report depend on the order the rows arrived in,
//! which is not a property of the batch.

use std::collections::HashMap;

use toolkit_macros::domain_model;

use crate::domain::concurrency::RowVersion;
use crate::domain::price_record::PriceContent;
use crate::domain::publish::rules::{PRIMITIVE_RULES_UNBUILT, unjudged_primitives};
use crate::domain::scope_key::ScopeKey;

/// The wire code for two rows on one canonical scope key.
///
/// The same code the store raises when a row loses that race to a concurrent
/// author (`infra::error_mapping`); one collision, one code, whichever end of the
/// pipeline notices it. Phase 1 catching it earlier is a better *report*, not a
/// different fault.
pub const DUPLICATE_SCOPE_KEY: &str = "DUPLICATE_SCOPE_KEY";

/// One row an operator submitted.
///
/// `if_match` is `None` for a row claiming a **new** scope key and `Some` for a
/// row editing an existing draft under its version (`inst-bk-phase2`: the token
/// is the price row's own column, D-141). Phase 1 does not resolve it — that
/// needs the store — but it is carried here because the duplicate rule below has
/// to treat "a new row on this key" and "an edit of the row on this key" as the
/// same occupancy: two rows aimed at one draft row collide whether or not either
/// claims to own it already.
#[domain_model]
#[derive(Clone, Debug)]
pub struct ImportRow {
    /// The canonical key the row would occupy.
    pub scope_key: ScopeKey,
    /// The authored content.
    pub content: PriceContent,
    /// The version the row asserts, when it edits an existing draft.
    pub if_match: Option<RowVersion>,
}

/// One violation against one row.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowViolation {
    /// The machine-readable code, which is the discriminator a client acts on.
    pub code: &'static str,
    /// The sentence an operator reads.
    pub detail: String,
}

/// What Phase 1 found against one row, by its position in the submitted batch.
///
/// The **index** identifies the row rather than its key, because a row whose key
/// is the problem cannot be named by that key without ambiguity — that is the
/// duplicate case exactly, where two rows share one.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowOutcome {
    /// Zero-based position in the submitted batch.
    pub row: usize,
    /// Every violation found against it, never only the first.
    pub violations: Vec<RowViolation>,
}

/// Phase 1's per-row report over a batch.
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchReport {
    /// One entry per row that failed, in batch order. A row that passed every
    /// batch-only rule has **no** entry — the report is of violations, and a
    /// clean row's absence is what "nothing to fix here" looks like.
    pub rows: Vec<RowOutcome>,
}

impl BatchReport {
    /// Whether Phase 1 blocks the batch (`inst-bk-phase1`, O2).
    ///
    /// Any violation at all blocks it: all-or-nothing is the whole posture, and a
    /// report that let some rows through would make "nothing partially validated
    /// sneaks through" untrue by construction.
    #[must_use]
    pub fn blocks_the_batch(&self) -> bool {
        self.rows
            .iter()
            .any(|outcome| !outcome.violations.is_empty())
    }
}

/// Run Phase 1's **batch-only** rules over the submitted rows.
///
/// Every rule answers for every row; nothing short-circuits.
///
/// The store-dependent rules will join the same report, and there is **no merge
/// API yet** — this function is the only thing that establishes
/// [`BatchReport`]'s one-entry-per-row invariant, and a caller appending to
/// `rows` would break it silently. Whoever builds that half owes the merge, not
/// an append.
#[must_use]
pub fn classify(rows: &[ImportRow]) -> BatchReport {
    let mut violations: HashMap<usize, Vec<RowViolation>> = HashMap::new();
    for (row, found) in duplicate_scope_keys(rows) {
        violations.entry(row).or_default().push(found);
    }
    for (row, found) in unbuilt_primitives(rows) {
        violations.entry(row).or_default().push(found);
    }
    for found in violations.values_mut() {
        // A stable order, so a report is a function of the batch and not of the
        // order the rules happen to run in. Rows already sort by index; this is
        // the same claim one level down.
        found.sort_by_key(|violation| violation.code);
    }

    let mut outcomes: Vec<RowOutcome> = violations
        .into_iter()
        .map(|(row, violations)| RowOutcome { row, violations })
        .collect();
    outcomes.sort_by_key(|outcome| outcome.row);
    BatchReport { rows: outcomes }
}

/// Rows carrying a Slice-10 primitive whose rules are unbuilt (D-177).
///
/// **The refusal moves earlier, it does not move.** `domain::publish::rules`
/// already refuses these fields at publish on its own authority (D-179), whatever
/// put the value there, and that backstop stays — its removal has an order, and
/// this is not it. What building this arm changes is *where the operator hears*:
/// in the per-row Phase 1 report, where the batch can still be fixed, instead of
/// at a publish that rejects work already committed.
///
/// It **inherits** `PRIMITIVE_RULES_UNBUILT` rather than minting a second code,
/// which the design set states outright: the code names the reason, not the
/// field, and one reason wants one code however many surfaces notice it.
fn unbuilt_primitives(rows: &[ImportRow]) -> Vec<(usize, RowViolation)> {
    let mut found = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        for (field, missing) in unjudged_primitives(&row.content.row) {
            found.push((
                index,
                RowViolation {
                    code: PRIMITIVE_RULES_UNBUILT,
                    detail: format!(
                        "this row carries a {field} and the Slice-10 rules that would judge it \
                         are unbuilt, as is {missing}; the batch stops here, where the row can \
                         still be fixed, rather than at the publish that would otherwise catch it"
                    ),
                },
            ));
        }
    }
    found
}

/// Rows sharing one draft-row identity, each told which other rows it collides
/// with.
fn duplicate_scope_keys(rows: &[ImportRow]) -> Vec<(usize, RowViolation)> {
    let mut occupants: HashMap<_, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        occupants
            .entry(row.scope_key.clone())
            .or_default()
            .push(index);
    }

    let mut found = Vec::new();
    for (_, group) in occupants {
        if group.len() < 2 {
            continue;
        }
        for &index in &group {
            let others: Vec<String> = group
                .iter()
                .filter(|&&other| other != index)
                .map(ToString::to_string)
                .collect();
            found.push((
                index,
                RowViolation {
                    code: DUPLICATE_SCOPE_KEY,
                    detail: format!(
                        "this row occupies the same draft row as row(s) {} — {}; the draft \
                         plane admits one row per canonical scope key, and two rows differing \
                         only in their usage line are the same row",
                        others.join(", "),
                        rows[index].scope_key
                    ),
                },
            ));
        }
    }
    found
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod import_tests;
