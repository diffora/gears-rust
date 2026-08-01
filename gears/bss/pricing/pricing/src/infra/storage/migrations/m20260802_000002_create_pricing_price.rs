//! Create `bss.pricing_price` — the price rows, and with them the price
//! **history** (`design/01-foundation.md` §3.7): superseded rows are retained
//! in this same table, chained by `supersedes_price_id`; there is no separate
//! history table and no row is ever moved or deleted.
//!
//! The eight canonical scope-key columns (`plan_id`, `currency`, `region`,
//! `price_overlay`, `phase`, `price_eligibility`, `charge_kind`, `cohort` —
//! §4.1) carry a partial `UNIQUE` over `lifecycle_state = 'published'`: at most
//! one **current** row per key. `cohort` is stored as a `NOT NULL` text token
//! (`none`, or the cutover instant) rather than a nullable timestamp precisely
//! because it is an index column — distinct `NULL`s compare as distinct in a
//! Postgres unique index, so a nullable cohort would let two current rows share
//! a key.
//!
//! The physical guard is the append-only trigger with a **column whitelist**
//! (§4.3). A published row permits exactly two moves: the state-machine
//! transition `published -> superseded` (its two sanctioned producers are the
//! supersession unit and the grandfathering cutover commit, D-100), and
//! **monotonic tightening** of `grandfather_until` — setting it when null, or
//! moving it earlier. Loosening it (clearing it, or moving it later) is
//! rejected, as is any change to a price, scope or model column; DELETE of a
//! non-draft row is always rejected. Never-published draft rows stay mutable
//! and deletable.
//!
//! **Backend differences.** As in the plan table, Postgres uses one PL/pgSQL
//! trigger with interpolated messages and `SQLite` uses four `RAISE(ABORT, ...)`
//! triggers with literal ones. One further `SQLite` caveat is real rather than
//! cosmetic: `grandfather_until` is `text` there, so the monotonicity comparison
//! is **lexicographic**, which coincides with chronological order only for the
//! canonical fixed-width UTC rendering `SeaORM` writes. Postgres compares
//! `timestamptz` values.
//!
//! Kind-specific band and package columns (`pricing_price_tier_band`,
//! `package_size`, `package_price_minor`, `quantity_source`, ...) are
//! Slice-3-owned (`design/03-price-structure.md` §6) and arrive with that
//! slice; what lands here is the Foundation-declared set §3.7 enumerates.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_price (
        price_id                  uuid        NOT NULL PRIMARY KEY,
        tenant_id                 uuid        NOT NULL,
        plan_id                   uuid        NOT NULL,
        currency                  varchar(3)  NOT NULL,
        region                    text        NOT NULL,
        price_overlay             text        NOT NULL DEFAULT 'base',
        phase                     uuid        NOT NULL,
        price_eligibility         text        NOT NULL DEFAULT 'all_subscriptions',
        charge_kind               text        NOT NULL,
        cohort                    text        NOT NULL DEFAULT 'none',
        amount_minor              bigint,
        model_kind                text,
        tax_inclusive             boolean     NOT NULL DEFAULT false,
        billing_timing            text,
        meter                     text,
        dimension_key             text        NOT NULL DEFAULT '',
        billing_granularity       text,
        aggregation_function      text,
        aggregation_granularity   text,
        tier_aggregation_window   text,
        tier_qualification_window text,
        max_hold_granules         integer,
        rounding_policy_ref       text,
        grandfather_until         timestamptz,
        supersedes_price_id       uuid,
        lifecycle_state           text        NOT NULL,
        created_by                uuid        NOT NULL,
        created_at_utc            timestamptz NOT NULL DEFAULT now(),
        CONSTRAINT chk_pricing_price_lifecycle_state CHECK (
            lifecycle_state IN ('draft','published','superseded','retired')),
        CONSTRAINT chk_pricing_price_overlay CHECK (price_overlay = 'base'),
        CONSTRAINT chk_pricing_price_eligibility CHECK (
            price_eligibility IN ('all_subscriptions','existing_grandfathered')),
        CONSTRAINT chk_pricing_price_charge_kind CHECK (
            charge_kind IN ('recurring','usage','one_time','one_time_setup')),
        CONSTRAINT chk_pricing_price_model_kind CHECK (
            model_kind IS NULL
            OR model_kind IN ('flat','per_unit','graduated','volume','package')),
        CONSTRAINT chk_pricing_price_billing_timing CHECK (
            billing_timing IS NULL OR billing_timing IN ('advance','arrears')),
        CONSTRAINT chk_pricing_price_amount_non_negative CHECK (
            amount_minor IS NULL OR amount_minor >= 0),
        CONSTRAINT chk_pricing_price_max_hold_granules CHECK (
            max_hold_granules IS NULL OR max_hold_granules >= 1),
        -- The cohort / eligibility biconditional (design 4.1): a cohort is set if
        -- and only if the row is grandfathered. Cheap here, and the domain
        -- re-establishes it on every rehydration because the two axes are read
        -- back as two independent columns.
        CONSTRAINT chk_pricing_price_cohort_eligibility CHECK (
            (cohort <> 'none') = (price_eligibility = 'existing_grandfathered')),
        -- Only a grandfathered row can carry a grandfathering horizon.
        CONSTRAINT chk_pricing_price_grandfather_until CHECK (
            grandfather_until IS NULL OR price_eligibility = 'existing_grandfathered')
    )",
    // At most one CURRENT row per canonical scope key. Sufficient on its own
    // under the flip-at-commit rule: the predecessor reads `superseded` the
    // instant its successor commits.
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current
        ON bss.pricing_price (
            plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort)
        WHERE lifecycle_state = 'published'",
    "CREATE INDEX idx_pricing_price_plan
        ON bss.pricing_price (tenant_id, plan_id, lifecycle_state)",
    // The history chain: walk a key's supersession lineage without a table scan.
    "CREATE INDEX idx_pricing_price_supersedes
        ON bss.pricing_price (tenant_id, supersedes_price_id)
        WHERE supersedes_price_id IS NOT NULL",
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
          OR NEW.billing_timing            IS DISTINCT FROM OLD.billing_timing
          OR NEW.meter                     IS DISTINCT FROM OLD.meter
          OR NEW.dimension_key             IS DISTINCT FROM OLD.dimension_key
          OR NEW.billing_granularity       IS DISTINCT FROM OLD.billing_granularity
          OR NEW.aggregation_function      IS DISTINCT FROM OLD.aggregation_function
          OR NEW.aggregation_granularity   IS DISTINCT FROM OLD.aggregation_granularity
          OR NEW.tier_aggregation_window   IS DISTINCT FROM OLD.tier_aggregation_window
          OR NEW.tier_qualification_window IS DISTINCT FROM OLD.tier_qualification_window
          OR NEW.max_hold_granules         IS DISTINCT FROM OLD.max_hold_granules
          OR NEW.rounding_policy_ref       IS DISTINCT FROM OLD.rounding_policy_ref
          OR NEW.supersedes_price_id       IS DISTINCT FROM OLD.supersedes_price_id
          OR NEW.created_by                IS DISTINCT FROM OLD.created_by
          OR NEW.created_at_utc            IS DISTINCT FROM OLD.created_at_utc THEN
            RAISE EXCEPTION
              'pricing_price: row % is published; price, scope and model columns are immutable',
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
    "CREATE TRIGGER trg_pricing_price_append_only
        BEFORE UPDATE OR DELETE ON bss.pricing_price
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_price_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_price",
    "DROP FUNCTION IF EXISTS bss.pricing_price_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transforms: `bss.` dropped, `uuid` -> `text`,
// `timestamptz` -> `text`, `now()` -> `(CURRENT_TIMESTAMP)`,
// `IS DISTINCT FROM` -> `IS NOT`, and the one PL/pgSQL trigger split into four
// literal-message `RAISE(ABORT, ...)` triggers. Every CHECK, index and PK is
// preserved. See the module doc for the lexicographic `grandfather_until`
// caveat.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_price (
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
        meter                     text,
        dimension_key             text        NOT NULL DEFAULT '',
        billing_granularity       text,
        aggregation_function      text,
        aggregation_granularity   text,
        tier_aggregation_window   text,
        tier_qualification_window text,
        max_hold_granules         integer,
        rounding_policy_ref       text,
        grandfather_until         text,
        supersedes_price_id       text,
        lifecycle_state           text        NOT NULL,
        created_by                text        NOT NULL,
        created_at_utc            text        NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        CONSTRAINT chk_pricing_price_lifecycle_state CHECK (
            lifecycle_state IN ('draft','published','superseded','retired')),
        CONSTRAINT chk_pricing_price_overlay CHECK (price_overlay = 'base'),
        CONSTRAINT chk_pricing_price_eligibility CHECK (
            price_eligibility IN ('all_subscriptions','existing_grandfathered')),
        CONSTRAINT chk_pricing_price_charge_kind CHECK (
            charge_kind IN ('recurring','usage','one_time','one_time_setup')),
        CONSTRAINT chk_pricing_price_model_kind CHECK (
            model_kind IS NULL
            OR model_kind IN ('flat','per_unit','graduated','volume','package')),
        CONSTRAINT chk_pricing_price_billing_timing CHECK (
            billing_timing IS NULL OR billing_timing IN ('advance','arrears')),
        CONSTRAINT chk_pricing_price_amount_non_negative CHECK (
            amount_minor IS NULL OR amount_minor >= 0),
        CONSTRAINT chk_pricing_price_max_hold_granules CHECK (
            max_hold_granules IS NULL OR max_hold_granules >= 1),
        CONSTRAINT chk_pricing_price_cohort_eligibility CHECK (
            (cohort <> 'none') = (price_eligibility = 'existing_grandfathered')),
        CONSTRAINT chk_pricing_price_grandfather_until CHECK (
            grandfather_until IS NULL OR price_eligibility = 'existing_grandfathered')
    )",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current
        ON pricing_price (
            plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort)
        WHERE lifecycle_state = 'published'",
    "CREATE INDEX idx_pricing_price_plan
        ON pricing_price (tenant_id, plan_id, lifecycle_state)",
    "CREATE INDEX idx_pricing_price_supersedes
        ON pricing_price (tenant_id, supersedes_price_id)
        WHERE supersedes_price_id IS NOT NULL",
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
            OR NEW.billing_timing            IS NOT OLD.billing_timing
            OR NEW.meter                     IS NOT OLD.meter
            OR NEW.dimension_key             IS NOT OLD.dimension_key
            OR NEW.billing_granularity       IS NOT OLD.billing_granularity
            OR NEW.aggregation_function      IS NOT OLD.aggregation_function
            OR NEW.aggregation_granularity   IS NOT OLD.aggregation_granularity
            OR NEW.tier_aggregation_window   IS NOT OLD.tier_aggregation_window
            OR NEW.tier_qualification_window IS NOT OLD.tier_qualification_window
            OR NEW.max_hold_granules         IS NOT OLD.max_hold_granules
            OR NEW.rounding_policy_ref       IS NOT OLD.rounding_policy_ref
            OR NEW.supersedes_price_id       IS NOT OLD.supersedes_price_id
            OR NEW.created_by                IS NOT OLD.created_by
            OR NEW.created_at_utc            IS NOT OLD.created_at_utc)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price: row is published; price, scope and model columns are immutable');
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
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_price"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
