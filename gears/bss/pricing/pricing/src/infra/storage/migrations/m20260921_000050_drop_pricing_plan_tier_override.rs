//! **D-383**: `plan_tier_override` leaves `pricing_plan`.
//!
//! The flag recorded *whether the tier diverges from the parent SKU's under an
//! audited override*. It audited nothing for as long as it existed: on the
//! benidorm stand all 137 plans diverged and the flag read `false` on every
//! one, because nothing computes the equality it overrides — the caller sets it
//! for itself. `domain/plan_rules/composition.rs` names the two rules still
//! waiting on a registry client for the comparison to be possible at all.
//!
//! # The trigger is rebuilt, not edited, and that repairs a drift
//!
//! `trg_pricing_plan_frozen_columns` names every frozen column individually, so
//! dropping one means the whitelist is rewritten. It is **re-derived** here
//! rather than hand-edited: the rule is *every column of `pricing_plan` except
//! `lifecycle_state`*, the sanctioned flip, which after this migration is 22
//! columns. Both readings were taken when this was written — the clause list
//! transformed out of `000021`, and the column list read off the live table —
//! and they agree on the same 22. A name quietly dropped from that disjunction
//! is a frozen column that silently stops being frozen, which is why one
//! reading was not considered enough.
//!
//! **The drift it repairs.** `000021` was edited in place twice after it
//! shipped (`3116b0b3d`, `42bbbdee3`), and an applied migration never re-runs —
//! so a database that ran the earlier text carries a whitelist that a fresh one
//! does not. Recreating the trigger from a single source here brings every
//! database to the same definition regardless of which text it originally ran.
//!
//! # One-way
//!
//! The image applying this is the image that no longer reads the column, so the
//! deploy is safe; redeploying an older image afterwards is not. `down` restores
//! the column and the old whitelist, but the values are gone — and they were
//! `false` on every diverging plan, which is the whole reason for the drop.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_plan DROP COLUMN IF EXISTS plan_tier_override",
    r"CREATE OR REPLACE FUNCTION bss.pricing_plan_append_only() RETURNS trigger AS $$
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
         
          OR NEW.frequency            IS DISTINCT FROM OLD.frequency
          OR NEW.custom_interval_n    IS DISTINCT FROM OLD.custom_interval_n
          OR NEW.custom_interval_unit IS DISTINCT FROM OLD.custom_interval_unit
          OR NEW.purchase_min_qty     IS DISTINCT FROM OLD.purchase_min_qty
          OR NEW.purchase_max_qty     IS DISTINCT FROM OLD.purchase_max_qty
          OR NEW.descriptor_ext       IS DISTINCT FROM OLD.descriptor_ext
          OR NEW.available_from       IS DISTINCT FROM OLD.available_from
          OR NEW.available_to         IS DISTINCT FROM OLD.available_to
          OR NEW.created_by           IS DISTINCT FROM OLD.created_by
          OR NEW.created_at_utc       IS DISTINCT FROM OLD.created_at_utc
          OR NEW.allowed_change_targets       IS DISTINCT FROM OLD.allowed_change_targets
          OR NEW.comparability_rank           IS DISTINCT FROM OLD.comparability_rank
          OR NEW.usage_counter_on_plan_change IS DISTINCT FROM OLD.usage_counter_on_plan_change
          OR NEW.entitlement_grants           IS DISTINCT FROM OLD.entitlement_grants
          OR NEW.cloned_from                  IS DISTINCT FROM OLD.cloned_from
          OR NEW.plan_name            IS DISTINCT FROM OLD.plan_name
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

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_plan ADD COLUMN IF NOT EXISTS plan_tier_override boolean NOT NULL DEFAULT false",
    r"CREATE OR REPLACE FUNCTION bss.pricing_plan_append_only() RETURNS trigger AS $$
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
         
          OR NEW.frequency            IS DISTINCT FROM OLD.frequency
          OR NEW.custom_interval_n    IS DISTINCT FROM OLD.custom_interval_n
          OR NEW.custom_interval_unit IS DISTINCT FROM OLD.custom_interval_unit
          OR NEW.plan_tier_override   IS DISTINCT FROM OLD.plan_tier_override
          OR NEW.purchase_min_qty     IS DISTINCT FROM OLD.purchase_min_qty
          OR NEW.purchase_max_qty     IS DISTINCT FROM OLD.purchase_max_qty
          OR NEW.descriptor_ext       IS DISTINCT FROM OLD.descriptor_ext
          OR NEW.available_from       IS DISTINCT FROM OLD.available_from
          OR NEW.available_to         IS DISTINCT FROM OLD.available_to
          OR NEW.created_by           IS DISTINCT FROM OLD.created_by
          OR NEW.created_at_utc       IS DISTINCT FROM OLD.created_at_utc
          OR NEW.allowed_change_targets       IS DISTINCT FROM OLD.allowed_change_targets
          OR NEW.comparability_rank           IS DISTINCT FROM OLD.comparability_rank
          OR NEW.usage_counter_on_plan_change IS DISTINCT FROM OLD.usage_counter_on_plan_change
          OR NEW.entitlement_grants           IS DISTINCT FROM OLD.entitlement_grants
          OR NEW.cloned_from                  IS DISTINCT FROM OLD.cloned_from
          OR NEW.plan_name            IS DISTINCT FROM OLD.plan_name
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

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_plan_frozen_columns",
    "ALTER TABLE pricing_plan DROP COLUMN plan_tier_override",
    r"CREATE TRIGGER trg_pricing_plan_frozen_columns BEFORE UPDATE ON pricing_plan FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' AND (NEW.plan_id IS NOT OLD.plan_id OR NEW.revision IS NOT OLD.revision OR NEW.tenant_id IS NOT OLD.tenant_id OR NEW.sku_id IS NOT OLD.sku_id OR NEW.plan_tier IS NOT OLD.plan_tier OR NEW.frequency IS NOT OLD.frequency OR NEW.custom_interval_n IS NOT OLD.custom_interval_n OR NEW.custom_interval_unit IS NOT OLD.custom_interval_unit OR NEW.purchase_min_qty IS NOT OLD.purchase_min_qty OR NEW.purchase_max_qty IS NOT OLD.purchase_max_qty OR NEW.descriptor_ext IS NOT OLD.descriptor_ext OR NEW.available_from IS NOT OLD.available_from OR NEW.available_to IS NOT OLD.available_to OR NEW.created_by IS NOT OLD.created_by OR NEW.created_at_utc IS NOT OLD.created_at_utc OR NEW.allowed_change_targets IS NOT OLD.allowed_change_targets OR NEW.comparability_rank IS NOT OLD.comparability_rank OR NEW.usage_counter_on_plan_change IS NOT OLD.usage_counter_on_plan_change OR NEW.entitlement_grants IS NOT OLD.entitlement_grants OR NEW.cloned_from IS NOT OLD.cloned_from OR NEW.plan_name IS NOT OLD.plan_name OR NEW.row_version IS NOT OLD.row_version) BEGIN SELECT RAISE(ABORT, 'pricing_plan: revision is frozen; only a sanctioned lifecycle_state flip is permitted'); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_plan_frozen_columns",
    "ALTER TABLE pricing_plan ADD COLUMN plan_tier_override boolean NOT NULL DEFAULT 0",
    r"CREATE TRIGGER trg_pricing_plan_frozen_columns BEFORE UPDATE ON pricing_plan FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft' AND (NEW.plan_id IS NOT OLD.plan_id OR NEW.revision IS NOT OLD.revision OR NEW.tenant_id IS NOT OLD.tenant_id OR NEW.sku_id IS NOT OLD.sku_id OR NEW.plan_tier IS NOT OLD.plan_tier OR NEW.frequency IS NOT OLD.frequency OR NEW.custom_interval_n IS NOT OLD.custom_interval_n OR NEW.custom_interval_unit IS NOT OLD.custom_interval_unit OR NEW.plan_tier_override IS NOT OLD.plan_tier_override OR NEW.purchase_min_qty IS NOT OLD.purchase_min_qty OR NEW.purchase_max_qty IS NOT OLD.purchase_max_qty OR NEW.descriptor_ext IS NOT OLD.descriptor_ext OR NEW.available_from IS NOT OLD.available_from OR NEW.available_to IS NOT OLD.available_to OR NEW.created_by IS NOT OLD.created_by OR NEW.created_at_utc IS NOT OLD.created_at_utc OR NEW.allowed_change_targets IS NOT OLD.allowed_change_targets OR NEW.comparability_rank IS NOT OLD.comparability_rank OR NEW.usage_counter_on_plan_change IS NOT OLD.usage_counter_on_plan_change OR NEW.entitlement_grants IS NOT OLD.entitlement_grants OR NEW.cloned_from IS NOT OLD.cloned_from OR NEW.plan_name IS NOT OLD.plan_name OR NEW.row_version IS NOT OLD.row_version) BEGIN SELECT RAISE(ABORT, 'pricing_plan: revision is frozen; only a sanctioned lifecycle_state flip is permitted'); END",
];

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
