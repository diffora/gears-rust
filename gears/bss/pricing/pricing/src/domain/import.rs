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
//! fail Phase 1 per-row. The subtlety is which key: not the whole [`ScopeKey`],
//! but [`ScopeKey::draft_row_identity`] — every axis except `meter` and
//! `dimension_key`, which is the column list of the draft plane's partial
//! `UNIQUE`. **Two rows differing only in their usage line collide**, and an
//! equality over the whole key would call them distinct and let both through to
//! fail at commit, which is the outcome D-148 moved into Phase 1 precisely
//! because commit is per-row and cannot report a collision as a batch fault.
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
        !self.rows.is_empty()
    }
}

/// Run Phase 1's **batch-only** rules over the submitted rows.
///
/// Every rule answers for every row; nothing short-circuits. The store-dependent
/// rules are the caller's to add to the same report.
#[must_use]
pub fn classify(rows: &[ImportRow]) -> BatchReport {
    let mut violations: HashMap<usize, Vec<RowViolation>> = HashMap::new();
    for (row, found) in duplicate_scope_keys(rows) {
        violations.entry(row).or_default().push(found);
    }

    let mut outcomes: Vec<RowOutcome> = violations
        .into_iter()
        .map(|(row, violations)| RowOutcome { row, violations })
        .collect();
    outcomes.sort_by_key(|outcome| outcome.row);
    BatchReport { rows: outcomes }
}

/// Rows sharing one draft-row identity, each told which other rows it collides
/// with.
fn duplicate_scope_keys(rows: &[ImportRow]) -> Vec<(usize, RowViolation)> {
    let mut occupants: HashMap<_, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        occupants
            .entry(row.scope_key.draft_row_identity())
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
