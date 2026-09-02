//! Claim protocol and the two deferral populations.

use chrono::{Duration, TimeZone, Utc};
use uuid::Uuid;

use super::{
    ACTIVATION_LANE, AttemptBudget, ClaimDecision, ClaimLease, DeferralPopulation, DoorRefusal,
    PreAuthorizedCall, RunFinish, ScheduledActivation, ScheduledFinishState, StoredRunState,
    activation_idempotency_key, claim_decision, classify_door_refusal, defer_flip_guard,
    scheduled_pin_holds, verify_activation_pin,
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
    let id = Uuid::from_u128(0xbb);
    let call = PreAuthorizedCall::from_row(id);
    assert_eq!(
        PreAuthorizedCall::mode_debug(),
        "GateMode::PreAuthorized(approval_id)"
    );
    assert_eq!(
        call.mode(),
        crate::domain::governance::GateMode::PreAuthorized(
            crate::domain::governance::ApprovalId::new(id)
        )
    );
}

fn entity_subject(entity_id: Uuid) -> crate::domain::governance::GateSubject {
    crate::domain::governance::GateSubject::entity_publish(crate::domain::governance::EntityRef {
        tenant_id: Uuid::from_u128(0x10),
        entity_kind: bss_products_sdk::models::EntityKind::Product,
        entity_id,
    })
}

#[test]
fn a_consumed_record_the_row_names_is_admitted_even_when_the_subject_is_a_child() {
    use crate::domain::approval::{ApprovalState, CandidateApproval, StoredApprovalGate};
    use crate::domain::concurrency::InternalRevision;
    use crate::domain::governance::{ApprovalId, GateMode, GateVerdict, GovernanceGate};

    let parent_id = Uuid::from_u128(0x21);
    let child_id = Uuid::from_u128(0x22);
    let approval = Uuid::from_u128(0x23);
    let parent_subject = entity_subject(parent_id);
    let child_subject = entity_subject(child_id);
    let pin = ScheduledActivation {
        row_approval_ref: approval,
        record_id: approval,
        record_consumed: true,
    };
    assert!(
        scheduled_pin_holds(&pin),
        "P-D-105 admits a leg that names the parent's consumed record"
    );

    let host = StoredApprovalGate::governed(vec![CandidateApproval {
        approval_id: ApprovalId::new(approval),
        subject: parent_subject,
        internal_revision: 1,
        state: ApprovalState::Consumed,
        override_acknowledged: false,
    }]);
    let old = host
        .evaluate(
            child_subject.clone(),
            InternalRevision::new(1),
            GateMode::PreAuthorized(ApprovalId::new(approval)),
        )
        .expect("the host reaches a verdict");
    assert!(
        matches!(old, GateVerdict::Refused { .. }),
        "B's current host still matches subject + revision and refuses the leg"
    );

    let finish = verify_activation_pin(&host, child_subject, InternalRevision::new(1), &pin);
    assert_eq!(
        finish,
        RunFinish::Applied,
        "the runner admits on the pin the row carries"
    );
}

#[test]
fn a_pin_that_does_not_verify_fails_terminally() {
    use crate::domain::concurrency::InternalRevision;
    use crate::domain::error::DomainError;
    use crate::domain::governance::{
        ApprovalDisposition, ApprovalId, GateMode, GateVerdict, GovernanceGate,
    };

    struct RecordingGate {
        seen: std::cell::RefCell<Option<GateMode>>,
    }
    impl GovernanceGate for RecordingGate {
        fn evaluate(
            &self,
            _subject: crate::domain::governance::GateSubject,
            _expected: InternalRevision,
            mode: GateMode,
        ) -> Result<GateVerdict, DomainError> {
            *self.seen.borrow_mut() = Some(mode);
            Ok(GateVerdict::authorized(
                ApprovalDisposition::Verified(ApprovalId::new(Uuid::from_u128(0x31))),
                false,
                "recording".to_owned(),
            ))
        }
    }

    let named = Uuid::from_u128(0x31);
    let gate = RecordingGate {
        seen: std::cell::RefCell::new(None),
    };
    let subject = entity_subject(Uuid::from_u128(0x32));
    let mismatched = ScheduledActivation {
        row_approval_ref: named,
        record_id: Uuid::from_u128(0x33),
        record_consumed: true,
    };
    let finish = verify_activation_pin(
        &gate,
        subject.clone(),
        InternalRevision::new(1),
        &mismatched,
    );
    assert!(matches!(finish, RunFinish::Failed { .. }));
    assert!(!matches!(finish, RunFinish::Deferred { .. }));
    assert_eq!(
        *gate.seen.borrow(),
        Some(GateMode::PreAuthorized(
            crate::domain::governance::ApprovalId::new(named)
        ))
    );

    let unconsumed = ScheduledActivation {
        row_approval_ref: named,
        record_id: named,
        record_consumed: false,
    };
    assert!(matches!(
        verify_activation_pin(&gate, subject, InternalRevision::new(1), &unconsumed),
        RunFinish::Failed { .. }
    ));
}
