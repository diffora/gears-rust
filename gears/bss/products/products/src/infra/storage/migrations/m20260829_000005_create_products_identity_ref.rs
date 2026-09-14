//! Create `bss.products_identity_ref` — the pseudonym-to-identity map
//! (`design/10-retention-erasure.md` `inst-im-map`, keyed `(tenant_id,
//! actor_ref)`).
//!
//! # Ownership: slice 10's table, minted by slice 01
//!
//! This table is **slice 10's by ownership**: it is the erasure operand and
//! the only table in this gear where PII may live. **This slice creates it**
//! because resolution is a precondition of every door that can refuse
//! (`design/01-foundation.md` P-D-26): the mint runs in its own transaction
//! ahead of the authorization gate, so a first-time principal whose opening
//! act is *refused* still has a committed `actor_ref` for the refusal's audit
//! row to attribute to. A door written before this table exists cannot
//! refuse, and every door in this gear refuses. The erasure and DSAR paths —
//! the tombstone write, the compliance export — are **slice 10's** and are
//! not built by this migration; this file only lays the table down so slice
//! 01's doors can mint and resolve against it.
//!
//! # `principal_ref` is `NOT NULL` — P-D-49
//!
//! Three rules read this map *by principal*, and a key of `(tenant_id,
//! actor_ref)` alone admits no such read: erasure's resolve, the DSAR
//! export's "per named principal", and the first-appearance predicate all had
//! no operand. `principal_ref` is the pseudonym, **not** the identity — which
//! is why a tombstone destroys `identity_payload` and leaves this column
//! standing, and why a repeat DSAR and the age predicate both keep working
//! after an erasure.
//!
//! # `identity_payload` is the only PII in this gear
//!
//! This is the identity side of the map, and this table is **the only one in
//! the gear where PII may live**. It is nullable because a tombstone destroys
//! it, and it is exactly what `chk_products_identity_ref_tombstone` requires
//! to be absent once `tombstoned_at` is set.
//!
//! # `tombstoned_at` retires a ref permanently
//!
//! A tombstoned ref is retired **permanently**: erasure tombstones the map
//! entry while every append-only record (audit rows, approvals, versions)
//! keeps the `actor_ref` it was stamped with. Re-minting that key for the
//! same principal later would make render-time joins show the **new**
//! identity against historical rows — the partial unique index below is what
//! prevents that re-mint from reusing the retired key.
//!
//! # `last_seen_at` is advanced by resolution, not by minting
//!
//! `last_seen_at` is the age operand slice 10's pseudonymization reads. It is
//! **advanced by every act that resolves the ref, not by minting it**:
//! minting happens once per active ref, on the first appearance of a
//! principal with no live ref. An earlier version of this rule read
//! "refreshed by every ref-minting act", which pinned the column to
//! `first_seen_at` forever and let age-based erasure tombstone an active
//! employee mid-employment. Every door that stamps an `actor_ref` onto an
//! audit row, an approval, a decision, a session or an override resolves the
//! ref and therefore advances this column, as a same-transaction touch, not a
//! separate act — no code for that touch is written by this migration.
//!
//! # The partial unique index is "one active ref per principal"
//!
//! `uq_products_identity_ref_active` is a partial unique index on
//! `(tenant_id, principal_ref)` `WHERE tombstoned_at IS NULL`, the physical
//! form of the L5 rule "one active ref per `(tenant, principal)`" — the same
//! idiom as `uq_products_sku_code` in `m20260829_000003_create_products_sku`.
//! The partial predicate is also what makes "first appearance" mean *first
//! appearance of a principal with no live ref*: a principal acting after its
//! erasure mints a **fresh** row for the same `(tenant_id, principal_ref)`,
//! and the index admits that second row — its `tombstoned_at` is `NULL` and
//! the retired row's is not — while still refusing a second *live* one.
//!
//! # No append-only guard on this table — and why
//!
//! Unlike `products_audit_log`, this table carries **no append-only
//! trigger**, and a later reader must not "restore" one by analogy. This
//! table is mutable by design: `last_seen_at` is advanced by every
//! resolution, and erasure tombstones the row and nulls its payload. Guarding
//! it append-only would break the two writes the design requires of it. The
//! append-only posture belongs to the record tables — head rows, version
//! history and `products_audit_log` — not to this map.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` for `actor_ref`; `tenant_id` is `text` on `SQLite`,
//! and was already `text`-shaped there since neither `uuid` nor
//! `timestamptz` exist on that engine. Timestamps become `text`, and the
//! `bss.` qualification is dropped. Every `CHECK`, both indexes and the
//! primary key are preserved on both sides, and the unique index is partial
//! on both.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-actor-ref:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_identity_ref (
            tenant_id         uuid        NOT NULL,
            actor_ref         uuid        NOT NULL,
            principal_ref     text        NOT NULL,
            identity_payload  text,
            tombstoned_at     timestamptz,
            first_seen_at     timestamptz NOT NULL,
            last_seen_at      timestamptz NOT NULL,
            CONSTRAINT products_identity_ref_pkey PRIMARY KEY (tenant_id, actor_ref),
            CONSTRAINT chk_products_identity_ref_tombstone CHECK (tombstoned_at IS NULL OR identity_payload IS NULL),
            CONSTRAINT chk_products_identity_ref_seen_order CHECK (last_seen_at >= first_seen_at)
        )",
    "CREATE INDEX idx_products_identity_ref_principal ON bss.products_identity_ref USING btree (tenant_id, principal_ref)",
    "CREATE UNIQUE INDEX uq_products_identity_ref_active ON bss.products_identity_ref USING btree (tenant_id, principal_ref) WHERE tombstoned_at IS NULL",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.products_identity_ref"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_identity_ref (
            tenant_id         text NOT NULL,
            actor_ref         text NOT NULL,
            principal_ref     text NOT NULL,
            identity_payload  text,
            tombstoned_at     text,
            first_seen_at     text NOT NULL,
            last_seen_at      text NOT NULL,
            PRIMARY KEY (tenant_id, actor_ref),
            CONSTRAINT chk_products_identity_ref_tombstone CHECK (tombstoned_at IS NULL OR identity_payload IS NULL),
            CONSTRAINT chk_products_identity_ref_seen_order CHECK (last_seen_at >= first_seen_at)
        )",
    "CREATE INDEX idx_products_identity_ref_principal ON products_identity_ref (tenant_id, principal_ref)",
    "CREATE UNIQUE INDEX uq_products_identity_ref_active ON products_identity_ref (tenant_id, principal_ref) WHERE tombstoned_at IS NULL",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_identity_ref"];

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
