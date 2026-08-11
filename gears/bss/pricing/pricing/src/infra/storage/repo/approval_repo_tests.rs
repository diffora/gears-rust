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
    // **The token has now moved three times, and all three moves are the same
    // event.** It was `window` until the three window surfaces mounted; then
    // `overlay`, chosen as *"the next member of S5 §6's enumeration with no writer
    // here"*; `overlay` stopped being that on 2026-08-06 (D-221's audit writer);
    // this example was then moved to `membership` — and `membership` stopped being
    // that on 2026-08-11, when `group_membership_repo` (Task 4 of the
    // customer-group plane) gave it a writer and `AuditSubjectKind::Membership` a
    // variant. Asserting any of the three unreadable now would assert the opposite
    // of what the gear does — exactly the drift this test is named for catching in
    // *stored data* and had, ironically, twice now failed to catch in *itself*.
    //
    // **The replacement is `not_a_subject_kind`, and it is synthetic rather than
    // borrowed from S5 §6 — that is the fix, not a stronger version of the same
    // pick.** Every prior probe was a real domain word and every one of them
    // eventually got declared: `window` and `overlay` because their writers
    // landed; `membership` the same way, days after it was chosen *for* being
    // undeclared. A fourth real-word pick, `historical_import`, was tried next
    // and rejected before it could repeat the pattern a third time — S5 §6
    // already promises the plane it names a `BackdateGrant` with a mandatory
    // reason and a full audit contract (`05-governance.md`), and it is a
    // declared authz resource in this crate today (`src/authz.rs`'s label,
    // roster and `ResourceType`). A plane the design set already commits to
    // auditing is exactly the kind of "next undeclared member" this test keeps
    // discovering the hard way. No domain word is safe for this probe, because
    // this test's whole claim is that an *undeclared* token is refused, and
    // every domain word here is a candidate to become declared. A token with no
    // referent in the design set or the codebase cannot be declared out from
    // under the test, because there is nothing for anyone to declare — this one
    // is spelled `not_a_subject_kind` specifically so it reads as "this is not a
    // kind" rather than as a plausible next feature to build.
    //
    // The count is deliberately not in this sentence, for the same reason the
    // previous version of this comment gave: `AuditSubjectKind::ALL` is the
    // roster, not a number restated here to go stale again.
    let err = to_domain(row("submitted", "not_a_subject_kind"))
        .expect_err("`not_a_subject_kind` is not a kind this gear declares");
    match err {
        RepoError::CorruptRow(detail) => {
            assert!(
                detail.contains("pricing_approval.subject_kind"),
                "got: {detail}"
            );
            assert!(detail.contains("not_a_subject_kind"), "got: {detail}");
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
fn every_declared_subject_kind_but_membership_has_an_approval_plane_writer() {
    // Six of the seven declared members are storable on this plane — D-158
    // requires the store to declare what the audit store declares, which the
    // roster below is silent about `Membership` for the reason below the roster
    // gives — and each of the six is opened by something here:
    // `ApprovalService::submit` opens a plan revision, `submit_supersession_on`
    // opens a price unit, `submit_window_mutation` opens a window,
    // `infra::approval::open_policy_unit` opens the D-10 threshold-policy unit,
    // `submit_overlay_on` opens an overlay revision, and — as of 2026-08-11 —
    // `api::rest::repricing_runs::advance_on_verdict` opens a bulk operation.
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
            // **`bulk_operation` joined on 2026-08-11**, when
            // `api::rest::repricing_runs::advance_on_verdict` became its writer here —
            // the audit-plane writer (the run's own open) has existed since the token
            // was minted, but this roster is the *approval* plane's and `inst-bs-approval`
            // was unwired until now.
            AuditSubjectKind::BulkOperation,
        ]
    );

    // **The roster was `AuditSubjectKind::ALL`'s whole length for exactly one
    // change.** It had been one short of the enum since the fifth member
    // (`price_unit`, `overlay`, then `bulk_operation` in turn) each landed with the
    // audit-plane writer ahead of the approval one, and the next-minted member —
    // `Membership` (2026-08-11, `group_membership_repo`) — made it one short again,
    // predicted almost word-for-word by the comment this one replaces. This shape
    // computes the gap from `AuditSubjectKind::ALL` rather than hard-coding a count
    // or an `[]`, which is what lets the test notice the *next* addition too,
    // whatever it turns out to be, instead of going stale a third time the way the
    // count-based wording already had once.
    //
    // `Membership` has an audit-plane writer (`group_membership_repo::enroll` /
    // `end_membership`) and no approval-plane one: a membership mutation's
    // materiality (`inst-mm-*`, the renewal-aligned default vs. the immediate /
    // bulk-move material edges) is unwired, and giving it a writer here is what
    // would close this gap — the same way D-221/D-225/`inst-bs-approval` closed the
    // three before it.
    let without_a_writer: Vec<AuditSubjectKind> = AuditSubjectKind::ALL
        .iter()
        .copied()
        .filter(|kind| !SUBJECT_KINDS_WITH_A_WRITER.contains(kind))
        .collect();
    assert_eq!(
        without_a_writer,
        [AuditSubjectKind::Membership],
        "every declared kind but `membership` has an approval-plane writer; \
         wiring `inst-mm-*`'s material edge is what would close this gap"
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
