//! The `RegisteredValidators` host for `04-lifecycle` (**P-D-97**).
//!
//! # Two fillings of one phase slot, not a second vocabulary
//!
//! `ValidationRule`, `Phase` and `ValidationPipeline` stay foundation's.
//! This feature fills [`Phase::RegisteredValidators`] for the `→ published`
//! target state in either of two ways:
//!
//! 1. A **registered rule** whose operand is subject-local or a single fact
//!    the door prefetches — [`ParentPublishedRequired`] on
//!    [`PublishOrderingSubject`]. The subject carries the parent's
//!    [`LifecycleState`], not a boolean: a boolean loses the terminal
//!    carve-out and would raise `PARENT_NOT_PUBLISHED` on a `retired` /
//!    `discarded` parent (P-D-96). Both fillings call
//!    [`parent_must_be_published`], so they cannot drift.
//! 2. A **continuation** of that phase on the same transaction, positioned
//!    immediately after the pipeline and before the edge and the gate — the
//!    position §4.1 asks for. [`parent_must_be_published`] is that filling
//!    for the same rule.
//!
//! **Residue (P-D-97):** a continuation raises a [`LifecycleRefusal`]
//! directly and does **not** append to a [`ValidationReport`]. It refuses on
//! the first finding. The two fillings are not interchangeable in every
//! respect: only the registered form can collect several violations in one
//! phase. Every cross-row rule here is a single-condition refusal.
//!
//! The insertion site *is* the keying. The lead wires
//! `.with_rule(Box::new(ParentPublishedRequired))` on the SKU publish
//! re-validation pipeline, prefetching the parent's [`LifecycleState`]
//! into [`PublishOrderingSubject`]. Until that line lands the rule type
//! reaches no runtime.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-registered-validator-host:p1
//! @cpt-cf-bss-products-dod-publish-ordering

use bss_products_sdk::models::LifecycleState;

use super::validation::{Phase, ValidationReport, ValidationRule};

/// A lifecycle refusal whose `DomainError` arm is a D7 patch.
///
/// The wire code is the contract today; the arm is applied from that patch.
/// Continuations return this rather than inventing a `DomainError` variant
/// in a forbidden file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleRefusal {
    /// The declared wire code.
    pub code: &'static str,
    /// Human detail. Does **not** name falling-out children (P-D-96 / §7
    /// row 27): both refusals disclose the same amount.
    pub detail: String,
}

impl LifecycleRefusal {
    /// Slice 04's owned slot (P-D-24 prices 409; P-D-96 admits the arm).
    pub const PARENT_NOT_PUBLISHED: &'static str = "PARENT_NOT_PUBLISHED";
    /// Same seam as [`Self::PARENT_NOT_PUBLISHED`] (P-D-96).
    pub const RETIREMENT_PENDING: &'static str = "RETIREMENT_PENDING";
    /// Runner wrapping a stale or missing pinned approval.
    pub const SCHEDULE_STALE_APPROVAL: &'static str = "SCHEDULE_STALE_APPROVAL";
    /// Successor named at retirement initiation is not `published`.
    pub const REPLACED_BY_NOT_PUBLISHED: &'static str = "REPLACED_BY_NOT_PUBLISHED";
    /// Operator `effectiveAt` is earlier than the configured lead.
    pub const RETIREMENT_LEAD_TIME: &'static str = "RETIREMENT_LEAD_TIME";
    /// Product retirement without a confirmed cascade.
    pub const CASCADE_CONFIRMATION_REQUIRED: &'static str = "CASCADE_CONFIRMATION_REQUIRED";
    /// `mustMigrateBy` / consumer-ack while the v1 flag is off.
    pub const EOL_DISABLED: &'static str = "EOL_DISABLED";

    /// Parent is live but not `published`. Terminal parents stay
    /// foundation's `PARENT_TERMINAL`.
    #[must_use]
    pub fn parent_not_published() -> Self {
        Self {
            code: Self::PARENT_NOT_PUBLISHED,
            detail: "the parent Product is not published".to_owned(),
        }
    }

    /// A live retire intent blocks un-deprecation. `named` is entity ids,
    /// not children-outside-scope (different refusal).
    #[must_use]
    pub fn retirement_pending(named: &[impl core::fmt::Display]) -> Self {
        let list = named
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            code: Self::RETIREMENT_PENDING,
            detail: format!("a live retire intent remains on: {list}"),
        }
    }

    /// The runner's own raising door.
    #[must_use]
    pub fn schedule_stale_approval(detail: impl Into<String>) -> Self {
        Self {
            code: Self::SCHEDULE_STALE_APPROVAL,
            detail: detail.into(),
        }
    }

    #[must_use]
    pub fn replaced_by_not_published() -> Self {
        Self {
            code: Self::REPLACED_BY_NOT_PUBLISHED,
            detail: "replacedBy must name a published SKU".to_owned(),
        }
    }

    #[must_use]
    pub fn retirement_lead_time() -> Self {
        Self {
            code: Self::RETIREMENT_LEAD_TIME,
            detail: "effectiveAt is earlier than the configured lead".to_owned(),
        }
    }

    #[must_use]
    pub fn cascade_confirmation_required() -> Self {
        Self {
            code: Self::CASCADE_CONFIRMATION_REQUIRED,
            detail: "a Product retirement over live SKUs requires a confirmed cascade".to_owned(),
        }
    }

    #[must_use]
    pub fn eol_disabled() -> Self {
        Self {
            code: Self::EOL_DISABLED,
            detail: "mustMigrateBy is refused while EOL is disabled".to_owned(),
        }
    }
}

/// The prefetch fact the publish-ordering rule reads (**P-D-97** registered
/// filling). The door reads the parent row; this subject carries its
/// [`LifecycleState`].
///
/// A boolean would collapse `draft`/`deprecated` with `retired`/`discarded`
/// and make the registered filling disagree with
/// [`parent_must_be_published`] on a terminal parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishOrderingSubject {
    /// The parent Product's stored lifecycle state.
    pub parent: LifecycleState,
}

/// `inst-pc-ordering` — a SKU reaching `published` needs a `published` parent.
///
/// Registered on the **target state**, not the edge: the same rule belongs
/// on the first-publish pipeline and on the re-validation re-run.
pub struct ParentPublishedRequired;

impl ParentPublishedRequired {
    /// The instruction this rule answers.
    pub const NAME: &'static str = "inst-pc-ordering";
}

impl ValidationRule<PublishOrderingSubject> for ParentPublishedRequired {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn phase(&self) -> Phase {
        Phase::RegisteredValidators
    }

    fn evaluate(&self, subject: &PublishOrderingSubject, report: &mut ValidationReport) {
        if let Err(refusal) = parent_must_be_published(subject.parent) {
            report.violate(refusal.code, "product_id", refusal.detail);
        }
    }
}

/// Continuation filling of the same rule: refuses on the first finding,
/// no `ValidationReport`.
///
/// A terminal parent is **not** this rule — `PARENT_TERMINAL` already fires
/// on create and `recheck_parent_containment`. Draft and deprecated are.
///
/// # Errors
///
/// [`LifecycleRefusal`] with [`LifecycleRefusal::PARENT_NOT_PUBLISHED`].
pub fn parent_must_be_published(parent: LifecycleState) -> Result<(), LifecycleRefusal> {
    if parent == LifecycleState::Published {
        return Ok(());
    }
    if parent.is_terminal() {
        return Ok(());
    }
    Err(LifecycleRefusal::parent_not_published())
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
