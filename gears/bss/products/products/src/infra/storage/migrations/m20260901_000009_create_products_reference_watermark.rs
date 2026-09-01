//! Create `bss.products_reference_watermark` and
//! `bss.products_reference_member` — the liveness signal's stores
//! (`design/07-reference-signal.md` §4, `inst-wm-tables`).
//!
//! # A registered producer that has never posted has no row here
//!
//! `never-received` is the **absence** of the watermark row (**P-D-71**):
//! registration writes only `products_reference_producer`, and this table
//! gains a row on the producer's first post. A sentinel timestamp would be
//! the poison-value class, and row-absence is what P-D-59's
//! "deregistration removes the series" already reads as. That is why this
//! migration seeds nothing and why no column here is nullable.
//!
//! # `set_hash` is stored at ingestion — P-D-71
//!
//! `SHA-256` over the member `sku_id`s **sorted bytewise**, stored when the
//! post lands. `inst-ws-monotonic` compares an equal `watermark_at`'s set by
//! this column: equal hash is an idempotent no-op success, different is
//! `WATERMARK_CONFLICT`. Recomputing the hash from 10K member rows at every
//! comparison was the declined arm.
//!
//! # `posted_at` is the receiving clock's audit record
//!
//! Written from the clock `inst-ws-not-future`'s bound was evaluated against;
//! **read by no freshness evaluation** — freshness reads `watermark_at`, the
//! producer's claim instant. `chk_products_reference_watermark_hash_len` pins
//! the hash to 64 lowercase hex characters so a truncated or upper-cased
//! digest cannot land and silently never match again.
//!
//! # The member set is replaced as a set, per post
//!
//! `products_reference_member` holds the current set per `(tenant_id,
//! producer)` and is **swapped atomically with the watermark advance** in one
//! transaction (`inst-wm-tables`) — no concurrent reader observes a half-set.
//! Member ids are **accepted unvalidated** (P-D-71, `inst-ws-members`): no
//! foreign key to `products_sku`, deliberately — a producer's catalog lags
//! `10`'s erasure legitimately, and refusing a 10K post for one unknown id
//! would wedge the producer on this gear's lifecycle. The unknown ids are
//! counted per post and alarmed (`reference_unknown_member`) by the door, not
//! guarded here.
//!
//! The primary key `(tenant_id, producer, sku_id)` is also the membership
//! lookup's index: the predicate's per-SKU read
//! (`… WHERE tenant_id = ? AND sku_id = ?`) rides
//! `idx_products_reference_member_sku`, an index hit rather than a scan, as
//! `dod-watermark-tables` requires.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `timestamptz` becomes `text`, and the
//! `bss.` qualification is dropped. Every CHECK, the primary keys and the
//! member index are preserved on both sides.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-watermark-tables:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_reference_watermark (
            tenant_id     uuid        NOT NULL,
            producer      text        NOT NULL,
            watermark_at  timestamptz NOT NULL,
            posted_at     timestamptz NOT NULL,
            set_hash      text        NOT NULL,
            CONSTRAINT products_reference_watermark_pkey PRIMARY KEY (tenant_id, producer),
            CONSTRAINT chk_products_reference_watermark_producer CHECK (producer <> ''),
            CONSTRAINT chk_products_reference_watermark_hash_len CHECK (set_hash ~ '^[0-9a-f]{64}$')
        )",
    "CREATE TABLE bss.products_reference_member (
            tenant_id  uuid NOT NULL,
            producer   text NOT NULL,
            sku_id     uuid NOT NULL,
            CONSTRAINT products_reference_member_pkey PRIMARY KEY (tenant_id, producer, sku_id),
            CONSTRAINT chk_products_reference_member_producer CHECK (producer <> '')
        )",
    "CREATE INDEX idx_products_reference_member_sku ON bss.products_reference_member USING btree (tenant_id, sku_id)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.products_reference_member",
    "DROP TABLE IF EXISTS bss.products_reference_watermark",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_reference_watermark (
            tenant_id     text NOT NULL,
            producer      text NOT NULL,
            watermark_at  text NOT NULL,
            posted_at     text NOT NULL,
            set_hash      text NOT NULL,
            PRIMARY KEY (tenant_id, producer),
            CONSTRAINT chk_products_reference_watermark_producer CHECK (producer <> ''),
            CONSTRAINT chk_products_reference_watermark_hash_len CHECK (length(set_hash) = 64 AND set_hash NOT GLOB '*[^0-9a-f]*')
        )",
    "CREATE TABLE products_reference_member (
            tenant_id  text NOT NULL,
            producer   text NOT NULL,
            sku_id     text NOT NULL,
            PRIMARY KEY (tenant_id, producer, sku_id),
            CONSTRAINT chk_products_reference_member_producer CHECK (producer <> '')
        )",
    "CREATE INDEX idx_products_reference_member_sku ON products_reference_member (tenant_id, sku_id)",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS products_reference_member",
    "DROP TABLE IF EXISTS products_reference_watermark",
];

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
