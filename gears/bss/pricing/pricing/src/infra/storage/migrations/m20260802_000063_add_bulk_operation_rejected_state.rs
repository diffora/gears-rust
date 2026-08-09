//! `rejected` joins the bulk-operation state machine -- D-267's code half.
//!
//! Section 4 gave the machine six states and one exit from `awaiting_approval`:
//! `-> committing`, on approval. A batch approval that is **refused** therefore
//! had nowhere to put the run. `DELETE` is refused by the table's own trigger, so
//! the record cannot be withdrawn; `uq_pricing_bulk_operation_client_key` is
//! unique per tenant, so the operator cannot resubmit the same work under the
//! same key; and no edge leaves `awaiting_approval` except the one the rejection
//! did not take. A rejected repricing run was stranded in `awaiting_approval`
//! permanently, holding its client key against every retry.
//!
//! D-260 recorded the gap on the day `m20260802_000047` built the machine and
//! left it, deliberately, for a decision. This is that decision built: one
//! state, one edge, and nothing else.
//!
//! # `rejected` is terminal, and it is reachable from exactly one state
//!
//! ```text
//! awaiting_approval -> rejected            (inst-bs-reject)
//! ```
//!
//! No arm names `rejected` on the left of an edge, so the machine leaves it by
//! nothing -- the same way `m20260802_000045` made `abandoned` terminal without
//! a clause naming it, and for the same reason: an edge list that is a whitelist
//! makes every unnamed move a refusal already.
//!
//! **A rejected run is over, so it carries a `completed_at`.** The terminal set
//! in `chk_pricing_bulk_operation_completed_at` gains it beside
//! `validation_failed`, `completed` and `completed_with_conflicts`, which is what
//! keeps the instant and the state from disagreeing about whether the run ended.
//!
//! # D-137 still holds, and it holds without a new clause
//!
//! An import can never be **rejected**, because `rejected` is reachable only from
//! `awaiting_approval` and `chk_pricing_bulk_operation_import_never_awaits`
//! forbids an import that state. The new edge inherits the old `CHECK` rather
//! than restating it: a second constraint saying the same thing would be a rule
//! that can never fail, which is indistinguishable from a rule that holds.
//! Verified behaviourally on both engines rather than argued -- an import walking
//! `validating -> rejected` is refused by the edge list, and one born `rejected`
//! by the born-validating arm.
//!
//! # Why Postgres is five statements and `SQLite` is a rebuild
//!
//! `m20260802_000045`'s asymmetry exactly. Postgres drops and re-adds each of the
//! two **named** constraints and replaces the function whole, because `CREATE OR
//! REPLACE` takes a body rather than a patch. `SQLite` cannot alter a `CHECK` at all, so the
//! table is rebuilt -- and **`DROP TABLE` takes the indexes and the triggers with
//! it**, so both indexes and all four of this table's triggers are recreated
//! after the rename. Leaving one out would silently delete the enforcement this
//! migration exists to widen, and nothing in the fast tier reads a trigger that
//! is merely absent.
//!
//! **Three more triggers have to move, on two other tables.** `ALTER TABLE ...
//! RENAME TO` re-parses the whole schema, so a trigger left dangling by the
//! `DROP TABLE` two statements earlier fails the *rename* rather than waiting to
//! fail at run time: `trg_pricing_repricing_journal_only_under_a_repricing_run`
//! (`m20260802_000048`) and the bulk lock's two parent-reading arms
//! (`m20260802_000049`) each sub-select this table. All three are dropped
//! **before** the rebuild and re-created after it, verbatim from their own
//! migrations. The other five triggers on those two tables name only their own
//! tables and stay put -- checked per trigger body rather than assumed from the
//! table, and the generator asserts the split.
//!
//! Both child tables carry a foreign key to this one. Neither is re-issued and
//! neither needs to be: `SQLite` records a foreign key in the **child's** DDL, and
//! the rename restores the name it points at. `DROP TABLE` fires no row trigger,
//! so the unconditional `trg_pricing_bulk_operation_no_delete` does not refuse the
//! rebuild's own drop -- which is also why that drop must come **after** the
//! `INSERT ... SELECT` that copies the rows out.
//!
//! # How the restatement was produced, and what checked it
//!
//! `m20260802_000051`'s rule and `m20260802_000058`'s procedure: **not by hand
//! and not by a free-form script.** Every block below was read out of
//! `m20260802_000047`'s own `UP` text (and the three sibling triggers out of
//! `m20260802_000048`/`m20260802_000049`), and the generator asserted, before
//! writing anything, that it had found **exactly one** arm per engine, that the
//! two edge lists each grew by **exactly one** line with every original line
//! surviving, that each of the four `CHECK` vocabularies replaced **exactly one**
//! named line and kept the rest, that the new state appears exactly once per
//! block that gains it and **not at all** in the three triggers copied
//! unchanged, that the copy names all nine columns read off the table, and that
//! the seven triggers naming this table across the three source migrations are
//! exactly the seven accounted for here.
//!
//! # `down` restores the six-state machine, and will refuse a `rejected` row
//!
//! `m20260802_000045`'s posture, for its reason: a rollback with a rejected run
//! present fails the restored `CHECK` on Postgres and the rebuild's
//! `INSERT ... SELECT` on `SQLite`. That is correct -- the row is the record that
//! an approval was refused, and a rollback that quietly dropped it would strand
//! the operator's client key with nothing saying why.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres - the named constraints re-added and the function restated whole.
// ---------------------------------------------------------------------------

/// The state machine restated whole. One change against
/// `m20260802_000047`: the `awaiting_approval` disjunct gains its second
/// edge.
const PG_TRANSITIONS_WITH_REJECTED: &str =
    "CREATE OR REPLACE FUNCTION bss.pricing_bulk_operation_transitions() RETURNS trigger AS $$
        BEGIN
          -- **A run is born `validating` and in no other state.** -4 names it the
          -- initial state, and without this arm the whole machine is a rule about
          -- UPDATE with a row free to be born `committing` -- past the approval
          -- gate transitions 2 and 3 exist to impose - or born terminal,
          -- reporting outcomes for rows it never committed. `m20260802_000015`
          -- carries the same arm for the same reason, in the same words.
          IF TG_OP = 'INSERT' THEN
            IF NEW.state <> 'validating' THEN
              RAISE EXCEPTION
                'pricing_bulk_operation: a run is born validating, not %', NEW.state;
            END IF;
            RETURN NEW;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_bulk_operation: DELETE of operation % is not permitted; a run is a record, not a draft',
              OLD.operation_id;
          END IF;

          -- Identity and provenance are frozen; only the run's progress moves.
          IF NEW.operation_id IS DISTINCT FROM OLD.operation_id
          OR NEW.tenant_id    IS DISTINCT FROM OLD.tenant_id
          OR NEW.kind         IS DISTINCT FROM OLD.kind
          OR NEW.client_key   IS DISTINCT FROM OLD.client_key
          OR NEW.submitted_by IS DISTINCT FROM OLD.submitted_by
          OR NEW.submitted_at IS DISTINCT FROM OLD.submitted_at THEN
            RAISE EXCEPTION
              'pricing_bulk_operation: operation % is frozen; only state, report and completed_at move',
              OLD.operation_id;
          END IF;

          IF NEW.state = OLD.state THEN
            RETURN NEW;
          END IF;

          -- Section 4's edges, and nothing else.
          IF NOT (
               (OLD.state = 'validating'        AND NEW.state IN ('validation_failed', 'awaiting_approval', 'committing'))
            OR (OLD.state = 'awaiting_approval' AND NEW.state = 'committing')
            OR (OLD.state = 'awaiting_approval' AND NEW.state = 'rejected')
            OR (OLD.state = 'committing'        AND NEW.state IN ('completed', 'completed_with_conflicts'))
          ) THEN
            RAISE EXCEPTION
              'pricing_bulk_operation: state % -> % is not an edge of the bulk state machine',
              OLD.state, NEW.state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql";

/// `m20260802_000047`'s function verbatim, for the rollback.
const PG_TRANSITIONS_WITHOUT_REJECTED: &str =
    "CREATE OR REPLACE FUNCTION bss.pricing_bulk_operation_transitions() RETURNS trigger AS $$
        BEGIN
          -- **A run is born `validating` and in no other state.** -4 names it the
          -- initial state, and without this arm the whole machine is a rule about
          -- UPDATE with a row free to be born `committing` -- past the approval
          -- gate transitions 2 and 3 exist to impose - or born terminal,
          -- reporting outcomes for rows it never committed. `m20260802_000015`
          -- carries the same arm for the same reason, in the same words.
          IF TG_OP = 'INSERT' THEN
            IF NEW.state <> 'validating' THEN
              RAISE EXCEPTION
                'pricing_bulk_operation: a run is born validating, not %', NEW.state;
            END IF;
            RETURN NEW;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_bulk_operation: DELETE of operation % is not permitted; a run is a record, not a draft',
              OLD.operation_id;
          END IF;

          -- Identity and provenance are frozen; only the run's progress moves.
          IF NEW.operation_id IS DISTINCT FROM OLD.operation_id
          OR NEW.tenant_id    IS DISTINCT FROM OLD.tenant_id
          OR NEW.kind         IS DISTINCT FROM OLD.kind
          OR NEW.client_key   IS DISTINCT FROM OLD.client_key
          OR NEW.submitted_by IS DISTINCT FROM OLD.submitted_by
          OR NEW.submitted_at IS DISTINCT FROM OLD.submitted_at THEN
            RAISE EXCEPTION
              'pricing_bulk_operation: operation % is frozen; only state, report and completed_at move',
              OLD.operation_id;
          END IF;

          IF NEW.state = OLD.state THEN
            RETURN NEW;
          END IF;

          -- Section 4's edges, and nothing else.
          IF NOT (
               (OLD.state = 'validating'        AND NEW.state IN ('validation_failed', 'awaiting_approval', 'committing'))
            OR (OLD.state = 'awaiting_approval' AND NEW.state = 'committing')
            OR (OLD.state = 'committing'        AND NEW.state IN ('completed', 'completed_with_conflicts'))
          ) THEN
            RAISE EXCEPTION
              'pricing_bulk_operation: state % -> % is not an edge of the bulk state machine',
              OLD.state, NEW.state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql";

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_bulk_operation
        DROP CONSTRAINT chk_pricing_bulk_operation_state",
    "ALTER TABLE bss.pricing_bulk_operation
        ADD CONSTRAINT chk_pricing_bulk_operation_state CHECK (
            state IN ('validating', 'validation_failed', 'awaiting_approval',
                      'committing', 'completed', 'completed_with_conflicts',
                      'rejected'))",
    "ALTER TABLE bss.pricing_bulk_operation
        DROP CONSTRAINT chk_pricing_bulk_operation_completed_at",
    "ALTER TABLE bss.pricing_bulk_operation
        ADD CONSTRAINT chk_pricing_bulk_operation_completed_at CHECK (
            (completed_at IS NOT NULL) =
            (state IN ('validation_failed', 'completed', 'completed_with_conflicts',
                       'rejected')))",
    PG_TRANSITIONS_WITH_REJECTED,
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    PG_TRANSITIONS_WITHOUT_REJECTED,
    "ALTER TABLE bss.pricing_bulk_operation
        DROP CONSTRAINT chk_pricing_bulk_operation_completed_at",
    "ALTER TABLE bss.pricing_bulk_operation
        ADD CONSTRAINT chk_pricing_bulk_operation_completed_at CHECK (
            (completed_at IS NOT NULL) =
            (state IN ('validation_failed', 'completed', 'completed_with_conflicts')))",
    "ALTER TABLE bss.pricing_bulk_operation
        DROP CONSTRAINT chk_pricing_bulk_operation_state",
    "ALTER TABLE bss.pricing_bulk_operation
        ADD CONSTRAINT chk_pricing_bulk_operation_state CHECK (
            state IN ('validating', 'validation_failed', 'awaiting_approval',
                      'committing', 'completed', 'completed_with_conflicts'))",
];

// ---------------------------------------------------------------------------
// SQLite - one rebuild, because a `CHECK` cannot be altered. The table text is
// `m20260802_000047`'s with the two vocabularies widened, and both indexes, all
// four of this table's triggers and the three sibling triggers that read it are
// recreated because `DROP TABLE` and the rename between them take every one.
// ---------------------------------------------------------------------------

macro_rules! sqlite_rebuild {
    ($table:literal, $transitions:literal) => {
        &[
            // The three dangling-trigger drops come first. See the module doc:
            // the rename re-parses the schema and would fail on any of them.
            "DROP TRIGGER IF EXISTS trg_pricing_repricing_journal_only_under_a_repricing_run",
            "DROP TRIGGER IF EXISTS trg_pricing_bulk_row_lock_same_tenant_as_its_run",
            "DROP TRIGGER IF EXISTS trg_pricing_bulk_row_lock_only_while_committing",
            $table,
            "INSERT INTO pricing_bulk_operation_rebuilt (
        operation_id, tenant_id, kind, state, client_key, report, submitted_by, submitted_at, completed_at)
     SELECT
        operation_id, tenant_id, kind, state, client_key, report, submitted_by, submitted_at, completed_at
     FROM pricing_bulk_operation",
            "DROP TABLE pricing_bulk_operation",
            "ALTER TABLE pricing_bulk_operation_rebuilt RENAME TO pricing_bulk_operation",
            "CREATE UNIQUE INDEX uq_pricing_bulk_operation_client_key
        ON pricing_bulk_operation (tenant_id, client_key)",
            "CREATE INDEX idx_pricing_bulk_operation_live
        ON pricing_bulk_operation (tenant_id, state, submitted_at)",
            "CREATE TRIGGER trg_pricing_bulk_operation_born_validating
        BEFORE INSERT ON pricing_bulk_operation
        FOR EACH ROW WHEN NEW.state <> 'validating'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bulk_operation: a run is born validating and in no other state');
        END",
            "CREATE TRIGGER trg_pricing_bulk_operation_no_delete
        BEFORE DELETE ON pricing_bulk_operation
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bulk_operation: DELETE of an operation is not permitted; a run is a record, not a draft');
        END",
            "CREATE TRIGGER trg_pricing_bulk_operation_frozen_columns
        BEFORE UPDATE ON pricing_bulk_operation
        FOR EACH ROW WHEN NEW.operation_id IS NOT OLD.operation_id
          OR NEW.tenant_id    IS NOT OLD.tenant_id
          OR NEW.kind         IS NOT OLD.kind
          OR NEW.client_key   IS NOT OLD.client_key
          OR NEW.submitted_by IS NOT OLD.submitted_by
          OR NEW.submitted_at IS NOT OLD.submitted_at
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bulk_operation: the operation is frozen; only state, report and completed_at move');
        END",
            $transitions,
            // --- the three, restated verbatim from `000048` and `000049` ---
            "CREATE TRIGGER trg_pricing_repricing_journal_only_under_a_repricing_run
        BEFORE INSERT ON pricing_repricing_journal
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_repricing_journal: only a repricing run journals per-row progress')
          WHERE EXISTS (
            SELECT 1 FROM pricing_bulk_operation WHERE operation_id = NEW.run_id)
            AND NOT EXISTS (
            SELECT 1 FROM pricing_bulk_operation
             WHERE operation_id = NEW.run_id AND kind = 'repricing');
        END",
            "CREATE TRIGGER trg_pricing_bulk_row_lock_same_tenant_as_its_run
        BEFORE INSERT ON pricing_bulk_row_lock
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bulk_row_lock: the operation belongs to another tenant and may not lock this row')
          WHERE EXISTS (
            SELECT 1 FROM pricing_bulk_operation
             WHERE operation_id = NEW.bulk_operation_id)
            AND NOT EXISTS (
            SELECT 1 FROM pricing_bulk_operation
             WHERE operation_id = NEW.bulk_operation_id
               AND tenant_id = NEW.tenant_id);
        END",
            "CREATE TRIGGER trg_pricing_bulk_row_lock_only_while_committing
        BEFORE INSERT ON pricing_bulk_row_lock
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bulk_row_lock: the bulk lock takes effect only on entry to committing')
          WHERE EXISTS (
            SELECT 1 FROM pricing_bulk_operation
             WHERE operation_id = NEW.bulk_operation_id)
            AND NOT EXISTS (
            SELECT 1 FROM pricing_bulk_operation
             WHERE operation_id = NEW.bulk_operation_id
               AND state = 'committing');
        END",
        ]
    };
}

const SQLITE_UP_STATEMENTS: &[&str] = sqlite_rebuild!(
    "CREATE TABLE pricing_bulk_operation_rebuilt (
        operation_id  text   NOT NULL PRIMARY KEY,
        tenant_id     text   NOT NULL,
        kind          text   NOT NULL,
        state         text   NOT NULL,
        client_key    text   NOT NULL,
        report        text   NOT NULL DEFAULT '{}',
        submitted_by  text   NOT NULL,
        submitted_at  text   NOT NULL,
        completed_at  text,
        CONSTRAINT chk_pricing_bulk_operation_kind CHECK (
            kind IN ('import', 'repricing')),
        CONSTRAINT chk_pricing_bulk_operation_state CHECK (
            state IN ('validating', 'validation_failed', 'awaiting_approval',
                      'committing', 'completed', 'completed_with_conflicts',
                      'rejected')),
        CONSTRAINT chk_pricing_bulk_operation_import_never_awaits CHECK (
            NOT (kind = 'import' AND state = 'awaiting_approval')),
        CONSTRAINT chk_pricing_bulk_operation_completed_at CHECK (
            (completed_at IS NOT NULL) =
            (state IN ('validation_failed', 'completed', 'completed_with_conflicts',
                       'rejected')))
    )",
    "CREATE TRIGGER trg_pricing_bulk_operation_transitions
        BEFORE UPDATE ON pricing_bulk_operation
        FOR EACH ROW WHEN NEW.state IS NOT OLD.state
          AND NOT (
               (OLD.state = 'validating'
                AND NEW.state IN ('validation_failed', 'awaiting_approval', 'committing'))
            OR (OLD.state = 'awaiting_approval' AND NEW.state = 'committing')
            OR (OLD.state = 'awaiting_approval' AND NEW.state = 'rejected')
            OR (OLD.state = 'committing'
                AND NEW.state IN ('completed', 'completed_with_conflicts')))
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bulk_operation: that state move is not an edge of the bulk state machine');
        END"
);

const SQLITE_DOWN_STATEMENTS: &[&str] = sqlite_rebuild!(
    "CREATE TABLE pricing_bulk_operation_rebuilt (
        operation_id  text   NOT NULL PRIMARY KEY,
        tenant_id     text   NOT NULL,
        kind          text   NOT NULL,
        state         text   NOT NULL,
        client_key    text   NOT NULL,
        report        text   NOT NULL DEFAULT '{}',
        submitted_by  text   NOT NULL,
        submitted_at  text   NOT NULL,
        completed_at  text,
        CONSTRAINT chk_pricing_bulk_operation_kind CHECK (
            kind IN ('import', 'repricing')),
        CONSTRAINT chk_pricing_bulk_operation_state CHECK (
            state IN ('validating', 'validation_failed', 'awaiting_approval',
                      'committing', 'completed', 'completed_with_conflicts')),
        CONSTRAINT chk_pricing_bulk_operation_import_never_awaits CHECK (
            NOT (kind = 'import' AND state = 'awaiting_approval')),
        CONSTRAINT chk_pricing_bulk_operation_completed_at CHECK (
            (completed_at IS NOT NULL) =
            (state IN ('validation_failed', 'completed', 'completed_with_conflicts')))
    )",
    "CREATE TRIGGER trg_pricing_bulk_operation_transitions
        BEFORE UPDATE ON pricing_bulk_operation
        FOR EACH ROW WHEN NEW.state IS NOT OLD.state
          AND NOT (
               (OLD.state = 'validating'
                AND NEW.state IN ('validation_failed', 'awaiting_approval', 'committing'))
            OR (OLD.state = 'awaiting_approval' AND NEW.state = 'committing')
            OR (OLD.state = 'committing'
                AND NEW.state IN ('completed', 'completed_with_conflicts')))
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bulk_operation: that state move is not an edge of the bulk state machine');
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
