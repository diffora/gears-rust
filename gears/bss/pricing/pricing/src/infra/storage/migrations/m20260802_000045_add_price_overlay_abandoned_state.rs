//! `abandoned` joins the overlay's lifecycle — D-231's code half.
//!
//! §6 gave `pricing_price_overlay` three states and no tombstone, so a discarded
//! draft revision left by `DELETE` and the number it consumed went with it. The
//! next `open_revision` then re-minted that number, and a client still holding
//! the discarded draft's entity tag found a **different** revision under the same
//! overlay identity — its stale `If-Match` matching the fresh row's
//! compare-and-swap and landing an edit against content that no longer exists.
//!
//! `pricing_plan` has had the answer since D-145 and this is deliberately its
//! shape, not a variation on it: a terminal `draft -> abandoned` flip, `DELETE`
//! refused outright, and a number that is never freed.
//!
//! # What actually closes the hazard is the `DELETE` refusal, not the new state
//!
//! Adding `abandoned` to the `CHECK` alone would leave `DELETE` available and the
//! re-mint reachable by the path it already took. So the flip is *sanctioned* and
//! the deletion is *removed*, in one migration, and `open_revision`'s
//! `max(revision) + 1` becomes a true high-water read for the first time — it was
//! already written that way (a `superseded` predecessor can outrank the published
//! row, which is its own smaller reason) and simply could not see a deleted row.
//!
//! # `abandoned` is terminal, and it costs no new arm to make it so
//!
//! The post-draft branch already refuses every flip except `published ->
//! superseded`, so a row that reaches `abandoned` is frozen in content by the
//! column whitelist and left by no flip — without a clause naming `abandoned` at
//! all. The only edit the state machine needs is the draft's **exit** list, which
//! is why the diff is smaller than the rule.
//!
//! # Why the open-draft index needs no predicate change
//!
//! `uq_pricing_price_overlay_open_draft` is partial on `lifecycle_state =
//! 'draft'`. An abandoned row is not a draft, so it leaves the index the moment it
//! flips and a fresh draft opens against the same overlay — which is the whole
//! mechanism: the tombstone occupies a **revision number** without occupying the
//! *open draft* slot. Had the index been unconditional, a tombstone would have
//! blocked every future revision.
//!
//! # Why `SQLite` is a rebuild, and why it moves **ten** triggers across three
//! tables
//!
//! Postgres drops and re-adds a named constraint, and does. `SQLite` cannot alter
//! a `CHECK` at all, so the table is rebuilt on `m20260802_000060`'s shape — and a
//! rebuild restates the table by hand, which means **`DROP TABLE` takes the
//! indexes and the triggers with it**. All three indexes and this table's own four
//! triggers are recreated after the rename. Leaving one out would silently delete
//! the append-only enforcement this migration exists to tighten, and nothing in
//! the fast tier reads a trigger that is merely absent.
//!
//! **Six more triggers have to move, on two other tables**, and `SQLite` said so
//! at the rename rather than later:
//!
//! ```text
//! error in trigger trg_pricing_price_overlay_line_no_insert:
//!   no such table: main.pricing_price_overlay
//! ```
//!
//! `ALTER TABLE … RENAME TO` **re-parses the whole schema** (since 3.25, with
//! `legacy_alter_table` off), so a trigger left dangling by the `DROP TABLE` two
//! statements earlier fails the rename instead of waiting to fail at run time.
//! The three `pricing_price_overlay_line` triggers (`000033`) and the three
//! `pricing_price_overlay_line_amount` triggers (`000034`) each sub-select this
//! table's `lifecycle_state`, so all six are dropped **before** the rebuild and
//! re-created after it, verbatim from their own migrations. The two
//! `…_append_only` triggers on those tables name only their own table and stay put
//! — checked per trigger body rather than assumed from the table.
//!
//! This is `m20260802_000019`'s lesson exactly, which moved eight triggers across
//! two tables for the same reason and recorded that **a rebuild's census cannot
//! stop at the table being rebuilt**. `PRAGMA legacy_alter_table` is the other way
//! out and is not taken here for the reason that migration gives: it would leave
//! the six dangling triggers in the schema to fail on the next write instead,
//! which is the same defect discovered later by a caller.
//!
//! `DROP TABLE` fires no row trigger in `SQLite`, so the unconditional
//! `trg_pricing_price_overlay_no_delete` this migration installs does not refuse
//! the rebuild's own drop — the one interaction that would make this unrunnable
//! rather than merely wrong. It is also why the drop must come **after** the
//! `INSERT … SELECT` that copies the rows out.
//!
//! # `down` is not symmetric, deliberately
//!
//! Rolling back with an `abandoned` row present fails the restored `CHECK` on
//! Postgres and the rebuild's `INSERT … SELECT` on `SQLite`. That is correct: the
//! row is the record that a revision number was consumed, and a rollback that
//! quietly dropped it would re-open the exact hazard — with a client's stale tag
//! still in flight. A rollback past this point is a decision about discarded
//! revisions, not a schema step.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres - canonical production schema.
// ---------------------------------------------------------------------------

/// The append-only function, restated whole because `CREATE OR REPLACE` takes a
/// body rather than a patch. Two changes against `m20260802_000032`: `DELETE` is
/// refused for every state rather than for every state but `draft`, and the
/// draft's exit list gains `abandoned`.
const PG_APPEND_ONLY_WITH_ABANDONED: &str =
    "CREATE OR REPLACE FUNCTION bss.pricing_price_overlay_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_price_overlay: DELETE of revision % of overlay % is not permitted; a discarded draft revision is abandoned',
              OLD.revision, OLD.price_overlay_id;
          END IF;

          -- The draft plane is where content moves, so its columns are
          -- unguarded - but its exits are not. A draft leaves by publishing or by
          -- being abandoned (D-231), and by nothing else; `draft -> superseded`
          -- would mint a superseded row that never published, which the projector
          -- would then source from.
          IF OLD.lifecycle_state = 'draft' THEN
            IF NEW.lifecycle_state NOT IN ('draft', 'published', 'abandoned') THEN
              RAISE EXCEPTION
                'pricing_price_overlay: lifecycle_state % -> % is not a sanctioned flip',
                OLD.lifecycle_state, NEW.lifecycle_state;
            END IF;
            RETURN NEW;
          END IF;

          -- Past here the row is published, superseded or abandoned. Once
          -- abandoned it is a tombstone: frozen in content by the whitelist below
          -- and left by no flip, so the number it consumed can never be attached
          -- to a different shape.

          IF NEW.price_overlay_id IS DISTINCT FROM OLD.price_overlay_id
          OR NEW.revision         IS DISTINCT FROM OLD.revision
          OR NEW.tenant_id        IS DISTINCT FROM OLD.tenant_id
          OR NEW.scope_class      IS DISTINCT FROM OLD.scope_class
          OR NEW.scope_value      IS DISTINCT FROM OLD.scope_value
          OR NEW.precedence       IS DISTINCT FROM OLD.precedence
          OR NEW.effective_from   IS DISTINCT FROM OLD.effective_from
          OR NEW.effective_to     IS DISTINCT FROM OLD.effective_to
          OR NEW.tax_basis        IS DISTINCT FROM OLD.tax_basis
          OR NEW.disclosure       IS DISTINCT FROM OLD.disclosure
          OR NEW.target_ref       IS DISTINCT FROM OLD.target_ref
          OR NEW.row_version      IS DISTINCT FROM OLD.row_version THEN
            RAISE EXCEPTION
              'pricing_price_overlay: revision % of overlay % is frozen; only a sanctioned lifecycle_state flip is permitted',
              OLD.revision, OLD.price_overlay_id;
          END IF;

          IF NEW.lifecycle_state IS DISTINCT FROM OLD.lifecycle_state
             AND NOT (OLD.lifecycle_state = 'published'
                      AND NEW.lifecycle_state = 'superseded') THEN
            RAISE EXCEPTION
              'pricing_price_overlay: lifecycle_state % -> % is not a sanctioned flip',
              OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql";

/// `m20260802_000032`'s function verbatim, for the rollback.
const PG_APPEND_ONLY_WITHOUT_ABANDONED: &str =
    "CREATE OR REPLACE FUNCTION bss.pricing_price_overlay_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            IF OLD.lifecycle_state <> 'draft' THEN
              RAISE EXCEPTION
                'pricing_price_overlay: DELETE of revision % of overlay % is not permitted; it is %',
                OLD.revision, OLD.price_overlay_id, OLD.lifecycle_state;
            END IF;
            RETURN OLD;
          END IF;

          IF OLD.lifecycle_state = 'draft' THEN
            IF NEW.lifecycle_state NOT IN ('draft', 'published') THEN
              RAISE EXCEPTION
                'pricing_price_overlay: lifecycle_state % -> % is not a sanctioned flip',
                OLD.lifecycle_state, NEW.lifecycle_state;
            END IF;
            RETURN NEW;
          END IF;

          IF NEW.price_overlay_id IS DISTINCT FROM OLD.price_overlay_id
          OR NEW.revision         IS DISTINCT FROM OLD.revision
          OR NEW.tenant_id        IS DISTINCT FROM OLD.tenant_id
          OR NEW.scope_class      IS DISTINCT FROM OLD.scope_class
          OR NEW.scope_value      IS DISTINCT FROM OLD.scope_value
          OR NEW.precedence       IS DISTINCT FROM OLD.precedence
          OR NEW.effective_from   IS DISTINCT FROM OLD.effective_from
          OR NEW.effective_to     IS DISTINCT FROM OLD.effective_to
          OR NEW.tax_basis        IS DISTINCT FROM OLD.tax_basis
          OR NEW.disclosure       IS DISTINCT FROM OLD.disclosure
          OR NEW.target_ref       IS DISTINCT FROM OLD.target_ref
          OR NEW.row_version      IS DISTINCT FROM OLD.row_version THEN
            RAISE EXCEPTION
              'pricing_price_overlay: revision % of overlay % is frozen; only a sanctioned lifecycle_state flip is permitted',
              OLD.revision, OLD.price_overlay_id;
          END IF;

          IF NEW.lifecycle_state IS DISTINCT FROM OLD.lifecycle_state
             AND NOT (OLD.lifecycle_state = 'published'
                      AND NEW.lifecycle_state = 'superseded') THEN
            RAISE EXCEPTION
              'pricing_price_overlay: lifecycle_state % -> % is not a sanctioned flip',
              OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql";

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price_overlay
        DROP CONSTRAINT chk_pricing_price_overlay_lifecycle_state",
    "ALTER TABLE bss.pricing_price_overlay
        ADD CONSTRAINT chk_pricing_price_overlay_lifecycle_state CHECK (
            lifecycle_state IN ('draft', 'published', 'superseded', 'abandoned'))",
    PG_APPEND_ONLY_WITH_ABANDONED,
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    PG_APPEND_ONLY_WITHOUT_ABANDONED,
    "ALTER TABLE bss.pricing_price_overlay
        DROP CONSTRAINT chk_pricing_price_overlay_lifecycle_state",
    "ALTER TABLE bss.pricing_price_overlay
        ADD CONSTRAINT chk_pricing_price_overlay_lifecycle_state CHECK (
            lifecycle_state IN ('draft', 'published', 'superseded'))",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// One rebuild. The table text is `m20260802_000032`'s, restated by hand with the
// one state added, and all three indexes **and all four triggers** recreated
// because `DROP TABLE` took them.

macro_rules! sqlite_rebuild {
    ($states:literal, $draft_exits:literal, $delete_guard:literal) => {
        &[
            // The six dangling-trigger drops come first. See the module doc: the
            // rename re-parses the schema and would fail on any of them.
            "DROP TRIGGER IF EXISTS trg_pricing_price_overlay_line_no_insert",
            "DROP TRIGGER IF EXISTS trg_pricing_price_overlay_line_no_update",
            "DROP TRIGGER IF EXISTS trg_pricing_price_overlay_line_no_delete",
            "DROP TRIGGER IF EXISTS trg_pricing_price_overlay_line_amount_no_insert",
            "DROP TRIGGER IF EXISTS trg_pricing_price_overlay_line_amount_no_update",
            "DROP TRIGGER IF EXISTS trg_pricing_price_overlay_line_amount_no_delete",
            concat!(
                "CREATE TABLE pricing_price_overlay_rebuilt (
        price_overlay_id text   NOT NULL,
        revision         bigint NOT NULL,
        tenant_id        text   NOT NULL,
        lifecycle_state  text   NOT NULL,
        scope_class      text   NOT NULL,
        scope_value      text   NOT NULL,
        precedence       int    NOT NULL,
        effective_from   text,
        effective_to     text,
        tax_basis        text   NOT NULL,
        disclosure       text   NOT NULL DEFAULT 'restricted',
        target_ref       text   NOT NULL DEFAULT '{}',
        row_version      bigint NOT NULL DEFAULT 0,
        PRIMARY KEY (price_overlay_id, revision),
        CONSTRAINT chk_pricing_price_overlay_lifecycle_state CHECK (
            lifecycle_state IN (",
                $states,
                ")),
        CONSTRAINT chk_pricing_price_overlay_revision CHECK (revision >= 0),
        CONSTRAINT chk_pricing_price_overlay_scope_class CHECK (
            scope_class IN (
                'partner', 'org_tier', 'brand', 'region', 'customer_group', 'global')),
        CONSTRAINT chk_pricing_price_overlay_scope_value CHECK (
            (scope_class = 'global') = (length(scope_value) = 0)),
        CONSTRAINT chk_pricing_price_overlay_tax_basis CHECK (
            tax_basis IN ('inclusive', 'exclusive', 'delegated_tariffs')),
        CONSTRAINT chk_pricing_price_overlay_disclosure CHECK (
            disclosure IN ('restricted', 'public')),
        CONSTRAINT chk_pricing_price_overlay_interval CHECK (
            effective_from IS NULL OR effective_to IS NULL
            OR effective_to > effective_from)
    )"
            ),
            "INSERT INTO pricing_price_overlay_rebuilt (
        price_overlay_id, revision, tenant_id, lifecycle_state, scope_class,
        scope_value, precedence, effective_from, effective_to, tax_basis,
        disclosure, target_ref, row_version)
     SELECT
        price_overlay_id, revision, tenant_id, lifecycle_state, scope_class,
        scope_value, precedence, effective_from, effective_to, tax_basis,
        disclosure, target_ref, row_version
     FROM pricing_price_overlay",
            "DROP TABLE pricing_price_overlay",
            "ALTER TABLE pricing_price_overlay_rebuilt RENAME TO pricing_price_overlay",
            "CREATE UNIQUE INDEX uq_pricing_price_overlay_precedence
        ON pricing_price_overlay (tenant_id, scope_class, precedence)
        WHERE lifecycle_state = 'published'",
            "CREATE UNIQUE INDEX uq_pricing_price_overlay_open_draft
        ON pricing_price_overlay (price_overlay_id)
        WHERE lifecycle_state = 'draft'",
            "CREATE INDEX idx_pricing_price_overlay_scope
        ON pricing_price_overlay (tenant_id, scope_class, scope_value, lifecycle_state)",
            "CREATE TRIGGER trg_pricing_price_overlay_frozen_columns
        BEFORE UPDATE ON pricing_price_overlay
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND (NEW.price_overlay_id IS NOT OLD.price_overlay_id
            OR NEW.revision         IS NOT OLD.revision
            OR NEW.tenant_id        IS NOT OLD.tenant_id
            OR NEW.scope_class      IS NOT OLD.scope_class
            OR NEW.scope_value      IS NOT OLD.scope_value
            OR NEW.precedence       IS NOT OLD.precedence
            OR NEW.effective_from   IS NOT OLD.effective_from
            OR NEW.effective_to     IS NOT OLD.effective_to
            OR NEW.tax_basis        IS NOT OLD.tax_basis
            OR NEW.disclosure       IS NOT OLD.disclosure
            OR NEW.target_ref       IS NOT OLD.target_ref
            OR NEW.row_version      IS NOT OLD.row_version)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay: the revision is frozen; only a sanctioned lifecycle_state flip is permitted');
        END",
            concat!(
                "CREATE TRIGGER trg_pricing_price_overlay_draft_exit
        BEFORE UPDATE ON pricing_price_overlay
        FOR EACH ROW WHEN OLD.lifecycle_state = 'draft'
          AND NEW.lifecycle_state NOT IN (",
                $draft_exits,
                ")
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay: that lifecycle_state move is not a sanctioned flip');
        END"
            ),
            "CREATE TRIGGER trg_pricing_price_overlay_frozen_flip
        BEFORE UPDATE ON pricing_price_overlay
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND NEW.lifecycle_state IS NOT OLD.lifecycle_state
          AND NOT (OLD.lifecycle_state = 'published'
                   AND NEW.lifecycle_state = 'superseded')
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay: that lifecycle_state move is not a sanctioned flip');
        END",
            $delete_guard,
            // --- the six, restated verbatim from `000033` and `000034` ---
            "CREATE TRIGGER trg_pricing_price_overlay_line_no_insert
        BEFORE INSERT ON pricing_price_overlay_line
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line: INSERT of a line under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay o
             WHERE o.price_overlay_id = NEW.price_overlay_id
               AND o.revision = NEW.overlay_revision
               AND o.lifecycle_state = 'draft');
        END",
            "CREATE TRIGGER trg_pricing_price_overlay_line_no_update
        BEFORE UPDATE ON pricing_price_overlay_line
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line: UPDATE of a line under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay o
             WHERE o.price_overlay_id = OLD.price_overlay_id
               AND o.revision = OLD.overlay_revision
               AND o.lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay o
             WHERE o.price_overlay_id = NEW.price_overlay_id
               AND o.revision = NEW.overlay_revision
               AND o.lifecycle_state = 'draft');
        END",
            "CREATE TRIGGER trg_pricing_price_overlay_line_no_delete
        BEFORE DELETE ON pricing_price_overlay_line
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line: DELETE of a line under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay o
             WHERE o.price_overlay_id = OLD.price_overlay_id
               AND o.revision = OLD.overlay_revision
               AND o.lifecycle_state = 'draft');
        END",
            "CREATE TRIGGER trg_pricing_price_overlay_line_amount_no_insert
        BEFORE INSERT ON pricing_price_overlay_line_amount
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line_amount: INSERT of a value under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay_line l
              JOIN pricing_price_overlay o
                ON o.price_overlay_id = l.price_overlay_id
               AND o.revision = l.overlay_revision
             WHERE l.line_id = NEW.line_id AND l.overlay_revision = NEW.overlay_revision
               AND o.lifecycle_state = 'draft');
        END",
            "CREATE TRIGGER trg_pricing_price_overlay_line_amount_no_update
        BEFORE UPDATE ON pricing_price_overlay_line_amount
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line_amount: UPDATE of a value under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay_line l
              JOIN pricing_price_overlay o
                ON o.price_overlay_id = l.price_overlay_id
               AND o.revision = l.overlay_revision
             WHERE l.line_id = OLD.line_id AND l.overlay_revision = OLD.overlay_revision
               AND o.lifecycle_state = 'draft')
             OR NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay_line l
              JOIN pricing_price_overlay o
                ON o.price_overlay_id = l.price_overlay_id
               AND o.revision = l.overlay_revision
             WHERE l.line_id = NEW.line_id AND l.overlay_revision = NEW.overlay_revision
               AND o.lifecycle_state = 'draft');
        END",
            "CREATE TRIGGER trg_pricing_price_overlay_line_amount_no_delete
        BEFORE DELETE ON pricing_price_overlay_line_amount
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay_line_amount: DELETE of a value under a non-draft overlay revision is not permitted')
          WHERE NOT EXISTS (
            SELECT 1 FROM pricing_price_overlay_line l
              JOIN pricing_price_overlay o
                ON o.price_overlay_id = l.price_overlay_id
               AND o.revision = l.overlay_revision
             WHERE l.line_id = OLD.line_id AND l.overlay_revision = OLD.overlay_revision
               AND o.lifecycle_state = 'draft');
        END",
        ]
    };
}

const SQLITE_UP_STATEMENTS: &[&str] = sqlite_rebuild!(
    "'draft', 'published', 'superseded', 'abandoned'",
    "'draft', 'published', 'abandoned'",
    // Unconditional: D-231's whole point is that a discarded draft does not leave
    // by DELETE. The `WHEN OLD.lifecycle_state <> 'draft'` this replaces is what
    // let the re-mint happen.
    "CREATE TRIGGER trg_pricing_price_overlay_no_delete
        BEFORE DELETE ON pricing_price_overlay
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay: DELETE of a revision is not permitted; a discarded draft revision is abandoned');
        END"
);

const SQLITE_DOWN_STATEMENTS: &[&str] = sqlite_rebuild!(
    "'draft', 'published', 'superseded'",
    "'draft', 'published'",
    "CREATE TRIGGER trg_pricing_price_overlay_no_delete
        BEFORE DELETE ON pricing_price_overlay
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay: DELETE of a revision that is not a draft is not permitted');
        END"
);

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
