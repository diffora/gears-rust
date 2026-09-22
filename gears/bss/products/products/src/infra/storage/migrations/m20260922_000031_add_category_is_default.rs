//! Add `is_default` to `products_category` on estates that already applied
//! `m20260901_000018` before **P-D-182** edited that create in place.
//!
//! Fresh installs get the column and the index from `m000018`. Postgres uses
//! `IF NOT EXISTS` for both; `SQLite` checks the schema before adding the
//! column and takes the index statement alone when the column is already
//! there, so both paths converge. `DOWN` is a no-op: dropping the column
//! would undo the create migration on a greenfield chain.
//!
//! The index is the decision's whole enforcement — at most one default per
//! tenant — so an estate that gained the column without it would admit a
//! second default silently. Both statements travel together for that reason.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.products_category ADD COLUMN IF NOT EXISTS is_default boolean NOT NULL DEFAULT false",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_products_category_default ON bss.products_category USING btree (tenant_id) WHERE is_default",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE products_category ADD COLUMN is_default integer NOT NULL DEFAULT 0",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_products_category_default ON products_category (tenant_id) WHERE is_default",
];

/// A fresh `SQLite` install already carries the column from `m000018` and
/// needs the index statement alone, which is itself idempotent.
const SQLITE_INDEX_ONLY: &[&str] = &[
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_products_category_default ON products_category (tenant_id) WHERE is_default",
];

const EMPTY: &[&str] = &[];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite has no ADD COLUMN IF NOT EXISTS syntax. The migration runner
        // serializes schema changes; checking here also preserves fresh
        // installs, which reach this migration with the column already in
        // place.
        if manager.get_database_backend() == sea_orm::DatabaseBackend::Sqlite
            && manager
                .has_column("products_category", "is_default")
                .await?
        {
            return super::exec_backend(self.name(), manager, EMPTY, SQLITE_INDEX_ONLY).await;
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

    /// An old `SQLite` estate gains the column **and** the index; a replay
    /// keeps its data and the index still refuses a second default.
    #[tokio::test]
    async fn is_default_upgrade_handles_old_and_current_sqlite_schemas() {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE products_category (tenant_id text NOT NULL, category_id text PRIMARY KEY)",
        )
        .await
        .unwrap();
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.unwrap();
        assert!(
            manager
                .has_column("products_category", "is_default")
                .await
                .unwrap()
        );

        db.execute_unprepared("INSERT INTO products_category VALUES ('t', 'one', 1)")
            .await
            .unwrap();
        // Replay is idempotent, and the index survives it.
        Migration.up(&manager).await.unwrap();
        assert!(
            db.execute_unprepared("INSERT INTO products_category VALUES ('t', 'two', 1)")
                .await
                .is_err(),
            "the partial unique index refuses a second default after the replay"
        );
        db.execute_unprepared("INSERT INTO products_category VALUES ('t', 'three', 0)")
            .await
            .expect("and it is partial: unflagged rows are unconstrained");

        Migration.down(&manager).await.unwrap();
    }
}
