//! Tests for the retention gate (`dod-retention-gate`). Each case is one of
//! the `DoD`'s named failures, armed as the failure rather than as its fix.

use super::{RetentionHold, RetentionVerdict, evaluate};
use crate::domain::states::FreezeAckState;
use crate::infra::storage::repo::FreezeRegistration;

fn reg(participant: &str, state: FreezeAckState, stamped: bool) -> FreezeRegistration {
    FreezeRegistration {
        participant: participant.to_owned(),
        state,
        released_at_stamped: stamped,
    }
}

fn snap(members: &[&str]) -> Vec<String> {
    members.iter().map(|m| (*m).to_owned()).collect()
}

/// **The vacuity the `DoD` names first.** An empty ledger against a non-empty
/// snapshot must HOLD — quantifying over registrations instead of the
/// snapshot is what let a version nobody had frozen be collected.
#[test]
fn an_empty_ledger_holds_a_non_empty_snapshot() {
    let verdict = evaluate(&snap(&["pricing", "contracts"]), &[]);
    let RetentionVerdict::Held(holds) = verdict else {
        panic!("an empty ledger must not satisfy the gate vacuously");
    };
    assert_eq!(holds.len(), 2, "every member holds, not just the first");
    for hold in &holds {
        assert!(matches!(hold, RetentionHold::NoRegistration { .. }));
        assert_eq!(RetentionHold::REASON, "retention_orphan_blocked");
    }
}

/// **The other vacuity, and it is admitted.** An empty snapshot is
/// collectable: nobody ever owed an ack. The two cases above and here differ
/// in which store is empty, and only one of them is a defect.
#[test]
fn an_empty_snapshot_is_collectable() {
    assert_eq!(evaluate(&[], &[]), RetentionVerdict::Collectable);
    assert_eq!(
        evaluate(&[], &[reg("pricing", FreezeAckState::Pending, false)]),
        RetentionVerdict::Collectable,
        "a registration outside the snapshot is not the gate's business"
    );
}

/// A door-released row carries `state = released` with the stamp **NULL**, and
/// that satisfies the first arm — so a gate reading the timestamp would refuse
/// every ordinary release.
#[test]
fn a_door_released_row_satisfies_the_gate_without_a_stamp() {
    assert_eq!(
        evaluate(
            &snap(&["pricing"]),
            &[reg("pricing", FreezeAckState::Released, false)]
        ),
        RetentionVerdict::Collectable
    );
}

/// **The failure the second arm exists for.** A forced participant that later
/// recovered and acked leaves `state = acked` beside a live `released_at` — so
/// reading the timestamp alone collected a version holding live grandfathered
/// references.
#[test]
fn a_stamp_beside_a_live_state_does_not_satisfy_the_gate() {
    let verdict = evaluate(
        &snap(&["pricing"]),
        &[reg("pricing", FreezeAckState::Acked, true)],
    );
    let RetentionVerdict::Held(holds) = verdict else {
        panic!("the timestamp alone must not satisfy the gate");
    };
    assert_eq!(
        holds,
        vec![RetentionHold::LiveRegistration {
            participant: "pricing".to_owned(),
            state: FreezeAckState::Acked,
        }]
    );
}

/// The forced arm needs both halves, and the shape `CHECK` refuses the
/// half-written row on both engines — so this arm reports a row that reached
/// the table past its guard rather than an ordinary state.
#[test]
fn the_forced_arm_needs_its_stamp() {
    let verdict = evaluate(
        &snap(&["pricing"]),
        &[reg("pricing", FreezeAckState::NotFrozenForced, false)],
    );
    assert_eq!(
        verdict,
        RetentionVerdict::Held(vec![RetentionHold::ForcedWithoutStamp {
            participant: "pricing".to_owned()
        }])
    );
    assert_eq!(
        evaluate(
            &snap(&["pricing"]),
            &[reg("pricing", FreezeAckState::NotFrozenForced, true)]
        ),
        RetentionVerdict::Collectable,
        "with the stamp the forced arm is satisfied"
    );
}

/// `pending` holds, which is the ordinary in-flight case, and the verdict
/// carries the state so the skip reason can name it.
#[test]
fn a_pending_registration_holds() {
    let verdict = evaluate(
        &snap(&["pricing"]),
        &[reg("pricing", FreezeAckState::Pending, false)],
    );
    assert_eq!(
        verdict,
        RetentionVerdict::Held(vec![RetentionHold::LiveRegistration {
            participant: "pricing".to_owned(),
            state: FreezeAckState::Pending,
        }])
    );
}

/// **Every** hold is reported rather than the first: an operator repairing one
/// and re-running would otherwise meet the rest one pass at a time.
#[test]
fn every_hold_is_reported_and_a_mixed_set_still_holds() {
    let verdict = evaluate(
        &snap(&["pricing", "contracts", "billing", "rating"]),
        &[
            reg("pricing", FreezeAckState::Released, false),
            reg("contracts", FreezeAckState::Pending, false),
            reg("billing", FreezeAckState::NotFrozenForced, true),
            // `rating` has no row at all.
        ],
    );
    let RetentionVerdict::Held(holds) = verdict else {
        panic!("two members hold");
    };
    let named: Vec<&str> = holds.iter().map(RetentionHold::participant).collect();
    assert_eq!(
        named,
        vec!["contracts", "rating"],
        "the two satisfied members are absent from the holds and the two unsatisfied are present, \
         in snapshot order"
    );
}
