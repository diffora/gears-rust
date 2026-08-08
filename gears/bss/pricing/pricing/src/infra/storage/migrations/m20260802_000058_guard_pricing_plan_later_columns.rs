//! `pricing_plan`'s frozen-column guard gains the four columns Slices 6 and 10
//! added after `m20260802_000001` wrote it.
//!
//! `m20260802_000052` added `allowed_change_targets`, `comparability_rank` and
//! `usage_counter_on_plan_change`; `m20260802_000053` added
//! `entitlement_grants`. Neither put its columns in either engine's guard, so a
//! **published** plan revision's grant set and plan-change contract could be
//! moved by an ad-hoc `UPDATE` and the trigger said nothing. Measured before this
//! migration was written rather than inferred: on the mirror,
//! `UPDATE pricing_plan SET entitlement_grants = ... WHERE ... revision = 0`
//! against a `published` row **succeeded**.
//!
//! This is the same gap `m20260802_000039`/`m20260802_000040` opened and paid for
//! the tax columns, `m20260802_000050`/`m20260802_000051` for the proration ones,
//! `m20260802_000054`/`m20260802_000055` for the reservation pair and
//! `m20260802_000056`/`m20260802_000057` for the floor and discount columns. Four
//! waves followed the discipline on `pricing_price`; the two `pricing_plan` waves
//! did not, and nothing noticed because the census that guards this class pins an
//! array copied from the trigger and is blind to a column the trigger omits.
//!
//! # What each column is, and why moving it after publish is not a small thing
//!
//! - **`entitlement_grants`** is what Subscriptions provisions from
//!   (`inst-gs-shape`, D-41). Moving it on a published revision changes what a
//!   subscriber is entitled to use, with no approval and no delta.
//! - **`allowed_change_targets`** is the explicit set a self-service change may
//!   travel to (`inst-pc-targets`, D-23). Adding an edge after publish routes
//!   plan changes nobody approved; removing one strands the changes in flight.
//! - **`comparability_rank`** decides whether a change reads as an upgrade, a
//!   downgrade or a switch (K4), on **both** ends of every edge that names it.
//! - **`usage_counter_on_plan_change`** is D-113's carry-vs-reset flag, frozen
//!   into the snapshot Rating reads — whether a subscriber's usage counter
//!   survives their plan change.
//!
//! **All four are inside the approval content pin.** `assemble_from` destructures
//! the draft so both reach the shape the rules judge (D-258, D-259) and
//! `content_pin::put_plan_shape` hashes them. The digest is computed at publish,
//! and these columns were mutable *after* it — so a published revision could be
//! made to publish a different entitlement than the one its approval signed
//! **with the digest unchanged**, which is precisely the hole the pin exists to
//! close.
//!
//! # Why a migration of its own
//!
//! `m20260802_000040`'s reason, unchanged: `SQLite` keeps the frozen-column check
//! in a trigger of its own, while **Postgres keeps it as one arm of
//! `bss.pricing_plan_append_only()`** — a function that also carries the `DELETE`
//! ban and the lifecycle-flip whitelist. Neither engine has an incremental form,
//! so the Postgres half is the whole function restated, and a restatement
//! deserves a diff a reviewer can read on its own. Nothing but the frozen-column
//! arm changes.
//!
//! # How the restatement was produced, and what checked it
//!
//! `m20260802_000051`'s rule, followed: **not by hand and not by a free-form
//! script.** Both blocks were read out of `m20260802_000001`'s own `UP` text; four
//! lines were inserted before the `row_version` line in each; and the generator
//! asserted, before writing anything, that it had found **exactly one**
//! frozen-column arm per engine, that each block came out **exactly four lines
//! longer**, that each new column appears **exactly once** per arm, and that
//! **every original line survives**. This program has twice had a scripted
//! replacement do something other than intended, once deleting whole function
//! bodies.

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

// The `DOWN` of a guard restatement is the guard as it stood, which is the weaker
// one. It exists so the chain rolls back and re-applies, which
// `the_chain_survives_a_roll_back_and_a_re_apply` exercises.
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
// SQLite - the one trigger dropped and recreated. Its two siblings are not
// re-issued: only this arm moves.
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
