//! Tests for the `GovernedLiveOp` envelope (`dod-governed-live-op`).

use super::GovernedLiveOp;
use crate::domain::error::DomainError;

/// A set member's state, standing in for any live row's — the envelope is
/// generic precisely so 03 can pass its own.
#[derive(Clone, Debug, PartialEq, Eq)]
enum MemberState {
    Active,
    Deprecated,
    Removed,
}

fn op(expected: MemberState) -> GovernedLiveOp<MemberState> {
    GovernedLiveOp {
        kind: "recognized_set.deprecate".to_owned(),
        target: "metering_unit/vCPU-hour".to_owned(),
        payload: r#"{"reason":"superseded"}"#.to_owned(),
        expected_state: expected,
    }
}

/// The pinned state matching the live row is the whole admission condition —
/// nothing about the payload, the kind or a revision enters it.
#[test]
fn a_matching_pinned_state_is_current() {
    op(MemberState::Active)
        .check_still_current(&MemberState::Active)
        .expect("the world did not move");
}

/// The world moving is `STALE_LIVE_OP` — **not** `STALE_REVISION`, which a
/// live row cannot be stale against because it carries no revision, and not
/// `STALE_CATEGORY_TOKEN`, which is the category live-value door's own
/// precondition (`design/02` §3.5 draws both lines).
#[test]
fn a_moved_world_is_stale_live_op_and_names_both_states() {
    let refusal = op(MemberState::Active)
        .check_still_current(&MemberState::Deprecated)
        .expect_err("the pinned state no longer holds");
    assert_eq!(refusal.code(), "STALE_LIVE_OP");
    let text = refusal.to_string();
    for fragment in [
        "recognized_set.deprecate",
        "metering_unit/vCPU-hour",
        "Active",
        "Deprecated",
    ] {
        assert!(
            text.contains(fragment),
            "the refusal must name the op, the target and BOTH states so an operator can see \
             what moved: {text} lacks {fragment}"
        );
    }
    assert!(
        matches!(refusal, DomainError::StaleLiveOp(_)),
        "the variant is the live-op one"
    );
}

/// Every state pair that differs is refused, and every pair that matches is
/// admitted — the check is equality and nothing more, so a third state added
/// later needs no arm here.
#[test]
fn the_check_is_equality_over_the_whole_state_space() {
    let states = [
        MemberState::Active,
        MemberState::Deprecated,
        MemberState::Removed,
    ];
    for pinned in &states {
        for live in &states {
            let verdict = op(pinned.clone()).check_still_current(live);
            assert_eq!(
                verdict.is_ok(),
                pinned == live,
                "pinned {pinned:?} against live {live:?}"
            );
        }
    }
}

/// The envelope carries the payload **uninspected**: an approval pins exactly
/// these bytes, so a type that parsed or normalized them would let an
/// approved op apply different content than was approved.
#[test]
fn the_payload_is_carried_and_never_inspected() {
    let odd = GovernedLiveOp {
        kind: "category.reparent".to_owned(),
        target: "category/7f".to_owned(),
        payload: "not json at all, and that is admitted".to_owned(),
        expected_state: MemberState::Active,
    };
    odd.check_still_current(&MemberState::Active)
        .expect("the payload plays no part in the currency check");
    assert_eq!(
        odd.payload, "not json at all, and that is admitted",
        "the bytes are carried through unchanged"
    );
}

/// `inst-gl-atomic`: the closure runs **only** when the pinned state still
/// holds, so a stale op cannot mutate. Proven by observing the closure rather
/// than by reading `apply` — a check that ran after the mutation would pass
/// the currency test and still have written.
#[test]
fn a_stale_op_never_runs_its_mutation() {
    let mut ran = false;
    let refusal = op(MemberState::Active)
        .apply(&MemberState::Removed, || {
            ran = true;
            Ok(())
        })
        .expect_err("the pinned state moved");
    assert_eq!(refusal.code(), "STALE_LIVE_OP");
    assert!(
        !ran,
        "the mutation must not run: a check after the write would pass this test while having \
         already written"
    );
}

/// The positive control: a current op runs its closure exactly once and hands
/// back what the closure produced.
#[test]
fn a_current_op_runs_its_mutation_once() {
    let mut runs = 0_u32;
    let out = op(MemberState::Active)
        .apply(&MemberState::Active, || {
            runs += 1;
            Ok("written")
        })
        .expect("the pinned state holds");
    assert_eq!((out, runs), ("written", 1));
}

/// A failing mutation's error travels unchanged — `apply` adds no
/// classification of its own, because the mutation's refusal belongs to the
/// slice that wrote it.
#[test]
fn a_failing_mutation_keeps_its_own_error() {
    let refusal = op(MemberState::Active)
        .apply(&MemberState::Active, || {
            Err::<(), _>(DomainError::DuplicateCode("already reserved".to_owned()))
        })
        .expect_err("the mutation refused");
    assert_eq!(refusal.code(), "DUPLICATE_CODE");
}
