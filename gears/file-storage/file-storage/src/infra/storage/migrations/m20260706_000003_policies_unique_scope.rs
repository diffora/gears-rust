//! Close the policy upsert race with two partial unique indexes (P2
//! remediation 2.4).
//!
//! `PolicyRepo::upsert` previously issued a `DELETE` followed by an
//! independent `INSERT` with no transaction wrapper and no unique constraint
//! on `(tenant_id, scope, scope_owner_id)`. Two concurrent `PUT /policy`
//! calls for the same scope could each see zero matching rows to delete and
//! then both insert, leaving two rows for what is supposed to be an at-most-
//! one-per-scope table — after that, `PolicyRepo::get` (which does not order
//! or limit) becomes non-deterministic about which row it returns.
//!
//! This migration adds two **partial** unique indexes instead of one plain
//! composite unique index, because Postgres (and `SQLite`, which follows the
//! same NULL-distinctness rule) treat every `NULL` as distinct for
//! uniqueness purposes: a single `UNIQUE (tenant_id, scope, scope_owner_id)`
//! index would correctly dedupe user-scope rows (non-null `scope_owner_id`)
//! but would silently allow unlimited tenant-scope rows (`scope_owner_id IS
//! NULL`) for the same tenant, since NULL never equals NULL for uniqueness
//! checks. Splitting into two partial indexes closes both gaps explicitly:
//!
//! - `policies_user_scope_unique_idx` — at most one row per
//!   `(tenant_id, scope, scope_owner_id)` when `scope_owner_id IS NOT NULL`
//!   (user-scope rows).
//! - `policies_tenant_scope_unique_idx` — at most one row per
//!   `(tenant_id, scope)` when `scope_owner_id IS NULL` (tenant-scope rows).
//!
//! The old non-unique `policies_scope_idx` (added in the P2-initial
//! migration) is left in place — it is harmless and still useful for the
//! `get()` query's lookup plan.
//!
//! ⚠️ **No existing `SQLite` partial-index precedent in this gear**: the P2
//! initial migration's `retention_rules_file_scope_idx` is partial only on
//! the Postgres side (`WHERE scope = 'file'`); its `SQLite` counterpart is a
//! plain composite index with no `WHERE`. `SQLite` has supported partial
//! (`WHERE`-qualified) indexes since 3.8.0, and the syntax is identical to
//! Postgres's, but since this is new ground for the gear it is covered
//! directly by `tests/migration_test.rs::policies_unique_index_rejects_duplicate_scope_tuple`
//! rather than assumed to work by analogy.
//!
//! `PolicyRepo::upsert` itself is additionally wrapped in an explicit DB
//! transaction (`Store::upsert_policy`) so the delete+insert pair is atomic;
//! this unique index is the backstop that turns the remaining
//! no-existing-row race (two concurrent first-time upserts, neither of which
//! has anything to delete) into a clean constraint-violation error for the
//! losing writer instead of a silently duplicated row.
//!
//! @cpt-cf-file-storage-fr-allowed-types-policy
//! @cpt-cf-file-storage-fr-size-limits-policy
//! @cpt-cf-file-storage-fr-metadata-limits

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::ConnectionTrait;

#[derive(DeriveMigrationName)]
pub struct Migration;

const POSTGRES_UP: &str = r"
CREATE UNIQUE INDEX IF NOT EXISTS policies_user_scope_unique_idx
    ON policies (tenant_id, scope, scope_owner_id) WHERE scope_owner_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS policies_tenant_scope_unique_idx
    ON policies (tenant_id, scope) WHERE scope_owner_id IS NULL;
";

const SQLITE_UP: &str = r"
CREATE UNIQUE INDEX IF NOT EXISTS policies_user_scope_unique_idx
    ON policies (tenant_id, scope, scope_owner_id) WHERE scope_owner_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS policies_tenant_scope_unique_idx
    ON policies (tenant_id, scope) WHERE scope_owner_id IS NULL;
";

const DOWN: &str = r"
DROP INDEX IF EXISTS policies_user_scope_unique_idx;
DROP INDEX IF EXISTS policies_tenant_scope_unique_idx;
";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let sql = match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres => POSTGRES_UP,
            sea_orm::DatabaseBackend::Sqlite => SQLITE_UP,
            sea_orm::DatabaseBackend::MySql => {
                return Err(DbErr::Custom(
                    "file-storage migrations support Postgres and SQLite only".to_owned(),
                ));
            }
        };
        conn.execute_unprepared(sql).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres | sea_orm::DatabaseBackend::Sqlite => {
                conn.execute_unprepared(DOWN).await?;
                Ok(())
            }
            sea_orm::DatabaseBackend::MySql => Err(DbErr::Custom(
                "file-storage migrations support Postgres and SQLite only".to_owned(),
            )),
        }
    }
}
