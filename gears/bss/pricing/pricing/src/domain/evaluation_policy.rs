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
//!
//! # Two rostered fields have no rules and no compile, and publish will freeze them
//!
//! `tier_qualification_window` and `included_allowance` are in the roster below
//! and **Slice 10 has landed nothing else**: not one of the ten refusals
//! `inst-ac-gate` / `inst-tt-forbidden` / `inst-tt-window-pair` /
//! `inst-tt-zero-band` / `inst-tt-fixture` state, and not the allowance compile
//! (`inst-ac-band`, `inst-ac-marker`, `inst-ac-carry`) that gives the declaration
//! its meaning. So the generation stamp tells a consumer these two fields are part of the set an
//! evaluator reads, and nothing in this gear judges either value or honours the
//! allowance.
//!
//! What holds the line is **three refusals, and the load-bearing one is a rule**:
//! [`crate::domain::publish::rules::NoUnjudgedPrimitive`] is registered in the
//! publish set and refuses either field at precheck and again inside the commit
//! transaction; `api::rest::prices::refuse_unlanded_primitives` rejects a non-null
//! value on `POST …/plans/{planId}/prices` and `PATCH …/prices/{priceId}`; and
//! `domain::import` inherits the same code on the bulk plane. None of the three is
//! a property of the type, the column or the roster — the domain model, the
//! storage round trip and the D-129 supersession guard all carry both fields quite
//! happily.
//!
//! **This paragraph said "the freeze is one route away", and that route is now
//! mounted** (D-298). `POST …/plans/{planId}/publish` is merged in `module.rs` and
//! declared in `module_test`'s path census; the sentence predicting the freeze
//! would go live the day Slice 5 mounted it was falsified by the mount, and the
//! freeze did not happen, because the publish rule landed with it. Corrected in
//! place rather than deleted: the danger it described is real, and the reason it
//! is closed should be readable by whoever next widens this roster. A group that
//! adds a writer of `PriceRow` still owes the ten
//! refusals or a second refusal at its own boundary. No DTO in this gear sets
//! `deny_unknown_fields`, so the surface refusal is also contingent on both
//! members remaining modelled fields rather than silently ignored ones (D-174
//! clause 1).

use crate::domain::contracts::PlanChangeContract;
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
pub const EVALUATION_POLICY_GENERATION: &str = "ep-4";

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
        reserved_rate_minor: _,
        reservation_flavor: _,
        min_qty_purchase: _,
        min_qty_usage: _,
        min_qty_usage_fallback: _,
        discount_ref: _,
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
        "reservation_flavor",
        "min_qty_usage",
        "min_qty_usage_fallback",
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
        "reserved_rate_minor",
        "min_qty_purchase",
        "discount_ref",
    ];

    (roster, outside)
}

/// The same partition over the **plan-scoped** contract, added by D-113's field.
///
/// # Why a second function rather than a wider first one
///
/// D-162 built the guard around [`PriceRow`] because every rostered field was a
/// row field, and it warned in the same breath that *"a slice that lands \[a
/// plan-scoped member\] and leaves the roster alone leaves the generation claiming
/// more than it covers."* Slice 6 landed exactly that: `usage_counter_on_plan_change`
/// is frozen with the snapshot — D-113 exists because two gears were already reading
/// it *"from the pinned snapshot"* while no gear defined it — so it is evaluation
/// policy by the roster's own test, and it is not on a row.
///
/// The exhaustive destructure is what makes either function worth having, and it
/// cannot span two structs: a pattern that misses a new field must fail to compile.
/// So the roster is the **union** of the two partitions, and each struct keeps its
/// own arm.
///
/// # Why only one of the three is rostered
///
/// `allowed_change_targets` is a permission — whether a self-service change may
/// happen at all — and `comparability_rank` drives the proration sign Subscriptions
/// computes. Neither changes how a rated line is evaluated, which is the roster's
/// subject; both are consumer-contract inputs that a snapshot carries for other
/// reasons.
#[must_use]
pub fn partition_plan_fields(
    contract: &PlanChangeContract,
) -> (Vec<&'static str>, Vec<&'static str>) {
    let PlanChangeContract {
        allowed_change_targets: _,
        comparability_rank: _,
        usage_counter_on_plan_change: _,
    } = contract;

    let roster = vec!["usage_counter_on_plan_change"];
    let outside = vec!["allowed_change_targets", "comparability_rank"];

    (roster, outside)
}

#[cfg(test)]
#[path = "evaluation_policy_tests.rs"]
mod evaluation_policy_tests;
