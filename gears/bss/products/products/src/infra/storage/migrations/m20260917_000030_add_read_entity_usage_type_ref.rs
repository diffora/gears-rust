//! Add `usage_type_ref` to `products_read_entity` on estates that already
//! applied `m20260901_000023` before Task 6 edited that create in place.
//!
//! Fresh installs get the column from `m000023`. Postgres uses `IF NOT
//! EXISTS`; `SQLite` checks the schema before adding the column, so both paths converge.
//! `DOWN` is a no-op: dropping the column
//! would undo the create migration on a greenfield chain.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.products_read_entity ADD COLUMN IF NOT EXISTS usage_type_ref text"];

const SQLITE_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE products_read_entity ADD COLUMN usage_type_ref text"];

const EMPTY: &[&str] = &[];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite has no ADD COLUMN IF NOT EXISTS syntax. The migration runner
        // serializes schema changes; checking here also preserves fresh installs.
        if manager.get_database_backend() == sea_orm::DatabaseBackend::Sqlite
            && manager
                .has_column("products_read_entity", "usage_type_ref")
                .await?
        {
            return Ok(());
        }
        super::exec_backend(self.name(), manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, EMPTY, EMPTY).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{Migration, MigrationTrait, SchemaManager};
    use sea_orm::ConnectionTrait as _;

    /// An old `SQLite` estate gains the column; a fresh one keeps its data on replay.
    #[tokio::test]
    async fn usage_type_ref_upgrade_handles_old_and_current_sqlite_schemas() {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("CREATE TABLE products_read_entity (entity_id text PRIMARY KEY)")
            .await
            .unwrap();
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.unwrap();
        assert!(
            manager
                .has_column("products_read_entity", "usage_type_ref")
                .await
                .unwrap()
        );
        db.execute_unprepared("INSERT INTO products_read_entity VALUES ('one', 'cpu.hours')")
            .await
            .unwrap();
        Migration.up(&manager).await.unwrap();
        Migration.down(&manager).await.unwrap();
        let row = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT usage_type_ref FROM products_read_entity WHERE entity_id = 'one'",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.try_get::<String>("", "usage_type_ref").unwrap(),
            "cpu.hours"
        );
    }
}
