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

use std::collections::BTreeSet;

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

/// The roster is the set of kinds this crate **actually opens a unit of**, and
/// this test is what makes that sentence checkable.
///
/// # Why the operands are not what they were
///
/// The assertion this replaces compared [`SUBJECT_KINDS_WITH_A_WRITER`] against
/// a hard-coded copy of itself, and then compared `ALL \ ROSTER` against a
/// hard-coded copy of *that*. Both operands moved with the constant: an author
/// adding a kind to the roster edited the literal in the same keystroke, and an
/// author adding a **writer** without touching the roster moved neither. So the
/// test could only ever fail on a typo, and the drift it is named for went
/// through it three times — `price_unit` (D-88), `overlay` (D-225, caught by
/// review finding Z8-5 rather than by this test) and now `membership`, whose
/// writer `ApprovalService::submit_membership_move_on` landed on 2026-08-12
/// while the roster and its own doc kept saying the plane had none.
///
/// The operand here moves **independently**: it is the crate's own source, read
/// at test time, and specifically every production construction of
/// [`NewApproval`] — the one value that can reach [`open`], and therefore the
/// one thing that makes a kind *written* on this plane. A writer cannot be added
/// without constructing one, so the scan sees the new kind the moment the writer
/// compiles, whether or not anybody remembers this file.
///
/// # Why a source scan rather than a driver
///
/// `tests/sqlite_publish_commit.rs`'s `every_declared_action_has_a_production_writer`
/// drives every audited path and reads the rows back, which is the stronger
/// shape and the one to prefer where the paths are already driven. It is not
/// available here: three of the seven kinds are opened only behind a REST route
/// with an idempotency gate, a threshold policy and an approved verdict in front
/// of them, and a census that had to build all of that would be a publish-plane
/// integration test that happens to end in an assertion about a constant.
/// `tests/rest_authz.rs`'s `every_mounted_router_is_merged_into_both_censuses`
/// is the precedent for reading this crate's sources to bind a roster to the
/// code rather than to a second copy of the roster.
#[test]
fn the_roster_is_exactly_the_kinds_production_opens_a_unit_of() {
    let opened = kinds_production_opens();

    // The floor is the broken-parse guard `every_mounted_router_is_merged_into_both_censuses`
    // carries for the same reason: a scan that silently matched nothing would
    // make this test assert that the roster is empty, and pass the day somebody
    // emptied it.
    assert!(
        opened.len() >= 5,
        "the scan found {opened:?}, which is fewer kinds than this gear has opened units of \
         since D-88 — the scan is broken, not the roster"
    );

    let roster: BTreeSet<AuditSubjectKind> = SUBJECT_KINDS_WITH_A_WRITER.iter().copied().collect();
    assert_eq!(
        roster,
        opened,
        "the roster and the crate's own approval-unit writers disagree. Kinds a writer opens \
         and the roster omits: {:?}. Kinds the roster claims and nothing opens: {:?}. \
         `SUBJECT_KINDS_WITH_A_WRITER` and its doc comment are a maintained list — the day \
         they are wrong is the day a reader takes the roster as normative and the writer as \
         the mistake.",
        opened.difference(&roster).collect::<Vec<_>>(),
        roster.difference(&opened).collect::<Vec<_>>(),
    );

    // The roster is a *set* spelled as a slice, so the two properties a slice can
    // break and a set cannot are asserted rather than assumed: no kind twice, and
    // `AuditSubjectKind::ALL`'s order, which is the order every other roster in
    // this crate is read in.
    assert_eq!(
        SUBJECT_KINDS_WITH_A_WRITER.len(),
        roster.len(),
        "the roster names a kind twice: {SUBJECT_KINDS_WITH_A_WRITER:?}"
    );
    let in_declaration_order: Vec<AuditSubjectKind> = AuditSubjectKind::ALL
        .iter()
        .copied()
        .filter(|kind| roster.contains(kind))
        .collect();
    assert_eq!(
        SUBJECT_KINDS_WITH_A_WRITER, in_declaration_order,
        "the roster is out of `AuditSubjectKind::ALL` order"
    );
}

/// Every [`AuditSubjectKind`] some **production** source of this crate opens an
/// approval unit of.
///
/// The evidence is a `NewApproval { … subject_kind: AuditSubjectKind::X … }`
/// construction, which every writer performs and which nothing but a writer has
/// a reason to perform: `open` takes the value and the store takes it from
/// `open` alone. Every site spells the kind as a literal path — a caller passing
/// a variable would be a writer of *whatever kind it was handed*, which no
/// caller in this crate is and which this scan would (deliberately) not count,
/// since a roster cannot name a kind nobody can predict.
///
/// `*_tests.rs` files are skipped: a test opening a unit of some kind proves the
/// **store** takes it (that is `sqlite_approval_repo`'s
/// `every_subject_kind_d158_declares_is_storable_on_the_mirror`), not that
/// anything in production ever does. Counting them would make the roster mean
/// "storable", which is D-158's property and already has its own test.
fn kinds_production_opens() -> BTreeSet<AuditSubjectKind> {
    let mut found = BTreeSet::new();
    for path in production_sources() {
        let text = std::fs::read_to_string(&path).expect("a readable source file");
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            // The type's own declaration ends in the same four characters and
            // writes nothing.
            if !line.trim_end().ends_with("NewApproval {") || line.contains("struct ") {
                continue;
            }
            // The field is written within the initializer, and `NewApproval`'s
            // longest is nine lines of fields plus their comments. Bounded rather
            // than scanned to the closing brace: a brace matcher over Rust source
            // is a parser, and what this needs is the next `subject_kind:`.
            let field = lines[index..]
                .iter()
                .take(40)
                .find_map(|line| line.trim_start().strip_prefix("subject_kind:"))
                .unwrap_or_else(|| {
                    panic!(
                        "{}:{} constructs a `NewApproval` and names no `subject_kind` within \
                         40 lines; the scan cannot tell which kind it writes",
                        path.display(),
                        index + 1
                    )
                });
            let Some(rest) = field.trim().strip_prefix("AuditSubjectKind::") else {
                panic!(
                    "{}:{} opens a unit whose kind is not a literal `AuditSubjectKind::…`, but \
                     {field:?}; a roster cannot name a kind chosen at run time",
                    path.display(),
                    index + 1
                )
            };
            let variant = rest
                .trim_end_matches(',')
                .trim_end_matches(|c: char| !c.is_alphanumeric());
            let kind = AuditSubjectKind::ALL
                .iter()
                .copied()
                .find(|kind| format!("{kind:?}") == variant)
                .unwrap_or_else(|| {
                    panic!(
                        "{}:{} opens a `AuditSubjectKind::{variant}`, which is not a member of \
                         `AuditSubjectKind::ALL`",
                        path.display(),
                        index + 1
                    )
                });
            found.insert(kind);
        }
    }
    found
}

/// Every `.rs` file under `src`, recursively, that is not a test module.
///
/// Recursive for `rest_sources`' reason one suite over: a scan that reads one
/// directory level is a guard a reorganisation switches off silently.
fn production_sources() -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("the crate's own source tree") {
            let path = entry.expect("a readable dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with("_tests.rs"))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
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
