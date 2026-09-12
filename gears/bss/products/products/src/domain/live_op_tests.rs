//! Tests for the `GovernedLiveOp` envelope (`dod-governed-live-op`).

use super::GovernedLiveOp;
use crate::domain::concurrency::InternalRevision;
use crate::domain::error::DomainError;
use crate::domain::governance::SubjectPin;

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

// -- The gate seam, which the FEATURE's DoD body said did not exist. --

/// **The envelope's target renders straight into a gate subject**, so the
/// seam `dod-governed-live-op`'s body called unbuildable is one call wide.
///
/// The body said submitting *"means inventing a mapping from a live target to
/// an entity ref"*, citing P-D-93. Measured at this commit that is false
/// twice over: `GovernanceGate::evaluate` takes a [`GateSubject`], **not** an
/// `EntityRef`, and `GateSubject::governed_live_op` is the constructor for
/// exactly this case -- granted by **P-D-67 arm 4** on 2026-08-31, the day
/// before P-D-93. This case is what keeps that correction honest: it reddens
/// if the constructor or the `SubjectKind` variant is withdrawn.
///
/// **The negative half is why this is a probe.** The entity constructor is
/// asserted to answer a *different* kind, so a `governed_live_op` that
/// silently produced `EntityPublish` -- the exact mapping the stale sentence
/// imagined was necessary -- would fail here rather than pass.
#[test]
fn the_envelopes_target_is_a_gate_subject_of_its_own_kind() {
    use bss_products_sdk::models::EntityKind;

    use crate::domain::governance::{EntityRef, GateSubject, SubjectKind};

    let tenant = uuid::Uuid::from_u128(0x7e_11);
    let envelope = op(MemberState::Active);
    let subject = GateSubject::governed_live_op(tenant, &envelope.target, SubjectPin::Unpinned);

    assert_eq!(subject.kind, SubjectKind::GovernedLiveOp);
    assert_eq!(
        subject.reference, envelope.target,
        "the approval record carries the envelope's own target, unmapped"
    );
    assert_eq!(subject.tenant_id, tenant);

    let entity = GateSubject::entity_publish(
        EntityRef {
            tenant_id: tenant,
            entity_kind: EntityKind::Product,
            entity_id: uuid::Uuid::from_u128(0xf0_01),
        },
        InternalRevision::new(1),
    );
    assert_ne!(
        entity.kind, subject.kind,
        "a live op is not an entity publish: if these collapsed, the seam \
         would be the invented mapping the DoD body feared rather than the \
         widened subject P-D-67 granted"
    );
}
