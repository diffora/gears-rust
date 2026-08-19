//! `pricing_price.resolved_rounding_policy` joins the frozen-column guard.
//!
//! The sibling of `m20260802_000040`, `000051`, `000055`, `000057` and `000069`,
//! and it exists for the same reason every one of those did: a content column was
//! added to this table and the guard's whole-function body was not restated, so
//! an ad-hoc `UPDATE` could move it under a frozen `CatalogVersion`. That census
//! (`postgres_schema_price::the_frozen_whitelist_names_every_content_column_the_table_holds`)
//! now reads the column list off the table, so the omission reddens instead of
//! being found by a person reading a diff — which is how all five earlier ones
//! were found.
//!
//! # `UP` widens the body, `DOWN` restores `m20260802_000086`'s verbatim
//!
//! The asymmetry is the point and getting it wrong broke the reverse walk once
//! already: `m20260802_000089`'s `down` drops the column, and `SQLite` refuses to
//! drop a column a trigger body still names. So the `DOWN` here must put back a
//! guard that does **not** know about it — which is exactly `m086`'s body, spliced
//! rather than copied so the two cannot drift.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
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
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_price_frozen_columns",
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
            OR NEW.resolved_rounding_policy  IS NOT OLD.resolved_rounding_policy
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
            OR NEW.reserved_rate_nano       IS NOT OLD.reserved_rate_nano
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
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // **Spliced, which the header has claimed since this migration landed and
        // which was not true until 2026-08-19** (review F8). The two `DOWN`
        // arrays here were a literal second copy of `m086`'s 45-column body —
        // byte-identical, and `m086`'s constants had been made `pub(super)` in
        // the very same wave, with a doc reading "one source for the body,
        // because two would drift and the digest pin only measures the final
        // one", precisely so they could be used here. `m082` does splice; this
        // did not.
        //
        // Restoring `m086`'s **`UP`** is what "restore the previous guard" means:
        // that is the body `m086` left armed, and it is the one that does not
        // name `resolved_rounding_policy` — which `m089`'s `down` is about to
        // drop.
        super::exec_backend(
            manager,
            super::m20260802_000086_guard_pricing_price_reserved_rate_nano::PG_UP_STATEMENTS,
            super::m20260802_000086_guard_pricing_price_reserved_rate_nano::SQLITE_UP_STATEMENTS,
        )
        .await
    }
}
