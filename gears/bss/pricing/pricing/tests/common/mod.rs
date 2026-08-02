//! The database-under-test, and the three verbs the schema suites drive it with.
//!
//! Everything here is shared because it has **no per-suite content at all**: a
//! migrated `SQLite` database, a statement that must land, a statement whose
//! result is read back. Each of the schema suites once carried its own copy,
//! and the copies had already stopped agreeing — not about what the helpers do,
//! but about the schema they were describing, so one file said `pricing_price`
//! carried nine `CHECK` constraints while the migration that had grown six more
//! sat two tasks away in the same branch. A helper each suite re-types is a
//! sentence each suite has to keep true on its own.
//!
//! What is deliberately **not** here is `must_be_rejected`. Every suite asserts
//! that a refusal is *the one under test* — a raw "some error happened" would
//! pass with the guard it means to prove switched off — and the fragment that
//! makes that assertion sharp is different in each: the table name for the two
//! trigger suites, the constraint name for the CHECK suite. Hoisting them into
//! one helper would mean taking the weakest of the three, which is how a suite
//! ends up green against a schema that no longer holds.
//!
//! The chain is applied **sorted by migration name**, which is the order the
//! `Migrator` itself defines and which `tests/module_test.rs` pins: a table's
//! foreign key and every trigger that reads a parent depend on that parent
//! existing, so a suite that applied the chain in declaration order would fail
//! for a reason having nothing to do with what it is testing.

#![allow(
    dead_code,
    reason = "each test binary compiles the whole module and uses part of it"
)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::{MigrationTrait, MigratorTrait, SchemaManager};

use bss_pricing::infra::storage::migrations::Migrator;

/// An in-memory `SQLite` database carrying the whole migration chain.
pub async fn migrated_db() -> DatabaseConnection {
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    let mut chain: Vec<Box<dyn MigrationTrait>> = Migrator::migrations();
    chain.sort_by(|a, b| a.name().cmp(b.name()));
    for migration in &chain {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }
    conn
}

/// Run one statement and hand back whatever the driver said about it.
///
/// # Errors
/// Whatever the driver refused with — which is the point: the schema suites are
/// about which rejections the database produces.
pub async fn exec(conn: &DatabaseConnection, sql: &str) -> Result<(), sea_orm::DbErr> {
    conn.execute(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        sql.to_owned(),
    ))
    .await
    .map(|_| ())
}

/// Run one statement that must land.
///
/// # Panics
/// When the statement is refused. Every suite here proves a *whitelist* rather
/// than a blanket ban, so the moves that are supposed to work are as
/// load-bearing as the ones that are not.
pub async fn must_succeed(conn: &DatabaseConnection, sql: &str) {
    exec(conn, sql)
        .await
        .unwrap_or_else(|e| panic!("statement must succeed: {sql}\n{e}"));
}

/// Read one value back, from a query that aliases it `v`.
///
/// # Panics
/// When the query fails, returns no row, or the value is not text. Assertions
/// on what actually landed are how these suites tell "the guard refused" from
/// "the guard refused and the statement took effect anyway".
pub async fn scalar(conn: &DatabaseConnection, sql: &str) -> String {
    let row = conn
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .expect("query")
        .expect("one row");
    row.try_get::<String>("", "v").expect("read value")
}
