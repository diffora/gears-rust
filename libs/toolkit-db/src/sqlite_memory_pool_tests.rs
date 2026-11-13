#![allow(clippy::unwrap_used, clippy::expect_used)]

//! An in-memory `SQLite` database exists only while one of its connections is open, so a
//! pool that replaces an idle or aged connection silently starts over with an empty one.
//! Both connection paths, the gear configuration (`build_db_handle`) and a bare DSN
//! (`DbHandle::connect`), must disable connection expiry for a memory database even when
//! the pool configuration asks for it, and must keep that configuration for a file database.
//! The assertions read the options of the pool actually built.

use std::time::Duration;

use sea_orm::ConnectionTrait;
use toolkit_utils::SecretString;

use crate::config::{DbConnConfig, DbEngineCfg, PoolCfg};
use crate::options::build_db_handle;
use crate::{ConnectOpts, DbHandle};

const EXPIRY: Duration = Duration::from_secs(5);

fn expiring_pool() -> PoolCfg {
    PoolCfg {
        idle_timeout: Some(EXPIRY),
        max_lifetime: Some(EXPIRY),
        ..PoolCfg::default()
    }
}

fn expiring_connect_opts() -> ConnectOpts {
    ConnectOpts {
        idle_timeout: Some(EXPIRY),
        max_lifetime: Some(EXPIRY),
        ..ConnectOpts::default()
    }
}

/// The configured expiries of the handle's pool: `(idle_timeout, max_lifetime)`.
fn expiries(handle: &DbHandle) -> (Option<Duration>, Option<Duration>) {
    let sea = handle.sea_internal();
    let options = sea.get_sqlite_connection_pool().options();
    (options.get_idle_timeout(), options.get_max_lifetime())
}

/// Writes a row and reads it back through the pool.
async fn assert_round_trip(handle: &DbHandle) {
    let conn = handle.sea_internal();
    conn.execute_unprepared("CREATE TABLE evidence (x INTEGER)")
        .await
        .unwrap();
    conn.execute_unprepared("INSERT INTO evidence VALUES (1)")
        .await
        .unwrap();
    let row = conn
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT x FROM evidence",
        ))
        .await
        .unwrap();
    assert!(row.is_some(), "the written row is not readable");
}

#[tokio::test]
async fn configured_memory_database_disables_explicit_expiry() {
    let cfg = DbConnConfig {
        dsn: Some(SecretString::new("sqlite::memory:")),
        pool: Some(expiring_pool()),
        ..DbConnConfig::default()
    };
    let handle = build_db_handle(cfg, None).await.unwrap();

    assert_eq!(expiries(&handle), (None, None));
    assert_round_trip(&handle).await;
}

#[tokio::test]
async fn dsn_memory_database_disables_explicit_expiry() {
    let handle = DbHandle::connect("sqlite::memory:", expiring_connect_opts())
        .await
        .unwrap();

    assert_eq!(expiries(&handle), (None, None));
    assert_round_trip(&handle).await;
}

#[tokio::test]
async fn dsn_mode_memory_database_disables_explicit_expiry() {
    let handle = DbHandle::connect(
        "sqlite:file:toolkit-db-expiry?mode=memory&cache=shared",
        expiring_connect_opts(),
    )
    .await
    .unwrap();

    assert_eq!(expiries(&handle), (None, None));
    assert_round_trip(&handle).await;
}

#[tokio::test]
async fn configured_file_database_keeps_configured_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = DbConnConfig {
        engine: Some(DbEngineCfg::Sqlite),
        path: Some(dir.path().join("persistent.db")),
        pool: Some(expiring_pool()),
        ..DbConnConfig::default()
    };
    let handle = build_db_handle(cfg, None).await.unwrap();

    assert_eq!(expiries(&handle), (Some(EXPIRY), Some(EXPIRY)));
    assert_round_trip(&handle).await;
}

#[tokio::test]
async fn dsn_file_database_keeps_configured_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let dsn = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("persistent.db").display()
    );
    let handle = DbHandle::connect(&dsn, expiring_connect_opts())
        .await
        .unwrap();

    assert_eq!(expiries(&handle), (Some(EXPIRY), Some(EXPIRY)));
    assert_round_trip(&handle).await;
}
