//! Claim protocol and the two deferral populations.

use chrono::{Duration, TimeZone, Utc};
use uuid::Uuid;

use super::{
    ACTIVATION_LANE, AttemptBudget, ClaimDecision, ClaimLease, DeferralPopulation, DoorRefusal,
    PreAuthorizedCall, RunFinish, ScheduledFinishState, StoredRunState, activation_idempotency_key,
    claim_decision, classify_door_refusal, defer_flip_guard,
};
use crate::domain::lifecycle::LifecycleRefusal;
use crate::domain::retirement::{FlipPredicate, RetirementHeld, flip_guard};

fn lease() -> ClaimLease {
    ClaimLease {
        ttl: Duration::seconds(30),
    }
}

fn t0() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap()
}

#[test]
fn a_due_pending_or_deferred_row_is_claimed() {
    let now = t0();
    let at = now - Duration::seconds(1);
    assert_eq!(
        claim_decision(StoredRunState::Pending, at, None, now, lease()),
        ClaimDecision::Claim
    );
    assert_eq!(
        claim_decision(StoredRunState::Deferred, at, None, now, lease()),
        ClaimDecision::Claim,
        "deferred is re-claimed on the same poll"
    );
}

#[test]
fn a_future_row_is_skipped() {
    let now = t0();
    assert_eq!(
        claim_decision(
            StoredRunState::Pending,
            now + Duration::hours(1),
            None,
            now,
            lease()
        ),
        ClaimDecision::Skip
    );
}

#[test]
fn a_running_row_past_the_lease_is_reclaimed() {
    let now = t0();
    let claimed = now - Duration::seconds(31);
    assert_eq!(
        claim_decision(
            StoredRunState::Running,
            now - Duration::hours(1),
            Some(claimed),
            now,
            lease()
        ),
        ClaimDecision::ReclaimLease
    );
    assert_eq!(
        claim_decision(
            StoredRunState::Running,
            now - Duration::hours(1),
            Some(now - Duration::seconds(5)),
            now,
            lease()
        ),
        ClaimDecision::Skip
    );
}

#[test]
fn a_terminal_row_is_never_claimed() {
    assert_eq!(
        claim_decision(StoredRunState::Terminal, t0(), None, t0(), lease()),
        ClaimDecision::Skip
    );
}

#[test]
fn the_idempotency_key_is_the_lane_and_the_transition_id() {
    let id = Uuid::from_u128(0xaa);
    assert_eq!(
        activation_idempotency_key(id),
        format!("{ACTIVATION_LANE}:{id}")
    );
}

#[test]
fn stale_revision_and_approval_required_become_schedule_stale_approval() {
    let budget = AttemptBudget { max: 3 };
    for code in ["STALE_REVISION", "APPROVAL_REQUIRED"] {
        let err = classify_door_refusal(
            DoorRefusal {
                code,
                transient: false,
            },
            0,
            budget,
        )
        .expect_err(code);
        assert_eq!(err.code, LifecycleRefusal::SCHEDULE_STALE_APPROVAL);
    }
}

#[test]
fn flip_guard_deferral_bypasses_the_door_classifier() {
    let held = flip_guard(FlipPredicate::FreshPositive).expect_err("held");
    let finish = defer_flip_guard(&held);
    assert_eq!(finish.state(), ScheduledFinishState::Deferred);
    assert_eq!(
        finish,
        RunFinish::Deferred {
            population: DeferralPopulation::FlipGuard,
            reason: "flip guard: stub:fresh-positive".to_owned(),
        }
    );
    let empty = defer_flip_guard(&RetirementHeld {
        blocking_producers: vec![],
    });
    assert!(matches!(
        empty,
        RunFinish::Deferred {
            population: DeferralPopulation::FlipGuard,
            ..
        }
    ));
}

#[test]
fn transient_deferral_is_bounded_by_the_budget() {
    let budget = AttemptBudget { max: 2 };
    let door = DoorRefusal {
        code: "UNAVAILABLE",
        transient: true,
    };
    let hold = classify_door_refusal(door, 0, budget).expect("still in budget");
    assert!(matches!(
        hold,
        RunFinish::Deferred {
            population: DeferralPopulation::TransientDependency,
            ..
        }
    ));
    let fail = classify_door_refusal(door, 1, budget).expect("exhausted");
    assert!(matches!(fail, RunFinish::Failed { .. }));
}

#[test]
fn the_preauthorized_call_names_the_mode_and_does_not_consume() {
    let call = PreAuthorizedCall {
        approval_id: Uuid::from_u128(0xbb),
    };
    assert_eq!(
        PreAuthorizedCall::mode_debug(),
        "GateMode::PreAuthorized(approval_id)"
    );
    let _ = call.approval_id;
}
