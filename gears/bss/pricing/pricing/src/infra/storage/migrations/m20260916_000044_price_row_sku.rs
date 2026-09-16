//! D-372: every price row names the SKU it prices, and a plan always names its
//! own.
//!
//! Appended after the shipped chain (`m20260821_000001` through `m20260821_000043`
//! are on `main`); nothing before this file changes.
//!
//! Three moves, in this order and for a reason:
//!
//! 1. **`ADD`**: the column arrives nullable, and fee rows
//!    (`charge_kind <> 'usage'`) take their plan's `sku_id`, which is the one SKU
//!    fact the database holds.
//! 2. **[`refuse_unless_backfilled`]**: a usage row's SKU is a *registry* fact
//!    (which SKU declares `GB-hour`), not a database fact. Any row still NULL, or
//!    any plan without a SKU, stops the migration and is named in the error. The
//!    operator repairs the data and re-runs; the migration never guesses.
//! 3. **`TIGHTEN`**: `NOT NULL`, the scope-key indexes over `sku_id`, and the
//!    append-only guard restated with `sku_id` in its frozen list. The guard is
//!    restated *after* the backfill: step 1 updates published rows in a column the
//!    guard does not yet name, and restated first it would refuse them.
//!
//! # Why this migration insists on a transaction
//!
//! [`MigrationTrait::use_transaction`] is overridden to `Some(true)` rather than
//! left at the default, which is "a transaction on Postgres only". The refusal in
//! step 2 is the whole point of the file, and a refusal that leaves the nullable
//! column behind is not a refusal the operator can act on: the re-run they are
//! told to make would fail on `ADD COLUMN` instead, and `SQLite` has no
//! `ADD COLUMN IF NOT EXISTS` to soften it. Inside one transaction the refusal
//! rolls the `ADD` and the backfill back with it, so a re-run starts where the
//! first attempt did. The platform runner (`toolkit_db::migration_runner`) already
//! wraps every `up` in an explicit transaction on both engines; this override is
//! what makes `Migrator::up` -- the path the in-crate `SQLite` suites take -- agree
//! with it.
//!
//! `SQLite` physically rebuilds both tables and their foreign-key descendants,
//! preserving their data, indexes and triggers without disabling foreign keys.
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Step 1 on Postgres: the column, then the one backfill the database can justify.
///
/// Price rows have no plan revision. Only a SKU shared by every revision of
/// their own tenant's plan is safe to infer; an ambiguous history is refused.
const PG_ADD: &[&str] = &[
    "ALTER TABLE bss.pricing_price ADD COLUMN sku_id uuid",
    "UPDATE bss.pricing_price p SET sku_id = pl.sku_id
       FROM (SELECT tenant_id, plan_id, (array_agg(sku_id))[1] AS sku_id FROM bss.pricing_plan GROUP BY tenant_id, plan_id HAVING count(DISTINCT sku_id) = 1 AND count(sku_id) = count(*)) pl
      WHERE pl.tenant_id = p.tenant_id AND pl.plan_id = p.plan_id AND p.charge_kind <> 'usage' AND p.sku_id IS NULL AND pl.sku_id IS NOT NULL",
];

/// Step 1 on `SQLite`: the same two moves, in the dialect's own spelling.
///
/// The correlated subquery admits only an unambiguous tenant-scoped SKU.
const SQLITE_ADD: &[&str] = &[
    "ALTER TABLE pricing_price ADD COLUMN sku_id text",
    "UPDATE pricing_price SET sku_id = (SELECT min(pl.sku_id) FROM pricing_plan pl WHERE pl.tenant_id = pricing_price.tenant_id AND pl.plan_id = pricing_price.plan_id HAVING count(DISTINCT pl.sku_id) = 1 AND count(pl.sku_id) = count(*))
      WHERE charge_kind <> 'usage' AND sku_id IS NULL",
];

/// Step 3 on Postgres.
///
/// The guard function is `m20260821_000023:387-475` verbatim with exactly one line
/// added, `OR NEW.sku_id IS DISTINCT FROM OLD.sku_id`, immediately after the
/// `plan_id` clause: `postgres_migrations` reads the body back through
/// `pg_get_functiondef` and compares it against the frozen list, so every other
/// byte of it has to be the byte that shipped.
const PG_TIGHTEN: &[&str] = &[
    "ALTER TABLE bss.pricing_price ALTER COLUMN sku_id SET NOT NULL",
    "ALTER TABLE bss.pricing_plan ALTER COLUMN sku_id SET NOT NULL",
    // D-196's third partial UNIQUE keyed the usage line as a market of its own.
    // D-372 deletes the premise: two units of one SKU are one key, so the index
    // that made them two goes rather than being re-keyed.
    "DROP INDEX bss.uq_pricing_price_meter_line_current",
    "DROP INDEX bss.uq_pricing_price_scope_key_current",
    "DROP INDEX bss.uq_pricing_price_scope_key_draft",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current ON bss.pricing_price USING btree (tenant_id, plan_id, sku_id, currency, region, price_overlay, phase, price_eligibility, charge_kind, cohort, dimension_key) WHERE (lifecycle_state = 'published'::text)",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_draft ON bss.pricing_price USING btree (tenant_id, plan_id, sku_id, currency, region, price_overlay, phase, price_eligibility, charge_kind, cohort, dimension_key) WHERE (lifecycle_state = 'draft'::text)",
    "CREATE OR REPLACE FUNCTION bss.pricing_price_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            IF OLD.lifecycle_state <> 'draft' THEN
              RAISE EXCEPTION 'pricing_price: DELETE of a % row is not permitted',
                OLD.lifecycle_state;
            END IF;
            RETURN OLD;
          END IF;

          IF OLD.lifecycle_state = 'draft' THEN
            IF NEW.lifecycle_state NOT IN ('draft', 'published') THEN
              RAISE EXCEPTION
                'pricing_price: lifecycle_state draft -> % is not a sanctioned transition',
                NEW.lifecycle_state;
            END IF;
            RETURN NEW;
          END IF;

          IF NEW.price_id                  IS DISTINCT FROM OLD.price_id
          OR NEW.tenant_id                 IS DISTINCT FROM OLD.tenant_id
          OR NEW.plan_id                   IS DISTINCT FROM OLD.plan_id
          OR NEW.sku_id                    IS DISTINCT FROM OLD.sku_id
          OR NEW.currency                  IS DISTINCT FROM OLD.currency
          OR NEW.region                    IS DISTINCT FROM OLD.region
          OR NEW.price_overlay             IS DISTINCT FROM OLD.price_overlay
          OR NEW.phase                     IS DISTINCT FROM OLD.phase
          OR NEW.price_eligibility         IS DISTINCT FROM OLD.price_eligibility
          OR NEW.charge_kind               IS DISTINCT FROM OLD.charge_kind
          OR NEW.cohort                    IS DISTINCT FROM OLD.cohort
          OR NEW.amount_minor              IS DISTINCT FROM OLD.amount_minor
          OR NEW.unit_rate_nano            IS DISTINCT FROM OLD.unit_rate_nano
          OR NEW.model_kind                IS DISTINCT FROM OLD.model_kind
          OR NEW.tax_inclusive             IS DISTINCT FROM OLD.tax_inclusive
          OR NEW.tax_category_ref          IS DISTINCT FROM OLD.tax_category_ref
          OR NEW.resolved_tax_category     IS DISTINCT FROM OLD.resolved_tax_category
          OR NEW.resolved_rounding_policy  IS DISTINCT FROM OLD.resolved_rounding_policy
          OR NEW.billing_timing            IS DISTINCT FROM OLD.billing_timing
          OR NEW.billing_anchor_policy     IS DISTINCT FROM OLD.billing_anchor_policy
          OR NEW.anchor_day                IS DISTINCT FROM OLD.anchor_day
          OR NEW.proration_basis           IS DISTINCT FROM OLD.proration_basis
          OR NEW.credit_on_downgrade       IS DISTINCT FROM OLD.credit_on_downgrade
          OR NEW.quantity_source           IS DISTINCT FROM OLD.quantity_source
          OR NEW.manual_quantity           IS DISTINCT FROM OLD.manual_quantity
          OR NEW.package_size              IS DISTINCT FROM OLD.package_size
          OR NEW.package_price_minor       IS DISTINCT FROM OLD.package_price_minor
          OR NEW.meter                     IS DISTINCT FROM OLD.meter
          OR NEW.dimension_key             IS DISTINCT FROM OLD.dimension_key
          OR NEW.billing_granularity       IS DISTINCT FROM OLD.billing_granularity
          OR NEW.aggregation_function      IS DISTINCT FROM OLD.aggregation_function
          OR NEW.aggregation_granularity   IS DISTINCT FROM OLD.aggregation_granularity
          OR NEW.tier_aggregation_window   IS DISTINCT FROM OLD.tier_aggregation_window
          OR NEW.tier_qualification_window IS DISTINCT FROM OLD.tier_qualification_window
          OR NEW.max_hold_granules         IS DISTINCT FROM OLD.max_hold_granules
          OR NEW.included_allowance        IS DISTINCT FROM OLD.included_allowance
          OR NEW.reserved_rate_nano       IS DISTINCT FROM OLD.reserved_rate_nano
          OR NEW.reservation_flavor        IS DISTINCT FROM OLD.reservation_flavor
          OR NEW.min_qty_purchase          IS DISTINCT FROM OLD.min_qty_purchase
          OR NEW.min_qty_usage             IS DISTINCT FROM OLD.min_qty_usage
          OR NEW.min_qty_usage_fallback    IS DISTINCT FROM OLD.min_qty_usage_fallback
          OR NEW.discount_ref              IS DISTINCT FROM OLD.discount_ref
          OR NEW.rounding_policy_ref       IS DISTINCT FROM OLD.rounding_policy_ref
          OR NEW.supersedes_price_id       IS DISTINCT FROM OLD.supersedes_price_id
          OR NEW.created_by                IS DISTINCT FROM OLD.created_by
          OR NEW.created_at_utc            IS DISTINCT FROM OLD.created_at_utc
          OR NEW.row_version               IS DISTINCT FROM OLD.row_version THEN
            RAISE EXCEPTION
              'pricing_price: row % is published; price, scope, model and entity-tag columns are immutable',
              OLD.price_id;
          END IF;

          IF NEW.lifecycle_state IS DISTINCT FROM OLD.lifecycle_state
             AND NOT (OLD.lifecycle_state = 'published'
                      AND NEW.lifecycle_state = 'superseded') THEN
            RAISE EXCEPTION 'pricing_price: lifecycle_state % -> % is not a sanctioned transition',
              OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          IF NEW.grandfather_until IS DISTINCT FROM OLD.grandfather_until
             AND (NEW.grandfather_until IS NULL
                  OR (OLD.grandfather_until IS NOT NULL
                      AND NEW.grandfather_until > OLD.grandfather_until)) THEN
            RAISE EXCEPTION
              'pricing_price: grandfather_until may only be tightened, never loosened (row %)',
              OLD.price_id;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    // The line's SKU becomes a uuid, so the check that a text SKU was not blank
    // has nothing left to refuse: a uuid column admits no blank at all. The
    // sibling `chk_..._sku_needs_plan` survives the re-type untouched.
    "ALTER TABLE bss.pricing_price_overlay_line DROP CONSTRAINT chk_pricing_price_overlay_line_target_sku_present",
    "DROP INDEX bss.uq_pricing_price_overlay_line_key",
    "ALTER TABLE bss.pricing_price_overlay_line ALTER COLUMN target_sku TYPE uuid USING target_sku::uuid",
    "ALTER TABLE bss.pricing_price_overlay_line ADD CONSTRAINT chk_pricing_price_overlay_line_target_sku_not_nil CHECK (target_sku IS NULL OR target_sku <> '00000000-0000-0000-0000-000000000000'::uuid)",
    "CREATE UNIQUE INDEX uq_pricing_price_overlay_line_key ON bss.pricing_price_overlay_line USING btree (price_overlay_id, overlay_revision, COALESCE(plan_id, '00000000-0000-0000-0000-000000000000'::uuid), COALESCE(target_sku, '00000000-0000-0000-0000-000000000000'::uuid), COALESCE(cohort, '-infinity'::timestamp with time zone))",
];

/// Step 3 on `SQLite`.
///
/// Physical NOT NULL constraints are installed by the rebuild before these
/// indexes and the SKU-aware frozen-column guard replace their shipped forms.
///
/// The UUID driver binds `target_sku` as a 16-byte blob on `SQLite`; the
/// rebuild normalizes legacy UUID text to that representation before restore.
const SQLITE_TIGHTEN: &[&str] = &[
    "DROP INDEX uq_pricing_price_meter_line_current",
    "DROP INDEX uq_pricing_price_scope_key_current",
    "DROP INDEX uq_pricing_price_scope_key_draft",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current ON pricing_price (tenant_id, plan_id, sku_id, currency, region, price_overlay, phase, price_eligibility, charge_kind, cohort, dimension_key) WHERE lifecycle_state = 'published'",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_draft ON pricing_price (tenant_id, plan_id, sku_id, currency, region, price_overlay, phase, price_eligibility, charge_kind, cohort, dimension_key) WHERE lifecycle_state = 'draft'",
    "DROP TRIGGER trg_pricing_price_frozen_columns",
    "CREATE TRIGGER trg_pricing_price_frozen_columns BEFORE UPDATE ON pricing_price FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' AND (NEW.price_id IS NOT OLD.price_id OR NEW.tenant_id IS NOT OLD.tenant_id OR NEW.plan_id IS NOT OLD.plan_id OR NEW.sku_id IS NOT OLD.sku_id OR NEW.currency IS NOT OLD.currency OR NEW.region IS NOT OLD.region OR NEW.price_overlay IS NOT OLD.price_overlay OR NEW.phase IS NOT OLD.phase OR NEW.price_eligibility IS NOT OLD.price_eligibility OR NEW.charge_kind IS NOT OLD.charge_kind OR NEW.cohort IS NOT OLD.cohort OR NEW.amount_minor IS NOT OLD.amount_minor OR NEW.unit_rate_nano IS NOT OLD.unit_rate_nano OR NEW.model_kind IS NOT OLD.model_kind OR NEW.tax_inclusive IS NOT OLD.tax_inclusive OR NEW.tax_category_ref IS NOT OLD.tax_category_ref OR NEW.resolved_tax_category IS NOT OLD.resolved_tax_category OR NEW.resolved_rounding_policy IS NOT OLD.resolved_rounding_policy OR NEW.billing_timing IS NOT OLD.billing_timing OR NEW.billing_anchor_policy IS NOT OLD.billing_anchor_policy OR NEW.anchor_day IS NOT OLD.anchor_day OR NEW.proration_basis IS NOT OLD.proration_basis OR NEW.credit_on_downgrade IS NOT OLD.credit_on_downgrade OR NEW.quantity_source IS NOT OLD.quantity_source OR NEW.manual_quantity IS NOT OLD.manual_quantity OR NEW.package_size IS NOT OLD.package_size OR NEW.package_price_minor IS NOT OLD.package_price_minor OR NEW.meter IS NOT OLD.meter OR NEW.dimension_key IS NOT OLD.dimension_key OR NEW.billing_granularity IS NOT OLD.billing_granularity OR NEW.aggregation_function IS NOT OLD.aggregation_function OR NEW.aggregation_granularity IS NOT OLD.aggregation_granularity OR NEW.tier_aggregation_window IS NOT OLD.tier_aggregation_window OR NEW.tier_qualification_window IS NOT OLD.tier_qualification_window OR NEW.max_hold_granules IS NOT OLD.max_hold_granules OR NEW.included_allowance IS NOT OLD.included_allowance OR NEW.reserved_rate_nano IS NOT OLD.reserved_rate_nano OR NEW.reservation_flavor IS NOT OLD.reservation_flavor OR NEW.min_qty_purchase IS NOT OLD.min_qty_purchase OR NEW.min_qty_usage IS NOT OLD.min_qty_usage OR NEW.min_qty_usage_fallback IS NOT OLD.min_qty_usage_fallback OR NEW.discount_ref IS NOT OLD.discount_ref OR NEW.rounding_policy_ref IS NOT OLD.rounding_policy_ref OR NEW.supersedes_price_id IS NOT OLD.supersedes_price_id OR NEW.created_by IS NOT OLD.created_by OR NEW.created_at_utc IS NOT OLD.created_at_utc OR NEW.row_version IS NOT OLD.row_version) BEGIN SELECT RAISE(ABORT, 'pricing_price: row is published; price, scope, model and entity-tag columns are immutable'); END",
];

/// The rollback on Postgres: every statement of [`PG_TIGHTEN`] inverted, then the
/// column itself.
///
/// The three indexes and the guard function are `m20260821_000023:384-386` and
/// `:387-475` verbatim -- the shipped text, not a re-derivation of it -- so a
/// rollback leaves the schema the shipped chain produced.
const PG_DOWN: &[&str] = &[
    "DROP INDEX bss.uq_pricing_price_scope_key_current",
    "DROP INDEX bss.uq_pricing_price_scope_key_draft",
    "CREATE UNIQUE INDEX uq_pricing_price_meter_line_current ON bss.pricing_price USING btree (tenant_id, plan_id, currency, region, price_overlay, phase, price_eligibility, cohort, meter, dimension_key) WHERE ((lifecycle_state = 'published'::text) AND (meter IS NOT NULL))",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current ON bss.pricing_price USING btree (tenant_id, plan_id, currency, region, price_overlay, phase, price_eligibility, charge_kind, cohort, COALESCE(meter, ''::text), dimension_key) WHERE (lifecycle_state = 'published'::text)",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_draft ON bss.pricing_price USING btree (tenant_id, plan_id, currency, region, price_overlay, phase, price_eligibility, charge_kind, cohort, COALESCE(meter, ''::text), dimension_key) WHERE (lifecycle_state = 'draft'::text)",
    "CREATE OR REPLACE FUNCTION bss.pricing_price_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            IF OLD.lifecycle_state <> 'draft' THEN
              RAISE EXCEPTION 'pricing_price: DELETE of a % row is not permitted',
                OLD.lifecycle_state;
            END IF;
            RETURN OLD;
          END IF;

          IF OLD.lifecycle_state = 'draft' THEN
            IF NEW.lifecycle_state NOT IN ('draft', 'published') THEN
              RAISE EXCEPTION
                'pricing_price: lifecycle_state draft -> % is not a sanctioned transition',
                NEW.lifecycle_state;
            END IF;
            RETURN NEW;
          END IF;

          IF NEW.price_id                  IS DISTINCT FROM OLD.price_id
          OR NEW.tenant_id                 IS DISTINCT FROM OLD.tenant_id
          OR NEW.plan_id                   IS DISTINCT FROM OLD.plan_id
          OR NEW.currency                  IS DISTINCT FROM OLD.currency
          OR NEW.region                    IS DISTINCT FROM OLD.region
          OR NEW.price_overlay             IS DISTINCT FROM OLD.price_overlay
          OR NEW.phase                     IS DISTINCT FROM OLD.phase
          OR NEW.price_eligibility         IS DISTINCT FROM OLD.price_eligibility
          OR NEW.charge_kind               IS DISTINCT FROM OLD.charge_kind
          OR NEW.cohort                    IS DISTINCT FROM OLD.cohort
          OR NEW.amount_minor              IS DISTINCT FROM OLD.amount_minor
          OR NEW.unit_rate_nano            IS DISTINCT FROM OLD.unit_rate_nano
          OR NEW.model_kind                IS DISTINCT FROM OLD.model_kind
          OR NEW.tax_inclusive             IS DISTINCT FROM OLD.tax_inclusive
          OR NEW.tax_category_ref          IS DISTINCT FROM OLD.tax_category_ref
          OR NEW.resolved_tax_category     IS DISTINCT FROM OLD.resolved_tax_category
          OR NEW.resolved_rounding_policy  IS DISTINCT FROM OLD.resolved_rounding_policy
          OR NEW.billing_timing            IS DISTINCT FROM OLD.billing_timing
          OR NEW.billing_anchor_policy     IS DISTINCT FROM OLD.billing_anchor_policy
          OR NEW.anchor_day                IS DISTINCT FROM OLD.anchor_day
          OR NEW.proration_basis           IS DISTINCT FROM OLD.proration_basis
          OR NEW.credit_on_downgrade       IS DISTINCT FROM OLD.credit_on_downgrade
          OR NEW.quantity_source           IS DISTINCT FROM OLD.quantity_source
          OR NEW.manual_quantity           IS DISTINCT FROM OLD.manual_quantity
          OR NEW.package_size              IS DISTINCT FROM OLD.package_size
          OR NEW.package_price_minor       IS DISTINCT FROM OLD.package_price_minor
          OR NEW.meter                     IS DISTINCT FROM OLD.meter
          OR NEW.dimension_key             IS DISTINCT FROM OLD.dimension_key
          OR NEW.billing_granularity       IS DISTINCT FROM OLD.billing_granularity
          OR NEW.aggregation_function      IS DISTINCT FROM OLD.aggregation_function
          OR NEW.aggregation_granularity   IS DISTINCT FROM OLD.aggregation_granularity
          OR NEW.tier_aggregation_window   IS DISTINCT FROM OLD.tier_aggregation_window
          OR NEW.tier_qualification_window IS DISTINCT FROM OLD.tier_qualification_window
          OR NEW.max_hold_granules         IS DISTINCT FROM OLD.max_hold_granules
          OR NEW.included_allowance        IS DISTINCT FROM OLD.included_allowance
          OR NEW.reserved_rate_nano       IS DISTINCT FROM OLD.reserved_rate_nano
          OR NEW.reservation_flavor        IS DISTINCT FROM OLD.reservation_flavor
          OR NEW.min_qty_purchase          IS DISTINCT FROM OLD.min_qty_purchase
          OR NEW.min_qty_usage             IS DISTINCT FROM OLD.min_qty_usage
          OR NEW.min_qty_usage_fallback    IS DISTINCT FROM OLD.min_qty_usage_fallback
          OR NEW.discount_ref              IS DISTINCT FROM OLD.discount_ref
          OR NEW.rounding_policy_ref       IS DISTINCT FROM OLD.rounding_policy_ref
          OR NEW.supersedes_price_id       IS DISTINCT FROM OLD.supersedes_price_id
          OR NEW.created_by                IS DISTINCT FROM OLD.created_by
          OR NEW.created_at_utc            IS DISTINCT FROM OLD.created_at_utc
          OR NEW.row_version               IS DISTINCT FROM OLD.row_version THEN
            RAISE EXCEPTION
              'pricing_price: row % is published; price, scope, model and entity-tag columns are immutable',
              OLD.price_id;
          END IF;

          IF NEW.lifecycle_state IS DISTINCT FROM OLD.lifecycle_state
             AND NOT (OLD.lifecycle_state = 'published'
                      AND NEW.lifecycle_state = 'superseded') THEN
            RAISE EXCEPTION 'pricing_price: lifecycle_state % -> % is not a sanctioned transition',
              OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          IF NEW.grandfather_until IS DISTINCT FROM OLD.grandfather_until
             AND (NEW.grandfather_until IS NULL
                  OR (OLD.grandfather_until IS NOT NULL
                      AND NEW.grandfather_until > OLD.grandfather_until)) THEN
            RAISE EXCEPTION
              'pricing_price: grandfather_until may only be tightened, never loosened (row %)',
              OLD.price_id;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "ALTER TABLE bss.pricing_plan ALTER COLUMN sku_id DROP NOT NULL",
    "ALTER TABLE bss.pricing_price DROP COLUMN sku_id",
    "DROP INDEX bss.uq_pricing_price_overlay_line_key",
    "ALTER TABLE bss.pricing_price_overlay_line DROP CONSTRAINT chk_pricing_price_overlay_line_target_sku_not_nil",
    "ALTER TABLE bss.pricing_price_overlay_line ALTER COLUMN target_sku TYPE text USING target_sku::text",
    "ALTER TABLE bss.pricing_price_overlay_line ADD CONSTRAINT chk_pricing_price_overlay_line_target_sku_present CHECK (target_sku IS NULL OR length(btrim(target_sku, chr(9) || chr(10) || chr(11) || chr(12) || chr(13) || chr(32))) > 0)",
    "CREATE UNIQUE INDEX uq_pricing_price_overlay_line_key ON bss.pricing_price_overlay_line USING btree (price_overlay_id, overlay_revision, COALESCE(plan_id, '00000000-0000-0000-0000-000000000000'::uuid), COALESCE(target_sku, ''::text), COALESCE(cohort, '-infinity'::timestamp with time zone))",
];

/// The rollback on `SQLite`.
///
/// Remove SKU-dependent objects before dropping the column, then rebuild the
/// plan and its descendants to restore the nullable plan SKU declaration.
const SQLITE_DOWN: &[&str] = &[
    "DROP TRIGGER trg_pricing_price_frozen_columns",
    "CREATE TRIGGER trg_pricing_price_frozen_columns BEFORE UPDATE ON pricing_price FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' AND (NEW.price_id IS NOT OLD.price_id OR NEW.tenant_id IS NOT OLD.tenant_id OR NEW.plan_id IS NOT OLD.plan_id OR NEW.currency IS NOT OLD.currency OR NEW.region IS NOT OLD.region OR NEW.price_overlay IS NOT OLD.price_overlay OR NEW.phase IS NOT OLD.phase OR NEW.price_eligibility IS NOT OLD.price_eligibility OR NEW.charge_kind IS NOT OLD.charge_kind OR NEW.cohort IS NOT OLD.cohort OR NEW.amount_minor IS NOT OLD.amount_minor OR NEW.unit_rate_nano IS NOT OLD.unit_rate_nano OR NEW.model_kind IS NOT OLD.model_kind OR NEW.tax_inclusive IS NOT OLD.tax_inclusive OR NEW.tax_category_ref IS NOT OLD.tax_category_ref OR NEW.resolved_tax_category IS NOT OLD.resolved_tax_category OR NEW.resolved_rounding_policy IS NOT OLD.resolved_rounding_policy OR NEW.billing_timing IS NOT OLD.billing_timing OR NEW.billing_anchor_policy IS NOT OLD.billing_anchor_policy OR NEW.anchor_day IS NOT OLD.anchor_day OR NEW.proration_basis IS NOT OLD.proration_basis OR NEW.credit_on_downgrade IS NOT OLD.credit_on_downgrade OR NEW.quantity_source IS NOT OLD.quantity_source OR NEW.manual_quantity IS NOT OLD.manual_quantity OR NEW.package_size IS NOT OLD.package_size OR NEW.package_price_minor IS NOT OLD.package_price_minor OR NEW.meter IS NOT OLD.meter OR NEW.dimension_key IS NOT OLD.dimension_key OR NEW.billing_granularity IS NOT OLD.billing_granularity OR NEW.aggregation_function IS NOT OLD.aggregation_function OR NEW.aggregation_granularity IS NOT OLD.aggregation_granularity OR NEW.tier_aggregation_window IS NOT OLD.tier_aggregation_window OR NEW.tier_qualification_window IS NOT OLD.tier_qualification_window OR NEW.max_hold_granules IS NOT OLD.max_hold_granules OR NEW.included_allowance IS NOT OLD.included_allowance OR NEW.reserved_rate_nano IS NOT OLD.reserved_rate_nano OR NEW.reservation_flavor IS NOT OLD.reservation_flavor OR NEW.min_qty_purchase IS NOT OLD.min_qty_purchase OR NEW.min_qty_usage IS NOT OLD.min_qty_usage OR NEW.min_qty_usage_fallback IS NOT OLD.min_qty_usage_fallback OR NEW.discount_ref IS NOT OLD.discount_ref OR NEW.rounding_policy_ref IS NOT OLD.rounding_policy_ref OR NEW.supersedes_price_id IS NOT OLD.supersedes_price_id OR NEW.created_by IS NOT OLD.created_by OR NEW.created_at_utc IS NOT OLD.created_at_utc OR NEW.row_version IS NOT OLD.row_version) BEGIN SELECT RAISE(ABORT, 'pricing_price: row is published; price, scope, model and entity-tag columns are immutable'); END",
    "DROP INDEX uq_pricing_price_scope_key_current",
    "DROP INDEX uq_pricing_price_scope_key_draft",
    "CREATE UNIQUE INDEX uq_pricing_price_meter_line_current ON pricing_price (tenant_id, plan_id, currency, region, price_overlay, phase, price_eligibility, cohort, meter, dimension_key) WHERE lifecycle_state = 'published' AND meter IS NOT NULL",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current ON pricing_price (tenant_id, plan_id, currency, region, price_overlay, phase, price_eligibility, charge_kind, cohort, COALESCE(meter, ''), dimension_key) WHERE lifecycle_state = 'published'",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_draft ON pricing_price (tenant_id, plan_id, currency, region, price_overlay, phase, price_eligibility, charge_kind, cohort, COALESCE(meter, ''), dimension_key) WHERE lifecycle_state = 'draft'",
    "ALTER TABLE pricing_price DROP COLUMN sku_id",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// See the module doc: the refusal below is only actionable if it takes the
    /// `ADD` back with it, and the default is a transaction on Postgres alone.
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_ADD, SQLITE_ADD).await?;
        refuse_unless_backfilled(manager).await?;
        if manager.get_database_backend() == DbBackend::Sqlite {
            rebuild_sqlite_sku_tables(manager, true).await?;
        }
        super::exec_backend(self.name(), manager, PG_TIGHTEN, SQLITE_TIGHTEN).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_DOWN, SQLITE_DOWN).await?;
        if manager.get_database_backend() == DbBackend::Sqlite {
            rebuild_sqlite_sku_tables(manager, false).await?;
        }
        Ok(())
    }
}

/// The one place this migration can stop.
///
/// It reads **before** it tightens, so the message names rows while they are still
/// reachable: after `SET NOT NULL` the statement that would have found them is the
/// statement that failed, and its error names a constraint rather than a row.
///
/// # Errors
/// [`DbErr::Custom`] naming the first twenty price rows with no `sku_id` and the
/// number of plans with none, when either is non-empty.
async fn refuse_unless_backfilled(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let db = manager.get_connection();
    let backend = manager.get_database_backend();
    let (rows_sql, plans_sql) = match backend {
        DbBackend::Postgres => (
            "SELECT price_id::text AS price_id, plan_id::text AS plan_id, charge_kind, COALESCE(meter, '') AS meter
               FROM bss.pricing_price WHERE sku_id IS NULL ORDER BY plan_id, price_id LIMIT 20",
            "SELECT count(*)::bigint AS n FROM bss.pricing_plan WHERE sku_id IS NULL",
        ),
        _ => (
            "SELECT price_id, plan_id, charge_kind, COALESCE(meter, '') AS meter
               FROM pricing_price WHERE sku_id IS NULL ORDER BY plan_id, price_id LIMIT 20",
            "SELECT count(*) AS n FROM pricing_plan WHERE sku_id IS NULL",
        ),
    };
    let rows = db
        .query_all_raw(Statement::from_string(backend, rows_sql))
        .await?;
    let plans = db
        .query_one_raw(Statement::from_string(backend, plans_sql))
        .await?;
    let plans_without_sku: i64 = plans.map(|r| r.try_get("", "n")).transpose()?.unwrap_or(0);
    if rows.is_empty() && plans_without_sku == 0 {
        return Ok(());
    }
    let named: Vec<String> = rows
        .iter()
        .map(|r| {
            let price: String = r.try_get("", "price_id").unwrap_or_default();
            let plan: String = r.try_get("", "plan_id").unwrap_or_default();
            let kind: String = r.try_get("", "charge_kind").unwrap_or_default();
            let meter: String = r.try_get("", "meter").unwrap_or_default();
            format!("{price} (plan {plan}, {kind}, meter {meter:?})")
        })
        .collect();
    Err(DbErr::Custom(format!(
        "D-372 backfill incomplete, refusing to tighten. plans without sku_id: \
         {plans_without_sku}; price rows without sku_id (first 20): [{}]. A usage row's SKU is a \
         registry fact: set it by hand (or clear the demo data) and re-run.",
        named.join(", ")
    )))
}

/// Rebuild parents together with their complete inbound foreign-key closure.
/// Child-first drops and parent-first restores keep `foreign_keys` enabled even
/// inside the platform runner's transaction. Temporary copies have no foreign
/// keys or triggers, so neither CASCADE nor append-only guards can erase data.
/// All original DDL is replayed; only the two SKU declarations change.
#[allow(
    clippy::cognitive_complexity,
    reason = "ordered FK closure rebuild preserves each dependent table in one transaction"
)]
async fn rebuild_sqlite_sku_tables(
    manager: &SchemaManager<'_>,
    required: bool,
) -> Result<(), DbErr> {
    use std::collections::{BTreeMap, BTreeSet};

    let db = manager.get_connection();
    let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
    let rows = db.query_all_raw(Statement::from_string(DbBackend::Sqlite,
        "SELECT name, sql FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")) .await?;
    let mut schemas = BTreeMap::new();
    let mut parents = BTreeMap::new();
    for row in rows {
        let name: String = row.try_get("", "name")?;
        let sql: String = row.try_get("", "sql")?;
        let fks = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("PRAGMA foreign_key_list({})", quote(&name)),
            ))
            .await?;
        let refs = fks
            .iter()
            .map(|fk| fk.try_get::<String>("", "table"))
            .collect::<Result<BTreeSet<_>, _>>()?;
        parents.insert(name.clone(), refs);
        schemas.insert(name, sql);
    }
    let mut selected = BTreeSet::from([
        "pricing_plan".to_owned(),
        "pricing_price".to_owned(),
        "pricing_price_overlay_line".to_owned(),
    ]);
    loop {
        let before = selected.len();
        for (table, refs) in &parents {
            if !refs.is_disjoint(&selected) {
                selected.insert(table.clone());
            }
        }
        if before == selected.len() {
            break;
        }
    }
    let mut ordered = Vec::new();
    let mut remaining = selected.clone();
    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .find(|table| parents[*table].is_disjoint(&remaining))
            .cloned()
            .ok_or_else(|| {
                DbErr::Custom("D-372: cannot rebuild cyclic SQLite foreign keys".into())
            })?;
        remaining.remove(&next);
        ordered.push(next);
    }
    let objects = db.query_all_raw(Statement::from_string(DbBackend::Sqlite,
        "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE type IN ('index', 'trigger') AND sql IS NOT NULL ORDER BY type, name")).await?;
    let mut indexes = Vec::new();
    let mut triggers = Vec::new();
    for row in objects {
        let table: String = row.try_get("", "tbl_name")?;
        if !selected.contains(&table) {
            continue;
        }
        let sql: String = row.try_get("", "sql")?;
        let kind: String = row.try_get("", "type")?;
        if kind == "trigger" {
            triggers.push(sql);
        } else {
            indexes.push(sql);
        }
    }
    for table in &ordered {
        db.execute_unprepared(&format!(
            "CREATE TEMP TABLE {} AS SELECT * FROM {}",
            quote(&format!("d372_backup_{table}")),
            quote(table)
        ))
        .await?;
    }
    normalize_overlay_targets(manager, required).await?;
    for table in ordered.iter().rev() {
        db.execute_unprepared(&format!("DROP TABLE {}", quote(table)))
            .await?;
    }
    for table in &ordered {
        let mut sql = schemas[table].clone();
        if table == "pricing_plan" || (required && table == "pricing_price") {
            // Split at commas only to locate this plain column declaration;
            // every other byte of the stored CREATE TABLE statement survives.
            let declaration = sql
                .split(',')
                .find(|part| part.trim_start().starts_with("sku_id "))
                .ok_or_else(|| DbErr::Custom(format!("D-372: missing {table}.sku_id declaration")))?
                .to_owned();
            let replacement = if required {
                format!("{} NOT NULL", declaration.trim_end())
            } else {
                declaration.replace(" NOT NULL", "")
            };
            sql = sql.replacen(&declaration, &replacement, 1);
        }
        if table == "pricing_price_overlay_line" {
            let legacy =
                "target_sku IS NULL OR length(trim(target_sku, char(9,10,11,12,13,32))) > 0";
            let uuid_aware = "target_sku IS NULL OR (typeof(target_sku) = 'blob' AND length(target_sku) = 16 AND target_sku <> zeroblob(16))";
            sql = if required {
                sql.replace(legacy, uuid_aware)
            } else {
                sql.replace(uuid_aware, legacy)
            };
        }
        db.execute_unprepared(&sql).await?;
    }
    for sql in indexes {
        db.execute_unprepared(&sql).await?;
    }
    for table in &ordered {
        db.execute_unprepared(&format!(
            "INSERT INTO {} SELECT * FROM {}",
            quote(table),
            quote(&format!("d372_backup_{table}"))
        ))
        .await?;
    }
    for sql in triggers {
        db.execute_unprepared(&sql).await?;
    }
    for table in &ordered {
        db.execute_unprepared(&format!(
            "DROP TABLE {}",
            quote(&format!("d372_backup_{table}"))
        ))
        .await?;
    }
    Ok(())
}

/// Normalize UUIDs before the guarded tables are restored. Both representations
/// cannot coexist: `SQLite` compares text and blob keys as different values.
async fn normalize_overlay_targets(
    manager: &SchemaManager<'_>,
    required: bool,
) -> Result<(), DbErr> {
    let db = manager.get_connection();
    let rows = db.query_all_raw(Statement::from_string(DbBackend::Sqlite,
        "SELECT rowid AS rid, CASE typeof(target_sku) WHEN 'blob' THEN hex(target_sku) ELSE target_sku END AS sku FROM d372_backup_pricing_price_overlay_line WHERE target_sku IS NOT NULL")).await?;
    for row in rows {
        let rid: i64 = row.try_get("", "rid")?;
        let text: String = row.try_get("", "sku")?;
        let sku = uuid::Uuid::parse_str(&text)
            .ok()
            .filter(|sku| !sku.is_nil())
            .ok_or_else(|| {
                DbErr::Custom(format!(
                    "D-372: overlay target_sku {text:?} is not a non-nil UUID"
                ))
            })?;
        let value: sea_orm::Value = if required {
            sku.into()
        } else {
            sku.to_string().into()
        };
        db.execute_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE d372_backup_pricing_price_overlay_line SET target_sku = ? WHERE rowid = ?",
            [value, rid.into()],
        ))
        .await?;
    }
    Ok(())
}
