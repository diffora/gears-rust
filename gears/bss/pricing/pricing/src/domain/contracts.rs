//! Slice 6's consumer contracts: the fields a downstream system computes from
//! (`design/06-consumer-contracts.md`).
//!
//! The catalog publishes **inputs**; the math and the enforcement live
//! downstream. Every rule here is therefore a presence/consistency statement
//! about what a publish freezes, never a computation over it — `inst-rc-nocompute`
//! is the slice's own name for that boundary and it is why no rule in this
//! module reads an amount.
//!
//! # Why the subject is the [`PlanShape`] and not a [`PriceRow`]
//!
//! Slice 3's row rules judge a [`PriceRow`](crate::domain::price_row::PriceRow),
//! which deliberately carries no identity and none of the columns this slice
//! owns: `billing_timing` is a column of the
//! [`PriceRecord`](crate::domain::price_record::PriceRecord) that wraps a row,
//! not of the row. A rule here could not read its own field from Slice 3's
//! subject, so the subject is the plan and each rule walks `shape.rows`.
//!
//! That is also what the market-uniformity rule needs (`inst-pi-uniform`): a
//! statement about a *set* of rows crossed with a set of markets has no
//! row-local form at all.

use crate::domain::plan_shape::PlanShape;
use crate::domain::scope_key::ChargeKind;
use crate::domain::validation::{ValidationPipeline, ValidationReport, ValidationRule};

/// A published recurring row carries no `billingTiming`
/// (`06-consumer-contracts.md` §3 `inst-bt-required`, §5).
///
/// The code is §5's verbatim. **Slice 2 names it too** and does not register it:
/// `plan_rules::cycle_shape`'s module doc records that `billingTiming` REQUIRED
/// on a recurring row is Slice 6's rule, cross-referenced and never
/// re-registered, while Slice 2 owns the opposite statement about a *setup* row
/// (`inst-cs-setup`, `SETUP_ROW_INVALID`). The two can never disagree because
/// the `chargeKind` axis makes no row both.
pub const BILLING_TIMING_MISSING: &str = "BILLING_TIMING_MISSING";

/// Every recurring row of the publish subject states its `billingTiming`.
///
/// **Recurring only, and every recurring row.** Usage and one-time rows do not
/// author the field — `inst-bt-usage` projects a constant for them — and a rule
/// that demanded it there would reject a row whose value is not the author's to
/// give. `existing_grandfathered` rows are *not* excluded: D-132's exclusion is
/// an argument about comparing a frozen generation against current rows, which
/// is the uniformity rule's problem and not this one. A grandfathered row that
/// published without the field would leave Billing deriving a deferral policy
/// for a live subscriber by heuristic, which is exactly what `inst-bt-frozen`
/// exists to prevent.
pub struct BillingTimingPresent;

impl ValidationRule<PlanShape> for BillingTimingPresent {
    fn name(&self) -> &'static str {
        "inst-bt-required"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        for record in &subject.rows {
            if !is_recurring(record.scope_key.charge_kind()) || record.billing_timing.is_some() {
                continue;
            }
            report.violate(
                BILLING_TIMING_MISSING,
                record.price_id.to_string(),
                "a recurring row MUST state its billingTiming (advance | arrears): Billing \
                 derives its deferral policy from the frozen value and never from a heuristic, so \
                 an absent one is a row no consumer can defer correctly (inst-bt-required, \
                 inst-bt-frozen)"
                    .to_owned(),
            );
        }
    }
}

/// Slice 6's registered set over one publish subject.
#[must_use]
pub fn consumer_contract_rules() -> ValidationPipeline<PlanShape> {
    ValidationPipeline::new().with_rule(Box::new(BillingTimingPresent))
}

/// Is this row one the recurring-row contracts apply to?
fn is_recurring(charge_kind: ChargeKind) -> bool {
    matches!(charge_kind, ChargeKind::Recurring)
}

#[cfg(test)]
#[path = "contracts_tests.rs"]
mod contracts_tests;
