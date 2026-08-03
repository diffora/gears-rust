//! The evaluation-policy generation — `pricingSnapshotRef`'s third segment.
//!
//! A snapshot ref names the resolved rows (the price ids), the version they were
//! resolved at (the version ref), and **which evaluation-policy field set the
//! frozen row content is to be read under**. That third segment is this module:
//! a declared constant, `ep-<n>`, that moves when a decision moves the field
//! set (`design/01-foundation.md` §4.4, D-162).
//!
//! # Why a constant needs a guard more than it needs a value
//!
//! The segment exists so that a period rated under one evaluation semantics is
//! tellable from a period rated under another. A generation nobody remembers to
//! bump is therefore **worse than no generation**: it asserts a stability that
//! is not there, immutably, on posted money. So the deliverable here is not
//! [`EVALUATION_POLICY_GENERATION`] — it is the chain that fails when the field
//! set moves and the constant does not, and the chain is deliberately anchored
//! outside this file:
//!
//! 1. [`partition_row_fields`] destructures [`PriceRow`] with a pattern that has
//!    **no `..` arm**. Adding a field to the row does not compile here (E0027)
//!    until whoever added it says which side of the roster boundary it falls on.
//!    That is the step an author cannot route around, and it is why the function
//!    takes a row it never reads: Rust has no way to match a struct's fields
//!    exhaustively without a value of it.
//! 2. The **document** is the statement, not this file. `01-foundation.md` §4.4
//!    declares the roster as an append-only log of `ep-<n>  <decision>  ± field`
//!    lines, plus the out-of-roster set. `evaluation_policy_tests.rs` reads that
//!    block, replays the log, and asserts both halves of the partition against
//!    it. A test that merely restated this crate's fields would prove nothing —
//!    the same edit that adds a field would update it.
//! 3. The bump is therefore a **mechanism rather than a request**: a field
//!    cannot enter the roster without a log line, a log line *is* a generation,
//!    the last generation is the declared one, and the declared one is
//!    [`EVALUATION_POLICY_GENERATION`]. There is no edit that adds a rostered
//!    field and leaves the constant where it was.
//!
//! Backdating a field into an existing log line is still possible, and is
//! deliberately not defended against: it falsifies what a numbered decision
//! admitted, in a normative document, which is a different act with a different
//! reviewer.
//!
//! # What the generation does not claim
//!
//! It tracks the field **set**, not the meaning of a field. D-58 made
//! `tier_aggregation_window` mandatory on `package` rows and would not move it;
//! a decision redefining `peak` would not move it either. That is the coverage a
//! content digest would have bought and this does not, and it is stated in §4.4
//! for the same reason it is stated here — a reader is entitled to know which
//! risk they are carrying.

use crate::domain::price_row::PriceRow;

/// The evaluation-policy generation this gear stamps.
///
/// Format `ep-<n>`, `n` a positive integer, monotone, and **opaque to consumers
/// except for equality**: two snapshots carrying this string were frozen under
/// the same evaluation-policy field set, and two carrying different strings were
/// not. Nothing else is promised.
///
/// It is the gear's constant rather than a per-publish value — every publish of
/// one generation stamps the same string — and it moves only with the log in
/// `design/01-foundation.md` §4.4, whose last entry it must equal.
pub const EVALUATION_POLICY_GENERATION: &str = "ep-1";

/// Every field of [`PriceRow`], sorted into the evaluation-policy roster and out
/// of it.
///
/// The roster is the fields that tell an evaluator **how to derive the billable
/// quantity and select the rate**. Outside it sit the row's identity (the
/// scope-key axis and the metered line it prices), its money, and the fields
/// saying where a quantity *comes from* rather than how it is *derived* — the
/// distinction `EVAL_POLICY_MISPLACED` already draws by naming "an
/// evaluation-policy **or** quantity field".
///
/// Returns `(roster, outside)`, each in this row's declaration order. Both
/// halves are returned because both are pinned: a field added to the row and
/// quietly filed outside the roster would otherwise change nothing anyone has to
/// record, and the guard's job is to make every field of this row a stated
/// classification rather than a silent one.
///
/// # The pattern is the point
///
/// The `let PriceRow { .. }` below has **no rest pattern**. A field added to
/// [`PriceRow`] is a compile error here until it is named, which is the one step
/// an author adding a field cannot route around. The `row` argument exists only
/// to give that pattern something to match.
#[must_use]
pub fn partition_row_fields(row: &PriceRow) -> (Vec<&'static str>, Vec<&'static str>) {
    let PriceRow {
        charge_kind: _,
        model_kind: _,
        amount_minor: _,
        bands: _,
        package_size: _,
        package_price_minor: _,
        quantity_source: _,
        manual_quantity: _,
        meter: _,
        dimension_key: _,
        billing_granularity: _,
        tier_aggregation_window: _,
        tier_qualification_window: _,
        aggregation_function: _,
        aggregation_granularity: _,
        max_hold_granules: _,
        included_allowance: _,
    } = row;

    let roster = vec![
        "model_kind",
        "package_size",
        "billing_granularity",
        "tier_aggregation_window",
        "tier_qualification_window",
        "aggregation_function",
        "aggregation_granularity",
        "max_hold_granules",
        "included_allowance",
    ];
    let outside = vec![
        "charge_kind",
        "amount_minor",
        "bands",
        "package_price_minor",
        "quantity_source",
        "manual_quantity",
        "meter",
        "dimension_key",
    ];

    (roster, outside)
}

#[cfg(test)]
#[path = "evaluation_policy_tests.rs"]
mod evaluation_policy_tests;
