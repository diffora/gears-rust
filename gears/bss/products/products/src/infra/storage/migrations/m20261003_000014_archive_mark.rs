//! P-D-263: a retired SKU or a retired category can be archived. The mark is two columns,
//! `archived_at` and `archived_by`, on `products_sku` and `products_category`; nothing that reads
//! the lifecycle or the status changes.
//!
//! A forward migration: the shipped chain is frozen. Both engines add the four nullable columns in
//! place (no CHECK changes, so `SQLite` needs no rebuild). The partial index
//! `ix_products_sku_unarchived` serves the SKU list, which hides archived rows by default.
//!
//! # Down
//!
//! The index and the four columns go. Every archive mark is lost with them: an archived row is
//! listed again, as it was before this migration.

use sea_orm_migration::prelude::*;

use super::exec_backend;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP: &[&str] = &[
    "ALTER TABLE bss.products_sku \
     ADD COLUMN IF NOT EXISTS archived_at timestamptz NULL, \
     ADD COLUMN IF NOT EXISTS archived_by uuid NULL",
    "ALTER TABLE bss.products_category \
     ADD COLUMN IF NOT EXISTS archived_at timestamptz NULL, \
     ADD COLUMN IF NOT EXISTS archived_by uuid NULL",
    "CREATE INDEX IF NOT EXISTS ix_products_sku_unarchived \
     ON bss.products_sku (tenant_id) WHERE archived_at IS NULL",
];

const PG_DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS bss.ix_products_sku_unarchived",
    "ALTER TABLE bss.products_sku \
     DROP COLUMN IF EXISTS archived_at, DROP COLUMN IF EXISTS archived_by",
    "ALTER TABLE bss.products_category \
     DROP COLUMN IF EXISTS archived_at, DROP COLUMN IF EXISTS archived_by",
];

const SQLITE_UP: &[&str] = &[
    "ALTER TABLE products_sku ADD COLUMN archived_at text NULL",
    "ALTER TABLE products_sku ADD COLUMN archived_by text NULL",
    "ALTER TABLE products_category ADD COLUMN archived_at text NULL",
    "ALTER TABLE products_category ADD COLUMN archived_by text NULL",
    "CREATE INDEX ix_products_sku_unarchived ON products_sku (tenant_id) WHERE archived_at IS NULL",
];

const SQLITE_DOWN: &[&str] = &[
    "DROP INDEX IF EXISTS ix_products_sku_unarchived",
    "ALTER TABLE products_sku DROP COLUMN archived_at",
    "ALTER TABLE products_sku DROP COLUMN archived_by",
    "ALTER TABLE products_category DROP COLUMN archived_at",
    "ALTER TABLE products_category DROP COLUMN archived_by",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        exec_backend(self.name(), manager, PG_UP, SQLITE_UP).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        exec_backend(self.name(), manager, PG_DOWN, SQLITE_DOWN).await
    }
}

#[cfg(test)]
#[path = "m20261003_000014_archive_mark_tests.rs"]
mod tests;
