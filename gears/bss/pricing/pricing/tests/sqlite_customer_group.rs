//! `group_membership_repo` against the executed `SQLite` mirror — the
//! repository-level half of D-09's non-overlap invariant
//! (`design/09-price-overlays.md` §3 `inst-cg-record` / `inst-cg-resolve`,
//! `inst-mm-audit`, `inst-ms-time`).
//!
//! # What this suite proves that the migration's own suites do not
//!
//! `m20260802_000067`'s module doc reports the repository-level half of D-09 as
//! **owed** — "no `membership_repo` ... exists yet" — and `tests/sqlite_migrations.rs`'s
//! trigger census proves the schema's own guard fires, not that a caller reads a
//! named refusal before it does. This suite is that second thing: it proves
//! [`group_membership_repo::refuse_overlap`] answers `MEMBERSHIP_OVERLAP` /
//! `MEMBERSHIP_CONFLICT` **before** the statement that would trip the `SQLite`
//! trigger is even issued, and that the trigger stays a backstop this suite
//! never has to reach.
//!
//! # The overlap case that matters is cross-group
//!
//! `an_enrollment_overlapping_another_group_is_refused_by_name` puts a payer in
//! `groupA` and then enrolls them into `groupB` over an intersecting interval —
//! the shape the migration's own module doc calls out by name as "not merely the
//! narrower same-group case `MEMBERSHIP_OVERLAP` names": D-09's non-overlap rule
//! is per payer **across every group**, and a suite that only ever collided one
//! group with itself would never exercise the equality list's actual point (the
//! exclusion constraint's `tenant_id`/`payer_tenant_id` pair, with `group_value`
//! deliberately absent from it).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::entity::audit_log;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::group_membership_repo::{self, NewMembership};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureEntityExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_22);
const ACTOR: Uuid = Uuid::from_u128(0xac_01);
const PAYER: Uuid = Uuid::from_u128(0xda_01);
const OTHER_PAYER: Uuid = Uuid::from_u128(0xda_02);
const TEST_CORRELATION: Uuid = Uuid::from_u128(0x_c0_11_a7_10);

const GROUP_A: &str = "groupA";
const GROUP_B: &str = "groupB";

fn stamp_at(when: DateTime<Utc>) -> AuditStamp {
    AuditStamp {
        actor_principal_id: ACTOR,
        recorded_at: when,
        correlation_id: TEST_CORRELATION,
    }
}

/// `2099-09-<day>T00:00:00Z` — `window_repo`'s own test helper's reason: every
/// instant here is a **fact rather than a fixture that ages**, so "future" stays
/// true no matter when this suite runs.
fn t(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 9, day, 0, 0, 0).unwrap()
}

async fn provider() -> DBProvider<DbError> {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    DBProvider::<DbError>::new(db)
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

fn new_membership(
    membership_id: Uuid,
    payer_tenant_id: Uuid,
    group_value: &str,
    from: DateTime<Utc>,
    to: Option<DateTime<Utc>>,
) -> NewMembership {
    NewMembership {
        membership_id,
        tenant_id: TENANT,
        payer_tenant_id,
        group_value: group_value.to_owned(),
        effective_from: from,
        effective_to: to,
    }
}

/// How many audit records name one membership.
async fn audit_records_for(
    provider: &DBProvider<DbError>,
    subject_ref: &str,
) -> Vec<audit_log::Model> {
    let conn = provider.conn().expect("conn");
    audit_log::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(
            Condition::all()
                .add(audit_log::Column::SubjectKind.eq("membership"))
                .add(audit_log::Column::SubjectRef.eq(subject_ref)),
        )
        .all(&conn)
        .await
        .expect("read audit rows")
}

// ---------------------------------------------------------------------------
// D-09's non-overlap invariant, refused by name at the repository.
// ---------------------------------------------------------------------------

/// **The statement only the cross-group half of D-09 can refuse.** A payer
/// holds `groupA` over `[t1, t3)`; enrolling them into a *different* group
/// `groupB` over the intersecting `[t2, t4)` (`t1 < t2 < t3 < t4`) must be
/// refused, and refused **by name**: `RepoError::MembershipConflict`, not a
/// generic storage failure and not silence.
#[tokio::test]
async fn an_enrollment_overlapping_another_group_is_refused_by_name() {
    let provider = provider().await;
    let conn = provider.conn().expect("conn");

    group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_01), PAYER, GROUP_A, t(1), Some(t(10))),
        stamp_at(t(1)),
    )
    .await
    .expect("first enrollment");

    let refusal = group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_02), PAYER, GROUP_B, t(5), Some(t(15))),
        stamp_at(t(5)),
    )
    .await
    .expect_err("a cross-group collision must be refused");

    match refusal {
        RepoError::MembershipConflict {
            payer_tenant_id, ..
        } => {
            assert_eq!(payer_tenant_id, PAYER.to_string());
        }
        other => panic!("expected MembershipConflict, got {other:?}"),
    }

    // And nothing landed: the payer still resolves to exactly the first
    // membership.
    let held = group_membership_repo::intervals_for_payer(&conn, &scope(), TENANT, PAYER)
        .await
        .expect("read back");
    assert_eq!(held.len(), 1, "the refused enrollment wrote no row");
    assert_eq!(held[0].group_value, GROUP_A);
}

/// The narrower same-group case §5 also names, `MEMBERSHIP_OVERLAP`: two
/// intervals in **the same** group collide too — `group_value` is absent from
/// the exclusion constraint's equality list, so a same-group collision is not
/// a case the schema treats specially, and neither does this refusal.
#[tokio::test]
async fn an_enrollment_overlapping_the_same_group_is_refused_by_name() {
    let provider = provider().await;
    let conn = provider.conn().expect("conn");

    group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_03), PAYER, GROUP_A, t(1), Some(t(10))),
        stamp_at(t(1)),
    )
    .await
    .expect("first enrollment");

    let refusal = group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_04), PAYER, GROUP_A, t(5), Some(t(15))),
        stamp_at(t(5)),
    )
    .await
    .expect_err("a same-group collision must be refused");

    assert!(
        matches!(refusal, RepoError::MembershipOverlap { .. }),
        "expected MembershipOverlap, got {refusal:?}"
    );
}

/// **The open-ended collision, unproven at this layer until now.** An
/// open-ended membership (`effective_to: None`) has no stored upper bound to
/// compare against, and `intersects`'s `None` arm is what reads that as
/// infinity rather than as "no data" — the same NULL-safety the migration's
/// `SQLite` trigger carries in its `WHERE EXISTS`
/// (`m20260802_000067`'s module doc). Proven only on the Docker-gated
/// `tests/postgres_group_membership.rs` suite until this case: a bounded
/// enrollment that starts inside an *open-ended* predecessor must be refused
/// **by name** at this repository, not only by the trigger.
#[tokio::test]
async fn an_enrollment_colliding_with_an_open_ended_membership_is_refused_by_name() {
    let provider = provider().await;
    let conn = provider.conn().expect("conn");

    group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_0e), PAYER, GROUP_A, t(1), None),
        stamp_at(t(1)),
    )
    .await
    .expect("open-ended enrollment");

    let refusal = group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_0f), PAYER, GROUP_B, t(10), Some(t(20))),
        stamp_at(t(10)),
    )
    .await
    .expect_err("a bounded interval starting inside an open-ended one must be refused");

    match refusal {
        RepoError::MembershipConflict {
            payer_tenant_id, ..
        } => {
            assert_eq!(payer_tenant_id, PAYER.to_string());
        }
        other => panic!("expected MembershipConflict, got {other:?}"),
    }

    let held = group_membership_repo::intervals_for_payer(&conn, &scope(), TENANT, PAYER)
        .await
        .expect("read back");
    assert_eq!(held.len(), 1, "the refused enrollment wrote no row");
    assert_eq!(
        held[0].effective_to, None,
        "the open-ended predecessor is untouched"
    );
}

/// **A rule that refuses two sequential future-dated memberships is wrong** —
/// the 2026-07-28 review fix `design/09-price-overlays.md` §3 records: "scheduled
/// sequential future-dated memberships are legal". Two intervals with a real gap
/// between them, both starting after `stamp_at`'s own instant, must both be
/// accepted.
#[tokio::test]
async fn two_sequential_future_dated_memberships_are_accepted() {
    let provider = provider().await;
    let conn = provider.conn().expect("conn");

    let first = group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_05), PAYER, GROUP_A, t(10), Some(t(20))),
        stamp_at(t(1)),
    )
    .await
    .expect("first future-dated enrollment");

    let second = group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_06), PAYER, GROUP_B, t(25), Some(t(30))),
        stamp_at(t(1)),
    )
    .await
    .expect("second future-dated enrollment, with a real gap after the first");

    assert_eq!(first.group_value, GROUP_A);
    assert_eq!(second.group_value, GROUP_B);

    let held = group_membership_repo::intervals_for_payer(&conn, &scope(), TENANT, PAYER)
        .await
        .expect("read back");
    assert_eq!(held.len(), 2, "both scheduled memberships landed");
}

/// **The interval is half-open, and this is what proves it rather than assumes
/// it.** An interval starting exactly where another ends (`t3 == t2`) is legal:
/// `effective_to = next.effective_from` is adjacency, not a collision — §9's own
/// rule, and the migration's `[)` range spec on the Postgres side of the same
/// table.
#[tokio::test]
async fn an_interval_starting_exactly_where_another_ends_is_legal() {
    let provider = provider().await;
    let conn = provider.conn().expect("conn");

    group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_07), PAYER, GROUP_A, t(1), Some(t(10))),
        stamp_at(t(1)),
    )
    .await
    .expect("first enrollment");

    // Starts exactly at t(10), where the first ends. Different group, so a
    // failure here could only be the adjacency arithmetic getting this wrong —
    // not a same-group / cross-group distinction.
    let adjacent = group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_08), PAYER, GROUP_B, t(10), Some(t(20))),
        stamp_at(t(1)),
    )
    .await
    .expect("an interval starting where the prior one ends must be accepted");

    assert_eq!(adjacent.effective_from, t(10));

    let held = group_membership_repo::intervals_for_payer(&conn, &scope(), TENANT, PAYER)
        .await
        .expect("read back");
    assert_eq!(held.len(), 2, "both adjacent memberships landed");
}

// ---------------------------------------------------------------------------
// The tenant gate.
// ---------------------------------------------------------------------------

/// **Another tenant's membership is invisible and unwritable.** The same
/// payer id, held under a *different* tenant's row, answers absence to both a
/// read and a write — deliberately the same answer either way, `window_repo`'s
/// reason: membership is payer-level commercial data and a foreign scope must
/// not learn a row exists by being told "forbidden" instead of "not found".
#[tokio::test]
async fn another_tenants_membership_is_invisible_and_unwritable() {
    let provider = provider().await;
    let conn = provider.conn().expect("conn");

    let created = group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_09), OTHER_PAYER, GROUP_A, t(1), None),
        stamp_at(t(1)),
    )
    .await
    .expect("enrolled under TENANT");

    // Invisible: a read scoped to the other tenant sees nothing, even asking
    // about the very payer TENANT enrolled.
    let held = group_membership_repo::intervals_for_payer(
        &conn,
        &AccessScope::for_tenant(OTHER_TENANT),
        OTHER_TENANT,
        OTHER_PAYER,
    )
    .await
    .expect("read under the other tenant's scope");
    assert!(
        held.is_empty(),
        "another tenant's membership must not be visible"
    );

    // Unwritable: ending it under the other tenant's scope is answered as
    // absence, not as a permission error and not as success.
    let refusal = group_membership_repo::end_membership(
        &conn,
        &AccessScope::for_tenant(OTHER_TENANT),
        OTHER_TENANT,
        created.membership_id,
        t(5),
        created.row_version,
        stamp_at(t(5)),
    )
    .await
    .expect_err("another tenant must not be able to end this membership");
    assert!(
        matches!(refusal, RepoError::NotFound { .. }),
        "expected NotFound, got {refusal:?}"
    );

    // And it is untouched under its own tenant's scope.
    let held = group_membership_repo::intervals_for_payer(&conn, &scope(), TENANT, OTHER_PAYER)
        .await
        .expect("read under the owning tenant's scope");
    assert_eq!(held.len(), 1);
    assert_eq!(
        held[0].effective_to, None,
        "the other tenant's write never landed"
    );
}

// ---------------------------------------------------------------------------
// `inst-mm-audit`: every mutation writes exactly one audit record.
// ---------------------------------------------------------------------------

/// An `enroll` writes exactly one audit record, and it names the membership it
/// wrote — the test asserts the `subject_ref` **value**, not merely that a row
/// exists (Z9-1's own lesson: a cutover missed writing this trail at all, and a
/// row-count-only assertion would not have caught a `subject_ref` pointing at
/// the wrong thing).
#[tokio::test]
async fn an_enrollment_writes_exactly_one_audit_record_naming_the_membership() {
    let provider = provider().await;
    let conn = provider.conn().expect("conn");

    let created = group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_0a), PAYER, GROUP_A, t(1), Some(t(10))),
        stamp_at(t(1)),
    )
    .await
    .expect("enroll");

    let subject_ref = created.membership_id.to_string();
    let records = audit_records_for(&provider, &subject_ref).await;

    assert_eq!(records.len(), 1, "one enroll is one act");
    assert_eq!(records[0].subject_ref, subject_ref);
    assert_eq!(records[0].action, "create");
    assert_eq!(records[0].tenant_id, TENANT);
}

/// A refused enrollment writes **no** audit record — nothing happened.
#[tokio::test]
async fn a_refused_enrollment_writes_no_audit_record() {
    let provider = provider().await;
    let conn = provider.conn().expect("conn");

    group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_0b), PAYER, GROUP_A, t(1), Some(t(10))),
        stamp_at(t(1)),
    )
    .await
    .expect("seed");

    let refused_id = Uuid::from_u128(0xb0_0c);
    group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(refused_id, PAYER, GROUP_B, t(5), Some(t(15))),
        stamp_at(t(5)),
    )
    .await
    .expect_err("refused");

    let records = audit_records_for(&provider, &refused_id.to_string()).await;
    assert!(records.is_empty(), "the refused enrollment wrote nothing");
}

/// `end_membership` — `inst-ms-time`'s "ending early = setting `to` (audited)"
/// — writes exactly one **more** audit record naming the same membership, and
/// the interval it read back moved.
#[tokio::test]
async fn ending_a_membership_writes_exactly_one_more_audit_record() {
    let provider = provider().await;
    let conn = provider.conn().expect("conn");

    let created = group_membership_repo::enroll(
        &conn,
        &scope(),
        TENANT,
        new_membership(Uuid::from_u128(0xb0_0d), PAYER, GROUP_A, t(1), None),
        stamp_at(t(1)),
    )
    .await
    .expect("enroll open-ended");
    assert_eq!(created.effective_to, None);

    let ended = group_membership_repo::end_membership(
        &conn,
        &scope(),
        TENANT,
        created.membership_id,
        t(20),
        created.row_version,
        stamp_at(t(20)),
    )
    .await
    .expect("end");

    assert_eq!(ended.effective_to, Some(t(20)));
    assert_eq!(ended.row_version, created.row_version + 1);

    let subject_ref = created.membership_id.to_string();
    let records = audit_records_for(&provider, &subject_ref).await;
    assert_eq!(records.len(), 2, "the enroll's record, plus the end's");
    assert_eq!(records[1].action, "update");
    assert_eq!(records[1].subject_ref, subject_ref);
}
