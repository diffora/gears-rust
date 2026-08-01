//! The **supersession unit guard** (`inst-tb-supersession-units`; D-82, D-98,
//! D-122, D-127, D-129).
//!
//! The tier counter `Q` is derived per `(subscription, meter, dimensionKey,
//! window)` and belongs to the subscription's usage history, not to a price-row
//! version. Superseding a row therefore does **not** reset an in-window counter:
//! the successor's bands are simply applied to the continued `Q`.
//!
//! That continuity is what this guard protects. A successor landing on an
//! occupied published canonical scope key MUST NOT change the fields the
//! continued `Q` is **denominated in**, **derived from**, or **priced by** —
//! otherwise the same accumulated number is reinterpreted under a different
//! unit, read off a different stream, or re-priced under a different formula
//! mid-window:
//!
//! - `per_hour -> per_day` applies an hours-denominated `Q` to day-denominated
//!   bands: the D-77 factor-of-24 band-edge class, reintroduced through
//!   supersession.
//! - a changed `meter` or `dimensionKey` silently reads a different counter.
//! - a `graduated -> volume` / `package` flip re-prices the **already
//!   accumulated** window total under new math — `volume` applies the selected
//!   band's single rate to the whole window `Q`, including units already rated
//!   marginally under the predecessor (D-98).
//! - a changed `package_size` re-buckets an already-accumulated `used`, because
//!   rating counts blocks by a cumulative ceil-diff that presupposes one block
//!   size per window (D-122).
//! - a changed `carry` allowance would rewrite a **plan-scoped**,
//!   revision-immutable grant row that a supersession cannot open a revision to
//!   touch (D-129).
//!
//! Supersession is a **price** change on one key: new amounts, new bands. What
//! or how the key meters, and which formula prices it, is **structural** and
//! routes through plan revisioning and migration.
//!
//! ## The guard binds the key, not the mechanism (D-127)
//!
//! [`SupersessionUnitGuard`] takes a predecessor/successor **pair** and has no
//! notion of which mechanism produced it. That is the point. Both sanctioned
//! producers of `published -> superseded` are bound: the interactive
//! supersession unit, and the grandfathering cutover's `all_subscriptions`
//! successor, which lands on the predecessor's own scope key and inherits the
//! identical continued counter. Both set `supersedes_price_id` on the successor,
//! which is what pairs the two rows here.
//!
//! Before D-127 the rule was invoked from the supersession unit alone, so a
//! cutover successor could flip `per_hour -> per_day` — the same band-edge class
//! through its fifth door, on the one path that is always material and therefore
//! felt safe. A signature that asked "which mechanism?" would reopen exactly
//! that hole, so this one cannot ask.

use toolkit_macros::domain_model;

use crate::domain::price_row::PriceRow;
use crate::domain::rules::SUPERSESSION_UNIT_MISMATCH;
use crate::domain::validation::{ValidationReport, ValidationRule};

/// A predecessor and the successor landing on its canonical scope key.
///
/// The pair is the unit of judgement because the guard is a **comparison**:
/// there is no property of the successor alone that says whether it changed a
/// unit field.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupersessionPair {
    /// The published row being closed.
    pub predecessor: PriceRow,
    /// The row landing on its key.
    pub successor: PriceRow,
}

impl SupersessionPair {
    /// Pair a predecessor with its successor.
    #[must_use]
    pub const fn new(predecessor: PriceRow, successor: PriceRow) -> Self {
        Self {
            predecessor,
            successor,
        }
    }

    /// The unit/counter-determining fields the successor changed, in the
    /// spelling the design set names them.
    ///
    /// Empty means the successor is a pure **price** change and may publish. The
    /// list is what the violation names: "publish failed" without saying which
    /// field moved is not remediable, and the whole point of the rule is that
    /// the offending field is structural.
    ///
    /// Deliberately **not** listed, because they are the legitimate price
    /// levers: the band amounts, the band set itself, `amount_minor`,
    /// `package_price_minor`, and a `none`-policy allowance.
    #[must_use]
    pub fn mismatched_unit_fields(&self) -> Vec<&'static str> {
        let (before, after) = (&self.predecessor, &self.successor);
        let mut changed = Vec::new();
        if before.meter != after.meter {
            changed.push("meter");
        }
        if before.dimension_key != after.dimension_key {
            changed.push("dimensionKey");
        }
        if before.model_kind != after.model_kind {
            changed.push("model_kind");
        }
        if before.billing_granularity != after.billing_granularity {
            changed.push("billingGranularity");
        }
        // Read through the defaults: an unauthored `aggregationFunction` and an
        // authored `sum` are the same row, so comparing the raw `Option`s would
        // fail a supersession that changed nothing at all.
        if before.effective_aggregation_function() != after.effective_aggregation_function() {
            changed.push("aggregationFunction");
        }
        if before.effective_aggregation_granularity() != after.effective_aggregation_granularity() {
            changed.push("aggregationGranularity");
        }
        if before.tier_aggregation_window != after.tier_aggregation_window {
            changed.push("tierAggregationWindow");
        }
        // Same reading-through as the two above: the PRD states `current` as
        // this window's default, so an unauthored window and an authored
        // `current` are one row.
        if before.effective_tier_qualification_window()
            != after.effective_tier_qualification_window()
        {
            changed.push("tierQualificationWindow");
        }
        if before.package_size != after.package_size {
            changed.push("package_size");
        }
        if self.carry_allowance_changed() {
            changed.push("included_allowance");
        }
        changed
    }

    /// Did a **plan-scoped** allowance move (D-129)?
    ///
    /// Guarded only where a `carry` policy is involved, on either side. A
    /// `carry` declaration compiles into a plan-scoped, revision-immutable grant
    /// row, so changing one — or introducing one, or removing one — is a change
    /// to an artifact a supersession has no revision in which to rewrite. A
    /// `none`-policy allowance compiles to nothing plan-scoped and stays a free
    /// row-local lever, so `none -> none` quantity changes pass.
    fn carry_allowance_changed(&self) -> bool {
        let (before, after) = (
            self.predecessor.included_allowance,
            self.successor.included_allowance,
        );
        if before == after {
            return false;
        }
        before.is_some_and(|allowance| allowance.rollover_policy.is_carry())
            || after.is_some_and(|allowance| allowance.rollover_policy.is_carry())
    }
}

/// `inst-tb-supersession-units` — the guard itself.
///
/// Binds every **usage** row (`per_unit` / `graduated` / `volume` / `package`).
/// A non-usage key has no continued counter to protect, so the guard says
/// nothing about it and a recurring row's supersession is free to change
/// whatever the row-local rules allow.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct SupersessionUnitGuard;

impl ValidationRule<SupersessionPair> for SupersessionUnitGuard {
    fn name(&self) -> &'static str {
        "inst-tb-supersession-units"
    }

    fn evaluate(&self, subject: &SupersessionPair, report: &mut ValidationReport) {
        if !subject.successor.is_usage() {
            return;
        }
        let changed = subject.mismatched_unit_fields();
        if changed.is_empty() {
            return;
        }
        report.violate(
            SUPERSESSION_UNIT_MISMATCH,
            subject.successor.subject(),
            format!(
                "the successor changes {}; the tier counter continues across supersession, \
                 so what it is denominated in, derived from and priced by must not move. \
                 A structural change routes through plan revisioning, not a supersession",
                changed.join(", ")
            ),
        );
    }
}

#[cfg(test)]
#[path = "supersession_tests.rs"]
mod supersession_tests;
