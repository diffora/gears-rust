//! The two assembly decisions, proven where they are pure.
//!
//! The storage-backed half — that a real draft revision and a real published
//! plane assemble into the shape these constants describe — lands in
//! `tests/sqlite_publish_commit.rs`, where there is a database to put rows in.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use uuid::Uuid;

use super::CANDIDATE_ROW_STATES;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::publish::PlanPublishUnit;
use crate::domain::scope_key::PlanId;

#[test]
fn the_candidate_row_set_is_the_shape_the_commit_will_leave_behind() {
    // Published plus draft, and nothing else. Draft alone would fail every
    // completeness rule on a revision that authors no price rows; published
    // alone would let a newly authored market go unjudged.
    assert_eq!(
        CANDIDATE_ROW_STATES,
        [LifecycleState::Published, LifecycleState::Draft]
    );
}

#[test]
fn superseded_and_abandoned_rows_are_out_of_the_subject() {
    // A superseded row has had its key taken by a successor and is history;
    // `abandoned` is a plan-revision state that a price row never holds
    // (§4.3, `inst-ps-nodelete`), so naming it would be a second reading of the
    // price row's state set.
    for excluded in [LifecycleState::Superseded, LifecycleState::Abandoned] {
        assert!(
            !CANDIDATE_ROW_STATES.contains(&excluded),
            "{excluded} must not be a candidate state"
        );
    }
}

#[test]
fn the_candidate_states_are_exactly_the_two_a_publish_can_reach() {
    // Every state the machine admits is accounted for: two in, three out. A
    // state added later fails this rather than silently joining or missing the
    // subject.
    assert_eq!(LifecycleState::ALL.len(), 5);
    assert_eq!(CANDIDATE_ROW_STATES.len(), 2);
    assert!(
        !CANDIDATE_ROW_STATES.contains(&LifecycleState::Retired),
        "retirement is a plan state and its own publish unit (D-128), never a price row's"
    );
}

// ---------------------------------------------------------------------------
// What the commit derives before it touches anything.
// ---------------------------------------------------------------------------

#[test]
fn one_publish_unit_always_presents_the_same_request_id() {
    // The registry is idempotent on `request_id`, so a commit that rolled back
    // and is retried must present the id its first attempt did. A random id per
    // attempt orphans a pending ref at the registry on every rollback, and every
    // orphan trips `pricing.catalogversion.commit_overdue` for a publish that
    // never happened.
    let tenant = Uuid::from_u128(0x7e_11);
    let unit = PlanPublishUnit::plan_content(PlanId::new(Uuid::from_u128(0x9_1a4)), 2);

    assert_eq!(
        super::publish_request_id(tenant, unit),
        super::publish_request_id(tenant, unit)
    );
}

#[test]
fn two_publish_units_never_share_a_request_id() {
    let tenant = Uuid::from_u128(0x7e_11);
    let other_tenant = Uuid::from_u128(0x7e_22);
    let plan = PlanId::new(Uuid::from_u128(0x9_1a4));
    let other_plan = PlanId::new(Uuid::from_u128(0x9_1a5));
    let unit = PlanPublishUnit::plan_content(plan, 2);

    assert_ne!(
        super::publish_request_id(tenant, unit),
        super::publish_request_id(tenant, PlanPublishUnit::plan_content(plan, 3)),
        "a second revision of one plan is a second publish"
    );
    assert_ne!(
        super::publish_request_id(tenant, unit),
        super::publish_request_id(tenant, PlanPublishUnit::plan_content(other_plan, 2))
    );
    // The tenant is in it, unlike the outbox dedup key: that index is per
    // tenant and the registry is a cross-tenant service with no such scope.
    assert_ne!(
        super::publish_request_id(tenant, unit),
        super::publish_request_id(other_tenant, unit)
    );
}

/// Two units of one plan at one revision - a content publish and a window
/// mutation - are DIFFERENT units, so their version requests do not collide.
///
/// Without the discriminator inside `publish_request_id` the second request
/// dedups onto the first and the window mutation silently rides a version that
/// never re-projected it - which is exactly the "last-warmed delta advertises
/// coverage the truth side removed" exposure D-99 was raised to close.
///
/// **The world is what makes this the discriminator's test rather than the
/// revision's.** A window mutation is a unit over the plan's *current* revision,
/// so the two units genuinely share `(tenant, plan, revision)`: every other term
/// of the preimage is equal by construction here, and the only thing left that can
/// tell them apart is the kind.
#[test]
fn a_window_mutation_and_a_content_publish_of_one_revision_are_two_units() {
    use crate::domain::publish::PublishUnitKind;

    let tenant = Uuid::from_u128(0x7e_11);
    let plan = PlanId::new(Uuid::from_u128(0x9_1a4));
    let window = Uuid::from_u128(0xd0_01);
    let content = PlanPublishUnit::plan_content(plan, 2);
    let mutation = PlanPublishUnit::window_mutation(plan, 2, window);

    assert_eq!(
        (content.plan_id, content.revision),
        (mutation.plan_id, mutation.revision),
        "the two units must share the subject, or this test is about something else"
    );
    assert_ne!(
        super::publish_request_id(tenant, content),
        super::publish_request_id(tenant, mutation)
    );

    // And two window mutations of one revision are two units as well, which is
    // why the window's own id is in the token rather than a bare marker: a
    // schedule on one key and a cancellation on another are separate change sets
    // with separate materiality.
    let other_window = PlanPublishUnit::window_mutation(plan, 2, Uuid::from_u128(0xd0_02));
    assert_ne!(
        super::publish_request_id(tenant, mutation),
        super::publish_request_id(tenant, other_window)
    );

    // The plan-content token is the one it was before there was a kind at all.
    // `request_id` is the registry's idempotency handle, so re-spelling it would
    // make every outstanding pending ref unreachable by the retry meant to find
    // it - `PIN_DOMAIN_SEP`'s question, one plane over.
    assert_eq!(
        super::publish_request_id(tenant, content),
        format!("plan-publish/{tenant}/{plan}/2")
    );
    assert_eq!(PublishUnitKind::default(), PublishUnitKind::PlanContent);
}

#[test]
fn the_audit_before_state_names_the_flip_and_carries_no_version_ref() {
    // Before the commit there is no pending handle, and inventing one would put
    // an addressability claim in the record of the state that preceded it.
    assert_eq!(
        crate::domain::audit::subject_state(LifecycleState::Draft, 3, None),
        serde_json::json!({"lifecycleState": "draft", "rowVersion": 3})
    );
}

#[test]
fn the_audit_after_state_connects_the_flip_to_the_addressability_it_produced() {
    assert_eq!(
        crate::domain::audit::subject_state(LifecycleState::Published, 4, Some("pend-9")),
        serde_json::json!({
            "lifecycleState": "published",
            "rowVersion": 4,
            "pendingVersionRef": "pend-9",
        })
    );
}
