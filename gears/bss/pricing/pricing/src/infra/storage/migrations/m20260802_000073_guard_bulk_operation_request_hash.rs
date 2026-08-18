//! `pricing_bulk_operation`'s frozen-column guard gains `request_hash`.
//!
//! `m20260802_000072` added the column; this freezes it, in the same wave rather
//! than in someone else's — `m20260802_000061`/`000062`'s pairing and its reason.
//! The digest records what the run was opened *for*, and the replay compares an
//! arriving body against it, so a writer that could move it could relicense a
//! spent client key onto a batch the operator never submitted. That is the guard's
//! subject, not a tidiness argument: the column's whole value is that it cannot
//! move after the run is born.
//!
//! # Why the two engines are one statement and two
//!
//! `m20260802_000062`'s asymmetry, one table over. Postgres keeps the frozen-column
//! arm **inside** `bss.pricing_bulk_operation_transitions()` — a whole state
//! machine — and `CREATE OR REPLACE` takes a body rather than a patch, so the
//! function is restated whole. `SQLite` carries the same arm as its own trigger
//! (`trg_pricing_bulk_operation_frozen_columns`), which is dropped and recreated.
//! Nothing else in either engine's enforcement moves: no `CHECK`, no index, no
//! table rebuild, and the six other triggers on this table are untouched.
//!
//! # Produced the same way as `m20260802_000062`
//!
//! `m20260802_000051`'s rule: **not by hand and not by a free-form script.** Both
//! blocks were read out of `m20260802_000063`'s own `UP` text — the guard as it now
//! stands, after D-267 widened the edge list, not as `m20260802_000047` wrote it —
//! and the generator asserted before writing that it had found **exactly one**
//! frozen-column arm per engine, that the arm it found was the post-D-267 body,
//! that each block came out **exactly one line longer**, that `request_hash`
//! appears **exactly twice** per block (once per side of one conjunct), and that
//! **every original line survives**. On the Postgres side the ` THEN` moved from
//! the `submitted_at` conjunct to the new last one, which is the only original line
//! that changed and is asserted as such.
//!
//! # `down` restores the guard without the column's line
//!
//! An exact inverse, and it has to be: `m20260802_000072`'s `down` drops the column
//! immediately afterwards, and a trigger left naming a column the table no longer
//! holds fails the next `UPDATE` on `SQLite` — which is precisely why the
//! restatement is a migration of its own rather than a line edited into `000063`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres - the whole function restated, `CREATE OR REPLACE` in place.
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
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
          OR NEW.submitted_at IS DISTINCT FROM OLD.submitted_at
          OR NEW.request_hash IS DISTINCT FROM OLD.request_hash THEN
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
     $$ LANGUAGE plpgsql",
];

// The guard as `m20260802_000063` left it, so the chain rolls back and re-applies.
const PG_DOWN_STATEMENTS: &[&str] = &[
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
     $$ LANGUAGE plpgsql",
];

// ---------------------------------------------------------------------------
// SQLite - the one trigger dropped and recreated.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_bulk_operation_frozen_columns",
    "CREATE TRIGGER trg_pricing_bulk_operation_frozen_columns
        BEFORE UPDATE ON pricing_bulk_operation
        FOR EACH ROW WHEN NEW.operation_id IS NOT OLD.operation_id
          OR NEW.tenant_id    IS NOT OLD.tenant_id
          OR NEW.kind         IS NOT OLD.kind
          OR NEW.client_key   IS NOT OLD.client_key
          OR NEW.submitted_by IS NOT OLD.submitted_by
          OR NEW.submitted_at IS NOT OLD.submitted_at
          OR NEW.request_hash IS NOT OLD.request_hash
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_bulk_operation: the operation is frozen; only state, report and completed_at move');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_bulk_operation_frozen_columns",
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
