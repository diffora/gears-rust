//! Two declared indexes reach Postgres at last: `idx_pricing_migration_source`
//! and `idx_pricing_snapshot_provenance_plan` (review Z26-1).
//!
//! # What was wrong
//!
//! Both were written into the **`SQLite` arm only** of the migrations that declare
//! them, and neither Postgres arm ever created them:
//!
//! | index | declared at | arm |
//! |---|---|---|
//! | `idx_pricing_migration_source` | `m20260802_000043:342` | `SQLITE_UP_STATEMENTS` |
//! | `idx_pricing_snapshot_provenance_plan` | `m20260802_000044:192` | `SQLITE_UP_STATEMENTS` |
//!
//! `m20260802_000065`'s rebuild re-creates the first one, and re-creates it on one
//! engine again: the rebuild lives in the `sqlite_rebuild!` macro, which only
//! `SQLITE_UP_STATEMENTS` expands. `m20260802_000075`'s header says the index "is
//! rebuilt by the" widening — true of the mirror, and of nothing on the server.
//!
//! # How it was found, which is the part worth keeping
//!
//! Not by reading the migrations. `postgres_migrations`' index census —
//! `every_declared_index_reaches_the_server_by_name`, landed 2026-08-18 — compares
//! the server's roster against `EXPECTED_INDEXES` and reported **50 against 52**.
//! Its own doc states the design that made this visible: *one roster per engine, so
//! a missing statement in one arm of a migration reddens on that engine rather than
//! being averaged away by a shared list.* A shared roster would have been satisfied
//! by the `SQLite` half and these two would still be absent from the engine that
//! ships. The reverse direction was clean in the same run — `server - roster` is
//! empty — so this is two omissions, not a drifting roster.
//!
//! # Why a new migration and not a correction to those two arms
//!
//! Because a correction there fixes only databases that have never been migrated.
//! `m20260802_000043` and `_000044` are long applied on every stand, and
//! `sea_orm_migration` runs each name once: editing their `PG_UP_STATEMENTS` would
//! make a fresh chain correct and leave every existing Postgres database exactly as
//! it is now. The rule this follows is the one `m20260802_000069` states for
//! guards — *the chain moves forward; a past migration's text is history* — and it
//! is why the statements below are plain `CREATE INDEX` rather than
//! `IF NOT EXISTS`: on any database this migration can reach, neither index
//! exists, so a collision would be a fact worth failing on rather than skipping.
//!
//! # `down` is deliberately asymmetric between the engines
//!
//! It drops both indexes on Postgres and does nothing on `SQLite`, where they were
//! created by `_000043`/`_000044` and belong to those migrations to undo. This is
//! `m20260802_000086`'s lesson applied without having to be taught twice: a `down`
//! restores the previous state **as the chain below it leaves the schema**, not as
//! some earlier migration's `UP` text describes it. Reversing this one puts each
//! engine back where its own chain had it — the mirror keeps its indexes, the
//! server loses the two it just gained.
//!
//! # What the indexes are for
//!
//! Both are the reverse-lookup half of a pair whose forward half exists on both
//! engines. `pricing_migration` is keyed and indexed by `target_plan_id`, and a
//! plan being retired or superseded has to answer *"what migrates away from me"* —
//! `(tenant_id, source_plan_id)`. `pricing_snapshot_provenance` is unique per
//! `(tenant_id, subscription_ref)`, and the migrated-origin read answers *"which
//! subscriptions came off this legacy plan"* — `(tenant_id, source_plan_id)`. On a
//! table that is append-only over a >= 7-year retention, the missing index is a
//! sequential scan that grows with the retention rather than with the tenant, which
//! is why this is repaired rather than recorded.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE INDEX idx_pricing_migration_source
        ON bss.pricing_migration (tenant_id, source_plan_id)",
    "CREATE INDEX idx_pricing_snapshot_provenance_plan
        ON bss.pricing_snapshot_provenance (tenant_id, source_plan_id)",
];

/// Nothing: both indexes have existed on the mirror since `m20260802_000043` and
/// `m20260802_000044`, and `m20260802_000065`'s rebuild re-creates the first. A
/// statement here would fail on the duplicate name, which is the whole defect this
/// migration exists to correct, mirrored.
const SQLITE_UP_STATEMENTS: &[&str] = &[];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX bss.idx_pricing_snapshot_provenance_plan",
    "DROP INDEX bss.idx_pricing_migration_source",
];

/// Nothing, for [`SQLITE_UP_STATEMENTS`]' reason read backwards: these indexes are
/// not this migration's to drop on the mirror.
const SQLITE_DOWN_STATEMENTS: &[&str] = &[];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
