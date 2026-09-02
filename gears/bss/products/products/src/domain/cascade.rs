//! Product-retirement cascade plan, the parent's own path, and deferred
//! intent (`dod-cascade-plan`, `dod-cascade-parent-path`,
//! `dod-deferred-intent`).
//!
//! # Three arms, one transaction
//!
//! [`arm_for`] classifies one child. Application of the plan is the door's
//! — any failure rejects the whole mutation. This module does not write.
//!
//! Row 16 (leave-and-list population) is the PRD owner's. The instruction
//! scopes the arm to referenced children; that is the operand used here.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-cascade-plan:p1
//! @cpt-cf-bss-products-dod-cascade-parent-path
//! @cpt-cf-bss-products-dod-deferred-intent

use bss_products_sdk::models::LifecycleState;

use super::lifecycle::LifecycleRefusal;

/// One child's disposition in a confirmed Product retirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CascadeArm {
    /// Schedule the SKU retirement; provenance `cascaded`.
    Retire,
    /// Deprecate `cascaded` and list — flip guard cannot clear.
    LeaveAndList,
    /// Never-published draft: discard, release the code.
    AutoDiscard,
}

/// Classify one child. `referenced` is the flip-guard population.
#[must_use]
pub fn arm_for(child: LifecycleState, referenced: bool) -> Option<CascadeArm> {
    match child {
        LifecycleState::Retired | LifecycleState::Discarded => None,
        LifecycleState::Draft => Some(CascadeArm::AutoDiscard),
        LifecycleState::Published | LifecycleState::Deprecated if referenced => {
            Some(CascadeArm::LeaveAndList)
        }
        LifecycleState::Published | LifecycleState::Deprecated => Some(CascadeArm::Retire),
    }
}

/// A computed plan. `left` non-empty means the parent's flip defers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CascadePlan {
    /// Scheduled-retire children.
    pub retire: Vec<usize>,
    /// Leave-and-list children.
    pub leave: Vec<usize>,
    /// Auto-discard children.
    pub discard: Vec<usize>,
}

impl CascadePlan {
    /// Build from a parallel slice of states and reference flags.
    #[must_use]
    pub fn compute(children: &[(LifecycleState, bool)]) -> Self {
        let mut plan = Self {
            retire: Vec::new(),
            leave: Vec::new(),
            discard: Vec::new(),
        };
        for (index, (state, referenced)) in children.iter().enumerate() {
            match arm_for(*state, *referenced) {
                Some(CascadeArm::Retire) => plan.retire.push(index),
                Some(CascadeArm::LeaveAndList) => plan.leave.push(index),
                Some(CascadeArm::AutoDiscard) => plan.discard.push(index),
                None => {}
            }
        }
        plan
    }

    /// Whether the parent's flip must defer (any child left un-retired).
    #[must_use]
    pub fn defers_parent_flip(&self) -> bool {
        !self.leave.is_empty()
    }
}

/// An unconfirmed Product retirement over live children is refused.
///
/// # Errors
///
/// [`LifecycleRefusal::CASCADE_CONFIRMATION_REQUIRED`].
pub fn require_cascade_confirmation(
    confirmed: bool,
    live_children: usize,
) -> Result<(), LifecycleRefusal> {
    if live_children > 0 && !confirmed {
        return Err(LifecycleRefusal::cascade_confirmation_required());
    }
    Ok(())
}

/// The parent's own path: force `deprecated` / `direct`, own retire
/// intent, flip when every child is terminal.
#[must_use]
pub fn parent_flip_clears(children: &[LifecycleState]) -> bool {
    children
        .iter()
        .all(|s| matches!(*s, LifecycleState::Retired | LifecycleState::Discarded))
}

/// Resolution values a deferred-retirement row may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferralResolution {
    /// Listed children have cleared.
    ChildrenCleared,
    /// Operator cancelled the cascade.
    CascadeCancelled,
}

impl DeferralResolution {
    /// Stored spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChildrenCleared => "children_cleared",
            Self::CascadeCancelled => "cascade_cancelled",
        }
    }
}

#[cfg(test)]
#[path = "cascade_tests.rs"]
mod cascade_tests;
