//! Create `bss.products_product` — a Product's identity, lifecycle and the two
//! version counters (`design/01-foundation.md` §4.1).
//!
//! # The two partial unique indexes, and why both are partial
//!
//! `uq_products_product_name` admits one non-discarded holder per
//! `(tenant_id, brand_id, name_normalized)`. It is **partial on
//! `lifecycle_state <> 'discarded'`** because discard releases the name exactly
//! as it releases codes: holding it would let one typo in a never-published
//! draft burn a name forever. The asymmetry with `retired`, which *does* hold
//! its name, is the intended one — a discarded draft was never published and a
//! retired entity was.
//!
//! `uq_products_product_code` is partial on the same predicate **and** on the
//! column being set, since `product_code` is optional and a NULL is not a
//! reservation.
//!
//! Region scope plays no part in either. That is P-D-04, and it is the reason
//! the name index is described as **absolute** uniqueness rather than
//! scope-relative.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` and the `bss.` qualification is dropped. Every
//! `CHECK`, index and the primary key are preserved on both sides. Timestamps
//! are `timestamptz` on Postgres and `text` on `SQLite`, which is what the
//! driver pair stores a `DateTime<Utc>` as.
//!
//! No trigger yet: the append-only head-row guard is its own definition of done
//! and lands with the save and publish doors it constrains. The columns it will
//! whitelist are all present here, which is what lets it be added without
//! touching this file.
//!
//! @cpt-cf-bss-products-fr-create-product
//! @cpt-cf-bss-products-dod-entity-tables
//! @cpt-cf-bss-products-dod-name-uniqueness

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_product (
            tenant_id         uuid        NOT NULL,
            product_id        uuid        NOT NULL,
            brand_id          uuid        NOT NULL,
            name              text        NOT NULL,
            name_normalized   text        NOT NULL,
            product_code      text,
            lifecycle_state   text        NOT NULL,
            internal_revision bigint      NOT NULL,
            published_version bigint      NOT NULL,
            region_scope      text        NOT NULL DEFAULT '',
            brand_scope       text        NOT NULL DEFAULT '',
            created_by        text        NOT NULL,
            created_at        timestamptz NOT NULL,
            updated_at        timestamptz NOT NULL,
            CONSTRAINT products_product_pkey PRIMARY KEY (product_id),
            CONSTRAINT chk_products_product_lifecycle_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_product_internal_revision CHECK (internal_revision >= 1),
            CONSTRAINT chk_products_product_published_version CHECK (published_version >= 0)
        )",
    "CREATE INDEX idx_products_product_tenant ON bss.products_product USING btree (tenant_id, product_id)",
    "CREATE UNIQUE INDEX uq_products_product_name ON bss.products_product USING btree (tenant_id, brand_id, name_normalized) WHERE lifecycle_state <> 'discarded'",
    "CREATE UNIQUE INDEX uq_products_product_code ON bss.products_product USING btree (tenant_id, product_code) WHERE product_code IS NOT NULL AND lifecycle_state <> 'discarded'",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.products_product"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_product (
            tenant_id         text   NOT NULL,
            product_id        text   NOT NULL,
            brand_id          text   NOT NULL,
            name              text   NOT NULL,
            name_normalized   text   NOT NULL,
            product_code      text,
            lifecycle_state   text   NOT NULL,
            internal_revision bigint NOT NULL,
            published_version bigint NOT NULL,
            region_scope      text   NOT NULL DEFAULT '',
            brand_scope       text   NOT NULL DEFAULT '',
            created_by        text   NOT NULL,
            created_at        text   NOT NULL,
            updated_at        text   NOT NULL,
            PRIMARY KEY (product_id),
            CONSTRAINT chk_products_product_lifecycle_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_product_internal_revision CHECK (internal_revision >= 1),
            CONSTRAINT chk_products_product_published_version CHECK (published_version >= 0)
        )",
    "CREATE INDEX idx_products_product_tenant ON products_product (tenant_id, product_id)",
    "CREATE UNIQUE INDEX uq_products_product_name ON products_product (tenant_id, brand_id, name_normalized) WHERE lifecycle_state <> 'discarded'",
    "CREATE UNIQUE INDEX uq_products_product_code ON products_product (tenant_id, product_code) WHERE product_code IS NOT NULL AND lifecycle_state <> 'discarded'",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_product"];

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
