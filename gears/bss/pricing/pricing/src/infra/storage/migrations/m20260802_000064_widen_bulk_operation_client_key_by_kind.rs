//! `uq_pricing_bulk_operation_client_key` gains `kind` (D-307).
//!
//! O4 gives the two bulk flows **two different idempotency columns**: §5 spells
//! the import's as an `Idempotency-Key` and a repricing run's as its own `run_id`.
//! The index made them one exclusive namespace per tenant, and nothing in the
//! design asks for that.
//!
//! # The dangerous half is the replay, not the insert
//!
//! An insert refused across kinds is at least a refusal. What the shared
//! namespace really costs is on the read: `bulk_repo::find_by_client_key` filters
//! `(tenant_id, client_key)` and `BulkImportView` carries **no `kind` member**, so
//! once the repricing engine exists an import `POST` under a key a run holds
//! would answer `202 ACCEPTED` describing **the run**, import nothing, and hand
//! the caller a document with no field that could reveal the substitution. That
//! is the inversion D-295 fixed on the state axis and left open on this one; the
//! query is fixed with this index, and either fix alone would be half of it.
//!
//! # Per-`kind` uniqueness, not weaker
//!
//! `inst-bs-reject`'s auditability argument rests on the key staying spent: an
//! operator's remedy for a refused run is a fresh run under a new key, and "O4's
//! per-tenant uniqueness holds the old key against the rejected record".
//! `(tenant_id, kind, client_key)` keeps exactly that and separates only the two
//! flows, which is what §5 already separates.
//!
//! # No table rebuild
//!
//! `SQLite` cannot alter a `CHECK`, which is why `m20260802_000063` rebuilt this
//! table whole — but an index is droppable and re-creatable in place on both
//! engines, and no constraint moves here. So this migration is two statements per
//! engine and none of that machinery is needed.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "DROP INDEX IF EXISTS bss.uq_pricing_bulk_operation_client_key",
    "CREATE UNIQUE INDEX uq_pricing_bulk_operation_client_key
        ON bss.pricing_bulk_operation (tenant_id, kind, client_key)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX IF EXISTS bss.uq_pricing_bulk_operation_client_key",
    "CREATE UNIQUE INDEX uq_pricing_bulk_operation_client_key
        ON bss.pricing_bulk_operation (tenant_id, client_key)",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "DROP INDEX IF EXISTS uq_pricing_bulk_operation_client_key",
    "CREATE UNIQUE INDEX uq_pricing_bulk_operation_client_key
        ON pricing_bulk_operation (tenant_id, kind, client_key)",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX IF EXISTS uq_pricing_bulk_operation_client_key",
    "CREATE UNIQUE INDEX uq_pricing_bulk_operation_client_key
        ON pricing_bulk_operation (tenant_id, client_key)",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
