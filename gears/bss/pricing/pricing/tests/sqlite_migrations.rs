//! The migration chain — the Foundation tables and the slice-owned ones that
//! follow them — exercised end to end on an in-memory `SQLite`.
//!
//! Three properties, each a real boot failure mode:
//!
//! 1. **Completeness, and agreement with the entities** — after `up`, every
//!    table can be read through its `SeaORM` entity. That is a stronger check
//!    than "the table exists": `SeaORM` names every column in the `SELECT`, so
//!    a migration and an entity that disagree about a column fail here rather
//!    than at the first production read.
//! 2. **Re-run safety** — a second boot over the same database applies nothing
//!    and skips everything. The sibling ledger carries a whole Postgres
//!    regression for the version of this that bit it (bookkeeping landing in the
//!    wrong schema made every migration re-run and a non-`IF NOT EXISTS`
//!    `CREATE TABLE` abort in a crash loop); the cheap half of that check
//!    belongs in the fast suite.
//! 3. **Reversibility** — `down` then `up` round-trips, so a rollback leaves a
//!    database the chain can walk forward again rather than a half-dropped one.
//!    This one introspects `sqlite_master` directly, because it is also where
//!    the shared `coord_leases` table (spliced in for the singleton warm
//!    re-drive) is checked; `coord` does not export its entity.
//!
//! Postgres-backed coverage is testcontainers-gated by convention in this repo
//! and none is added: the append-only guards are mirrored onto `SQLite` as
//! `RAISE(ABORT, ...)` triggers, so `sqlite_append_only.rs` exercises them with
//! no Docker.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::infra::storage::entity::{
    audit_log, catalog_version_ref, idempotency_dedup, operator_flag, outbox, pin_frontier, plan,
    policy_object, price, price_tier_band, read_model,
};
use bss_pricing::infra::storage::migrations::Migrator;
use sea_orm::{ConnectionTrait, Database, EntityTrait, Statement};
use sea_orm_migration::{MigrationTrait, MigratorTrait, SchemaManager};
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureEntityExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

/// Every table the chain creates, `coord_leases` included.
const EXPECTED_TABLES: &[&str] = &[
    "pricing_plan",
    "pricing_price",
    "pricing_price_tier_band",
    "pricing_read_model",
    "pricing_catalog_version_ref",
    "pricing_pin_frontier",
    "pricing_policy_object",
    "pricing_operator_flag",
    "pricing_idempotency_dedup",
    "pricing_outbox",
    "pricing_audit_log",
    "coord_leases",
];

/// Read every row of an entity under a tenant scope, asserting the table and
/// its column set are what the entity expects.
macro_rules! assert_readable {
    ($conn:expr, $scope:expr, $($entity:path),+ $(,)?) => {
        $(
            let rows = <$entity>::find()
                .secure()
                .scope_with($scope)
                .all($conn)
                .await
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", stringify!($entity)));
            assert!(rows.is_empty(), "{} starts empty", stringify!($entity));
        )+
    };
}

async fn table_exists(conn: &sea_orm::DatabaseConnection, table: &str) -> bool {
    let sql = format!(
        "SELECT count(*) AS c FROM sqlite_master WHERE type = 'table' AND name = '{table}'"
    );
    let row = conn
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("query sqlite_master")
        .expect("count query returns a row");
    row.try_get::<i32>("", "c").expect("read count") == 1
}

/// The chain, in the order the platform runner applies it (by migration NAME).
fn name_ordered_chain() -> Vec<Box<dyn MigrationTrait>> {
    let mut chain = Migrator::migrations();
    chain.sort_by(|a, b| a.name().cmp(b.name()));
    chain
}

#[tokio::test]
async fn the_chain_creates_every_table_and_re_runs_cleanly() {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");

    let boot1 = run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("boot 1 must apply the whole chain");
    assert_eq!(
        boot1.applied,
        Migrator::migrations().len(),
        "boot 1 applies every migration"
    );
    assert_eq!(boot1.skipped, 0, "boot 1 skips nothing");

    // Boot 2 over the same database: nothing re-runs. No `CREATE TABLE` in this
    // chain is `IF NOT EXISTS`, so a re-run that actually executed would fail
    // loudly here rather than silently.
    let boot2 = run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("boot 2 must be a clean no-op");
    assert_eq!(boot2.applied, 0, "boot 2 applies nothing");
    assert_eq!(
        boot2.skipped,
        Migrator::migrations().len(),
        "boot 2 skips every migration"
    );

    let provider = DBProvider::<DbError>::new(db);
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(Uuid::from_u128(1));
    assert_readable!(
        &conn,
        &scope,
        plan::Entity,
        price::Entity,
        price_tier_band::Entity,
        read_model::Entity,
        catalog_version_ref::Entity,
        pin_frontier::Entity,
        policy_object::Entity,
        operator_flag::Entity,
        idempotency_dedup::Entity,
        outbox::Entity,
        audit_log::Entity,
    );
}

#[tokio::test]
async fn down_then_up_round_trips() {
    // A raw `SeaORM` connection: `SchemaManager` needs one, and the toolkit
    // runner owns bookkeeping but exposes no `down` — this walks the chain the
    // way a rollback would.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    let chain = name_ordered_chain();

    for migration in &chain {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }
    for table in EXPECTED_TABLES {
        assert!(
            table_exists(&conn, table).await,
            "the chain must create `{table}`"
        );
    }

    for migration in chain.iter().rev() {
        migration
            .down(&manager)
            .await
            .unwrap_or_else(|e| panic!("down {} must succeed: {e}", migration.name()));
    }
    for table in EXPECTED_TABLES {
        assert!(
            !table_exists(&conn, table).await,
            "`{table}` must be gone after down"
        );
    }

    for migration in &chain {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("re-up {} must succeed: {e}", migration.name()));
    }
    for table in EXPECTED_TABLES {
        assert!(
            table_exists(&conn, table).await,
            "`{table}` must be back after the re-up"
        );
    }
}
