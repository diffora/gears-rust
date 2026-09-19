//! Create `bss.pricing_market_price` — one currency/region variant of a line.
//!
//! Unique per `(tenant, charge_line, currency, region)`. Monetary versions of
//! this market live on `pricing_price` and may be several when their windows
//! do not overlap.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_market_price (
            tenant_id        uuid       NOT NULL,
            market_price_id  uuid       NOT NULL,
            charge_line_id   uuid       NOT NULL,
            currency         varchar(3) NOT NULL,
            region           text       NOT NULL,
            CONSTRAINT chk_pricing_market_price_region_no_separator CHECK (region NOT LIKE '%|%'),
            CONSTRAINT fk_pricing_market_price_line FOREIGN KEY (tenant_id, charge_line_id)
                REFERENCES bss.pricing_charge_line (tenant_id, charge_line_id),
            CONSTRAINT uq_pricing_market_price_line UNIQUE (
                tenant_id, market_price_id, charge_line_id),
            CONSTRAINT uq_pricing_market_price_scope UNIQUE (
                tenant_id, charge_line_id, currency, region),
            CONSTRAINT pricing_market_price_pkey PRIMARY KEY (tenant_id, market_price_id)
        )",
    "CREATE INDEX idx_pricing_market_price_line ON bss.pricing_market_price USING btree (tenant_id, charge_line_id)",
    "CREATE OR REPLACE FUNCTION bss.pricing_market_price_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RETURN OLD;
          END IF;
          RAISE EXCEPTION
            'pricing_market_price: market % is an identity row; currency and region are immutable',
            OLD.market_price_id;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_market_price_append_only BEFORE UPDATE ON bss.pricing_market_price FOR EACH ROW EXECUTE FUNCTION bss.pricing_market_price_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_market_price",
    "DROP FUNCTION IF EXISTS bss.pricing_market_price_append_only()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_market_price (
            tenant_id        text       NOT NULL,
            market_price_id  text       NOT NULL,
            charge_line_id   text       NOT NULL,
            currency         varchar(3) NOT NULL,
            region           text       NOT NULL,
            PRIMARY KEY (tenant_id, market_price_id),
            CONSTRAINT chk_pricing_market_price_region_no_separator CHECK (region NOT LIKE '%|%'),
            CONSTRAINT fk_pricing_market_price_line FOREIGN KEY (tenant_id, charge_line_id)
                REFERENCES pricing_charge_line (tenant_id, charge_line_id),
            CONSTRAINT uq_pricing_market_price_line UNIQUE (
                tenant_id, market_price_id, charge_line_id),
            CONSTRAINT uq_pricing_market_price_scope UNIQUE (
                tenant_id, charge_line_id, currency, region)
        )",
    "CREATE INDEX idx_pricing_market_price_line ON pricing_market_price (tenant_id, charge_line_id)",
    "CREATE TRIGGER trg_pricing_market_price_frozen_columns BEFORE UPDATE ON pricing_market_price FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_market_price: market is an identity row; currency and region are immutable'); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_market_price"];

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
