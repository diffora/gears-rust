//! The frozen-column guard stops naming a column that no longer exists.
//!
//! `m20260802_000082` renamed `reserved_rate_minor` to `reserved_rate_nano` and
//! rescaled its values, on the stated ground that **"`RENAME COLUMN` carries
//! `CHECK` and trigger references with it on both engines"**. For a `CHECK` that
//! is true. For the body of a PL/pgSQL function it is **false**: the body is an
//! opaque string to the dependency tracker, so `NEW.reserved_rate_minor` survived
//! the rename verbatim and now names a field the record does not have.
//!
//! Measured rather than reasoned about, against a migrated database:
//!
//! ```text
//! live trigger body names : reserved_rate_minor
//! column on the table     : reserved_rate_nano
//! ```
//!
//! # The damage is not "one column went unguarded"
//!
//! That was the first reading and it is too kind. A guard whose body dereferences
//! a missing field does not fail open on that column — it **raises at runtime for
//! every UPDATE it inspects**, so the whole append-only guarantee for
//! `pricing_price` was down: not just the reserved rate, but every frozen column,
//! the `grandfather_until` monotonicity, and the sanctioned `published ->
//! superseded` flip that legitimate supersession depends on. Six Postgres suites
//! said so at once, `the_supersession_flip_is_accepted` among them, which is the
//! shape of a broken guard rather than a missing entry.
//!
//! # `down` is a semantic no-op, and the convention has to bend here
//!
//! This crate's guard migrations follow "DOWN is the previous UP, verbatim". That
//! convention assumes the previous state was *correct*, and here it was not — so
//! restoring it literally broke the reverse walk, which is a regression this
//! migration shipped with and had to be corrected:
//!
//! ```text
//! down m20260802_000085 must succeed: error in trigger
//! trg_pricing_price_frozen_columns: no such column: NEW.reserved_rate_minor
//! ```
//!
//! The reverse walk reaches `down(000086)` **before** `down(000082)`, so the column
//! is still `reserved_rate_nano` at that point and the only body valid there is one
//! that names it. `down` therefore re-emits the same guard as `up`, and reversing
//! this migration changes nothing — which is honest: what it changed was a name
//! that the chain below it has not yet changed back.
//!
//! # The engine asymmetry underneath both halves of this bug
//!
//! `ALTER TABLE ... RENAME COLUMN` **does** rewrite trigger bodies on `SQLite` (since
//! 3.25) and **does not** on Postgres, where a PL/pgSQL body is an opaque string.
//! So `m20260802_000082`'s claim held on one engine and failed on the other, which
//! is why the original breakage was Postgres-only — and why writing `SQLite`'s `down`
//! from `m20260802_000069`'s literal text restored a state that never existed on
//! `SQLite` at all. One asymmetry, two defects, the same column.
//!
//! # Why a migration of its own, again
//!
//! The same reason `m20260802_000055`, `...057` and `...069` are their own: the
//! guard is a whole function body, `CREATE OR REPLACE` is the only way to change
//! one, and a migration that both renames a column and re-emits every guard that
//! mentions it would make the rename unreviewable. `m20260802_000082` chose the
//! narrow scope correctly and simply left this half undone.
//!
//! **The claim it relied on was still in the tree** — `m20260802_000069`'s header,
//! about `pricing_price_tier_band`'s guard surviving a rename of
//! `unit_price_nano` — and this header forwarded it as a live doubt. Both are
//! corrected as of 2026-08-18 (review P-7): the doubt was about an identifier that
//! occurs in **no** trigger body on either engine. The band rate is protected by
//! the parent row's `lifecycle_state`, wholesale, which needs no restatement on a
//! rename. Recorded here rather than only there because this file is where the
//! next reader meets the question.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// `pub(super)` so `m20260802_000082` can apply this repair **before** its own
/// rescale rather than four migrations after it: the rescale cannot run while the
/// guard still names the column the rename removed. One source for the body,
/// because two would drift and the digest pin only measures the final one.
pub(super) const PG_UP_STATEMENTS: &[&str] = &[
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

pub(super) const PG_DOWN_STATEMENTS: &[&str] = &[
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

/// `pub(super)` so `m20260802_000082` can apply this repair **before** its own
/// rescale rather than four migrations after it: the rescale cannot run while the
/// guard still names the column the rename removed. One source for the body,
/// because two would drift and the digest pin only measures the final one.
pub(super) const SQLITE_UP_STATEMENTS: &[&str] = &[
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

pub(super) const SQLITE_DOWN_STATEMENTS: &[&str] = &[
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
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
