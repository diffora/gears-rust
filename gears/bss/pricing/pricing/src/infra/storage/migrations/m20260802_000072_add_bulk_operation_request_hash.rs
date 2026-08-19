//! `pricing_bulk_operation.request_hash` — the digest of the request a client
//! key was first spent on (Z11-5).
//!
//! O4's replay on this table is `bulk_repo::find_by_client_key` and nothing else,
//! so until this column existed the *body* of a second `POST` was never compared
//! with what the key first carried. An operator who corrected a batch and
//! resubmitted it under the same key was answered `202`, handed the **first**
//! batch's report, and imported nothing — with no member of `BulkImportView` that
//! could reveal the substitution. That is the inversion D-295 closed on the state
//! axis and D-307 (`m20260802_000064`) on the kind axis, on the third one: the
//! payload.
//!
//! The crate's interactive gate has carried exactly this guard from the start —
//! `pricing_idempotency_dedup.request_hash` (`m20260802_000008`) and
//! `idempotency_repo::claim`'s `IDEMPOTENCY_PAYLOAD_MISMATCH`. This column is that
//! column, on the table `api::rest::bulk_imports` replaced the gate with for the
//! TTL reason its own module doc gives.
//!
//! # A digest and not the payload, for `m20260802_000008`'s reason
//!
//! `bytea`/`blob`, the SHA-256 of the canonical request rendering
//! (`preconditions::request_digest`). The run needs to know whether two requests
//! are the same, not what they said, and retaining request bodies on a run row
//! would put a second, unmanaged copy of what callers sent beside the audit trail
//! that is supposed to be the one place it lives.
//!
//! # `NOT NULL DEFAULT`, which is the backfill
//!
//! A run opened **before** this column existed has no digest and none can be
//! derived: the request is not stored anywhere. So the added column is `NOT NULL`
//! with an empty default, and the `ALTER` itself is the backfill — every existing
//! row reads back an empty digest rather than `NULL`, in one statement, on both
//! engines.
//!
//! That matters twice over. A nullable column added by `ALTER TABLE` with no
//! backfill wedged this gear's read-model frontier permanently once, because the
//! reader refused `NULL` and the retry loop never gave up; and a backfill written
//! as an `UPDATE … WHERE <uuid col> = <uuid-bearing col of another type>` is
//! always false on `SQLite`, blobs being the one class its affinity rules never
//! convert, so the statement quietly affects nothing. A column default reaches
//! every row without matching an id at all, so neither hazard is in play here.
//!
//! **Empty means "opened before the guard existed", and the reader says so.**
//! `bulk_imports`' replay skips the comparison on an empty stored digest and
//! answers as it did before Z11-5 — a bounded degradation confined to rows that
//! predate this migration, since both writers of the table set a real digest. A
//! 32-byte digest is never empty, so the sentinel cannot collide with a value.
//!
//! # Frozen, and its guard is `m20260802_000073`
//!
//! Provenance: the digest records what the run was opened *for*, so a writer
//! moving it rewrites which request the key was spent on — which is the whole of
//! what the replay then compares. It therefore belongs in the frozen-column
//! whitelist beside `client_key`, and that restatement is its own migration per
//! `m20260802_000040`'s rule and `m20260802_000061`/`000062`'s shape.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &["ALTER TABLE bss.pricing_bulk_operation
        ADD COLUMN request_hash bytea NOT NULL DEFAULT ''::bytea"];
const PG_DOWN_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.pricing_bulk_operation DROP COLUMN request_hash"];

const SQLITE_UP_STATEMENTS: &[&str] = &["ALTER TABLE pricing_bulk_operation
        ADD COLUMN request_hash blob NOT NULL DEFAULT X''"];
const SQLITE_DOWN_STATEMENTS: &[&str] =
    &["ALTER TABLE pricing_bulk_operation DROP COLUMN request_hash"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
