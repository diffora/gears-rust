//! Create `bss.pricing_window_guard` — one lock row per `(tenant_id, plan_id)`
//! (D-374).
//!
//! Guard rows are **lock identity, not business coverage**. `serial` starts at 0
//! and is advanced by a scoped `UPDATE serial = serial + 1` that holds the write
//! lock until the caller's transaction ends. Zero affected rows is an error.
//!
//! The `INSERT … SELECT DISTINCT` below seeds a row for every plan that already
//! exists when this migration applies. It is **not** an implicit-window backfill.
//! `SQLite` parses `INSERT … SELECT … FROM t ON CONFLICT` as a join on `CONFLICT`,
//! so the mirror uses `WHERE true` before `ON CONFLICT` (the documented UPSERT
//! disambiguation). Postgres does not need that.
//! Create and clone still insert one row in the same transaction as `pricing_plan`,
//! with `ON CONFLICT DO NOTHING` so a successor revision of the same `plan_id`
//! does not collide on this table's primary key.
//!
//! Dependency: `pricing_plan` (read for the seed; no foreign key, so a plan
//! delete is not this table's concern and the chain's plan table is append-only
//! anyway).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_window_guard (
            tenant_id  uuid    NOT NULL,
            plan_id    uuid    NOT NULL,
            serial     bigint  NOT NULL,
            CONSTRAINT pricing_window_guard_pkey PRIMARY KEY (tenant_id, plan_id)
        )",
    "INSERT INTO bss.pricing_window_guard (tenant_id, plan_id, serial)
        SELECT DISTINCT tenant_id, plan_id, 0 FROM bss.pricing_plan
        ON CONFLICT (tenant_id, plan_id) DO NOTHING",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.pricing_window_guard"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_window_guard (
            tenant_id  text    NOT NULL,
            plan_id    text    NOT NULL,
            serial     bigint  NOT NULL,
            PRIMARY KEY (tenant_id, plan_id)
        )",
    "INSERT INTO pricing_window_guard (tenant_id, plan_id, serial)
        SELECT DISTINCT tenant_id, plan_id, 0 FROM pricing_plan WHERE true
        ON CONFLICT (tenant_id, plan_id) DO NOTHING",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_window_guard"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(
            self.name(),
            manager,
            PG_DOWN_STATEMENTS,
            SQLITE_DOWN_STATEMENTS,
        )
        .await
    }
}
