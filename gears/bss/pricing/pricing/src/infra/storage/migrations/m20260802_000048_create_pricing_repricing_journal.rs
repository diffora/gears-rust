//! `pricing_repricing_journal` — the per-row idempotency spine of a mass
//! repricing run (`design/12-operator-efficiency.md` §6, `inst-mr-journal`,
//! `inst-mp-journal`, `inst-mp-pending`, O3).
//!
//! Slice 12's second group. A run expands its selector into a **frozen row
//! set** and writes one `pending` row here per selected price; the apply loop
//! then moves each to `applied` or `failed` inside the same transaction that
//! writes the successor row and its outbox record. That co-transaction is the
//! whole point: a crash mid-run leaves a journal that agrees with the store, so
//! the lease takeover re-drives from it and applies nothing twice.
//!
//! # The state machine, and where its INSERT arm matters most
//!
//! ```text
//! pending ─┬─> applied      (the successor row committed)
//!          └─> failed       (this row, or its whole plan under D-134)
//! ```
//!
//! **A journal row is born `pending` and in no other state**, and on this table
//! that arm is worth more than the edges are. `m20260802_000047` had no INSERT
//! arm for one commit and a run could be born past its own machine; here the
//! same hole is worse in kind, because a row born `applied` is a row the re-drive
//! **skips**. The failure is silent and permanent: the operator sees a completed
//! run, the price was never touched, and no state anywhere disagrees.
//!
//! **A decided row never moves again**, which is one guard rather than three:
//! `applied` and `failed` are terminal, and freezing the whole row at the moment
//! it stops being `pending` also stops `applied_price_id` being rewritten to name
//! a different successor. Written as `OLD.state <> 'pending'` rather than as a
//! disjunct list of the two sanctioned edges — with the vocabulary `CHECK`
//! already limiting `state` to three values, an edge list would be a rule that
//! cannot fail, which is indistinguishable from a rule that holds.
//!
//! # `not-attempted` is not a fourth state
//!
//! `inst-bs-abort` reports an aborted run's uncommitted rows as `not-attempted`.
//! That is how the **report** renders a row this table holds as `pending`; §6
//! gives the journal three states and this one keeps to them. The consequence is
//! deliberate and is why nothing here requires a terminal run's journal to be
//! free of `pending` rows: an aborted run leaves them behind forever, and §6's
//! "a run is complete when no `pending` rows remain" is a statement about the
//! runner's work rather than about the operation's state.
//!
//! # Three keys and one kind
//!
//! `run_id` names the operation; `price_id` and `applied_price_id` name the
//! selected row and the successor it produced, keyed for the reason
//! `pricing_price_tier_band` and `pricing_price_window` key theirs. A journal row
//! naming a price that does not exist is `pending` forever: the re-drive can
//! never apply it, §6's "a run is complete when no `pending` rows remain" is
//! never satisfied, and the only signal is the stalled-run alarm — which cannot
//! be told apart from a dead runner.
//!
//! **Only a repricing run journals here.** A bulk import's per-row outcomes live
//! in the operation's own `report` (`inst-bi-commit`, `inst-bk-idem`) and nothing
//! drives an import through this table, so a row here under an import is a record
//! no code will ever complete. Enforced rather than asserted in prose, because a
//! claim in a doc comment that the schema does not hold is the same class of
//! defect as a rule with no operand.
//!
//! **Both parent-reading arms defer to their foreign key when the run does not
//! exist.** A `BEFORE` trigger answers ahead of the key on either engine, so an
//! arm written as "the run is not a repricing run" would fire for a row naming
//! *no* run — reporting a fault the caller does not have, and leaving the key
//! unobservable and therefore unassertable.
//!
//! # What this table deliberately does not carry
//!
//! **No `plan_id`, though D-134 commits per plan.** §6 specifies four columns
//! beside the key and a plan is reachable from `price_id`; denormalizing it here
//! would be this group inventing a column for a group that has not been written
//! yet, and the run would then have two sources for one fact.
//!
//! **No secondary index.** Every read is run-scoped — the re-drive, the report,
//! and O3's "skip the `applied` rows" — and the primary key already leads with
//! `run_id`. The one genuinely cross-run question, *is this price row inside an
//! in-flight run*, is `pricing_bulk_row_lock`'s to answer, which is a large part
//! of why §6 makes the lock a table of its own.
//!
//! **It is revision-independent**, like `m20260802_000047` and unlike
//! `pricing_composite_meter` (D-256): a run is not part of any plan revision's
//! shape, so it owes no copy-forward, no drop-on-abandon, and the closed-set
//! guard does not reach it.
//!
//! # `SQLite`
//!
//! Systematic transforms only: `bss.` dropped, `uuid` -> `text`, `timestamptz`
//! -> `text`, and the one PL/pgSQL function split into fixed-message
//! `RAISE(ABORT, …)` triggers, one per guarded condition. Four of the five read
//! only `OLD`/`NEW` and are plain `WHEN` clauses. The fifth reads the parent run,
//! and a `SQLite` `WHEN` may not contain a subquery — so it goes in the trigger
//! **body** instead (`SELECT RAISE(ABORT, …) WHERE …`), which may,
//! `m20260802_000046`'s shape exactly.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_repricing_journal (
        run_id           uuid        NOT NULL,
        price_id         uuid        NOT NULL,
        tenant_id        uuid        NOT NULL,
        state            text        NOT NULL,
        failure_reason   text,
        applied_price_id uuid,
        applied_at       timestamptz,
        PRIMARY KEY (run_id, price_id),
        CONSTRAINT chk_pricing_repricing_journal_state CHECK (
            state IN ('pending', 'applied', 'failed')),
        -- An applied row names the successor it created and the instant it did.
        -- Written as an equality **per column** rather than against the pair:
        -- `(state = 'applied') = (id IS NOT NULL AND at IS NOT NULL)` reads the
        -- same and admits a pending row carrying a successor id with a null
        -- instant, which the re-drive would apply a second time.
        CONSTRAINT chk_pricing_repricing_journal_applied CHECK (
            (applied_price_id IS NOT NULL) = (state = 'applied')
            AND (applied_at IS NOT NULL) = (state = 'applied')),
        -- inst-mp-pending and D-134: a failed row carries the reason it failed,
        -- shared with every other row of its plan when the plan-level pass is
        -- what refused it.
        CONSTRAINT chk_pricing_repricing_journal_failed CHECK (
            (state = 'failed') = (failure_reason IS NOT NULL)),
        -- inst-mp-standard: an applied row is a NEW immutable row through the
        -- Foundation path, because bulk never mutates in place. A successor
        -- wearing the selected row's own id would be that mutation, recorded as
        -- though it were a supersession.
        CONSTRAINT chk_pricing_repricing_journal_successor_is_new CHECK (
            applied_price_id IS NULL OR applied_price_id <> price_id),
        CONSTRAINT fk_pricing_repricing_journal_run FOREIGN KEY (run_id)
            REFERENCES bss.pricing_bulk_operation (operation_id),
        -- The selected row and the successor it produced. Both keyed for the
        -- reason `pricing_price_tier_band` and `pricing_price_window` key theirs:
        -- a journal row naming a price that does not exist is pending forever,
        -- the re-drive can never apply it, and a run whose completion predicate
        -- is that no pending rows remain never completes -- surfacing only as
        -- the stalled-run alarm, indistinguishable from a dead runner.
        CONSTRAINT fk_pricing_repricing_journal_price FOREIGN KEY (price_id)
            REFERENCES bss.pricing_price (price_id),
        CONSTRAINT fk_pricing_repricing_journal_applied_price FOREIGN KEY (applied_price_id)
            REFERENCES bss.pricing_price (price_id)
    )",
    "CREATE OR REPLACE FUNCTION bss.pricing_repricing_journal_progress() RETURNS trigger AS $$
        DECLARE
          run_kind text;
        BEGIN
          -- Born pending. A row born `applied` is a row the re-drive skips, so
          -- the price is never touched and nothing anywhere disagrees -- the
          -- one failure this table exists to make impossible.
          IF TG_OP = 'INSERT' THEN
            IF NEW.state <> 'pending' THEN
              RAISE EXCEPTION
                'pricing_repricing_journal: a journal row is born pending, not %', NEW.state;
            END IF;

            -- The journal is mass repricing's spine. A bulk import's per-row
            -- outcomes live in the operation's own report (inst-bi-commit,
            -- inst-bk-idem) and nothing drives an import through this table, so a
            -- row here under an import is a record no code will ever complete.
            SELECT kind INTO run_kind
              FROM bss.pricing_bulk_operation
             WHERE operation_id = NEW.run_id;
            -- No such run: the foreign key is the accurate refusal and this arm
            -- has no opinion. Deferring keeps the key **observable** -- a BEFORE
            -- trigger answers ahead of it -- and stops this arm reporting a kind
            -- fault for a run that does not exist.
            IF FOUND AND run_kind <> 'repricing' THEN
              RAISE EXCEPTION
                'pricing_repricing_journal: operation % is a %, and only a repricing run journals per-row progress',
                NEW.run_id, run_kind;
            END IF;
            RETURN NEW;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_repricing_journal: DELETE of the row for price % of run % is not permitted; the journal is the idempotency spine and a missing row re-applies',
              OLD.price_id, OLD.run_id;
          END IF;

          -- The key and its tenant are what the row is about.
          IF NEW.run_id    IS DISTINCT FROM OLD.run_id
          OR NEW.price_id  IS DISTINCT FROM OLD.price_id
          OR NEW.tenant_id IS DISTINCT FROM OLD.tenant_id THEN
            RAISE EXCEPTION
              'pricing_repricing_journal: the row for price % of run % is keyed and its key is frozen',
              OLD.price_id, OLD.run_id;
          END IF;

          -- Decided is final, outcome columns included.
          IF OLD.state <> 'pending' THEN
            RAISE EXCEPTION
              'pricing_repricing_journal: the row for price % of run % is already %, and a decided row never moves again',
              OLD.price_id, OLD.run_id, OLD.state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_repricing_journal_progress
        BEFORE INSERT OR UPDATE OR DELETE ON bss.pricing_repricing_journal
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_repricing_journal_progress()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_repricing_journal",
    "DROP FUNCTION IF EXISTS bss.pricing_repricing_journal_progress()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_repricing_journal (
        run_id           text NOT NULL,
        price_id         text NOT NULL,
        tenant_id        text NOT NULL,
        state            text NOT NULL,
        failure_reason   text,
        applied_price_id text,
        applied_at       text,
        PRIMARY KEY (run_id, price_id),
        CONSTRAINT chk_pricing_repricing_journal_state CHECK (
            state IN ('pending', 'applied', 'failed')),
        CONSTRAINT chk_pricing_repricing_journal_applied CHECK (
            (applied_price_id IS NOT NULL) = (state = 'applied')
            AND (applied_at IS NOT NULL) = (state = 'applied')),
        CONSTRAINT chk_pricing_repricing_journal_failed CHECK (
            (state = 'failed') = (failure_reason IS NOT NULL)),
        CONSTRAINT chk_pricing_repricing_journal_successor_is_new CHECK (
            applied_price_id IS NULL OR applied_price_id <> price_id),
        CONSTRAINT fk_pricing_repricing_journal_run FOREIGN KEY (run_id)
            REFERENCES pricing_bulk_operation (operation_id),
        -- The selected row and the successor it produced. Both keyed for the
        -- reason `pricing_price_tier_band` and `pricing_price_window` key theirs:
        -- a journal row naming a price that does not exist is pending forever,
        -- the re-drive can never apply it, and a run whose completion predicate
        -- is that no pending rows remain never completes -- surfacing only as
        -- the stalled-run alarm, indistinguishable from a dead runner.
        CONSTRAINT fk_pricing_repricing_journal_price FOREIGN KEY (price_id)
            REFERENCES pricing_price (price_id),
        CONSTRAINT fk_pricing_repricing_journal_applied_price FOREIGN KEY (applied_price_id)
            REFERENCES pricing_price (price_id)
    )",
    "CREATE TRIGGER trg_pricing_repricing_journal_born_pending
        BEFORE INSERT ON pricing_repricing_journal
        FOR EACH ROW WHEN NEW.state <> 'pending'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_repricing_journal: a journal row is born pending and in no other state');
        END",
    // Conditioned on the run **existing**, exactly as the PL/pgSQL arm's `FOUND`
    // conjunct is: without it a row naming no run would be refused here as a kind
    // fault, the foreign key would never be reached, and the message would name a
    // problem the caller does not have.
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
    "CREATE TRIGGER trg_pricing_repricing_journal_no_delete
        BEFORE DELETE ON pricing_repricing_journal
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_repricing_journal: DELETE of a journal row is not permitted; the journal is the idempotency spine and a missing row re-applies');
        END",
    "CREATE TRIGGER trg_pricing_repricing_journal_frozen_key
        BEFORE UPDATE ON pricing_repricing_journal
        FOR EACH ROW WHEN NEW.run_id IS NOT OLD.run_id
          OR NEW.price_id  IS NOT OLD.price_id
          OR NEW.tenant_id IS NOT OLD.tenant_id
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_repricing_journal: the journal row is keyed and its key is frozen');
        END",
    "CREATE TRIGGER trg_pricing_repricing_journal_decided_is_final
        BEFORE UPDATE ON pricing_repricing_journal
        FOR EACH ROW WHEN OLD.state <> 'pending'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_repricing_journal: the journal row is already decided, and a decided row never moves again');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_repricing_journal"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
