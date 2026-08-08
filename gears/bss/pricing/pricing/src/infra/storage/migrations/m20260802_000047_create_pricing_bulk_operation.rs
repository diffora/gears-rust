//! `pricing_bulk_operation` — Slice 12's bulk-operation record and the state
//! machine §4 gives it (`design/12-operator-efficiency.md` §4, §6, `inst-bs-*`,
//! D-37, D-137, O4).
//!
//! Both of the slice's flows — bulk import and mass repricing — are one record
//! moving through six states, and nothing else in Slice 12 can be written until
//! that record exists. It is the first group of the slice for that reason.
//!
//! # The state machine is in the trigger, and the edges are §4's exactly
//!
//! ```text
//! validating ─┬─> validation_failed          (inst-bs-fail)
//!             ├─> awaiting_approval          (inst-bs-approval, repricing only)
//!             └─> committing                 (inst-bs-commit)
//! awaiting_approval ──> committing           (inst-bs-commit, on approval)
//! committing ─┬─> completed                  (inst-bs-done)
//!             └─> completed_with_conflicts    (inst-bs-done / inst-bs-abort)
//! ```
//!
//! **`awaiting_approval` is unreachable for `kind = import`, and the store says
//! so** rather than leaving it to a caller: D-137 makes draft-plane authoring
//! never material, so an import that parked awaiting an approval would be waiting
//! for a decision no trigger can ever produce. That is a `CHECK`, not a
//! convention — it is the one edge of §4 whose violation would strand a record
//! forever.
//!
//! **The three terminal states are terminal.** `validation_failed`, `completed`
//! and `completed_with_conflicts` have no outgoing edge in §4, and a record that
//! left one would re-run a commit whose rows are already applied — the journal
//! (`pricing_repricing_journal`, this slice's next table) is the idempotency
//! spine precisely because a re-drive must be safe, and a terminal record
//! re-entering `committing` is not a re-drive but a second run wearing the first
//! one's id.
//!
//! # What this table deliberately does not carry
//!
//! **The row lock and the journal are their own tables** (§6), and neither is
//! this group's: `pricing_bulk_row_lock` is a side table the Foundation's
//! concurrent-edit check reads, deliberately not a column on `pricing_price`
//! (2026-07-31 review fix), and `pricing_repricing_journal` is keyed
//! `(run_id, price_id)`. Both land with the flows that write them.
//!
//! **It is revision-independent**, so unlike `pricing_composite_meter` (D-256) it
//! owes no copy-forward and no drop-on-abandon: a bulk operation is not part of a
//! plan revision's shape, and
//! `the_revision_scoped_tables_are_a_closed_set_and_each_one_is_copied_and_dropped`
//! does not reach it. That is the one way this table is cheaper than the last.
//!
//! # `SQLite`
//!
//! Systematic transforms only: `bss.` dropped, `uuid` -> `text`, `jsonb` ->
//! `text`, and the one PL/pgSQL function split into fixed-message
//! `RAISE(ABORT, …)` triggers, one per guarded verb — a `SQLite` `WHEN` clause
//! may not contain a subquery, but every guard here reads only `OLD`/`NEW`, so
//! each becomes a plain `WHEN`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_bulk_operation (
        operation_id  uuid        NOT NULL PRIMARY KEY,
        tenant_id     uuid        NOT NULL,
        kind          text        NOT NULL,
        state         text        NOT NULL,
        client_key    text        NOT NULL,
        report        jsonb       NOT NULL DEFAULT '{}'::jsonb,
        submitted_by  uuid        NOT NULL,
        submitted_at  timestamptz NOT NULL,
        completed_at  timestamptz,
        CONSTRAINT chk_pricing_bulk_operation_kind CHECK (
            kind IN ('import', 'repricing')),
        CONSTRAINT chk_pricing_bulk_operation_state CHECK (
            state IN ('validating', 'validation_failed', 'awaiting_approval',
                      'committing', 'completed', 'completed_with_conflicts')),
        -- D-137: a draft-plane import is never material, so it can never park
        -- awaiting an approval that nothing would ever grant.
        CONSTRAINT chk_pricing_bulk_operation_import_never_awaits CHECK (
            NOT (kind = 'import' AND state = 'awaiting_approval')),
        -- A terminal record has an end instant and a live one does not, so the
        -- pair cannot disagree about whether the run is over.
        CONSTRAINT chk_pricing_bulk_operation_completed_at CHECK (
            (completed_at IS NOT NULL) =
            (state IN ('validation_failed', 'completed', 'completed_with_conflicts')))
    )",
    // O4's idempotency: one operation per client key per tenant.
    "CREATE UNIQUE INDEX uq_pricing_bulk_operation_client_key
        ON bss.pricing_bulk_operation (tenant_id, client_key)",
    // The operator's list, and the sweep that finds stalled runs (D-37).
    "CREATE INDEX idx_pricing_bulk_operation_live
        ON bss.pricing_bulk_operation (tenant_id, state, submitted_at)",
    "CREATE OR REPLACE FUNCTION bss.pricing_bulk_operation_transitions() RETURNS trigger AS $$
        BEGIN
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
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_bulk_operation_transitions
        BEFORE UPDATE OR DELETE ON bss.pricing_bulk_operation
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_bulk_operation_transitions()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_bulk_operation",
    "DROP FUNCTION IF EXISTS bss.pricing_bulk_operation_transitions()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_bulk_operation (
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
    "CREATE UNIQUE INDEX uq_pricing_bulk_operation_client_key
        ON pricing_bulk_operation (tenant_id, client_key)",
    "CREATE INDEX idx_pricing_bulk_operation_live
        ON pricing_bulk_operation (tenant_id, state, submitted_at)",
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
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_bulk_operation"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
