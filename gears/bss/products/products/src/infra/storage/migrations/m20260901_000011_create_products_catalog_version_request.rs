//! Create `bss.products_catalog_version_request` — the increment queue
//! (`design/06-catalog-version.md` §4, P-D-50, P-D-52, P-D-60).
//!
//! # The roster is two values, and the FK travels with the second
//!
//! `state ∈ {pending, coalesced}` — **P-D-60 struck `superseded`**: nothing
//! supersedes a request (a failed mechanical run re-coalesces and retries
//! fresh, an unregistered source is refused `REQUEST_SOURCE_UNKNOWN` at the
//! door before a row exists, an idempotent replay is caught by the UNIQUE
//! below). `coalesced` is **terminal** and is written by the increment
//! transaction **together with `satisfied_by_version_id`** — the set that
//! transaction produces is the set P-D-50 gave the column its existence to
//! let a replay rebuild. `chk_products_catalog_version_request_shape` makes
//! that pairing physical: a `pending` row carries no version and a
//! `coalesced` row always carries one — the poison-column lesson, applied
//! before the poison exists.
//!
//! # The UNIQUE is the idempotency and the `satisfiedRequests` operand
//!
//! `(tenant_id, source, request_key)` — the tenant column is part of the key
//! deliberately: it is what the per-tenant coalescer selects on, and without
//! it one `source` serving many tenants collides across them
//! (`dod-request-queue`, `dod-increment-request-port`).
//!
//! # `requested_at` is the door's stamp
//!
//! Stamped at ingress, never accepted from the caller — §1.7's entity
//! requires it and the lane SLO measures from it (interactive lane;
//! P-D-67 scoped the bulk lane to its window close).
//!
//! # No guard trigger — the queue is working state
//!
//! Unlike the version row one migration over, a request row is **mutable by
//! design**: the coalescer flips `pending → coalesced` and stamps the FK.
//! The CHECKs above are the physical floor; the door and the coalescer are
//! the writers.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `bigint` becomes `integer`,
//! `timestamptz` becomes `text`, and the `bss.` qualification is dropped.
//! The composite FK is preserved on both sides, as are the CHECKs and both
//! keys.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-request-queue:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_catalog_version_request (
            tenant_id                 uuid        NOT NULL,
            source                    text        NOT NULL,
            request_key               text        NOT NULL,
            lane                      text        NOT NULL,
            operation_key             text,
            requested_at              timestamptz NOT NULL,
            state                     text        NOT NULL,
            satisfied_by_version_id   bigint,
            CONSTRAINT products_catalog_version_request_pkey PRIMARY KEY (tenant_id, source, request_key),
            CONSTRAINT chk_products_catalog_version_request_lane CHECK (lane IN ('interactive', 'bulk')),
            CONSTRAINT chk_products_catalog_version_request_state CHECK (state IN ('pending', 'coalesced')),
            CONSTRAINT chk_products_catalog_version_request_shape CHECK (
                (state = 'pending' AND satisfied_by_version_id IS NULL)
                OR (state = 'coalesced' AND satisfied_by_version_id IS NOT NULL)
            ),
            CONSTRAINT chk_products_catalog_version_request_source CHECK (source <> ''),
            CONSTRAINT fk_products_catalog_version_request_version FOREIGN KEY (tenant_id, satisfied_by_version_id)
                REFERENCES bss.products_catalog_version (tenant_id, catalog_version_id)
        )",
    "CREATE INDEX idx_products_catalog_version_request_pending ON bss.products_catalog_version_request USING btree (tenant_id, state, requested_at)",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.products_catalog_version_request"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_catalog_version_request (
            tenant_id                 text    NOT NULL,
            source                    text    NOT NULL,
            request_key               text    NOT NULL,
            lane                      text    NOT NULL,
            operation_key             text,
            requested_at              text    NOT NULL,
            state                     text    NOT NULL,
            satisfied_by_version_id   integer,
            PRIMARY KEY (tenant_id, source, request_key),
            CONSTRAINT chk_products_catalog_version_request_lane CHECK (lane IN ('interactive', 'bulk')),
            CONSTRAINT chk_products_catalog_version_request_state CHECK (state IN ('pending', 'coalesced')),
            CONSTRAINT chk_products_catalog_version_request_shape CHECK (
                (state = 'pending' AND satisfied_by_version_id IS NULL)
                OR (state = 'coalesced' AND satisfied_by_version_id IS NOT NULL)
            ),
            CONSTRAINT chk_products_catalog_version_request_source CHECK (source <> ''),
            CONSTRAINT fk_products_catalog_version_request_version FOREIGN KEY (tenant_id, satisfied_by_version_id)
                REFERENCES products_catalog_version (tenant_id, catalog_version_id)
        )",
    "CREATE INDEX idx_products_catalog_version_request_pending ON products_catalog_version_request (tenant_id, state, requested_at)",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_catalog_version_request"];

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
