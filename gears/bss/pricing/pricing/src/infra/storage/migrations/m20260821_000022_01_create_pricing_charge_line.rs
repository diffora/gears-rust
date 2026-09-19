//! Create `bss.pricing_charge_line` — stable logical charge identity.
//!
//! One row is one canonical logical scope (`plan`, `sku`, overlay, phase,
//! eligibility, charge kind, cohort, dimension). Currency and region are not
//! here: those are [`super::m20260821_000022_03_create_pricing_market_price`].
//!
//! Sorts after plan creation (`000021`) and before line versions. Phase
//! membership is not an FK; the later phase-guard install point remains the
//! membership check.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_charge_line (
            tenant_id         uuid        NOT NULL,
            charge_line_id    uuid        NOT NULL,
            plan_id           uuid        NOT NULL,
            phase             uuid        NOT NULL,
            price_overlay     text        NOT NULL DEFAULT 'base'::text,
            price_eligibility text        NOT NULL DEFAULT 'all_subscriptions'::text,
            charge_kind       text        NOT NULL,
            cohort            text        NOT NULL DEFAULT 'none'::text,
            sku_id            uuid        NOT NULL,
            dimension_key     text        NOT NULL DEFAULT ''::text,
            CONSTRAINT chk_pricing_charge_line_overlay CHECK (price_overlay = 'base'),
            CONSTRAINT chk_pricing_charge_line_eligibility CHECK (price_eligibility IN (
                'all_subscriptions','new_subscriptions_only','existing_grandfathered')),
            CONSTRAINT chk_pricing_charge_line_charge_kind CHECK (charge_kind IN (
                'recurring','usage','one_time')),
            CONSTRAINT chk_pricing_charge_line_cohort_eligibility CHECK (
                (cohort <> 'none') = (price_eligibility = 'existing_grandfathered')),
            CONSTRAINT uq_pricing_charge_line_logical_scope UNIQUE (
                tenant_id, plan_id, sku_id, price_overlay, phase, price_eligibility,
                charge_kind, cohort, dimension_key),
            CONSTRAINT uq_pricing_charge_line_id_plan UNIQUE (
                tenant_id, charge_line_id, plan_id),
            CONSTRAINT pricing_charge_line_pkey PRIMARY KEY (tenant_id, charge_line_id)
        )",
    "CREATE INDEX idx_pricing_charge_line_plan ON bss.pricing_charge_line USING btree (tenant_id, plan_id)",
    "CREATE OR REPLACE FUNCTION bss.pricing_charge_line_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RETURN OLD;
          END IF;
          RAISE EXCEPTION
            'pricing_charge_line: charge line % is an identity row; its logical scope is immutable',
            OLD.charge_line_id;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_charge_line_append_only BEFORE UPDATE ON bss.pricing_charge_line FOR EACH ROW EXECUTE FUNCTION bss.pricing_charge_line_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_charge_line",
    "DROP FUNCTION IF EXISTS bss.pricing_charge_line_append_only()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_charge_line (
            tenant_id         text NOT NULL,
            charge_line_id    text NOT NULL,
            plan_id           text NOT NULL,
            phase             text NOT NULL,
            price_overlay     text NOT NULL DEFAULT 'base',
            price_eligibility text NOT NULL DEFAULT 'all_subscriptions',
            charge_kind       text NOT NULL,
            cohort            text NOT NULL DEFAULT 'none',
            sku_id            text NOT NULL,
            dimension_key     text NOT NULL DEFAULT '',
            PRIMARY KEY (tenant_id, charge_line_id),
            CONSTRAINT chk_pricing_charge_line_overlay CHECK (price_overlay = 'base'),
            CONSTRAINT chk_pricing_charge_line_eligibility CHECK (price_eligibility IN (
                'all_subscriptions','new_subscriptions_only','existing_grandfathered')),
            CONSTRAINT chk_pricing_charge_line_charge_kind CHECK (charge_kind IN (
                'recurring','usage','one_time')),
            CONSTRAINT chk_pricing_charge_line_cohort_eligibility CHECK (
                (cohort <> 'none') = (price_eligibility = 'existing_grandfathered')),
            CONSTRAINT uq_pricing_charge_line_logical_scope UNIQUE (
                tenant_id, plan_id, sku_id, price_overlay, phase, price_eligibility,
                charge_kind, cohort, dimension_key),
            CONSTRAINT uq_pricing_charge_line_id_plan UNIQUE (
                tenant_id, charge_line_id, plan_id)
        )",
    "CREATE INDEX idx_pricing_charge_line_plan ON pricing_charge_line (tenant_id, plan_id)",
    "CREATE TRIGGER trg_pricing_charge_line_frozen_columns BEFORE UPDATE ON pricing_charge_line FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'pricing_charge_line: charge line is an identity row; its logical scope is immutable'); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_charge_line"];

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
