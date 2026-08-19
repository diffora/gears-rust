//! `pricing_price`'s frozen-column guard gains Slice 10's floor and
//! discount columns.
//!
//! `m20260802_000056` added `min_qty_purchase`, `min_qty_usage`,
//! `min_qty_usage_fallback` and `discount_ref` and put none of them in either
//! engine's guard. The same gap `m20260802_000039`/`m20260802_000040` opened and
//! paid for the tax columns, `m20260802_000050`/`m20260802_000051` for the
//! proration ones and `m20260802_000054`/`m20260802_000055` for the reservation
//! pair -- and the argument is each column's own rather than a shared one:
//!
//! - **`min_qty_purchase`** is what Subscriptions rejects an order against. A
//!   writer moving it on a published row changes who may buy the plan.
//! - **`min_qty_usage`** and **`min_qty_usage_fallback`** decide whether
//!   below-floor usage is billable at all (`inst-ft-typed`) and what happens when
//!   it is not (`inst-ft-fallback`). Both are money: the fallback is the
//!   difference between a rating exception an operator resolves and a line that
//!   silently bills or silently does not.
//! - **`discount_ref`** points at the instrument Promotions applies. Re-pointing
//!   it on a published row is a discount nobody approved.
//!
//! All four are authored draft content a `PATCH` moves, and all four are inside
//! the approval content pin -- so a published row a writer outside this crate
//! edited would diverge from the very pin that approved it, silently.
//!
//! # Why a migration of its own, again
//!
//! `m20260802_000040`'s reason, unchanged: `SQLite` keeps the frozen-column
//! check in a trigger of its own, while **Postgres keeps it as one arm of
//! `bss.pricing_price_append_only()`** -- a function that also carries the
//! `DELETE` guard, the draft-transition guard, the published-to-superseded guard
//! and the `grandfather_until` monotonicity guard. Neither engine has an
//! incremental form, so the Postgres half is the whole function restated, and a
//! restatement deserves a diff a reviewer can read on its own.
//!
//! # How the restatement was produced, and what checked it
//!
//! **Not by hand and not by a free-form script**, which is `m20260802_000051`'s
//! rule. The two statement blocks were read out of `m20260802_000055`'s own `UP`
//! text; four lines were inserted after `reservation_flavor` in each; and the
//! generator asserted, before writing anything, that it had found exactly one
//! frozen-column arm per engine, that each block came out **exactly four lines
//! longer per arm**, that each new column appears exactly once per arm, and that
//! **every original line survives**. This program has twice had a scripted
//! replacement do something other than intended, once deleting whole function
//! bodies; those assertions are the cheapest check that catches the class.
//!
//! The `DOWN` is `m20260802_000055`'s `UP` **verbatim** -- the state this
//! migration moves away from.
//!
//! What watches the restatement is unchanged: `sqlite_migrations` pins trigger
//! **bodies** by digest, `postgres_schema_price` moves every frozen column one
//! `UPDATE` each, and a restatement that lost an arm reddens in both rather than
//! in production.
//!
//! # Where the four sit in the list
//!
//! Directly after `reservation_flavor`, the other Slice-10 columns on this table.
//! The list is not sorted by anything, so "beside the fact it belongs to" is the
//! only ordering rule it has -- `m20260802_000040`'s words, kept.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

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
          OR NEW.model_kind                IS DISTINCT FROM OLD.model_kind
          OR NEW.tax_inclusive             IS DISTINCT FROM OLD.tax_inclusive
          OR NEW.tax_category_ref          IS DISTINCT FROM OLD.tax_category_ref
          OR NEW.resolved_tax_category     IS DISTINCT FROM OLD.resolved_tax_category
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
          OR NEW.reserved_rate_minor       IS DISTINCT FROM OLD.reserved_rate_minor
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

const PG_DOWN_STATEMENTS: &[&str] = &[
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
          OR NEW.model_kind                IS DISTINCT FROM OLD.model_kind
          OR NEW.tax_inclusive             IS DISTINCT FROM OLD.tax_inclusive
          OR NEW.tax_category_ref          IS DISTINCT FROM OLD.tax_category_ref
          OR NEW.resolved_tax_category     IS DISTINCT FROM OLD.resolved_tax_category
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
          OR NEW.reserved_rate_minor       IS DISTINCT FROM OLD.reserved_rate_minor
          OR NEW.reservation_flavor        IS DISTINCT FROM OLD.reservation_flavor
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

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

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
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
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
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
