//! Create `bss.pricing_plan_addon_rule` — the add-on composition rules of
//! **one plan revision** (`design/02-plan-definition.md` §6, D-105), keyed
//! `(plan_id, plan_revision, addon_sku_id)`. The second of Slice 2's three
//! revision-scoped child tables, and the sibling of `pricing_plan_phase` in
//! every structural respect.
//!
//! # The key carries `addon_sku_id`, and the omission it repairs was fatal
//!
//! D-105. The earlier spelling keyed this table `(plan_id, plan_revision)`,
//! which admits **one** add-on rule per revision — and one row cannot hold the
//! data three rules of this slice are written over. The `depends_on` cycle walk
//! needs at least two rules to have an edge between them; the symmetric-conflict
//! normalization needs a second row for the back-edge to land on; "two required
//! conflicting add-ons fail publish" names a pair. All three were unsatisfiable
//! by construction, and none of them would have failed a test — a plan simply
//! could never reach the state the rule rejects, so the rule would have read as
//! holding while enforcing nothing. `pricing_plan_phase`'s
//! `(phase_id, plan_revision)` is the shape this table should have had from the
//! start, and now has.
//!
//! `plan_revision` is the copy-on-new-revision half (D-83), exactly as on the
//! phase table: a new revision copies these rows under its own number and the
//! open draft edits its own copies.
//!
//! **`tenant_id` is not in §6's column list, and is here anyway.** §6's own
//! preamble calls these tables "tenant-scoped, `SecureORM`" and
//! `01-foundation.md` §3.7 says it of every physical table in this gear;
//! `Scopable` has nowhere else to read the tenant from. The omission is
//! reported rather than treated as a decision. The value is copied from the
//! parent revision by the repository and never taken from a request (Global
//! Constraint 9): the foreign key covers the plan key alone, so nothing here
//! stops a child carrying a different tenant from the row it points at, and
//! under `SecureORM` such a child is invisible to its true owner while still
//! joined by key to their plan.
//!
//! # The two edge sets are JSON arrays, where §6 writes `uuid[]`
//!
//! `depends_on_addon_sku_id` and `conflicts_with_addon_sku_id` are §6's
//! plan-authored D-16 edges, and §6 spells both as SQL arrays. Postgres has
//! `uuid[]`; `SQLite` has no array type at all, so a literal reading would
//! leave the two backends holding different things — one an array, the other
//! whatever encoding the mirror invented — and the mirror would stop being a
//! mirror exactly where the cycle walk reads. They are therefore `jsonb` on
//! Postgres and `text` on `SQLite`, holding a JSON array of uuid strings: the
//! same transform this chain already applies to `included_allowance` on
//! `pricing_price`, so both backends hold **one** value in one encoding. The
//! divergence from §6 is stated rather than smoothed over.
//!
//! The columns are `NOT NULL DEFAULT '[]'`, because an add-on rule always has
//! both edge sets and an empty set is what "no edges" is. A nullable column
//! would give the empty set two spellings and would make every reader decide
//! which of them it meant.
//!
//! # `required` implies a `max_qty` that admits a selection
//!
//! `chk_pricing_plan_addon_rule_required_max_qty` is §6's "add-on rule
//! `max_qty >= 1 WHERE required`", verbatim. It is a **per-row** property, so a
//! CHECK can see it — the same split `pricing_price_tier_band` draws, where
//! per-row properties are constraints and set properties are pipeline rules.
//!
//! **Consequence, recorded rather than designed around:** §5 names no code for
//! this rejection, so an author who submits `required` with `max_qty = 0`
//! reaches a driver error rather than a report line. Minting a code for it is
//! forbidden (Global Constraint 3) and stretching a neighbouring one would put
//! a discriminator on the wire that no document defines. The constraint is kept
//! because the alternative is worse: without it a required add-on that can
//! never be taken publishes silently, and the plan is sellable and unbuyable.
//!
//! # Two constraints §6 does not name, and the defect each prevents
//!
//! Both are additions, both are reported as such, and both rest on the same
//! ground as the CHECK above — §5 names no code for either, so the pipeline
//! **cannot** report them at all, and a shape nothing rejects is a shape that
//! publishes.
//!
//! 1. `chk_pricing_plan_addon_rule_qty_range`:
//!    `min_qty IS NULL OR max_qty IS NULL OR min_qty <= max_qty`. An inverted
//!    pair admits no quantity whatsoever, so the add-on is unselectable and the
//!    plan carrying it is publishable and unbuyable — the defect
//!    `inst-cmp-addons` rejects for the both-required conflicting pair, arriving
//!    through the bounds instead of through the graph. The NULL arms keep a
//!    half-authored draft savable: an author may set one bound in one request
//!    and the other in the next, which is how `chk_pricing_plan_purchase_qty`
//!    is written for the plan-level bounds one table up.
//! 2. `chk_pricing_plan_addon_rule_step_qty`: `step_qty IS NULL OR step_qty > 0`.
//!    A step of zero means quantity increments of zero — either every quantity
//!    is admissible or none is, and which one a selection surface picks is
//!    undefined; a negative step names no selection at all.
//!
//! Neither is a rule this slice states, which is exactly why they are here and
//! not in the pipeline: a CHECK states a property of a stored row, and the
//! pipeline may only report what §5 gives it a code for.
//!
//! # No foreign key on the edge columns, the override ref, or the add-on SKU
//!
//! `depends_on_addon_sku_id` and `conflicts_with_addon_sku_id` name **other
//! add-on SKUs of this same plan's set** (D-16), which is a membership property
//! of a set the table cannot see one row at a time; `inst-cmp-addons` owns it as
//! `ADDON_INCOMPATIBLE`, and the repository closes conflicts under symmetry so
//! the two sides of a pair can never disagree. `addon_sku_id` and
//! `price_override_ref` point outside this gear entirely — into the product/SKU
//! registry and into a published price row of the add-on's own plan
//! (`inst-cmp-override-home`, D-97/D-116) — and neither referent is a row this
//! schema holds.
//!
//! # Append-only with its revision (`01-foundation.md` §3.7, the L-2 fix)
//!
//! Identical to `pricing_plan_phase`'s, and identical deliberately: every
//! revision-scoped child table carries the same discipline as its parent, so
//! child rows are physically immutable once **their** revision publishes while a
//! draft revision's copies stay freely mutable and deletable. There is no
//! `lifecycle_state` on this table — the parent revision's is the referent — so
//! the predicate reads `pricing_plan.lifecycle_state = 'draft'` for
//! `(plan_id, plan_revision)`. Without it, the add-on set of a frozen revision
//! could be rewritten under an unchanged `pricing_plan`, and the projector's
//! warm re-drive — which reads truth rows (§4.4) — would quietly re-materialize
//! a frozen `CatalogVersion` at a different composition.
//!
//! INSERT is guarded and not only UPDATE and DELETE, because an INSERT is the
//! one verb that **adds** a rule to a frozen revision. An UPDATE is checked
//! against **both** ends: the `OLD` parent, whose freeze governs the row now,
//! and the `NEW` parent, whose freeze forbids the append — re-pointing a child
//! row's `plan_revision` is precisely how one would otherwise append to a frozen
//! revision without ever issuing an INSERT.
//!
//! **`abandoned` is not `draft`**, so `PlanRepo::abandon_draft` drops these rows
//! **before** it flips the revision, and `PlanRepo::open_revision` copies them
//! **after** it inserts the new revision row. Both orderings are forced by this
//! trigger rather than chosen.
//!
//! **Backend differences.** Postgres carries the rule as one PL/pgSQL trigger
//! function with the offending state interpolated; `SQLite` has no procedural
//! language and `RAISE(ABORT, ...)` takes a literal message, so the same rule
//! becomes three fixed-message triggers, one per DML verb, whose parent lookup
//! is a `WHERE NOT EXISTS` subquery in the trigger **body** — a `SQLite` `WHEN`
//! clause may not contain a subquery. `uuid` becomes `text`, `jsonb` becomes
//! `text`, and the `bss.` qualification is dropped, as elsewhere in this chain.
//! Every CHECK, index, FK and PK is preserved on both sides.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_plan_addon_rule (
        plan_id                    uuid    NOT NULL,
        plan_revision              bigint  NOT NULL,
        addon_sku_id               uuid    NOT NULL,
        tenant_id                  uuid    NOT NULL,
        required                   boolean NOT NULL DEFAULT false,
        min_qty                    int,
        max_qty                    int,
        step_qty                   int,
        price_override_ref         uuid,
        depends_on_addon_sku_id    jsonb   NOT NULL DEFAULT '[]',
        conflicts_with_addon_sku_id jsonb  NOT NULL DEFAULT '[]',
        PRIMARY KEY (plan_id, plan_revision, addon_sku_id),
        -- Section 6, verbatim. A required add-on that admits no quantity is a
        -- plan that is sellable and unbuyable.
        CONSTRAINT chk_pricing_plan_addon_rule_required_max_qty CHECK (
            NOT required OR (max_qty IS NOT NULL AND max_qty >= 1)),
        -- An addition; see the module doc for the defect each prevents.
        CONSTRAINT chk_pricing_plan_addon_rule_qty_range CHECK (
            min_qty IS NULL OR max_qty IS NULL OR min_qty <= max_qty),
        CONSTRAINT chk_pricing_plan_addon_rule_step_qty CHECK (
            step_qty IS NULL OR step_qty > 0),
        CONSTRAINT fk_pricing_plan_addon_rule_revision FOREIGN KEY (plan_id, plan_revision)
            REFERENCES bss.pricing_plan (plan_id, revision)
    )",
    // The copy-forward, the drop-on-abandon and the projector all range over one
    // revision's add-on set under one tenant.
    "CREATE INDEX idx_pricing_plan_addon_rule_revision
        ON bss.pricing_plan_addon_rule (tenant_id, plan_id, plan_revision)",
    // The parent revision's `lifecycle_state` is the add-on rule row's. UPDATE
    // and DELETE consult the OLD parent - the revision the row is bound to now;
    // INSERT and UPDATE consult the NEW parent - the revision it would land
    // under.
    "CREATE OR REPLACE FUNCTION bss.pricing_plan_addon_rule_append_only() RETURNS trigger AS $$
        DECLARE
          parent_state text;
        BEGIN
          IF TG_OP <> 'INSERT' THEN
            SELECT lifecycle_state INTO parent_state
              FROM bss.pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision;
            IF parent_state IS DISTINCT FROM 'draft' THEN
              RAISE EXCEPTION
                'pricing_plan_addon_rule: % of an add-on rule under a % plan revision is not permitted',
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
              'pricing_plan_addon_rule: % of an add-on rule under a % plan revision is not permitted',
              TG_OP, coalesce(parent_state, 'missing');
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_plan_addon_rule_append_only
        BEFORE INSERT OR UPDATE OR DELETE ON bss.pricing_plan_addon_rule
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_plan_addon_rule_append_only()",
];

// This migration puts no trigger on a table it does not own, so dropping the
// table takes every trigger of this migration with it; the function is named
// separately because a Postgres function outlives the table it guarded.
const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_plan_addon_rule",
    "DROP FUNCTION IF EXISTS bss.pricing_plan_addon_rule_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transforms: `bss.` dropped, `uuid` -> `text`, `jsonb` -> `text`,
// and the single PL/pgSQL trigger function split into three fixed-message
// `RAISE(ABORT, ...)` triggers, one per DML verb, whose parent lookup is a
// `WHERE NOT EXISTS` subquery in the trigger *body*. Every CHECK, the FK, the
// index and the PK are preserved.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_plan_addon_rule (
        plan_id                    text    NOT NULL,
        plan_revision              bigint  NOT NULL,
        addon_sku_id               text    NOT NULL,
        tenant_id                  text    NOT NULL,
        required                   boolean NOT NULL DEFAULT false,
        min_qty                    int,
        max_qty                    int,
        step_qty                   int,
        price_override_ref         text,
        depends_on_addon_sku_id    text    NOT NULL DEFAULT '[]',
        conflicts_with_addon_sku_id text   NOT NULL DEFAULT '[]',
        PRIMARY KEY (plan_id, plan_revision, addon_sku_id),
        CONSTRAINT chk_pricing_plan_addon_rule_required_max_qty CHECK (
            NOT required OR (max_qty IS NOT NULL AND max_qty >= 1)),
        CONSTRAINT chk_pricing_plan_addon_rule_qty_range CHECK (
            min_qty IS NULL OR max_qty IS NULL OR min_qty <= max_qty),
        CONSTRAINT chk_pricing_plan_addon_rule_step_qty CHECK (
            step_qty IS NULL OR step_qty > 0),
        CONSTRAINT fk_pricing_plan_addon_rule_revision FOREIGN KEY (plan_id, plan_revision)
            REFERENCES pricing_plan (plan_id, revision)
    )",
    "CREATE INDEX idx_pricing_plan_addon_rule_revision
        ON pricing_plan_addon_rule (tenant_id, plan_id, plan_revision)",
    "CREATE TRIGGER trg_pricing_plan_addon_rule_no_insert
        BEFORE INSERT ON pricing_plan_addon_rule
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_plan_addon_rule: INSERT of an add-on rule under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision
               AND lifecycle_state = 'draft');
        END",
    // Both ends: the revision the row leaves and the revision it lands under.
    "CREATE TRIGGER trg_pricing_plan_addon_rule_no_update
        BEFORE UPDATE ON pricing_plan_addon_rule
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_plan_addon_rule: UPDATE of an add-on rule under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision
               AND lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = NEW.plan_id AND revision = NEW.plan_revision
               AND lifecycle_state = 'draft');
        END",
    "CREATE TRIGGER trg_pricing_plan_addon_rule_no_delete
        BEFORE DELETE ON pricing_plan_addon_rule
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_plan_addon_rule: DELETE of an add-on rule under a non-draft plan revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_plan
             WHERE plan_id = OLD.plan_id AND revision = OLD.plan_revision
               AND lifecycle_state = 'draft');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_plan_addon_rule"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
