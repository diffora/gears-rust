//! The two pieces of the projector that are decidable without a database.
//!
//! Everything else here is a statement about rows in several tables at once —
//! the finalize, the delta, the completeness check, the frontier walk — and is
//! proved in `tests/sqlite_read_model.rs` against a real one. What is worth
//! isolating is the subtraction that **is** the re-drive, and the refusal of a
//! subject kind this gear cannot have written: the first because a wrong
//! subtraction re-projects a warm subject (refused by the primary key, but as a
//! failed pass rather than as a no-op), and the second because its whole value
//! is the sentence it produces.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use super::{outstanding_subjects, plan_subject_of};
use crate::domain::error::DomainError;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::read_model::{OverlayIndexShard, SubjectKind, SubjectRef};
use crate::infra::storage::repo::PendingVersionRow;

fn ref_row(handle: &str, subject: &SubjectRef) -> PendingVersionRow {
    PendingVersionRow::for_subject(
        Uuid::from_u128(0x7e_11),
        handle.to_owned(),
        subject,
        Some(0),
        Some(LifecycleState::Published),
        Utc.with_ymd_and_hms(2026, 8, 3, 12, 0, 0).unwrap(),
    )
}

#[test]
fn the_re_drive_is_the_subtraction_of_the_warm_set() {
    let a = Uuid::from_u128(0xa);
    let b = Uuid::from_u128(0xb);
    let subjects = vec![
        ref_row("pend-a", &SubjectRef::Plan(a)),
        ref_row("pend-b", &SubjectRef::Plan(b)),
    ];

    // First warm: the difference is everything.
    assert_eq!(outstanding_subjects(&subjects, &[]).len(), 2);

    // A re-drive after one of the two landed: the difference is exactly what
    // failed last time. Not a second mechanism - this subtraction is the
    // whole of sec 4.4's unbounded degraded re-drive.
    let warm = vec![(SubjectKind::Plan, a.to_string())];
    let outstanding = outstanding_subjects(&subjects, &warm);
    assert_eq!(outstanding.len(), 1);
    assert_eq!(outstanding[0].subject_ref, b.to_string());

    // Complete: nothing outstanding, which is what lets the frontier advance.
    let warm = vec![
        (SubjectKind::Plan, a.to_string()),
        (SubjectKind::Plan, b.to_string()),
    ];
    assert!(outstanding_subjects(&subjects, &warm).is_empty());
}

#[test]
fn the_warm_set_is_matched_on_the_kind_as_well_as_the_reference() {
    // Two subject kinds can carry the same uuid - a plan and a membership
    // record are unrelated rows keyed alike - so a subtraction that compared
    // references alone would report a plan warm because a membership subject
    // of the same id was.
    let id = Uuid::from_u128(0xa);
    let subjects = vec![ref_row("pend-a", &SubjectRef::Plan(id))];
    let warm = vec![(SubjectKind::GroupMembership, id.to_string())];

    assert_eq!(
        outstanding_subjects(&subjects, &warm).len(),
        1,
        "a membership subject warm at this version says nothing about the plan"
    );
}

#[test]
fn a_subject_kind_this_gear_cannot_have_written_is_refused_by_name() {
    // Refused rather than skipped. A skipped subject holds its version
    // incomplete, and therefore holds the frontier, forever with nothing saying
    // why - which is the failure mode the materialized frontier makes silent.
    for subject in [
        SubjectRef::PriceOverlay(Uuid::from_u128(1)),
        SubjectRef::OverlayIndex(OverlayIndexShard::Global),
        SubjectRef::GroupMembership(Uuid::from_u128(2)),
    ] {
        let kind = subject.kind();
        let refusal = plan_subject_of(&ref_row("pend-x", &subject))
            .expect_err("no store in this gear can have written that subject");
        match refusal {
            DomainError::Internal(message) => assert!(
                message.contains(kind.as_str()),
                "the refusal must name the kind, got: {message}"
            ),
            other => panic!("expected an internal fault, got {other:?}"),
        }
    }
}

#[test]
fn a_plan_subject_whose_reference_is_not_a_plan_id_is_refused() {
    // The kind and the reference are written together by `for_subject`, so this
    // is unreachable through that constructor - and reachable through a row
    // something else wrote, which is the case the parse exists for.
    let mut row = ref_row("pend-x", &SubjectRef::Plan(Uuid::from_u128(1)));
    row.subject_ref = "not-a-uuid".to_owned();

    let refusal = plan_subject_of(&row).expect_err("a plan subject is keyed by a plan id");
    assert!(matches!(refusal, DomainError::Internal(_)), "{refusal:?}");
}

#[test]
fn a_plan_subject_resolves_to_its_plan() {
    let id = Uuid::from_u128(0x9_1a4);
    let (plan_id, revision, state) = plan_subject_of(&ref_row("pend-a", &SubjectRef::Plan(id)))
        .expect("a plan subject names a plan, the revision its publish judged, and its state");
    assert_eq!(plan_id.get(), id);
    assert_eq!(revision, 0);
    assert_eq!(state, LifecycleState::Published);
}

#[test]
fn a_plan_subject_with_no_pinned_lifecycle_state_is_refused_rather_than_read_live() {
    // Read live, the row's state admits `superseded` - a third value D-128 does
    // not contemplate for a projected subject, and one a consumer coding
    // sellability predicate (4) as "is published" reads as unsellable. Frozen
    // into an INSERT-only delta that is permanent, so the absence fails closed.
    let mut row = ref_row("pend-x", &SubjectRef::Plan(Uuid::from_u128(1)));
    row.subject_lifecycle_state = None;

    let refusal = plan_subject_of(&row).expect_err("a plan subject pins the state it judged");
    match refusal {
        DomainError::Internal(message) => assert!(
            message.contains("no lifecycle state"),
            "the refusal must say what is missing, got: {message}"
        ),
        other => panic!("expected an internal fault, got {other:?}"),
    }
}

#[test]
fn a_plan_subject_with_no_pinned_revision_is_refused_rather_than_defaulted() {
    // Defaulting to the current revision is the defect the column closes: it
    // is wrong exactly when a second publish beat the warm, and the wrong
    // answer freezes permanently into a store whose contract is that a
    // completed version never changes.
    let mut row = ref_row("pend-x", &SubjectRef::Plan(Uuid::from_u128(1)));
    row.subject_revision = None;

    let refusal = plan_subject_of(&row).expect_err("a plan subject pins the revision it judged");
    match refusal {
        DomainError::Internal(message) => assert!(
            message.contains("no revision"),
            "the refusal must say what is missing, got: {message}"
        ),
        other => panic!("expected an internal fault, got {other:?}"),
    }
}
