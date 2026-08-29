//! Create the `bss` schema.
//!
//! First in the roster and the only place `CREATE SCHEMA` appears in this
//! chain. The sibling BSS gears issue the same `IF NOT EXISTS` statement, so
//! the schema exists no matter which gear's runner reaches it first.
//!
//! `SQLite` has no schemas; the mirror is deliberately empty rather than absent,
//! which keeps the pair shape uniform across every migration in the chain.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &["CREATE SCHEMA IF NOT EXISTS bss"];
// Deliberately empty: a shared schema is not this gear's to drop, and another
// BSS gear's tables may live in it.
const PG_DOWN_STATEMENTS: &[&str] = &[];
const SQLITE_UP_STATEMENTS: &[&str] = &[];
const SQLITE_DOWN_STATEMENTS: &[&str] = &[];

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
