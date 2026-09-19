//! Create `bss.pricing_charge_line_version` — revision-owned shared structure.
//!
//! Holds every authorable non-monetary field of a charge (Task 2's
//! `ChargeStructure` plus billing timing and the proration contract). Identity
//! axes stay on [`super::m20260821_000022_01_create_pricing_charge_line`].
//! Published content is immutable; draft edits remain revision-owned.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_charge_line_version (
            tenant_id                      uuid        NOT NULL,
            line_version_id                uuid        NOT NULL,
            charge_line_id                 uuid        NOT NULL,
            plan_revision                  bigint      NOT NULL,
            lifecycle_state                text        NOT NULL,
            invoice_line_template          text,
            gl_code_ref                    text,
            resolved_invoice_line_template text,
            resolved_gl_code               text,
            model_kind                     text,
            package_size                   bigint,
            quantity_source                text,
            manual_quantity                bigint,
            meter                          text,
            billing_granularity            text,
            tier_aggregation_window        text,
            tier_qualification_window      text,
            aggregation_function           text,
            aggregation_granularity        text,
            max_hold_granules              bigint,
            included_allowance             jsonb,
            reservation_flavor             text,
            min_qty_purchase               bigint,
            min_qty_usage                  bigint,
            min_qty_usage_fallback         text,
            discount_ref                   text,
            billing_timing                 text,
            billing_anchor_policy          text,
            anchor_day                     integer,
            proration_basis                text,
            credit_on_downgrade            boolean,
            created_at_utc                 timestamptz NOT NULL DEFAULT now(),
            created_by                     uuid        NOT NULL,
            row_version                    bigint      NOT NULL DEFAULT 0,
            CONSTRAINT chk_pricing_charge_line_version_lifecycle_state CHECK (
                lifecycle_state IN ('draft','published','superseded')),
            CONSTRAINT chk_pricing_charge_line_version_meter_no_separator CHECK (
                meter IS NULL OR meter NOT LIKE '%|%'),
            CONSTRAINT chk_pricing_charge_line_version_manual_quantity CHECK (
                manual_quantity IS NULL OR manual_quantity >= 0),
            CONSTRAINT chk_pricing_charge_line_version_max_hold_granules CHECK (
                max_hold_granules IS NULL OR max_hold_granules >= 1),
            CONSTRAINT chk_pricing_charge_line_version_min_qty_purchase CHECK (
                min_qty_purchase IS NULL OR min_qty_purchase >= 0),
            CONSTRAINT chk_pricing_charge_line_version_min_qty_usage CHECK (
                min_qty_usage IS NULL OR min_qty_usage >= 0),
            CONSTRAINT chk_pricing_charge_line_version_model_kind CHECK (
                model_kind IS NULL OR model_kind IN (
                    'flat','per_unit','graduated','volume','package')),
            CONSTRAINT chk_pricing_charge_line_version_package_fields_kind CHECK (
                (package_size IS NULL) OR (model_kind IS NOT NULL AND model_kind = 'package')),
            CONSTRAINT chk_pricing_charge_line_version_package_size CHECK (
                package_size IS NULL OR package_size > 0),
            CONSTRAINT chk_pricing_charge_line_version_quantity_source CHECK (
                quantity_source IS NULL OR quantity_source IN (
                    'subscription_seat_count','manual')),
            CONSTRAINT chk_pricing_charge_line_version_billing_granularity CHECK (
                billing_granularity IS NULL OR billing_granularity IN (
                    'per_second','per_minute','per_hour','per_day','whole_unit')),
            CONSTRAINT chk_pricing_charge_line_version_tier_aggregation_window CHECK (
                tier_aggregation_window IS NULL OR tier_aggregation_window IN (
                    'calendar_month','invoice_period','subscription_lifetime','per_event','per_hour')),
            CONSTRAINT chk_pricing_charge_line_version_tier_qualification_window CHECK (
                tier_qualification_window IS NULL OR tier_qualification_window IN (
                    'current','trailing_period')),
            CONSTRAINT chk_pricing_charge_line_version_aggregation_function CHECK (
                aggregation_function IS NULL OR aggregation_function IN (
                    'sum','peak','time_weighted')),
            CONSTRAINT chk_pricing_charge_line_version_aggregation_granularity CHECK (
                aggregation_granularity IS NULL OR aggregation_granularity IN ('hour','day')),
            CONSTRAINT chk_pricing_charge_line_version_billing_timing CHECK (
                billing_timing IS NULL OR billing_timing IN ('advance','arrears')),
            CONSTRAINT chk_pricing_charge_line_version_row_version CHECK (row_version >= 0),
            CONSTRAINT chk_pricing_charge_line_version_revision CHECK (plan_revision >= 0),
            CONSTRAINT fk_pricing_charge_line_version_line FOREIGN KEY (tenant_id, charge_line_id)
                REFERENCES bss.pricing_charge_line (tenant_id, charge_line_id),
            CONSTRAINT uq_pricing_charge_line_version_line UNIQUE (
                tenant_id, line_version_id, charge_line_id),
            CONSTRAINT uq_pricing_charge_line_version_revision UNIQUE (
                tenant_id, charge_line_id, plan_revision),
            CONSTRAINT pricing_charge_line_version_pkey PRIMARY KEY (tenant_id, line_version_id)
        )",
    "CREATE INDEX idx_pricing_charge_line_version_line ON bss.pricing_charge_line_version USING btree (tenant_id, charge_line_id, lifecycle_state)",
    "CREATE OR REPLACE FUNCTION bss.pricing_charge_line_version_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            IF OLD.lifecycle_state <> 'draft' THEN
              RAISE EXCEPTION
                'pricing_charge_line_version: DELETE of a % line version is not permitted',
                OLD.lifecycle_state;
            END IF;
            RETURN OLD;
          END IF;

          IF OLD.lifecycle_state = 'draft' THEN
            IF NEW.lifecycle_state NOT IN ('draft', 'published') THEN
              RAISE EXCEPTION
                'pricing_charge_line_version: lifecycle_state draft -> % is not a sanctioned transition',
                NEW.lifecycle_state;
            END IF;
            RETURN NEW;
          END IF;

          IF NEW.tenant_id                      IS DISTINCT FROM OLD.tenant_id
          OR NEW.line_version_id                IS DISTINCT FROM OLD.line_version_id
          OR NEW.charge_line_id                 IS DISTINCT FROM OLD.charge_line_id
          OR NEW.plan_revision                  IS DISTINCT FROM OLD.plan_revision
          OR NEW.invoice_line_template          IS DISTINCT FROM OLD.invoice_line_template
          OR NEW.gl_code_ref                    IS DISTINCT FROM OLD.gl_code_ref
          OR NEW.resolved_invoice_line_template IS DISTINCT FROM OLD.resolved_invoice_line_template
          OR NEW.resolved_gl_code               IS DISTINCT FROM OLD.resolved_gl_code
          OR NEW.model_kind                     IS DISTINCT FROM OLD.model_kind
          OR NEW.package_size                   IS DISTINCT FROM OLD.package_size
          OR NEW.quantity_source                IS DISTINCT FROM OLD.quantity_source
          OR NEW.manual_quantity                IS DISTINCT FROM OLD.manual_quantity
          OR NEW.meter                          IS DISTINCT FROM OLD.meter
          OR NEW.billing_granularity            IS DISTINCT FROM OLD.billing_granularity
          OR NEW.tier_aggregation_window        IS DISTINCT FROM OLD.tier_aggregation_window
          OR NEW.tier_qualification_window      IS DISTINCT FROM OLD.tier_qualification_window
          OR NEW.aggregation_function           IS DISTINCT FROM OLD.aggregation_function
          OR NEW.aggregation_granularity        IS DISTINCT FROM OLD.aggregation_granularity
          OR NEW.max_hold_granules              IS DISTINCT FROM OLD.max_hold_granules
          OR NEW.included_allowance             IS DISTINCT FROM OLD.included_allowance
          OR NEW.reservation_flavor             IS DISTINCT FROM OLD.reservation_flavor
          OR NEW.min_qty_purchase               IS DISTINCT FROM OLD.min_qty_purchase
          OR NEW.min_qty_usage                  IS DISTINCT FROM OLD.min_qty_usage
          OR NEW.min_qty_usage_fallback         IS DISTINCT FROM OLD.min_qty_usage_fallback
          OR NEW.discount_ref                   IS DISTINCT FROM OLD.discount_ref
          OR NEW.billing_timing                 IS DISTINCT FROM OLD.billing_timing
          OR NEW.billing_anchor_policy          IS DISTINCT FROM OLD.billing_anchor_policy
          OR NEW.anchor_day                     IS DISTINCT FROM OLD.anchor_day
          OR NEW.proration_basis                IS DISTINCT FROM OLD.proration_basis
          OR NEW.credit_on_downgrade            IS DISTINCT FROM OLD.credit_on_downgrade
          OR NEW.created_by                     IS DISTINCT FROM OLD.created_by
          OR NEW.created_at_utc                 IS DISTINCT FROM OLD.created_at_utc
          OR NEW.row_version                    IS DISTINCT FROM OLD.row_version THEN
            RAISE EXCEPTION
              'pricing_charge_line_version: line version % is published; shared content is immutable',
              OLD.line_version_id;
          END IF;

          IF NEW.lifecycle_state IS DISTINCT FROM OLD.lifecycle_state
             AND NOT (OLD.lifecycle_state = 'published'
                      AND NEW.lifecycle_state = 'superseded') THEN
            RAISE EXCEPTION
              'pricing_charge_line_version: lifecycle_state % -> % is not a sanctioned transition',
              OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE OR REPLACE FUNCTION bss.pricing_charge_tier_parent_kind() RETURNS trigger AS $$
        BEGIN
          IF NEW.model_kind IS NULL OR NEW.model_kind NOT IN ('graduated','volume') THEN
            IF EXISTS (SELECT 1 FROM bss.pricing_charge_tier
                        WHERE tenant_id = OLD.tenant_id
                          AND line_version_id = OLD.line_version_id) THEN
              RAISE EXCEPTION
                'pricing_charge_tier: line version % still carries bands and may not become a % version',
                OLD.line_version_id, coalesce(NEW.model_kind, 'kindless');
            END IF;
          END IF;
          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_charge_line_version_append_only BEFORE DELETE OR UPDATE ON bss.pricing_charge_line_version FOR EACH ROW EXECUTE FUNCTION bss.pricing_charge_line_version_append_only()",
    "CREATE TRIGGER trg_pricing_charge_tier_parent_kind BEFORE UPDATE ON bss.pricing_charge_line_version FOR EACH ROW EXECUTE FUNCTION bss.pricing_charge_tier_parent_kind()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_charge_line_version",
    "DROP FUNCTION IF EXISTS bss.pricing_charge_line_version_append_only()",
    "DROP FUNCTION IF EXISTS bss.pricing_charge_tier_parent_kind()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_charge_line_version (
            tenant_id                      text    NOT NULL,
            line_version_id                text    NOT NULL,
            charge_line_id                 text    NOT NULL,
            plan_revision                  bigint  NOT NULL,
            lifecycle_state                text    NOT NULL,
            invoice_line_template          text,
            gl_code_ref                    text,
            resolved_invoice_line_template text,
            resolved_gl_code               text,
            model_kind                     text,
            package_size                   bigint,
            quantity_source                text,
            manual_quantity                bigint,
            meter                          text,
            billing_granularity            text,
            tier_aggregation_window        text,
            tier_qualification_window      text,
            aggregation_function           text,
            aggregation_granularity        text,
            max_hold_granules              bigint,
            included_allowance             text,
            reservation_flavor             text,
            min_qty_purchase               bigint,
            min_qty_usage                  bigint,
            min_qty_usage_fallback         text,
            discount_ref                   text,
            billing_timing                 text,
            billing_anchor_policy          text,
            anchor_day                     integer,
            proration_basis                text,
            credit_on_downgrade            boolean,
            created_at_utc                 text    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%S', 'now') || '+00:00'),
            created_by                     text    NOT NULL,
            row_version                    bigint  NOT NULL DEFAULT 0,
            PRIMARY KEY (tenant_id, line_version_id),
            CONSTRAINT chk_pricing_charge_line_version_lifecycle_state CHECK (
                lifecycle_state IN ('draft','published','superseded')),
            CONSTRAINT chk_pricing_charge_line_version_meter_no_separator CHECK (
                meter IS NULL OR meter NOT LIKE '%|%'),
            CONSTRAINT chk_pricing_charge_line_version_manual_quantity CHECK (
                manual_quantity IS NULL OR manual_quantity >= 0),
            CONSTRAINT chk_pricing_charge_line_version_max_hold_granules CHECK (
                max_hold_granules IS NULL OR max_hold_granules >= 1),
            CONSTRAINT chk_pricing_charge_line_version_min_qty_purchase CHECK (
                min_qty_purchase IS NULL OR min_qty_purchase >= 0),
            CONSTRAINT chk_pricing_charge_line_version_min_qty_usage CHECK (
                min_qty_usage IS NULL OR min_qty_usage >= 0),
            CONSTRAINT chk_pricing_charge_line_version_model_kind CHECK (
                model_kind IS NULL OR model_kind IN (
                    'flat','per_unit','graduated','volume','package')),
            CONSTRAINT chk_pricing_charge_line_version_package_fields_kind CHECK (
                (package_size IS NULL) OR (model_kind IS NOT NULL AND model_kind = 'package')),
            CONSTRAINT chk_pricing_charge_line_version_package_size CHECK (
                package_size IS NULL OR package_size > 0),
            CONSTRAINT chk_pricing_charge_line_version_quantity_source CHECK (
                quantity_source IS NULL OR quantity_source IN (
                    'subscription_seat_count','manual')),
            CONSTRAINT chk_pricing_charge_line_version_billing_granularity CHECK (
                billing_granularity IS NULL OR billing_granularity IN (
                    'per_second','per_minute','per_hour','per_day','whole_unit')),
            CONSTRAINT chk_pricing_charge_line_version_tier_aggregation_window CHECK (
                tier_aggregation_window IS NULL OR tier_aggregation_window IN (
                    'calendar_month','invoice_period','subscription_lifetime','per_event','per_hour')),
            CONSTRAINT chk_pricing_charge_line_version_tier_qualification_window CHECK (
                tier_qualification_window IS NULL OR tier_qualification_window IN (
                    'current','trailing_period')),
            CONSTRAINT chk_pricing_charge_line_version_aggregation_function CHECK (
                aggregation_function IS NULL OR aggregation_function IN (
                    'sum','peak','time_weighted')),
            CONSTRAINT chk_pricing_charge_line_version_aggregation_granularity CHECK (
                aggregation_granularity IS NULL OR aggregation_granularity IN ('hour','day')),
            CONSTRAINT chk_pricing_charge_line_version_billing_timing CHECK (
                billing_timing IS NULL OR billing_timing IN ('advance','arrears')),
            CONSTRAINT chk_pricing_charge_line_version_row_version CHECK (row_version >= 0),
            CONSTRAINT chk_pricing_charge_line_version_revision CHECK (plan_revision >= 0),
            CONSTRAINT fk_pricing_charge_line_version_line FOREIGN KEY (tenant_id, charge_line_id)
                REFERENCES pricing_charge_line (tenant_id, charge_line_id),
            CONSTRAINT uq_pricing_charge_line_version_line UNIQUE (
                tenant_id, line_version_id, charge_line_id),
            CONSTRAINT uq_pricing_charge_line_version_revision UNIQUE (
                tenant_id, charge_line_id, plan_revision)
        )",
    "CREATE INDEX idx_pricing_charge_line_version_line ON pricing_charge_line_version (tenant_id, charge_line_id, lifecycle_state)",
    "CREATE TRIGGER trg_pricing_charge_line_version_draft_flip_whitelist BEFORE UPDATE ON pricing_charge_line_version FOR EACH ROW WHEN OLD.lifecycle_state = 'draft' AND NEW.lifecycle_state NOT IN ('draft','published') BEGIN SELECT RAISE(ABORT, 'pricing_charge_line_version: lifecycle_state transition is not sanctioned'); END",
    "CREATE TRIGGER trg_pricing_charge_line_version_flip_whitelist BEFORE UPDATE ON pricing_charge_line_version FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' AND NEW.lifecycle_state IS NOT OLD.lifecycle_state AND NOT (OLD.lifecycle_state = 'published' AND NEW.lifecycle_state = 'superseded') BEGIN SELECT RAISE(ABORT, 'pricing_charge_line_version: lifecycle_state transition is not sanctioned'); END",
    "CREATE TRIGGER trg_pricing_charge_line_version_frozen_columns BEFORE UPDATE ON pricing_charge_line_version FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' AND (NEW.tenant_id IS NOT OLD.tenant_id OR NEW.line_version_id IS NOT OLD.line_version_id OR NEW.charge_line_id IS NOT OLD.charge_line_id OR NEW.plan_revision IS NOT OLD.plan_revision OR NEW.invoice_line_template IS NOT OLD.invoice_line_template OR NEW.gl_code_ref IS NOT OLD.gl_code_ref OR NEW.resolved_invoice_line_template IS NOT OLD.resolved_invoice_line_template OR NEW.resolved_gl_code IS NOT OLD.resolved_gl_code OR NEW.model_kind IS NOT OLD.model_kind OR NEW.package_size IS NOT OLD.package_size OR NEW.quantity_source IS NOT OLD.quantity_source OR NEW.manual_quantity IS NOT OLD.manual_quantity OR NEW.meter IS NOT OLD.meter OR NEW.billing_granularity IS NOT OLD.billing_granularity OR NEW.tier_aggregation_window IS NOT OLD.tier_aggregation_window OR NEW.tier_qualification_window IS NOT OLD.tier_qualification_window OR NEW.aggregation_function IS NOT OLD.aggregation_function OR NEW.aggregation_granularity IS NOT OLD.aggregation_granularity OR NEW.max_hold_granules IS NOT OLD.max_hold_granules OR NEW.included_allowance IS NOT OLD.included_allowance OR NEW.reservation_flavor IS NOT OLD.reservation_flavor OR NEW.min_qty_purchase IS NOT OLD.min_qty_purchase OR NEW.min_qty_usage IS NOT OLD.min_qty_usage OR NEW.min_qty_usage_fallback IS NOT OLD.min_qty_usage_fallback OR NEW.discount_ref IS NOT OLD.discount_ref OR NEW.billing_timing IS NOT OLD.billing_timing OR NEW.billing_anchor_policy IS NOT OLD.billing_anchor_policy OR NEW.anchor_day IS NOT OLD.anchor_day OR NEW.proration_basis IS NOT OLD.proration_basis OR NEW.credit_on_downgrade IS NOT OLD.credit_on_downgrade OR NEW.created_by IS NOT OLD.created_by OR NEW.created_at_utc IS NOT OLD.created_at_utc OR NEW.row_version IS NOT OLD.row_version) BEGIN SELECT RAISE(ABORT, 'pricing_charge_line_version: line version is published; shared content is immutable'); END",
    "CREATE TRIGGER trg_pricing_charge_line_version_no_delete BEFORE DELETE ON pricing_charge_line_version FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' BEGIN SELECT RAISE(ABORT, 'pricing_charge_line_version: DELETE of a non-draft line version is not permitted'); END",
    "CREATE TRIGGER trg_pricing_charge_tier_parent_kind BEFORE UPDATE ON pricing_charge_line_version FOR EACH ROW WHEN NEW.model_kind IS NULL OR NEW.model_kind NOT IN ('graduated','volume') BEGIN SELECT RAISE(ABORT, 'pricing_charge_tier: a line version that still carries bands may not leave the graduated or volume kinds') WHERE EXISTS (SELECT 1 FROM pricing_charge_tier WHERE tenant_id = OLD.tenant_id AND line_version_id = OLD.line_version_id); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_charge_line_version"];

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
