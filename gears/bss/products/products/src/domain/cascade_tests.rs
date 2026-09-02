//! Three-arm plan, confirmation, parent flip, and the deferral meaning
//! "children left", never a half-applied plan.

use bss_products_sdk::models::LifecycleState;

use super::{
    CascadeArm, CascadePlan, DeferralResolution, arm_for, parent_flip_clears,
    require_cascade_confirmation,
};
use crate::domain::lifecycle::LifecycleRefusal;

#[test]
fn the_three_arms_cover_the_live_states() {
    assert_eq!(
        arm_for(LifecycleState::Draft, false),
        Some(CascadeArm::AutoDiscard)
    );
    assert_eq!(
        arm_for(LifecycleState::Published, false),
        Some(CascadeArm::Retire)
    );
    assert_eq!(
        arm_for(LifecycleState::Deprecated, false),
        Some(CascadeArm::Retire)
    );
    assert_eq!(
        arm_for(LifecycleState::Published, true),
        Some(CascadeArm::LeaveAndList)
    );
    assert_eq!(arm_for(LifecycleState::Retired, false), None);
    assert_eq!(arm_for(LifecycleState::Discarded, true), None);
}

#[test]
fn a_three_child_fixture_splits_across_all_three_arms() {
    let plan = CascadePlan::compute(&[
        (LifecycleState::Published, true),
        (LifecycleState::Published, false),
        (LifecycleState::Draft, false),
    ]);
    assert_eq!(plan.leave, vec![0]);
    assert_eq!(plan.retire, vec![1]);
    assert_eq!(plan.discard, vec![2]);
    assert!(
        plan.defers_parent_flip(),
        "leave-and-list means the parent flip defers"
    );
}

#[test]
fn an_unconfirmed_retirement_over_live_children_is_refused() {
    let err = require_cascade_confirmation(false, 2).expect_err("unconfirmed");
    assert_eq!(err.code, LifecycleRefusal::CASCADE_CONFIRMATION_REQUIRED);
    require_cascade_confirmation(true, 2).expect("confirmed");
    require_cascade_confirmation(false, 0).expect("no live children, no confirmation");
}

#[test]
fn the_parent_flip_waits_for_every_child_to_be_terminal() {
    assert!(!parent_flip_clears(&[
        LifecycleState::Retired,
        LifecycleState::Deprecated
    ]));
    assert!(parent_flip_clears(&[
        LifecycleState::Retired,
        LifecycleState::Discarded
    ]));
    assert!(parent_flip_clears(&[]));
}

#[test]
fn deferral_resolutions_are_the_two_admitted_spellings() {
    assert_eq!(
        DeferralResolution::ChildrenCleared.as_str(),
        "children_cleared"
    );
    assert_eq!(
        DeferralResolution::CascadeCancelled.as_str(),
        "cascade_cancelled"
    );
}
