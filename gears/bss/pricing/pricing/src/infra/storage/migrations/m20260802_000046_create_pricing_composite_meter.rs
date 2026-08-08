//! `pricing_composite_meter` — Slice 10's derived-meter definition
//! (`design/10-advanced-primitives.md` §6, `inst-cm-constituents`,
//! `inst-cm-formula`, `inst-cm-frozen`, A4, D-32, D-106).
//!
//! A composite meter prices several constituent `meteringUnit`s as **one line**
//! — VM = vCPU + RAM — by declaring one `output_unit` and a formula over the
//! constituents. The catalog persists and freezes the definition and **never
//! evaluates it**: Rating does, from the snapshot (`inst-cm-frozen`).
//!
//! # Why it is its own table and keyed the way it is
//!
//! `PRIMARY KEY (composite_id, plan_revision)` is `pricing_plan_phase`'s shape
//! one table over, and for D-106's reason restated in §6: the formula is
//! plan-shape configuration *"versioned with the plan revision"*, so opening a
//! draft revision **copies** the rows under the new `plan_revision` with a
//! **stable `composite_id`**, and a published revision's rows are immutable with
//! it. A bare `revision` column whose referent was never stated is what §6
//! replaced to get here.
//!
//! # Two rules this table deliberately does **not** enforce
//!
//! **Arity (`≥ 2` constituents) and self-reference are publish rules, not column
//! constraints**, and the reason is the one `m20260802_000045` recorded for the
//! reservation pair: `SQLite` has no incremental table-`CHECK`, and a
//! Postgres-only `ALTER` splits the two `EXPECTED_CHECKS` censuses that exist to
//! keep the engines legible against each other. §6 says as much for
//! self-reference already — *"check application-level (graph walk over
//! `constituent_units` vs `output_unit`)"* — and arity joins it for the same
//! reason plus a second: `json_array_length` is an extension function on
//! `SQLite` and a constraint that silently degrades on one engine is worse than
//! one stated in a rule that runs on both. They are
//! `COMPOSITE_TOO_FEW_CONSTITUENTS` and `COMPOSITE_SELF_REFERENCE`, both 422.
//!
//! **A constituent's *publication* is not checked at all**, here or anywhere.
//! `inst-cm-constituents` requires "≥ 2 **published** constituent `meteringUnit`
//! ids (registry-declared)" and this gear has **no registry client**:
//! `metering_unit` / `MeteringUnit` appear nowhere in `src/`, and
//! `PriceRow::meter` is a free `Option<String>` validated against nothing. The
//! arity and self-reference halves need no counterparty and are built; the
//! publication half is owed with the registry seam, and is recorded on the
//! instruction rather than left for a reader to infer from a rule that is not
//! there. `inst-cm-output-unit` is a registry *declaration* act and is not this
//! gear's at all (D-32).
//!
//! # The append-only guard
//!
//! `pricing_plan_phase`'s, verbatim in shape: the parent revision's
//! `lifecycle_state` is this row's, so every verb consults it — `UPDATE` and
//! `DELETE` against the revision the row is bound to **now**, `INSERT` and
//! `UPDATE` against the revision it would land **under**. Postgres expresses it
//! as one PL/pgSQL function with the offending state interpolated; `SQLite` has
//! no procedural language and `RAISE(ABORT, …)` takes a literal, so the same
//! rule becomes three fixed-message triggers whose parent lookup is a
//! `WHERE NOT EXISTS` subquery in the trigger **body** — a `SQLite` `WHEN` clause
//! may not contain a subquery.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_composite_meter (
        composite_id      uuid   NOT NULL,
        plan_revision     bigint NOT NULL,
        tenant_id         uuid   NOT NULL,
        plan_id           uuid   NOT NULL,
        output_unit       text   NOT NULL,
        constituent_units jsonb  NOT NULL,
        formula           jsonb  NOT NULL,
        PRIMARY KEY (composite_id, plan_revision),
        -- The output unit is the row's whole reason for existing; an empty one
        -- would publish a composite nothing can rate.
        CONSTRAINT chk_pricing_composite_meter_output_unit CHECK (
            length(output_unit) > 0),
        CONSTRAINT fk_pricing_composite_meter_revision FOREIGN KEY (plan_id, plan_revision)
            REFERENCES bss.pricing_plan (plan_id, revision)
    )",
    // One output unit per revision: §3 step 3's injectivity (`inst-cm-output`)
    // as far as the schema can carry it -- two composites of one revision
    // rating into the same unit would produce two priced lines on one
    // `(meter, dimensionKey)`, which is exactly what Slice 2's meter
    // injectivity forbids.
    "CREATE UNIQUE INDEX uq_pricing_composite_meter_output
        ON bss.pricing_composite_meter (tenant_id, plan_id, plan_revision, output_unit)",
    // The copy-forward, the drop-on-abandon and the self-reference walk all
    // range over one revision's composites under one tenant.
    "CREATE INDEX idx_pricing_composite_meter_revision
        ON bss.pricing_composite_meter (tenant_id, plan_id, plan_revision)",
    "CREATE OR REPLACE FUNCTION bss.pricing_composite_meter_append_only() RETURNS trigger AS $$
        DECLARE
          parent_state text;
        BEGIN
          IF TG_OP <> 'INSERT' THEN
            SELECT lifecycle_state INTO parent_state
              FROM bss.pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision;
            IF parent_state IS DISTINCT FROM 'draft' THEN
              RAISE EXCEPTION
                'pricing_composite_meter: % of a composite under a % plan revision is not permitted',
                TG_OP, coalesce(parent_state, 'missing');
            END IF;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RETURN OLD;
          END IF;

          SELECT lifecycle_state INTO parent_state
            FROM bss.pricing_plan
           WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision;
          IF parent_state IS DISTINCT FROM 'draft' THEN
            RAISE EXCEPTION
              'pricing_composite_meter: % of a composite under a % plan revision is not permitted',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_composite_meter_append_only
        BEFORE INSERT OR UPDATE OR DELETE ON bss.pricing_composite_meter
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_composite_meter_append_only()",
];

// This migration puts no trigger on a table it does not own, so dropping the
// table takes every trigger of this migration with it; the function is named
// separately because a Postgres function outlives the table it guarded.
const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_composite_meter",
    "DROP FUNCTION IF EXISTS bss.pricing_composite_meter_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_composite_meter (
        composite_id      text   NOT NULL,
        plan_revision     bigint NOT NULL,
        tenant_id         text   NOT NULL,
        plan_id           text   NOT NULL,
        output_unit       text   NOT NULL,
        constituent_units text   NOT NULL,
        formula           text   NOT NULL,
        PRIMARY KEY (composite_id, plan_revision),
        CONSTRAINT chk_pricing_composite_meter_output_unit CHECK (
            length(output_unit) > 0),
        CONSTRAINT fk_pricing_composite_meter_revision FOREIGN KEY (plan_id, plan_revision)
            REFERENCES pricing_plan (plan_id, revision)
    )",
    "CREATE UNIQUE INDEX uq_pricing_composite_meter_output
        ON pricing_composite_meter (tenant_id, plan_id, plan_revision, output_unit)",
    "CREATE INDEX idx_pricing_composite_meter_revision
        ON pricing_composite_meter (tenant_id, plan_id, plan_revision)",
    "CREATE TRIGGER trg_pricing_composite_meter_no_insert
        BEFORE INSERT ON pricing_composite_meter
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_composite_meter: INSERT of a composite under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision
               AND lifecycle_state = 'draft');
        END",
    // Both ends: the revision the row leaves and the revision it lands under.
    "CREATE TRIGGER trg_pricing_composite_meter_no_update
        BEFORE UPDATE ON pricing_composite_meter
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_composite_meter: UPDATE of a composite under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision
               AND lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision
               AND lifecycle_state = 'draft');
        END",
    "CREATE TRIGGER trg_pricing_composite_meter_no_delete
        BEFORE DELETE ON pricing_composite_meter
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_composite_meter: DELETE of a composite under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision
               AND lifecycle_state = 'draft');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_composite_meter"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
