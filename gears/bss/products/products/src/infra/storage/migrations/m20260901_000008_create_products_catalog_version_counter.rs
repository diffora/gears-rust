//! Create `bss.products_catalog_version_counter` — the gapless per-tenant
//! allocator (`design/06-catalog-version.md` §4, `inst-cvc-serial`).
//!
//! # One row per tenant, holding the next id, starting at `1`
//!
//! `next_id` is the id the **next** increment will take, and its initial
//! value is pinned at `1` (**P-D-67**) so the dev-space ordering argument has
//! a stated premise: pricing's `LocalDevCatalogVersionRegistryV1` mints ids
//! at ~10¹² and this counter stays far below it; the sweep of that dev space
//! is pricing's own. The row is created on a tenant's first increment, by the
//! door, at `1` — this migration seeds nothing.
//!
//! # Gapless by construction, which is why there is no sequence
//!
//! A Postgres `SEQUENCE` is non-transactional: a refused run would burn an
//! id, and C1's counter is **gapless** — the allocation and the version
//! insert share one transaction (`dod-version-counter`), so a refusal rolls
//! both back. That is also why `staged_at` was struck (**P-D-67**): an
//! insert at stage time would burn ids on every `STAGED_ENTITY_CHANGED`
//! refusal. The `SELECT … FOR UPDATE` + `UPDATE` walk on this row is the
//! serialization point one tenant's increments contend on, and the per-tenant
//! coalescer (its `LeaseManager` lease) keeps that contention to one worker.
//!
//! # The CHECK is the floor, not a guess
//!
//! `chk_products_catalog_version_counter_floor` pins `next_id >= 1`: a row
//! that ever read `0` or negative would hand out an id below the pinned
//! start, and the poison-column lesson of this chain is that the columns a
//! corrupt row can poison are exactly the ones whose CHECK is missing.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `bigint` becomes `integer` (`SQLite`'s
//! 64-bit integer affinity), and the `bss.` qualification is dropped. The
//! CHECK and the primary key are preserved on both sides.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-version-counter:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &["CREATE TABLE bss.products_catalog_version_counter (
            tenant_id  uuid   NOT NULL,
            next_id    bigint NOT NULL,
            CONSTRAINT products_catalog_version_counter_pkey PRIMARY KEY (tenant_id),
            CONSTRAINT chk_products_catalog_version_counter_floor CHECK (next_id >= 1)
        )"];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.products_catalog_version_counter"];

const SQLITE_UP_STATEMENTS: &[&str] = &["CREATE TABLE products_catalog_version_counter (
            tenant_id  text    NOT NULL,
            next_id    integer NOT NULL,
            PRIMARY KEY (tenant_id),
            CONSTRAINT chk_products_catalog_version_counter_floor CHECK (next_id >= 1)
        )"];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_catalog_version_counter"];

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
