//! The migration schedule against a real database (`inst-ms-api`,
//! `inst-mg-idem`, `inst-mst-start`, `inst-mst-complete`, `inst-mst-cancel`,
//! `inst-mst-cancel-inflight`, D-34, D-65).
//!
//! Everything here is a property of a **statement** rather than of a branch in
//! Rust, which is why it is a store suite and not a unit one:
//!
//! * **M2's idempotency is the primary key.** `insert_or_load` is an
//!   `INSERT ... ON CONFLICT DO NOTHING` plus a load, so two creates of one
//!   `migration_id` cannot both land however they interleave. A read-then-insert
//!   would pass a unit test and race in production.
//! * **Every flip is a compare-and-swap.** Each `UPDATE` carries the state it
//!   expects to leave in its `WHERE`; delete that conjunct and the "a completed
//!   run cannot be cancelled" case below stops proving anything, because the
//!   refusal would then come from a Rust `if` that a second writer can step past.
//! * **The trigger refuses what the domain refuses, independently.** §4's edge
//!   set is stated in `domain::migration`, in the table's `CHECK`s and in
//!   `trg_pricing_migration_flip_whitelist`. The cases that drive a raw `UPDATE`
//!   past the repository are what prove the third one exists.
//!
//! `sqlite::memory:` serializes writers, so nothing here distinguishes isolation
//! levels; what it proves is which layer produced each refusal.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::migration::MigrationState;
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::migration_repo::{self, NewMigration};
use chrono::{DateTime, Duration, TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use serde_json::json;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

mod common;

use common::{exec, migrated_db};

const TENANT: Uuid = Uuid::from_u128(0x_7e_11_51);
const ACTOR: Uuid = Uuid::from_u128(0x_ac_11);
const SOURCE: Uuid = Uuid::from_u128(0x_50_04_ce);
const TARGET: Uuid = Uuid::from_u128(0x_7a_46_e7);

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 7, hour, 0, 0).unwrap()
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

async fn harness() -> DBProvider<DbError> {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    DBProvider::<DbError>::new(db)
}

fn new_migration(migration_id: Uuid) -> NewMigration {
    NewMigration {
        migration_id,
        tenant_id: TENANT,
        source_plan_id: PlanId::new(SOURCE),
        source_revision: 0,
        target_plan_id: PlanId::new(TARGET),
        effective_at: at(10) + Duration::days(90),
        announced_at: at(10),
        scope: json!({ "kind": "all" }),
        delta_report: json!({ "locked": [], "entitlement": [], "addon": [], "boundary": [] }),
        created_by: ACTOR,
        created_at: at(10),
    }
}

// ---------------------------------------------------------------------------
// M2 — the create is idempotent on the client-supplied key.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_schedule_is_created_once_and_read_back_whole() {
    let provider = harness().await;
    let conn = provider.conn().expect("conn");
    let id = Uuid::now_v7();

    let scheduled = migration_repo::insert_or_load(&conn, &scope(), new_migration(id))
        .await
        .expect("schedule");

    assert!(scheduled.created);
    assert_eq!(scheduled.record.migration_id, id);
    assert_eq!(scheduled.record.state, MigrationState::Scheduled);
    // D-65: nothing is persisted until a start declares itself.
    assert!(scheduled.record.exclusion_snapshot.is_none());
    assert!(scheduled.record.completion_record.is_none());
}

#[tokio::test]
async fn a_retry_of_one_migration_id_returns_the_original_schedule_and_never_a_second() {
    // `inst-ms-api`, verbatim: "a timed-out client retry returns the original
    // schedule, never a second one".
    let provider = harness().await;
    let conn = provider.conn().expect("conn");
    let id = Uuid::now_v7();

    let first = migration_repo::insert_or_load(&conn, &scope(), new_migration(id))
        .await
        .expect("first");

    // A retry whose body differs: the stored schedule is what answers, because
    // the key is the identity and the second body never lands.
    let mut retry = new_migration(id);
    retry.effective_at = at(10) + Duration::days(365);
    let second = migration_repo::insert_or_load(&conn, &scope(), retry)
        .await
        .expect("retry");

    assert!(first.created);
    assert!(!second.created, "the retry must not report a fresh create");
    assert_eq!(second.record, first.record);
    assert_eq!(second.record.effective_at, at(10) + Duration::days(90));

    assert_eq!(
        count_rows(&conn, id).await,
        1,
        "one migration_id must never hold two rows"
    );
}

// ---------------------------------------------------------------------------
// §4's edges, through the repository.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn start_persists_the_exclusion_set_and_a_repeat_start_replays_it_verbatim() {
    // D-65, sharpened 2026-07-31: the set is computed once, persisted, and repeat
    // calls return **that stored snapshot** without re-transitioning, because a
    // recompute could differ from the set the executor already honoured.
    let provider = harness().await;
    let conn = provider.conn().expect("conn");
    let id = Uuid::now_v7();
    migration_repo::insert_or_load(&conn, &scope(), new_migration(id))
        .await
        .expect("schedule");

    let first = migration_repo::start(
        &conn,
        &scope(),
        TENANT,
        id,
        json!({ "locked": ["sub-1"] }),
        at(11),
    )
    .await
    .expect("start")
    .expect("present");

    assert!(first.transitioned);
    assert_eq!(first.record.state, MigrationState::InProgress);

    // The replay carries a *different* set. It must be ignored.
    let replay = migration_repo::start(
        &conn,
        &scope(),
        TENANT,
        id,
        json!({ "locked": ["sub-1", "sub-2"] }),
        at(12),
    )
    .await
    .expect("replay")
    .expect("present");

    assert!(!replay.transitioned, "a replay must not re-transition");
    assert_eq!(
        replay.record.exclusion_snapshot,
        Some(json!({ "locked": ["sub-1"] })),
        "the replay must return the set the executor was already handed"
    );
}

#[tokio::test]
async fn a_started_run_completes_with_its_record() {
    let provider = harness().await;
    let conn = provider.conn().expect("conn");
    let id = Uuid::now_v7();
    migration_repo::insert_or_load(&conn, &scope(), new_migration(id))
        .await
        .expect("schedule");
    migration_repo::start(&conn, &scope(), TENANT, id, json!({}), at(11))
        .await
        .expect("start");

    let completed = migration_repo::complete(
        &conn,
        &scope(),
        TENANT,
        id,
        json!({ "processed": 12, "excluded": ["sub-1"], "failed": [] }),
        at(13),
    )
    .await
    .expect("complete");

    assert_eq!(completed.state, MigrationState::Completed);
    assert_eq!(
        completed.completion_record,
        Some(json!({ "processed": 12, "excluded": ["sub-1"], "failed": [] }))
    );
}

#[tokio::test]
async fn a_scheduled_run_cancels_with_nothing_attempted() {
    let provider = harness().await;
    let conn = provider.conn().expect("conn");
    let id = Uuid::now_v7();
    migration_repo::insert_or_load(&conn, &scope(), new_migration(id))
        .await
        .expect("schedule");

    let cancelled = migration_repo::cancel(
        &conn,
        &scope(),
        TENANT,
        id,
        MigrationState::Scheduled,
        None,
        at(11),
    )
    .await
    .expect("cancel");

    assert_eq!(cancelled.state, MigrationState::Cancelled);
    // Nothing ran, so there is no partial set to list.
    assert!(cancelled.completion_record.is_none());
    assert!(cancelled.exclusion_snapshot.is_none());
}

#[tokio::test]
async fn an_in_flight_run_cancels_and_lists_its_partial_sets() {
    // D-34, the stop-the-bleeding control. Already-migrated subscriptions are
    // unaffected **by construction**: the catalog never held their state.
    let provider = harness().await;
    let conn = provider.conn().expect("conn");
    let id = Uuid::now_v7();
    migration_repo::insert_or_load(&conn, &scope(), new_migration(id))
        .await
        .expect("schedule");
    migration_repo::start(&conn, &scope(), TENANT, id, json!({}), at(11))
        .await
        .expect("start");

    let cancelled = migration_repo::cancel(
        &conn,
        &scope(),
        TENANT,
        id,
        MigrationState::InProgress,
        Some(json!({ "migrated": ["sub-1"], "not_attempted": ["sub-2", "sub-3"] })),
        at(12),
    )
    .await
    .expect("cancel in flight");

    assert_eq!(cancelled.state, MigrationState::Cancelled);
    assert_eq!(
        cancelled.completion_record,
        Some(json!({ "migrated": ["sub-1"], "not_attempted": ["sub-2", "sub-3"] }))
    );
}

#[tokio::test]
async fn a_flip_that_lost_its_race_is_reported_and_never_silently_applied() {
    // The compare-and-swap's whole point: `complete` expects to leave
    // `in_progress`, and this row is still `scheduled`.
    let provider = harness().await;
    let conn = provider.conn().expect("conn");
    let id = Uuid::now_v7();
    migration_repo::insert_or_load(&conn, &scope(), new_migration(id))
        .await
        .expect("schedule");

    let err = migration_repo::complete(&conn, &scope(), TENANT, id, json!({}), at(13))
        .await
        .expect_err("a scheduled run has not started");

    assert!(
        matches!(err, RepoError::ConcurrentMutation { .. }),
        "expected ConcurrentMutation, got {err:?}"
    );

    let still = migration_repo::load(&conn, &scope(), TENANT, id)
        .await
        .expect("load")
        .expect("present");
    assert_eq!(still.state, MigrationState::Scheduled);
}

// ---------------------------------------------------------------------------
// The retirement guard's input.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn only_live_schedules_are_reported_as_targeting_a_plan() {
    // A completed or cancelled run moves nobody, so it must not refuse a
    // retirement. The narrowing is the store's, not the caller's.
    let provider = harness().await;
    let conn = provider.conn().expect("conn");
    let target = PlanId::new(Uuid::from_u128(0x_7a_46_e7));

    let live = Uuid::now_v7();
    migration_repo::insert_or_load(&conn, &scope(), new_migration(live))
        .await
        .expect("live");

    let done = Uuid::now_v7();
    migration_repo::insert_or_load(&conn, &scope(), new_migration(done))
        .await
        .expect("done");
    migration_repo::start(&conn, &scope(), TENANT, done, json!({}), at(11))
        .await
        .expect("start");
    migration_repo::complete(&conn, &scope(), TENANT, done, json!({}), at(13))
        .await
        .expect("complete");

    let targeting = migration_repo::live_targeting(&conn, &scope(), TENANT, target)
        .await
        .expect("live targeting");

    assert_eq!(targeting.len(), 1);
    assert_eq!(targeting[0].migration_id, live);
}

// ---------------------------------------------------------------------------
// The layer below the repository — the guards a raw statement meets.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_store_refuses_a_delete_because_cancel_is_a_state() {
    // An executor re-reading the schedule must be able to tell a cancelled run
    // from an absent one, so absence and cancellation must not be one answer.
    let conn = migrated_db().await;
    let id = Uuid::now_v7();
    seeded_schedule(&conn, id, "scheduled").await;

    let err = exec(
        &conn,
        &format!("DELETE FROM pricing_migration WHERE migration_id = '{id}'"),
    )
    .await
    .expect_err("DELETE must be refused");

    assert!(
        err.to_string()
            .contains("cancel is a state, not a deletion"),
        "{err}"
    );
}

#[tokio::test]
async fn the_store_refuses_an_unsanctioned_edge_the_repository_never_offers() {
    // §4 has no `scheduled -> completed`: a run cannot be reported processed by
    // an executor that never declared it started, because the D-36 re-validation
    // and D-65's exclusion set both hang off that declaration.
    let conn = migrated_db().await;
    let id = Uuid::now_v7();
    seeded_schedule(&conn, id, "scheduled").await;

    let err = exec(
        &conn,
        &format!(
            "UPDATE pricing_migration SET state = 'completed', \
             completed_at = '2026-08-07T13:00:00+00:00' WHERE migration_id = '{id}'"
        ),
    )
    .await
    .expect_err("scheduled -> completed must be refused");

    assert!(
        err.to_string().contains("not a sanctioned one")
            || err.to_string().contains("started_required"),
        "{err}"
    );
}

#[tokio::test]
async fn the_store_freezes_the_schedule_time_delta_report() {
    // §6 calls it the deltas "at schedule time": it is the evidence of what the
    // operator confirmed against, so D-36's execution-time re-resolution lands in
    // `exclusion_snapshot` instead and cannot overwrite it.
    let conn = migrated_db().await;
    let id = Uuid::now_v7();
    seeded_schedule(&conn, id, "scheduled").await;

    let err = exec(
        &conn,
        &format!(
            "UPDATE pricing_migration SET delta_report = '{{\"locked\":[\"sub-9\"]}}' \
             WHERE migration_id = '{id}'"
        ),
    )
    .await
    .expect_err("the schedule-time report is frozen");

    assert!(
        err.to_string()
            .contains("bound to its source, target, effective date, scope"),
        "{err}"
    );
}

#[tokio::test]
async fn the_store_refuses_a_second_exclusion_set_on_a_started_run() {
    // D-65's replay half, guarded below the repository: a recompute could differ
    // from the set the executor already honoured.
    let conn = migrated_db().await;
    let id = Uuid::now_v7();
    seeded_schedule(&conn, id, "in_progress").await;

    let err = exec(
        &conn,
        &format!(
            "UPDATE pricing_migration SET exclusion_snapshot = '{{\"locked\":[\"sub-9\"]}}' \
             WHERE migration_id = '{id}'"
        ),
    )
    .await
    .expect_err("the exclusion set is replayed, never recomputed");

    assert!(
        err.to_string()
            .contains("computed once and replayed verbatim"),
        "{err}"
    );
}

#[tokio::test]
async fn a_terminal_run_is_immutable_history() {
    let conn = migrated_db().await;
    let id = Uuid::now_v7();
    seeded_schedule(&conn, id, "cancelled").await;

    let err = exec(
        &conn,
        &format!(
            "UPDATE pricing_migration SET completion_record = '{{}}' WHERE migration_id = '{id}'"
        ),
    )
    .await
    .expect_err("a cancelled run is history");

    assert!(
        err.to_string()
            .contains("completed or cancelled run is immutable history"),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/// One schedule seeded through raw SQL, for the cases that drive statements
/// **past** the repository — the layer that cannot see a guard stop refusing.
async fn seeded_schedule(conn: &sea_orm::DatabaseConnection, id: Uuid, state: &str) {
    let started = if state == "scheduled" {
        ("NULL".to_owned(), "NULL".to_owned())
    } else {
        (
            "'2026-08-07T11:00:00+00:00'".to_owned(),
            "'{\"locked\":[]}'".to_owned(),
        )
    };
    let cancelled = if state == "cancelled" {
        "'2026-08-07T12:00:00+00:00'"
    } else {
        "NULL"
    };
    let completed = if state == "completed" {
        "'2026-08-07T13:00:00+00:00'"
    } else {
        "NULL"
    };
    common::must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_migration (
                 migration_id, tenant_id, source_plan_id, source_revision, target_plan_id,
                 effective_at, announced_at, scope, state, delta_report,
                 exclusion_snapshot, completion_record, created_by, created_at,
                 started_at, completed_at, cancelled_at)
             VALUES ('{id}', '{TENANT}', '{SOURCE}', 0, '{TARGET}',
                 '2026-11-05T10:00:00+00:00', '2026-08-07T10:00:00+00:00',
                 '{{\"kind\":\"all\"}}', '{state}', '{{\"locked\":[]}}',
                 {}, NULL, '{ACTOR}', '2026-08-07T10:00:00+00:00',
                 {}, {completed}, {cancelled})",
            started.1, started.0
        ),
    )
    .await;
}

async fn count_rows(conn: &impl toolkit_db::secure::DBRunner, id: Uuid) -> usize {
    migration_repo::live_targeting(conn, &scope(), TENANT, PlanId::new(TARGET))
        .await
        .expect("live targeting")
        .iter()
        .filter(|record| record.migration_id == id)
        .count()
}
