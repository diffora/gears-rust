//! `pricing_plan`'s frozen-column guard gains `cloned_from`.
//!
//! `m20260802_000061` added the column; this freezes it, in the same wave rather
//! than in someone else's. Provenance belongs beside `created_by` and
//! `created_at_utc`: a writer moving it on a published revision rewrites where
//! the plan came from, and lineage nobody can trust is worse than none.
//!
//! # Why this migration exists at all, given D-263
//!
//! `m20260802_000058` closed the same gap for four columns that had gone
//! unguarded since Slices 6 and 10, and the reason nothing had noticed was that
//! both engines' censuses enumerated the **guard**. They now read the column list
//! off the **table**, so a column added and left unguarded reddens two suites —
//! which means this migration was not written from memory or diligence. It was
//! written because the census demanded it, which is the whole point of moving a
//! rule out of a maintainer's head and into a test.
//!
//! # Produced the same way as `m20260802_000058`
//!
//! `m20260802_000051`'s rule: **not by hand and not by a free-form script.** Both
//! blocks were read out of `m20260802_000058`'s own `UP` text — the guard as it
//! now stands, not as `m20260802_000001` wrote it — one line inserted before the
//! `row_version` line in each, and the generator asserted before writing that it
//! had found **exactly one** frozen-column arm per engine, that each block came
//! out **exactly one line longer**, that the column appears **exactly once** per
//! arm, and that **every original line survives**.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres - the whole function restated, `CREATE OR REPLACE` in place.
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE OR REPLACE FUNCTION bss.pricing_plan_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_plan: DELETE of revision % of plan % is not permitted; a discarded draft revision is abandoned',
              OLD.revision, OLD.plan_id;
          END IF;

          -- The draft plane is where content moves, so its columns are
          -- unguarded - but its **exits** are not. A draft leaves by publishing
          -- or by being abandoned, and `NEW = draft` is the ordinary edit.
          -- Without the check a hand-run flip could mint a `retired` row that
          -- never published - one that satisfies the current-revision partial
          -- UNIQUE and is what the projector then sources a plan subject from.
          -- Membership is tested rather than change: a
          -- `NEW IS DISTINCT FROM OLD` conjunct would let the SQLite mirror
          -- accept a no-op UPDATE this branch refuses, and a backend divergence
          -- is worse than the hole it would close.
          IF OLD.lifecycle_state = 'draft' THEN
            IF NEW.lifecycle_state NOT IN ('draft','published','abandoned') THEN
              RAISE EXCEPTION 'pricing_plan: lifecycle_state % -> % is not a sanctioned flip',
                OLD.lifecycle_state, NEW.lifecycle_state;
            END IF;
            RETURN NEW;
          END IF;

          -- Past here the row is published, superseded, retired or abandoned.
          -- Once abandoned it is a tombstone: frozen in content by the whitelist
          -- below and left by no flip, so the number it consumed can never be
          -- attached to a different shape.

          IF NEW.plan_id              IS DISTINCT FROM OLD.plan_id
          OR NEW.revision             IS DISTINCT FROM OLD.revision
          OR NEW.tenant_id            IS DISTINCT FROM OLD.tenant_id
          OR NEW.sku_id               IS DISTINCT FROM OLD.sku_id
          OR NEW.plan_tier            IS DISTINCT FROM OLD.plan_tier
          OR NEW.billing_cycle        IS DISTINCT FROM OLD.billing_cycle
          OR NEW.frequency            IS DISTINCT FROM OLD.frequency
          OR NEW.custom_interval_n    IS DISTINCT FROM OLD.custom_interval_n
          OR NEW.custom_interval_unit IS DISTINCT FROM OLD.custom_interval_unit
          OR NEW.plan_tier_override   IS DISTINCT FROM OLD.plan_tier_override
          OR NEW.purchase_min_qty     IS DISTINCT FROM OLD.purchase_min_qty
          OR NEW.purchase_max_qty     IS DISTINCT FROM OLD.purchase_max_qty
          OR NEW.invoice_grouping_key IS DISTINCT FROM OLD.invoice_grouping_key
          OR NEW.available_from       IS DISTINCT FROM OLD.available_from
          OR NEW.available_to         IS DISTINCT FROM OLD.available_to
          OR NEW.created_by           IS DISTINCT FROM OLD.created_by
          OR NEW.created_at_utc       IS DISTINCT FROM OLD.created_at_utc
          OR NEW.allowed_change_targets       IS DISTINCT FROM OLD.allowed_change_targets
          OR NEW.comparability_rank           IS DISTINCT FROM OLD.comparability_rank
          OR NEW.usage_counter_on_plan_change IS DISTINCT FROM OLD.usage_counter_on_plan_change
          OR NEW.entitlement_grants           IS DISTINCT FROM OLD.entitlement_grants
          OR NEW.cloned_from                  IS DISTINCT FROM OLD.cloned_from
          OR NEW.row_version          IS DISTINCT FROM OLD.row_version THEN
            RAISE EXCEPTION
              'pricing_plan: revision % of plan % is frozen; only a sanctioned lifecycle_state flip is permitted',
              OLD.revision, OLD.plan_id;
          END IF;

          IF NOT (OLD.lifecycle_state = 'published'
                  AND NEW.lifecycle_state IN ('superseded','retired')) THEN
            RAISE EXCEPTION 'pricing_plan: lifecycle_state % -> % is not a sanctioned flip',
              OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
];

// The guard as `m20260802_000058` left it, so the chain rolls back and re-applies.
const PG_DOWN_STATEMENTS: &[&str] = &[
    "CREATE OR REPLACE FUNCTION bss.pricing_plan_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_plan: DELETE of revision % of plan % is not permitted; a discarded draft revision is abandoned',
              OLD.revision, OLD.plan_id;
          END IF;

          -- The draft plane is where content moves, so its columns are
          -- unguarded - but its **exits** are not. A draft leaves by publishing
          -- or by being abandoned, and `NEW = draft` is the ordinary edit.
          -- Without the check a hand-run flip could mint a `retired` row that
          -- never published - one that satisfies the current-revision partial
          -- UNIQUE and is what the projector then sources a plan subject from.
          -- Membership is tested rather than change: a
          -- `NEW IS DISTINCT FROM OLD` conjunct would let the SQLite mirror
          -- accept a no-op UPDATE this branch refuses, and a backend divergence
          -- is worse than the hole it would close.
          IF OLD.lifecycle_state = 'draft' THEN
            IF NEW.lifecycle_state NOT IN ('draft','published','abandoned') THEN
              RAISE EXCEPTION 'pricing_plan: lifecycle_state % -> % is not a sanctioned flip',
                OLD.lifecycle_state, NEW.lifecycle_state;
            END IF;
            RETURN NEW;
          END IF;

          -- Past here the row is published, superseded, retired or abandoned.
          -- Once abandoned it is a tombstone: frozen in content by the whitelist
          -- below and left by no flip, so the number it consumed can never be
          -- attached to a different shape.

          IF NEW.plan_id              IS DISTINCT FROM OLD.plan_id
          OR NEW.revision             IS DISTINCT FROM OLD.revision
          OR NEW.tenant_id            IS DISTINCT FROM OLD.tenant_id
          OR NEW.sku_id               IS DISTINCT FROM OLD.sku_id
          OR NEW.plan_tier            IS DISTINCT FROM OLD.plan_tier
          OR NEW.billing_cycle        IS DISTINCT FROM OLD.billing_cycle
          OR NEW.frequency            IS DISTINCT FROM OLD.frequency
          OR NEW.custom_interval_n    IS DISTINCT FROM OLD.custom_interval_n
          OR NEW.custom_interval_unit IS DISTINCT FROM OLD.custom_interval_unit
          OR NEW.plan_tier_override   IS DISTINCT FROM OLD.plan_tier_override
          OR NEW.purchase_min_qty     IS DISTINCT FROM OLD.purchase_min_qty
          OR NEW.purchase_max_qty     IS DISTINCT FROM OLD.purchase_max_qty
          OR NEW.invoice_grouping_key IS DISTINCT FROM OLD.invoice_grouping_key
          OR NEW.available_from       IS DISTINCT FROM OLD.available_from
          OR NEW.available_to         IS DISTINCT FROM OLD.available_to
          OR NEW.created_by           IS DISTINCT FROM OLD.created_by
          OR NEW.created_at_utc       IS DISTINCT FROM OLD.created_at_utc
          OR NEW.allowed_change_targets       IS DISTINCT FROM OLD.allowed_change_targets
          OR NEW.comparability_rank           IS DISTINCT FROM OLD.comparability_rank
          OR NEW.usage_counter_on_plan_change IS DISTINCT FROM OLD.usage_counter_on_plan_change
          OR NEW.entitlement_grants           IS DISTINCT FROM OLD.entitlement_grants
          OR NEW.row_version          IS DISTINCT FROM OLD.row_version THEN
            RAISE EXCEPTION
              'pricing_plan: revision % of plan % is frozen; only a sanctioned lifecycle_state flip is permitted',
              OLD.revision, OLD.plan_id;
          END IF;

          IF NOT (OLD.lifecycle_state = 'published'
                  AND NEW.lifecycle_state IN ('superseded','retired')) THEN
            RAISE EXCEPTION 'pricing_plan: lifecycle_state % -> % is not a sanctioned flip',
              OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
];

// ---------------------------------------------------------------------------
// SQLite - the one trigger dropped and recreated.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_plan_frozen_columns",
    "CREATE TRIGGER trg_pricing_plan_frozen_columns
        BEFORE UPDATE ON pricing_plan
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND (NEW.plan_id              IS NOT OLD.plan_id
            OR NEW.revision             IS NOT OLD.revision
            OR NEW.tenant_id            IS NOT OLD.tenant_id
            OR NEW.sku_id               IS NOT OLD.sku_id
            OR NEW.plan_tier            IS NOT OLD.plan_tier
            OR NEW.billing_cycle        IS NOT OLD.billing_cycle
            OR NEW.frequency            IS NOT OLD.frequency
            OR NEW.custom_interval_n    IS NOT OLD.custom_interval_n
            OR NEW.custom_interval_unit IS NOT OLD.custom_interval_unit
            OR NEW.plan_tier_override   IS NOT OLD.plan_tier_override
            OR NEW.purchase_min_qty     IS NOT OLD.purchase_min_qty
            OR NEW.purchase_max_qty     IS NOT OLD.purchase_max_qty
            OR NEW.invoice_grouping_key IS NOT OLD.invoice_grouping_key
            OR NEW.available_from       IS NOT OLD.available_from
            OR NEW.available_to         IS NOT OLD.available_to
            OR NEW.created_by           IS NOT OLD.created_by
            OR NEW.created_at_utc       IS NOT OLD.created_at_utc
            OR NEW.allowed_change_targets       IS NOT OLD.allowed_change_targets
            OR NEW.comparability_rank           IS NOT OLD.comparability_rank
            OR NEW.usage_counter_on_plan_change IS NOT OLD.usage_counter_on_plan_change
            OR NEW.entitlement_grants           IS NOT OLD.entitlement_grants
            OR NEW.cloned_from                  IS NOT OLD.cloned_from
            OR NEW.row_version          IS NOT OLD.row_version)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_plan: revision is frozen; only a sanctioned lifecycle_state flip is permitted');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_plan_frozen_columns",
    "CREATE TRIGGER trg_pricing_plan_frozen_columns
        BEFORE UPDATE ON pricing_plan
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND (NEW.plan_id              IS NOT OLD.plan_id
            OR NEW.revision             IS NOT OLD.revision
            OR NEW.tenant_id            IS NOT OLD.tenant_id
            OR NEW.sku_id               IS NOT OLD.sku_id
            OR NEW.plan_tier            IS NOT OLD.plan_tier
            OR NEW.billing_cycle        IS NOT OLD.billing_cycle
            OR NEW.frequency            IS NOT OLD.frequency
            OR NEW.custom_interval_n    IS NOT OLD.custom_interval_n
            OR NEW.custom_interval_unit IS NOT OLD.custom_interval_unit
            OR NEW.plan_tier_override   IS NOT OLD.plan_tier_override
            OR NEW.purchase_min_qty     IS NOT OLD.purchase_min_qty
            OR NEW.purchase_max_qty     IS NOT OLD.purchase_max_qty
            OR NEW.invoice_grouping_key IS NOT OLD.invoice_grouping_key
            OR NEW.available_from       IS NOT OLD.available_from
            OR NEW.available_to         IS NOT OLD.available_to
            OR NEW.created_by           IS NOT OLD.created_by
            OR NEW.created_at_utc       IS NOT OLD.created_at_utc
            OR NEW.allowed_change_targets       IS NOT OLD.allowed_change_targets
            OR NEW.comparability_rank           IS NOT OLD.comparability_rank
            OR NEW.usage_counter_on_plan_change IS NOT OLD.usage_counter_on_plan_change
            OR NEW.entitlement_grants           IS NOT OLD.entitlement_grants
            OR NEW.row_version          IS NOT OLD.row_version)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_plan: revision is frozen; only a sanctioned lifecycle_state flip is permitted');
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
