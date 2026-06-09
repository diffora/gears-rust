//! `SQLite` unit tests for [`account_management::infra::lease::LeaseManager`].
//!
//! Covers the acquire path: fresh INSERT, contention rejection, and
//! expired-row steal with `attempts` bump. The fence-in-tx and
//! heartbeat paths live in `lease_guard_sqlite.rs`; multi-replica
//! contention (40001 retry, concurrent acquire races) is exercised by
//! `coord_lease_integration_pg.rs` because the `SQLite` single-writer
//! model does not surface those scenarios.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::time::Duration;

use sea_orm::{ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use toolkit_db::secure::SecureEntityExt;
use toolkit_security::AccessScope;
use uuid::Uuid;

use account_management::infra::lease::{CoordError, LeaseManager};
use account_management::infra::storage::entity::am_leases;

use common::setup_sqlite;

const KEY: &str = "hierarchy_integrity";
const TTL: Duration = Duration::from_mins(10);

#[tokio::test]
async fn acquire_inserts_when_free() {
    let harness = setup_sqlite().await.expect("setup_sqlite");
    let db = harness.provider.db();
    let mgr = LeaseManager::new(db.clone());

    let guard = mgr.acquire(KEY, TTL).await.expect("acquire");

    let row = read_row(&harness.provider, KEY).await.expect("row present");
    assert_eq!(row.key, KEY);
    assert_eq!(row.locked_by, Some(guard.locked_by()));
    assert_eq!(row.attempts, 1, "fresh acquire records attempts=1");
    assert!(
        row.locked_until > OffsetDateTime::now_utc(),
        "locked_until must be in the future ({} vs now)",
        row.locked_until
    );
}

#[tokio::test]
async fn acquire_returns_held_when_active() {
    let harness = setup_sqlite().await.expect("setup_sqlite");
    let db = harness.provider.db();
    let mgr = LeaseManager::new(db.clone());

    // Pre-seed an active lease held by someone else.
    let peer = Uuid::new_v4();
    let far_future = OffsetDateTime::now_utc() + Duration::from_hours(1);
    insert_lease_row(&harness.provider, KEY, Some(peer), far_future, 1)
        .await
        .expect("seed peer lease");

    match mgr.acquire(KEY, TTL).await {
        Err(CoordError::LeaseHeld) => {}
        other => panic!("expected LeaseHeld, got {other:?}"),
    }

    // Row unchanged: still held by the peer with attempts=1.
    let row = read_row(&harness.provider, KEY).await.expect("row present");
    assert_eq!(row.locked_by, Some(peer));
    assert_eq!(row.attempts, 1, "rejected acquire must not bump attempts");
}

#[tokio::test]
async fn acquire_steals_when_expired() {
    let harness = setup_sqlite().await.expect("setup_sqlite");
    let db = harness.provider.db();
    let mgr = LeaseManager::new(db.clone());

    // Pre-seed an expired lease held by someone else.
    let peer = Uuid::new_v4();
    let epoch = OffsetDateTime::UNIX_EPOCH;
    insert_lease_row(&harness.provider, KEY, Some(peer), epoch, 1)
        .await
        .expect("seed expired peer lease");

    let guard = mgr.acquire(KEY, TTL).await.expect("steal expired lease");
    assert_ne!(
        guard.locked_by(),
        peer,
        "stealer must hold a fresh uuid, not the peer's"
    );

    let row = read_row(&harness.provider, KEY).await.expect("row present");
    assert_eq!(
        row.locked_by,
        Some(guard.locked_by()),
        "row's locked_by switched to the stealer"
    );
    assert_eq!(
        row.attempts, 2,
        "steal must bump attempts (peer's 1 -> 2 for the takeover)"
    );
    assert!(
        row.locked_until > OffsetDateTime::now_utc(),
        "locked_until pushed forward by ttl"
    );
}

#[tokio::test]
async fn acquire_writes_db_clock_locked_until() {
    let harness = setup_sqlite().await.expect("setup_sqlite");
    let db = harness.provider.db();
    let mgr = LeaseManager::new(db.clone());

    let before = OffsetDateTime::now_utc();
    let _guard = mgr
        .acquire(KEY, Duration::from_mins(1))
        .await
        .expect("acquire");
    let after = OffsetDateTime::now_utc();

    let row = read_row(&harness.provider, KEY).await.expect("row present");
    // Worker-clock INSERT (variant (a) in plan §4.3): locked_until ~
    // now + ttl. Allow a wide window to absorb test-runner jitter.
    assert!(
        row.locked_until > before,
        "locked_until {} must be after acquire start {}",
        row.locked_until,
        before
    );
    assert!(
        row.locked_until < after + Duration::from_secs(70),
        "locked_until {} must be at most ttl past acquire end {}",
        row.locked_until,
        after
    );
}

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

async fn read_row(
    provider: &std::sync::Arc<account_management::infra::storage::repo_impl::AmDbProvider>,
    key: &str,
) -> Option<am_leases::Model> {
    let conn = provider.conn().expect("conn");
    am_leases::Entity::find()
        .filter(am_leases::Column::Key.eq(key))
        .secure()
        .scope_with(&AccessScope::allow_all())
        .one(&conn)
        .await
        .expect("read am_leases row")
}

async fn insert_lease_row(
    provider: &std::sync::Arc<account_management::infra::storage::repo_impl::AmDbProvider>,
    key: &str,
    locked_by: Option<Uuid>,
    locked_until: OffsetDateTime,
    attempts: i32,
) -> anyhow::Result<()> {
    let conn = provider.conn()?;
    let am = am_leases::ActiveModel {
        key: ActiveValue::Set(key.to_owned()),
        locked_by: ActiveValue::Set(locked_by),
        locked_until: ActiveValue::Set(locked_until),
        attempts: ActiveValue::Set(attempts),
    };
    toolkit_db::secure::secure_insert::<am_leases::Entity>(am, &AccessScope::allow_all(), &conn)
        .await
        .map_err(|e| anyhow::anyhow!("insert am_leases failed: {e:?}"))?;
    Ok(())
}
