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
    plan_addon_rule, plan_descriptor_set, plan_phase, policy_object, price, price_tier_band,
    read_model,
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
    "pricing_plan_phase",
    "pricing_plan_addon_rule",
    "pricing_plan_descriptor_set",
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

/// The `CREATE INDEX ...` statement `SQLite` recorded for `index`, or `None`
/// when the chain created no index by that name.
async fn index_sql(conn: &sea_orm::DatabaseConnection, index: &str) -> Option<String> {
    let sql = format!(
        "SELECT count(*) AS c FROM sqlite_master WHERE type = 'index' AND name = '{index}'"
    );
    let present = conn
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("query sqlite_master")
        .expect("count query returns a row")
        .try_get::<i32>("", "c")
        .expect("read count")
        == 1;
    if !present {
        return None;
    }
    let sql =
        format!("SELECT sql AS v FROM sqlite_master WHERE type = 'index' AND name = '{index}'");
    let statement = conn
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql,
        ))
        .await
        .expect("query sqlite_master")
        .expect("the index row is there")
        .try_get::<String>("", "v")
        .expect("an index this chain created carries its DDL");
    Some(statement)
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
        plan_phase::Entity,
        plan_addon_rule::Entity,
        plan_descriptor_set::Entity,
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

#[tokio::test]
async fn the_version_index_on_the_ref_table_is_not_unique() {
    // The amendment of 2026-08-03, pinned so it cannot regress into the shape
    // it replaced. `uq_pricing_catalog_version_ref_version` asserted a
    // bijection from committed version to publish — and under the registry's
    // batching (D-47, §4.2 step 5) several of one tenant's pending refs commit
    // into ONE version, which is the case the whole model exists to serve and
    // which that index made physically impossible: the second finalize failed.
    // D-157's subject columns already answer "which publish produced this
    // version", and the honest answer is a set.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    for migration in &name_ordered_chain() {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    assert_eq!(
        index_sql(&conn, "uq_pricing_catalog_version_ref_version").await,
        None,
        "the unique version index is gone: it refused D-47's normal case"
    );
    let created = index_sql(&conn, "idx_pricing_catalog_version_ref_version")
        .await
        .expect("the version index the projector and the frontier walk read");
    assert!(
        !created.to_ascii_uppercase().contains("UNIQUE"),
        "the replacement must be non-unique, got: {created}"
    );
}

/// The `CREATE TABLE ...` statement `SQLite` recorded for `table`.
async fn table_sql(conn: &sea_orm::DatabaseConnection, table: &str) -> String {
    let sql =
        format!("SELECT sql AS v FROM sqlite_master WHERE type = 'table' AND name = '{table}'");
    conn.query_one(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        sql,
    ))
    .await
    .expect("query sqlite_master")
    .expect("the table row is there")
    .try_get::<String>("", "v")
    .expect("a table this chain created carries its DDL")
}

#[tokio::test]
async fn the_ref_table_records_the_commit_observation_and_pairs_it_with_nothing() {
    // D-166 clause (1). The column is what every post-commit clause in the set
    // was written against and none of them had: `requested_at` measures the
    // batching wait the requirement explicitly puts OUTSIDE degraded handling,
    // and `committed_at` is stamped by the finalize, which is the step that
    // never runs on the path the signal exists for.
    //
    // The absence of a CHECK is the assertion's other half.
    // `chk_pricing_catalog_version_ref_commit` exists because a version and its
    // commit instant are one fact; this column's whole purpose is to be settable
    // while `catalog_version` is still NULL.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    for migration in &name_ordered_chain() {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    let ddl = table_sql(&conn, "pricing_catalog_version_ref").await;
    assert!(
        ddl.contains("commit_observed_at"),
        "the SQLite arm must create the column: {ddl}"
    );
    assert!(
        !ddl.replace("commit_observed_at text", "")
            .contains("commit_observed_at"),
        "and pair it with nothing - no CHECK may mention it: {ddl}"
    );
}
