//! Initial credstore schema — `credstore_secrets` table (incl. the monotonic
//! `version` column) with indexes.
//!
//! Per-backend raw `SQL` is used (not `SeaORM`'s schema-builder) so that
//! `CHECK` constraints and partial unique indexes are preserved verbatim.
//! `MySQL` is not supported; the migration fails fast with a typed error.

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::ConnectionTrait;

const MYSQL_NOT_SUPPORTED: &str = "credstore migrations: MySQL is not supported \
    (this migration set targets PostgreSQL/SQLite)";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let conn = manager.get_connection();

        let statements: Vec<&str> = match backend {
            sea_orm::DatabaseBackend::Postgres => vec![
                r"
CREATE TABLE IF NOT EXISTS credstore_secrets (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    reference TEXT NOT NULL CHECK (length(reference) BETWEEN 1 AND 255),
    sharing SMALLINT NOT NULL CHECK (sharing IN (1, 2, 3)),
    owner_id UUID NOT NULL,
    status SMALLINT NOT NULL CHECK (status IN (1, 2)),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    version BIGINT NOT NULL DEFAULT 1
);
                ",
                "CREATE UNIQUE INDEX IF NOT EXISTS uq_credstore_nonprivate ON credstore_secrets (tenant_id, reference) WHERE sharing <> 1;",
                "CREATE UNIQUE INDEX IF NOT EXISTS uq_credstore_private ON credstore_secrets (tenant_id, reference, owner_id) WHERE sharing = 1;",
                "CREATE INDEX IF NOT EXISTS idx_credstore_lookup ON credstore_secrets (reference, tenant_id, status);",
                "CREATE INDEX IF NOT EXISTS idx_credstore_provisioning ON credstore_secrets (created_at) WHERE status = 1;",
            ],
            sea_orm::DatabaseBackend::Sqlite => vec![
                r"
CREATE TABLE IF NOT EXISTS credstore_secrets (
    id BLOB PRIMARY KEY NOT NULL,
    tenant_id BLOB NOT NULL,
    reference TEXT NOT NULL CHECK (length(reference) BETWEEN 1 AND 255),
    sharing SMALLINT NOT NULL CHECK (sharing IN (1, 2, 3)),
    owner_id BLOB NOT NULL,
    status SMALLINT NOT NULL CHECK (status IN (1, 2)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    version BIGINT NOT NULL DEFAULT 1
);
                ",
                "CREATE UNIQUE INDEX IF NOT EXISTS uq_credstore_nonprivate ON credstore_secrets (tenant_id, reference) WHERE sharing <> 1;",
                "CREATE UNIQUE INDEX IF NOT EXISTS uq_credstore_private ON credstore_secrets (tenant_id, reference, owner_id) WHERE sharing = 1;",
                "CREATE INDEX IF NOT EXISTS idx_credstore_lookup ON credstore_secrets (reference, tenant_id, status);",
                "CREATE INDEX IF NOT EXISTS idx_credstore_provisioning ON credstore_secrets (created_at) WHERE status = 1;",
            ],
            sea_orm::DatabaseBackend::MySql => {
                return Err(DbErr::Custom(MYSQL_NOT_SUPPORTED.to_owned()));
            }
        };

        for sql in statements {
            conn.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if matches!(
            manager.get_database_backend(),
            sea_orm::DatabaseBackend::MySql
        ) {
            return Err(DbErr::Custom(MYSQL_NOT_SUPPORTED.to_owned()));
        }
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS credstore_secrets;")
            .await?;
        Ok(())
    }
}
