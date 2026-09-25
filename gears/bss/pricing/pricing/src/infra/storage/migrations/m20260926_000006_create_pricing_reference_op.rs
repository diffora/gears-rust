//! Reference op schema.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP: &[&str] = &[
    r"CREATE TABLE IF NOT EXISTS bss.pricing_reference_op (
  op_id uuid PRIMARY KEY, tenant_id uuid NOT NULL,
  kind text NOT NULL CHECK (kind IN ('create_entry','delete_entry','rereserve_entry')),
  price_book_entry_id uuid NOT NULL, sku_id uuid NOT NULL, reservation_id uuid,
  idempotency_key text,
  state text NOT NULL CHECK (state IN ('reserving','written','cancelling','releasing','done')),
  outcome text, attempts integer NOT NULL DEFAULT 0, next_attempt_at timestamptz NOT NULL,
  last_error text, created_by uuid NOT NULL, created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL
)",
    r"CREATE INDEX IF NOT EXISTS pricing_reference_op_due ON bss.pricing_reference_op (state, next_attempt_at) WHERE state <> 'done'",
];
const SQLITE_UP: &[&str] = &[
    r"CREATE TABLE IF NOT EXISTS pricing_reference_op (
  op_id text PRIMARY KEY, tenant_id text NOT NULL,
  kind text NOT NULL CHECK (kind IN ('create_entry','delete_entry','rereserve_entry')),
  price_book_entry_id text NOT NULL, sku_id text NOT NULL, reservation_id text,
  idempotency_key text,
  state text NOT NULL CHECK (state IN ('reserving','written','cancelling','releasing','done')),
  outcome text, attempts integer NOT NULL DEFAULT 0, next_attempt_at text NOT NULL,
  last_error text, created_by text NOT NULL, created_at text NOT NULL, updated_at text NOT NULL
)",
    r"CREATE INDEX IF NOT EXISTS pricing_reference_op_due ON pricing_reference_op (state, next_attempt_at) WHERE state <> 'done'",
];
const PG_DOWN: &[&str] = &[r"DROP TABLE IF EXISTS bss.pricing_reference_op"];
const SQLITE_DOWN: &[&str] = &[r"DROP TABLE IF EXISTS pricing_reference_op"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_UP, SQLITE_UP).await
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_DOWN, SQLITE_DOWN).await
    }
}
