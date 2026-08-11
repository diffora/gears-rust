//! The row-to-record reading, and the compare-and-swap — the two things the
//! public surface cannot be driven to.
//!
//! The database suite (`tests/sqlite_approval_repo.rs`) proves what the store
//! does; what is here is what that suite cannot reach.
//!
//! The first is the boundary's own judgement, which needs no database: the
//! CHECKs make the interesting rows unwritable — `chk_pricing_approval_state`
//! and `chk_pricing_approval_subject_kind` refuse them — so the only way to ask
//! "what does this repository do when the table was written around" is to hand
//! it the model directly.
//!
//! The second is [`swap`], which needs a database *and* a caller reaching the
//! `UPDATE` with a record the store has already moved past. No sequence of
//! public calls produces that: [`decide`] reads the record immediately before it
//! acts, so a decided one is turned away by §4's machine and never reaches the
//! statement. Calling the private function directly is the only way to execute
//! the guard **deterministically and single-threaded** — a concurrency test
//! would prove nothing on `SQLite`, whose single writer serializes the
//! transactions the race is made of. `idempotency_repo_tests` is the precedent
//! in this crate and `coord::lease::sqlite_tests` outside it, both for the same
//! reason.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{DateTime, TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use serde_json::json;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{NewApproval, SUBJECT_KINDS_WITH_A_WRITER, decide, open, read, swap, to_domain};
use crate::domain::approval::{ApprovalDecision, ApprovalState};
use crate::domain::audit::{AuditStamp, AuditSubjectKind};
use crate::infra::storage::entity::approval;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::{RepoError, repo_failure};

fn row(state: &str, subject_kind: &str) -> approval::Model {
    approval::Model {
        approval_id: Uuid::from_u128(0xa1),
        tenant_id: Uuid::from_u128(0x7e),
        subject_ref: "0000ffff-0000-0000-0000-000000000001/3".to_owned(),
        subject_kind: subject_kind.to_owned(),
        content_hash: vec![1, 2, 3],
        state: state.to_owned(),
        submitter_principal: Uuid::from_u128(0x5b),
        approver_principal: None,
        reason: None,
        materiality: json!({}),
        submitted_at: Utc.with_ymd_and_hms(2026, 8, 3, 9, 0, 0).unwrap(),
        decided_at: None,
    }
}

#[test]
fn every_state_the_check_admits_reads_back_into_the_machine() {
    // The world in which the refusals below are observable. Without it the
    // corrupt-token tests would pass against a reading that refuses everything.
    for state in ApprovalState::ALL {
        let record = to_domain(row(state.as_str(), "plan_revision")).expect("a stored token reads");
        assert_eq!(record.state, *state);
    }
    for kind in AuditSubjectKind::ALL {
        let record = to_domain(row("submitted", kind.as_str())).expect("a stored token reads");
        assert_eq!(record.subject_kind, *kind);
    }
}

#[test]
fn a_state_token_outside_the_enumeration_is_a_corrupt_row_and_names_the_column() {
    // Not a caller mistake: the CHECK refuses this value, so a row holding it
    // means the table was written around. An operator needs the column named to
    // find it.
    let err = to_domain(row("withdrawn", "plan_revision"))
        .expect_err("a token no CHECK admits is not readable");
    match err {
        RepoError::CorruptRow(detail) => {
            assert!(detail.contains("pricing_approval.state"), "got: {detail}");
            assert!(detail.contains("withdrawn"), "got: {detail}");
        }
        other => panic!("expected a corrupt row, got: {other:?}"),
    }
}

#[test]
fn a_subject_kind_outside_d158s_enumeration_is_a_corrupt_row() {
    // The members S5 §6 lists and this gear does not declare. D-158 keeps the two
    // stores in step, so one of these arriving here means somebody widened one of
    // them alone.
    //
    // **The token has moved twice, and both moves are the same event.** It was
    // `window` until the three window surfaces mounted; then `overlay`, chosen as
    // *"the next member of S5 §6's enumeration with no writer here"*; and `overlay`
    // stopped being that on 2026-08-06, when D-221 gave the overlay plane its audit
    // writer and `chk_pricing_approval_subject_kind` its token. Asserting either is
    // unreadable would now assert the opposite of what the gear does.
    //
    // `membership` is the next one — S5 §6 lists it, this gear declares no such kind,
    // and Slice 9's membership half is not built. The property the test is named for
    // is unchanged: a token outside `AuditSubjectKind::ALL` is a corrupt row and never
    // silently the wrong variant.
    //
    // The count is deliberately not in this sentence: it read "three kinds" while
    // `AuditSubjectKind` declared four, and its sibling in `sqlite_approval_repo.rs`
    // was updated and this was not. `AuditSubjectKind::ALL` is the roster.
    let err = to_domain(row("submitted", "membership"))
        .expect_err("`membership` is not a kind this gear declares");
    match err {
        RepoError::CorruptRow(detail) => {
            assert!(
                detail.contains("pricing_approval.subject_kind"),
                "got: {detail}"
            );
            assert!(detail.contains("membership"), "got: {detail}");
        }
        other => panic!("expected a corrupt row, got: {other:?}"),
    }
}

#[test]
fn the_reading_carries_the_row_across_unchanged() {
    let record = to_domain(row("approved", "price_unit")).expect("reads");
    assert_eq!(record.approval_id, Uuid::from_u128(0xa1));
    assert_eq!(record.tenant_id, Uuid::from_u128(0x7e));
    assert_eq!(record.content_hash, vec![1, 2, 3]);
    assert_eq!(record.submitter_principal, Uuid::from_u128(0x5b));
    assert_eq!(record.subject_ref, "0000ffff-0000-0000-0000-000000000001/3");
}

#[test]
fn the_price_unit_kind_is_the_one_with_no_writer() {
    // All four members are storable — D-158 requires the store to declare what the
    // audit store declares — and three of them are opened by something here:
    // `ApprovalService::submit` opens a plan revision, `submit_window_mutation`
    // opens a window, and `infra::approval::open_policy_unit` opens the D-10
    // threshold-policy unit. Stated so a later slice submitting a price unit on its
    // own finds the sentence rather than assuming the narrower set was a constraint.
    assert_eq!(
        SUBJECT_KINDS_WITH_A_WRITER,
        &[
            AuditSubjectKind::PlanRevision,
            // **`price_unit` joined the roster on 2026-08-06**, when D-88's
            // `ApprovalService::submit_supersession_on` became its first writer. This
            // assertion asserted the opposite — "price_unit has none" — and stayed true
            // for exactly as long as nothing opened such a unit; a roster is a
            // maintained list, so the day it is wrong is the day a reader takes it as
            // normative and the writer as the mistake.
            AuditSubjectKind::PriceUnit,
            AuditSubjectKind::Window,
            AuditSubjectKind::Policy,
            // **`overlay` joined on 2026-08-06 too**, when D-225's
            // `ApprovalService::submit_overlay_on` became its writer. This assertion kept
            // asserting the opposite — overlay absent — after that landed, which is the
            // exact failure mode this comment already named for `price_unit` one entry up:
            // review finding Z8-5 (2026-08-10) is what caught it.
            AuditSubjectKind::Overlay,
        ]
    );
    assert!(AuditSubjectKind::ALL.contains(&AuditSubjectKind::PriceUnit));

    // **A sixth was minted on 2026-08-11, and this is the question being answered.**
    // The assertion above used to be an equality with `AuditSubjectKind::ALL.len()`,
    // written so that *"the day a fifth is minted, this equality is what asks whether
    // it has a writer"*. `bulk_operation` is the sixth, and the answer is: on the
    // **audit** plane yes — the mass-repricing run's open — and on **this** plane, the
    // approval one, no. `inst-bs-approval`'s batch approval is the unwired unit that
    // would open one, Overlay's own situation before D-225.
    //
    // So the roster is one short of the enum again, deliberately, and the arithmetic
    // names which one is missing rather than only how many. A test asserting a bare
    // count would go green the day `bulk_operation` gained an approval writer and some
    // other member lost its own.
    let without_a_writer: Vec<AuditSubjectKind> = AuditSubjectKind::ALL
        .iter()
        .copied()
        .filter(|kind| !SUBJECT_KINDS_WITH_A_WRITER.contains(kind))
        .collect();
    assert_eq!(
        without_a_writer,
        [AuditSubjectKind::BulkOperation],
        "exactly one declared kind has no approval-plane writer, and it is the bulk operation"
    );
}

// ---------------------------------------------------------------------------
// The compare-and-swap, against the executed `SQLite` mirror
// ---------------------------------------------------------------------------

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_22);
const SUBMITTER: Uuid = Uuid::from_u128(0x5b_01);
const APPROVER: Uuid = Uuid::from_u128(0xab_01);
const SECOND_APPROVER: Uuid = Uuid::from_u128(0xab_02);

async fn harness() -> DBProvider<DbError> {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    DBProvider::<DbError>::new(db)
}

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 3, hour, 0, 0).unwrap()
}

fn pending(approval_id: Uuid) -> NewApproval {
    NewApproval {
        approval_id,
        tenant_id: TENANT,
        subject_ref: "0000ffff-0000-0000-0000-000000000001/3".to_owned(),
        subject_kind: AuditSubjectKind::PlanRevision,
        content_hash: vec![0xde, 0xad, 0xbe, 0xef],
        materiality: json!({ "reason": "noConfiguredThreshold" }),
        // No key: this module's subject is the compare-and-swap, and the register
        // has its own suite. `tests/sqlite_approval_repo.rs` carries the same note
        // at length.
        held_keys: std::collections::BTreeSet::new(),
    }
}

/// One value for the whole module: these tests call the repository directly,
/// where the value the HTTP edge would have established has no producer.
const TEST_CORRELATION: Uuid = Uuid::from_u128(0x_c0_11_a7_10);

/// The stamp an audited call is made under.
fn stamp_of(actor: Uuid, when: DateTime<Utc>) -> AuditStamp {
    AuditStamp {
        actor_principal_id: actor,
        recorded_at: when,
        correlation_id: TEST_CORRELATION,
    }
}

/// The wire status the caller is told, through the ladder the surface uses.
fn status(err: &RepoError) -> u16 {
    CanonicalError::from(repo_failure(err)).status_code()
}

#[tokio::test]
async fn the_second_reviewer_of_one_record_is_told_the_conflict_and_not_a_storage_fault() {
    // Two reviewers, one record, and the state both of them read. The first
    // decision lands; the second arrives at the `UPDATE` still holding the
    // pending record it read a moment earlier — which is precisely what `swap`
    // is called with here, because no sequence of public calls can produce that
    // state (`decide` re-reads immediately before it acts).
    //
    // Without `state = 'submitted'` in the predicate this statement matches on
    // the primary key and overwrites `approver_principal` and `reason` on the
    // row that *is* the evidence of who agreed. The trigger would still refuse
    // it — as a driver error, so the caller reads a 500 for a race whose whole
    // remedy is to re-read.
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let id = Uuid::from_u128(0xca_01);

    open(&conn, &scope, pending(id), stamp_of(SUBMITTER, at(9)))
        .await
        .expect("open");
    decide(
        &conn,
        &scope,
        TENANT,
        id,
        ApprovalDecision::Approve,
        Some(APPROVER),
        None,
        stamp_of(APPROVER, at(11)),
    )
    .await
    .expect("the first reviewer decides");

    let err = swap(
        &conn,
        &scope,
        TENANT,
        id,
        ApprovalState::Rejected,
        Some(SECOND_APPROVER),
        Some("I disagree".to_owned()),
        at(12),
    )
    .await
    .expect_err("the record has moved past the state this caller read");

    match &err {
        RepoError::ApprovalNotPending { state, approval_id } => {
            // The state the row is *actually* in, not the one the loser read: it
            // read `submitted`, and answering with that would be a 409 saying
            // "approval X is submitted; only a submitted record is decidable".
            assert_eq!(state, "approved");
            assert_eq!(approval_id, &id.to_string());
        }
        RepoError::Db(detail) => panic!(
            "the predicate must keep the loser away from the trigger; \
             it reached it and the caller gets a 500: {detail}"
        ),
        other => panic!("expected the pendingness refusal, got: {other:?}"),
    }
    assert_eq!(
        status(&err),
        409,
        "governance sec 5 types this refusal 409, not 500"
    );

    // And the winner's decision stands, untouched.
    let stored = read(&conn, &scope, TENANT, id)
        .await
        .expect("read")
        .expect("still there");
    assert_eq!(stored.state, ApprovalState::Approved);
    assert_eq!(stored.approver_principal, Some(APPROVER));
    assert_eq!(stored.reason, None);
    assert_eq!(stored.decided_at, Some(at(11)));
}

#[tokio::test]
async fn a_swap_that_matches_no_row_is_a_refusal_and_never_a_silent_success() {
    // The other half of the compare-and-swap: the `rows_affected == 0` arm. This
    // record was never opened, so the predicate cannot be what excludes it and
    // no trigger fires — nothing but the arm stands between "the UPDATE touched
    // nothing" and a caller told its decision landed.
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let id = Uuid::from_u128(0xca_02);

    let err = swap(
        &conn,
        &AccessScope::for_tenant(TENANT),
        TENANT,
        id,
        ApprovalState::Approved,
        Some(APPROVER),
        None,
        at(11),
    )
    .await
    .expect_err("an UPDATE that matched nothing decided nothing");

    assert!(
        matches!(&err, RepoError::NotFound { id: named, .. } if named == &id.to_string()),
        "got: {err:?}"
    );
}

#[tokio::test]
async fn a_foreign_scope_writes_nothing_on_the_decision_path() {
    // The BOLA case on the **write** path. `decide`'s own read refuses a foreign
    // caller first, so a suite driving only the public function proves nothing
    // about the `UPDATE`'s gate: drop `.scope_with(scope)` there and every test
    // that goes through `decide` stays green while the statement is free to
    // decide another tenant's approval.
    //
    // What is asserted is therefore the row, not the refusal: whether an
    // unscoped write is *reported* is the `rows_affected` arm's subject, and its
    // own test is above. What must hold here, whatever the caller is told, is
    // that nothing was written.
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let owner = AccessScope::for_tenant(TENANT);
    let id = Uuid::from_u128(0xca_03);
    open(&conn, &owner, pending(id), stamp_of(SUBMITTER, at(9)))
        .await
        .expect("open");

    let _outcome = swap(
        &conn,
        &AccessScope::for_tenant(OTHER_TENANT),
        TENANT,
        id,
        ApprovalState::Approved,
        Some(APPROVER),
        None,
        at(11),
    )
    .await;

    let stored = read(&conn, &owner, TENANT, id)
        .await
        .expect("read")
        .expect("the record is still there");
    assert_eq!(
        stored.state,
        ApprovalState::Submitted,
        "a foreign caller decided this tenant's approval record"
    );
    assert_eq!(stored.approver_principal, None);
    assert_eq!(stored.decided_at, None);
}
