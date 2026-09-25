#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::{ApproveStep, evaluate_approve};
use crate::model::{ApprovalError, Decision, Policy, Unit, UnitState, Verdict};
use time::macros::datetime;
use uuid::Uuid;

fn unit(quorum: u32, submitted_by: Uuid) -> Unit {
    Unit {
        id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        kind: "prices".into(),
        ref_type: "book".into(),
        ref_id: Uuid::new_v4(),
        state: UnitState::Pending,
        common_effective_date: None,
        quorum_required: quorum,
        generation: 1,
        submitted_by,
        submitted_at: datetime!(2026-09-24 10:00 UTC),
        decided_at: None,
        decided_note: None,
        snapshot: serde_json::json!({}),
        snapshot_hash: "h".into(),
        version: 1,
    }
}
fn vote(unit: &Unit, actor: Uuid, generation: i32) -> Decision {
    Decision {
        unit_id: unit.id,
        actor,
        generation,
        verdict: Verdict::Approve,
        note: None,
        at: datetime!(2026-09-24 11:00 UTC),
        stale: generation < unit.generation,
    }
}

#[test]
fn quorum_one_applies_on_the_first_independent_approve() {
    let author = Uuid::new_v4();
    let u = unit(1, author);
    assert_eq!(
        evaluate_approve(&u, &[], Uuid::new_v4(), &[author]).unwrap(),
        ApproveStep::Apply
    );
}
#[test]
fn quorum_two_needs_a_second_distinct_approver() {
    let author = Uuid::new_v4();
    let u = unit(2, author);
    let first = Uuid::new_v4();
    assert_eq!(
        evaluate_approve(&u, &[], first, &[author]).unwrap(),
        ApproveStep::NeedMore { have: 1, need: 2 }
    );
    assert_eq!(
        evaluate_approve(&u, &[vote(&u, first, 1)], Uuid::new_v4(), &[author]).unwrap(),
        ApproveStep::Apply
    );
}
#[test]
fn the_submitter_and_every_item_author_are_excluded() {
    let author = Uuid::new_v4();
    let submitter = Uuid::new_v4();
    let u = unit(1, submitter);
    assert!(matches!(
        evaluate_approve(&u, &[], submitter, &[author]),
        Err(ApprovalError::SodViolation)
    ));
    assert!(matches!(
        evaluate_approve(&u, &[], author, &[author]),
        Err(ApprovalError::SodViolation)
    ));
}
#[test]
fn a_second_vote_by_the_same_actor_in_the_same_generation_is_refused() {
    let u = unit(2, Uuid::new_v4());
    let a = Uuid::new_v4();
    assert!(matches!(
        evaluate_approve(&u, &[vote(&u, a, 1)], a, &[]),
        Err(ApprovalError::DuplicateVote)
    ));
}
#[test]
fn a_vote_from_an_earlier_generation_neither_counts_nor_blocks_the_actor() {
    let mut u = unit(2, Uuid::new_v4());
    u.generation = 2;
    let a = Uuid::new_v4();
    let old = vote(&u, a, 1);
    assert!(old.stale);
    assert_eq!(
        evaluate_approve(&u, &[old], a, &[]).unwrap(),
        ApproveStep::NeedMore { have: 1, need: 2 }
    );
}
#[test]
fn a_decided_unit_takes_no_vote() {
    let mut u = unit(1, Uuid::new_v4());
    u.state = UnitState::Approved;
    assert!(matches!(
        evaluate_approve(&u, &[], Uuid::new_v4(), &[]),
        Err(ApprovalError::AlreadyDecided)
    ));
}
#[test]
fn the_policy_overrides_per_kind_and_falls_back_to_star() {
    let p = Policy {
        default_quorum: 1,
        overrides: [("promotion".to_owned(), 0u32)].into_iter().collect(),
    };
    assert_eq!(p.quorum_for("promotion"), 0);
    assert_eq!(p.quorum_for("prices"), 1);
}
#[test]
fn every_error_has_a_stable_code() {
    assert_eq!(ApprovalError::SodViolation.code(), "SOD_VIOLATION");
    assert_eq!(ApprovalError::Contended.code(), "UNIT_CONTENDED");
    assert_eq!(
        ApprovalError::Locked {
            item_type: "sku".into(),
            item_id: Uuid::nil()
        }
        .code(),
        "ROW_LOCKED_PENDING"
    );
    assert_eq!(
        ApprovalError::ApplyRefused {
            code: "SKU_NAME_TAKEN",
            detail: String::new()
        }
        .code(),
        "APPLY_REFUSED"
    );
    let db_error = sea_orm::DbErr::Custom("retry classification".into());
    let error = ApprovalError::from(db_error.clone());
    assert_eq!(error.db_err(), Some(&db_error));
    assert!(ApprovalError::Contended.db_err().is_none());
}
