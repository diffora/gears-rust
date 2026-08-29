//! Create `bss.products_sku` — a SKU's identity, its parent link, its lifecycle
//! and the two version counters (`design/01-foundation.md` §4.2).
//!
//! # `ReservationIndex`
//!
//! `uq_products_sku_code` is the reservation itself: a partial unique index on
//! `(tenant_id, sku_code)` where the row is not discarded. The **insert** is
//! what reserves, which is why the loser of a concurrent race is refused by the
//! index rather than by a read-then-act check that would lose the race it
//! exists to decide. Discard releases; first publish makes the column immutable,
//! and that half is the head-row guard's, not this file's.
//!
//! # `product_id` carries a real foreign key
//!
//! Unlike the sibling pricing gear's bundle table, this reference **can** be a
//! foreign key: `products_product` is keyed on `product_id` alone and carries
//! total uniqueness on it, so Postgres accepts it as a referent. The parent's
//! *state* checks — terminal, retire-intent — are the door's, because a foreign
//! key can express existence and not lifecycle.
//!
//! # Backend differences
//!
//! `uuid` becomes `text`, the `bss.` qualification is dropped, and timestamps
//! become `text`. Every `CHECK`, index, the primary key and the foreign key are
//! preserved on both sides.
//!
//! @cpt-cf-bss-products-fr-skucode-reservation-concurrency
//! @cpt-cf-bss-products-dod-entity-tables
//! @cpt-cf-bss-products-dod-code-reservation

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_sku (
            tenant_id         uuid        NOT NULL,
            sku_id            uuid        NOT NULL,
            product_id        uuid        NOT NULL,
            sku_code          text        NOT NULL,
            lifecycle_state   text        NOT NULL,
            internal_revision bigint      NOT NULL,
            published_version bigint      NOT NULL,
            region_scope      text        NOT NULL DEFAULT '',
            brand_scope       text        NOT NULL DEFAULT '',
            created_by        text        NOT NULL,
            created_at        timestamptz NOT NULL,
            updated_at        timestamptz NOT NULL,
            CONSTRAINT products_sku_pkey PRIMARY KEY (sku_id),
            CONSTRAINT fk_products_sku_product FOREIGN KEY (product_id) REFERENCES bss.products_product (product_id),
            CONSTRAINT chk_products_sku_lifecycle_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_sku_internal_revision CHECK (internal_revision >= 1),
            CONSTRAINT chk_products_sku_published_version CHECK (published_version >= 0)
        )",
    "CREATE INDEX idx_products_sku_tenant ON bss.products_sku USING btree (tenant_id, sku_id)",
    "CREATE INDEX idx_products_sku_parent ON bss.products_sku USING btree (tenant_id, product_id)",
    "CREATE UNIQUE INDEX uq_products_sku_code ON bss.products_sku USING btree (tenant_id, sku_code) WHERE lifecycle_state <> 'discarded'",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.products_sku"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_sku (
            tenant_id         text   NOT NULL,
            sku_id            text   NOT NULL,
            product_id        text   NOT NULL,
            sku_code          text   NOT NULL,
            lifecycle_state   text   NOT NULL,
            internal_revision bigint NOT NULL,
            published_version bigint NOT NULL,
            region_scope      text   NOT NULL DEFAULT '',
            brand_scope       text   NOT NULL DEFAULT '',
            created_by        text   NOT NULL,
            created_at        text   NOT NULL,
            updated_at        text   NOT NULL,
            PRIMARY KEY (sku_id),
            CONSTRAINT fk_products_sku_product FOREIGN KEY (product_id) REFERENCES products_product (product_id),
            CONSTRAINT chk_products_sku_lifecycle_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_sku_internal_revision CHECK (internal_revision >= 1),
            CONSTRAINT chk_products_sku_published_version CHECK (published_version >= 0)
        )",
    "CREATE INDEX idx_products_sku_tenant ON products_sku (tenant_id, sku_id)",
    "CREATE INDEX idx_products_sku_parent ON products_sku (tenant_id, product_id)",
    "CREATE UNIQUE INDEX uq_products_sku_code ON products_sku (tenant_id, sku_code) WHERE lifecycle_state <> 'discarded'",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_sku"];

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
