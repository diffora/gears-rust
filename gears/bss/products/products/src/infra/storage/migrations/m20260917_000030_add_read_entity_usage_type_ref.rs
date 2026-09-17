//! Add `usage_type_ref` to `products_read_entity` on estates that already
//! applied `m20260901_000023` before Task 6 edited that create in place.
//!
//! Fresh installs get the column from `m000023`. This migration is `IF NOT
//! EXISTS` so both paths converge. `DOWN` is a no-op: dropping the column
//! would undo the create migration on a greenfield chain.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.products_read_entity ADD COLUMN IF NOT EXISTS usage_type_ref text"];

const SQLITE_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE products_read_entity ADD COLUMN IF NOT EXISTS usage_type_ref text"];

const EMPTY: &[&str] = &[];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, EMPTY, EMPTY).await
    }
}
