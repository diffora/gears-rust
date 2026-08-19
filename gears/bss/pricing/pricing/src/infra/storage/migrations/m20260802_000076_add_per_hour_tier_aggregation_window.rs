//! `per_hour` joins `tierAggregationWindow` — D-313's storage half.
//!
//! The catalog could express "bill hourly" (`billingGranularity`) and "accumulate
//! the tier counter over the month" (`tierAggregationWindow`), but not "each hour
//! picks its own tier and the counter resets" — the shape a cloudlet-style `PaaS`
//! price is actually sold in. The enum gains a fifth member and this migration
//! lets the store hold it.
//!
//! # Postgres drops and re-adds a named constraint; `SQLite` rebuilds the table
//!
//! `SQLite` cannot alter a `CHECK`, so `pricing_price` is rebuilt — and it is the
//! **first** rebuild of this table, which is why every statement below is lifted
//! verbatim from `sqlite_master` on a freshly migrated database rather than
//! reconstructed by reading the seventy-five migrations that shaped it. Restating 47
//! columns and 21 constraints by hand is how a rebuild quietly drops a guard, and
//! `DROP TABLE` takes the indexes and triggers with it:
//!
//! * **5 indexes** (`idx_pricing_price_plan`, `idx_pricing_price_supersedes`,
//!   `uq_pricing_price_meter_line_current`, `uq_pricing_price_scope_key_current`,
//!   `uq_pricing_price_scope_key_draft`),
//! * **6 triggers of its own** — the two flip whitelists, the frozen-column guard,
//!   the grandfather monotonicity guard, the delete refusal, and the band/kind
//!   parent guard,
//! * **5 triggers on `pricing_price_tier_band`** which sub-select this table and
//!   therefore have to be dropped *before* the rename can re-parse the schema.
//!
//! All sixteen are recreated. The `INSERT` names every column instead of
//! `SELECT *`, because columns added by later `ALTER TABLE ADD COLUMN` sit at the
//! end and a positional copy would put them back in a different order.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_DROP: &str = "ALTER TABLE bss.pricing_price DROP CONSTRAINT IF EXISTS chk_pricing_price_tier_aggregation_window";
const PG_ADD: &str = "ALTER TABLE bss.pricing_price ADD CONSTRAINT \
     chk_pricing_price_tier_aggregation_window CHECK (\
       tier_aggregation_window IS NULL OR tier_aggregation_window IN (\
         'calendar_month','invoice_period','subscription_lifetime','per_event','per_hour'))";

/// The five triggers on `pricing_price_tier_band` that sub-select this table.
/// Dropped first so the rename can re-parse the schema, recreated verbatim at the end.
const DROP_FOREIGN_TRIGGERS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_price_tier_band_kind_insert",
    "DROP TRIGGER IF EXISTS trg_pricing_price_tier_band_kind_update",
    "DROP TRIGGER IF EXISTS trg_pricing_price_tier_band_no_delete",
    "DROP TRIGGER IF EXISTS trg_pricing_price_tier_band_no_insert",
    "DROP TRIGGER IF EXISTS trg_pricing_price_tier_band_no_update",
];

/// The table, restated from `sqlite_master` with one CHECK widened.
/// Every column by name: a `SELECT *` would silently reorder after an ALTER.
///
/// **One column per line since 2026-08-18** (review Z1-6). The thirteen columns
/// thirteen later migrations added stood inline on the `row_version` line, which
/// made this — the widest table in the schema, and the one rebuild whose column
/// list a reviewer most needs to check against a thirteen-`ALTER` history — the
/// single table definition in this chain that could not be diffed line-wise
/// against its Postgres arm. It defeated the 2026-08-17 review's own automated
/// parity check, which reported those thirteen as missing from `SQLite`: exactly
/// the false alarm the formatting invites, and exactly the true one it would hide.
/// The content is unchanged, and all 46 columns were re-verified against
/// `m20260802_000002` plus the thirteen `ALTER`s in the reflow.
const REBUILD_TABLE: &[&str] = &[
    "CREATE TABLE pricing_price_rebuilt (
        price_id                  text        NOT NULL PRIMARY KEY,
        tenant_id                 text        NOT NULL,
        plan_id                   text        NOT NULL,
        currency                  varchar(3)  NOT NULL,
        region                    text        NOT NULL,
        price_overlay             text        NOT NULL DEFAULT 'base',
        phase                     text        NOT NULL,
        price_eligibility         text        NOT NULL DEFAULT 'all_subscriptions',
        charge_kind               text        NOT NULL,
        cohort                    text        NOT NULL DEFAULT 'none',
        amount_minor              bigint,
        model_kind                text,
        tax_inclusive             boolean     NOT NULL DEFAULT false,
        billing_timing            text,
        quantity_source           text,
        manual_quantity           bigint,
        package_size              bigint,
        package_price_minor       bigint,
        meter                     text,
        dimension_key             text        NOT NULL DEFAULT '',
        billing_granularity       text,
        aggregation_function      text,
        aggregation_granularity   text,
        tier_aggregation_window   text,
        tier_qualification_window text,
        max_hold_granules         bigint,
        included_allowance        text,
        rounding_policy_ref       text,
        grandfather_until         text,
        supersedes_price_id       text,
        lifecycle_state           text        NOT NULL,
        created_by                text        NOT NULL,
        created_at_utc            text        NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        row_version               bigint      NOT NULL DEFAULT 0,
        -- The thirteen columns thirteen later migrations added. One per line since
        -- 2026-08-18; see the note above the statement.
        tax_category_ref          text,
        resolved_tax_category     text,
        billing_anchor_policy     text,
        anchor_day                integer,
        proration_basis           text,
        credit_on_downgrade       boolean,
        reserved_rate_minor       bigint,
        reservation_flavor        text,
        min_qty_purchase          bigint,
        min_qty_usage             bigint,
        min_qty_usage_fallback    text,
        discount_ref              text,
        unit_rate_nano            bigint,
        CONSTRAINT chk_pricing_price_lifecycle_state CHECK (
            lifecycle_state IN ('draft','published','superseded')),
        CONSTRAINT chk_pricing_price_overlay CHECK (price_overlay = 'base'),
        CONSTRAINT chk_pricing_price_eligibility CHECK (
            price_eligibility IN (
                'all_subscriptions','new_subscriptions_only','existing_grandfathered')),
        CONSTRAINT chk_pricing_price_charge_kind CHECK (
            charge_kind IN ('recurring','usage','one_time','one_time_setup')),
        CONSTRAINT chk_pricing_price_model_kind CHECK (
            model_kind IS NULL
            OR model_kind IN ('flat','per_unit','graduated','volume','package')),
        CONSTRAINT chk_pricing_price_billing_timing CHECK (
            billing_timing IS NULL OR billing_timing IN ('advance','arrears')),
        CONSTRAINT chk_pricing_price_amount_non_negative CHECK (
            amount_minor IS NULL OR amount_minor >= 0),
        -- **Added to this migration's text on 2026-08-19, which reaches only
        -- databases built from scratch afterwards.** `sea_orm_migration` records a
        -- migration by name and runs it once, so editing an applied migration's
        -- SQL changes nothing on any database that already holds it (review F3).
        -- The mirror of `amount_minor`'s rule is therefore a from-scratch
        -- guarantee here and a live one on Postgres, where `m20260802_000066`
        -- adds the named constraint directly. Closing the gap on an existing
        -- SQLite database needs its own forward migration - a whole-table rebuild
        -- restating the column set as of `m20260802_000089` - and that is a named
        -- debt (D-349) rather than something this text can do.
        CONSTRAINT chk_pricing_price_unit_rate_nano CHECK (
            unit_rate_nano IS NULL OR unit_rate_nano >= 0),
        CONSTRAINT chk_pricing_price_max_hold_granules CHECK (
            max_hold_granules IS NULL OR max_hold_granules >= 1),
        CONSTRAINT chk_pricing_price_quantity_source CHECK (
            quantity_source IS NULL
            OR quantity_source IN ('subscription_seat_count','manual')),
        CONSTRAINT chk_pricing_price_manual_quantity CHECK (
            manual_quantity IS NULL OR manual_quantity >= 0),
        CONSTRAINT chk_pricing_price_package_size CHECK (
            package_size IS NULL OR package_size > 0),
        CONSTRAINT chk_pricing_price_package_price CHECK (
            package_price_minor IS NULL OR package_price_minor >= 0),
        CONSTRAINT chk_pricing_price_billing_granularity CHECK (
            billing_granularity IS NULL
            OR billing_granularity IN (
                'per_second','per_minute','per_hour','per_day','whole_unit')),
        CONSTRAINT chk_pricing_price_aggregation_function CHECK (
            aggregation_function IS NULL
            OR aggregation_function IN ('sum','peak','time_weighted')),
        CONSTRAINT chk_pricing_price_aggregation_granularity CHECK (
            aggregation_granularity IS NULL
            OR aggregation_granularity IN ('hour','day')),
        CONSTRAINT chk_pricing_price_tier_aggregation_window CHECK (
            tier_aggregation_window IS NULL
            OR tier_aggregation_window IN (
                'calendar_month','invoice_period','subscription_lifetime','per_event','per_hour')),
        CONSTRAINT chk_pricing_price_tier_qualification_window CHECK (
            tier_qualification_window IS NULL
            OR tier_qualification_window IN ('current','trailing_period')),
        CONSTRAINT chk_pricing_price_package_fields_kind CHECK (
            (package_size IS NULL AND package_price_minor IS NULL)
            OR (model_kind IS NOT NULL AND model_kind = 'package')),
        CONSTRAINT chk_pricing_price_cohort_eligibility CHECK (
            (cohort <> 'none') = (price_eligibility = 'existing_grandfathered')),
        CONSTRAINT chk_pricing_price_grandfather_until CHECK (
            grandfather_until IS NULL OR price_eligibility = 'existing_grandfathered')
    )",
    "INSERT INTO pricing_price_rebuilt (
         price_id,
         tenant_id,
         plan_id,
         currency,
         region,
         price_overlay,
         phase,
         price_eligibility,
         charge_kind,
         cohort,
         amount_minor,
         model_kind,
         tax_inclusive,
         billing_timing,
         quantity_source,
         manual_quantity,
         package_size,
         package_price_minor,
         meter,
         dimension_key,
         billing_granularity,
         aggregation_function,
         aggregation_granularity,
         tier_aggregation_window,
         tier_qualification_window,
         max_hold_granules,
         included_allowance,
         rounding_policy_ref,
         grandfather_until,
         supersedes_price_id,
         lifecycle_state,
         created_by,
         created_at_utc,
         row_version,
         tax_category_ref,
         resolved_tax_category,
         billing_anchor_policy,
         anchor_day,
         proration_basis,
         credit_on_downgrade,
         reserved_rate_minor,
         reservation_flavor,
         min_qty_purchase,
         min_qty_usage,
         min_qty_usage_fallback,
         discount_ref,
         unit_rate_nano)
       SELECT
         price_id,
         tenant_id,
         plan_id,
         currency,
         region,
         price_overlay,
         phase,
         price_eligibility,
         charge_kind,
         cohort,
         amount_minor,
         model_kind,
         tax_inclusive,
         billing_timing,
         quantity_source,
         manual_quantity,
         package_size,
         package_price_minor,
         meter,
         dimension_key,
         billing_granularity,
         aggregation_function,
         aggregation_granularity,
         tier_aggregation_window,
         tier_qualification_window,
         max_hold_granules,
         included_allowance,
         rounding_policy_ref,
         grandfather_until,
         supersedes_price_id,
         lifecycle_state,
         created_by,
         created_at_utc,
         row_version,
         tax_category_ref,
         resolved_tax_category,
         billing_anchor_policy,
         anchor_day,
         proration_basis,
         credit_on_downgrade,
         reserved_rate_minor,
         reservation_flavor,
         min_qty_purchase,
         min_qty_usage,
         min_qty_usage_fallback,
         discount_ref,
         unit_rate_nano
       FROM pricing_price",
    "DROP TABLE pricing_price",
    "ALTER TABLE pricing_price_rebuilt RENAME TO pricing_price",
];

/// The five indexes the DROP took with it.
const RECREATE_INDEXES: &[&str] = &[
    "CREATE INDEX idx_pricing_price_plan
        ON pricing_price (tenant_id, plan_id, lifecycle_state)",
    "CREATE INDEX idx_pricing_price_supersedes
        ON pricing_price (tenant_id, supersedes_price_id)
        WHERE supersedes_price_id IS NOT NULL",
    "CREATE UNIQUE INDEX uq_pricing_price_meter_line_current
        ON pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, cohort, meter, dimension_key)
        WHERE lifecycle_state = 'published' AND meter IS NOT NULL",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current
        ON pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort,
            COALESCE(meter, ''), dimension_key)
        WHERE lifecycle_state = 'published'",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_draft
        ON pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort,
            COALESCE(meter, ''), dimension_key)
        WHERE lifecycle_state = 'draft'",
];

/// This table's own triggers.
const RECREATE_OWN_TRIGGERS: &[&str] = &[
            "CREATE TRIGGER trg_pricing_price_draft_flip_whitelist
        BEFORE UPDATE ON pricing_price
        FOR EACH ROW WHEN OLD.lifecycle_state = 'draft'
          AND NEW.lifecycle_state NOT IN ('draft','published')
        BEGIN
          SELECT RAISE(ABORT, 'pricing_price: lifecycle_state transition is not sanctioned');
        END",
            "CREATE TRIGGER trg_pricing_price_flip_whitelist
        BEFORE UPDATE ON pricing_price
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND NEW.lifecycle_state IS NOT OLD.lifecycle_state
          AND NOT (OLD.lifecycle_state = 'published'
                   AND NEW.lifecycle_state = 'superseded')
        BEGIN
          SELECT RAISE(ABORT, 'pricing_price: lifecycle_state transition is not sanctioned');
        END",
            "CREATE TRIGGER trg_pricing_price_frozen_columns
        BEFORE UPDATE ON pricing_price
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND (NEW.price_id                  IS NOT OLD.price_id
            OR NEW.tenant_id                 IS NOT OLD.tenant_id
            OR NEW.plan_id                   IS NOT OLD.plan_id
            OR NEW.currency                  IS NOT OLD.currency
            OR NEW.region                    IS NOT OLD.region
            OR NEW.price_overlay             IS NOT OLD.price_overlay
            OR NEW.phase                     IS NOT OLD.phase
            OR NEW.price_eligibility         IS NOT OLD.price_eligibility
            OR NEW.charge_kind               IS NOT OLD.charge_kind
            OR NEW.cohort                    IS NOT OLD.cohort
            OR NEW.amount_minor              IS NOT OLD.amount_minor
            OR NEW.unit_rate_nano            IS NOT OLD.unit_rate_nano
            OR NEW.model_kind                IS NOT OLD.model_kind
            OR NEW.tax_inclusive             IS NOT OLD.tax_inclusive
            OR NEW.tax_category_ref          IS NOT OLD.tax_category_ref
            OR NEW.resolved_tax_category     IS NOT OLD.resolved_tax_category
            OR NEW.billing_timing            IS NOT OLD.billing_timing
            OR NEW.billing_anchor_policy     IS NOT OLD.billing_anchor_policy
            OR NEW.anchor_day                IS NOT OLD.anchor_day
            OR NEW.proration_basis           IS NOT OLD.proration_basis
            OR NEW.credit_on_downgrade       IS NOT OLD.credit_on_downgrade
            OR NEW.quantity_source           IS NOT OLD.quantity_source
            OR NEW.manual_quantity           IS NOT OLD.manual_quantity
            OR NEW.package_size              IS NOT OLD.package_size
            OR NEW.package_price_minor       IS NOT OLD.package_price_minor
            OR NEW.meter                     IS NOT OLD.meter
            OR NEW.dimension_key             IS NOT OLD.dimension_key
            OR NEW.billing_granularity       IS NOT OLD.billing_granularity
            OR NEW.aggregation_function      IS NOT OLD.aggregation_function
            OR NEW.aggregation_granularity   IS NOT OLD.aggregation_granularity
            OR NEW.tier_aggregation_window   IS NOT OLD.tier_aggregation_window
            OR NEW.tier_qualification_window IS NOT OLD.tier_qualification_window
            OR NEW.max_hold_granules         IS NOT OLD.max_hold_granules
            OR NEW.included_allowance        IS NOT OLD.included_allowance
            OR NEW.reserved_rate_minor       IS NOT OLD.reserved_rate_minor
            OR NEW.reservation_flavor        IS NOT OLD.reservation_flavor
            OR NEW.min_qty_purchase          IS NOT OLD.min_qty_purchase
            OR NEW.min_qty_usage             IS NOT OLD.min_qty_usage
            OR NEW.min_qty_usage_fallback    IS NOT OLD.min_qty_usage_fallback
            OR NEW.discount_ref              IS NOT OLD.discount_ref
            OR NEW.rounding_policy_ref       IS NOT OLD.rounding_policy_ref
            OR NEW.supersedes_price_id       IS NOT OLD.supersedes_price_id
            OR NEW.created_by                IS NOT OLD.created_by
            OR NEW.created_at_utc            IS NOT OLD.created_at_utc
            OR NEW.row_version               IS NOT OLD.row_version)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price: row is published; price, scope, model and entity-tag columns are immutable');
        END",
            "CREATE TRIGGER trg_pricing_price_grandfather_monotonic
        BEFORE UPDATE ON pricing_price
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND NEW.grandfather_until IS NOT OLD.grandfather_until
          AND (NEW.grandfather_until IS NULL
            OR (OLD.grandfather_until IS NOT NULL
                AND NEW.grandfather_until > OLD.grandfather_until))
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price: grandfather_until may only be tightened, never loosened');
        END",
            "CREATE TRIGGER trg_pricing_price_no_delete
        BEFORE DELETE ON pricing_price
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
        BEGIN
          SELECT RAISE(ABORT, 'pricing_price: DELETE of a non-draft row is not permitted');
        END",
            "CREATE TRIGGER trg_pricing_price_tier_band_parent_kind
        BEFORE UPDATE ON pricing_price
        FOR EACH ROW
        WHEN NEW.model_kind IS NULL OR NEW.model_kind NOT IN ('graduated','volume')
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_tier_band: a price row that still carries bands may not leave the graduated or volume kinds')
          WHERE EXISTS (
            SELECT 1 FROM pricing_price_tier_band WHERE price_id = OLD.price_id);
        END"
];

/// And the foreign ones, back.
const RECREATE_FOREIGN_TRIGGERS: &[&str] = &[
            "CREATE TRIGGER trg_pricing_price_tier_band_kind_insert
        BEFORE INSERT ON pricing_price_tier_band
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_tier_band: band rows are permitted only on a graduated or volume price row')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price
             WHERE price_id = NEW.price_id AND model_kind IN ('graduated','volume'));
        END",
            "CREATE TRIGGER trg_pricing_price_tier_band_kind_update
        BEFORE UPDATE ON pricing_price_tier_band
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_tier_band: band rows are permitted only on a graduated or volume price row')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price
             WHERE price_id = NEW.price_id AND model_kind IN ('graduated','volume'));
        END",
            "CREATE TRIGGER trg_pricing_price_tier_band_no_delete
        BEFORE DELETE ON pricing_price_tier_band
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_tier_band: DELETE of a band under a non-draft price row is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price
             WHERE price_id = OLD.price_id AND lifecycle_state = 'draft');
        END",
            "CREATE TRIGGER trg_pricing_price_tier_band_no_insert
        BEFORE INSERT ON pricing_price_tier_band
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_tier_band: INSERT of a band under a non-draft price row is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price
             WHERE price_id = NEW.price_id AND lifecycle_state = 'draft');
        END",
            "CREATE TRIGGER trg_pricing_price_tier_band_no_update
        BEFORE UPDATE ON pricing_price_tier_band
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_tier_band: UPDATE of a band under a non-draft price row is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price
             WHERE price_id = OLD.price_id AND lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_price
             WHERE price_id = NEW.price_id AND lifecycle_state = 'draft');
        END"
];

/// The rebuild, in the order the engine needs it: unhook what points at the
/// table, restate it, put back the five indexes, its six triggers and the five
/// foreign ones. Assembled rather than written as one array so each phase can be
/// read — and so no phase can be dropped without the count changing.
fn sqlite_rebuild() -> Vec<&'static str> {
    [
        DROP_FOREIGN_TRIGGERS,
        REBUILD_TABLE,
        RECREATE_INDEXES,
        RECREATE_OWN_TRIGGERS,
        RECREATE_FOREIGN_TRIGGERS,
    ]
    .concat()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, &[PG_DROP, PG_ADD], &sqlite_rebuild()).await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Irreversible by design: a stored `per_hour` row would violate the
        // narrower constraint, so narrowing back is a data question, not a schema
        // one. The gear's migrations are forward-only (see `migrations.rs`).
        Ok(())
    }
}
