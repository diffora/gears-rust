//! `pricing_price_window` gains `mutation_seq` — the monotonic per-window counter
//! that names an **act** (D-190) and gives the surface an entity tag (D-191).
//!
//! Two owed items, one column, which is why they land together: D-190 needs
//! something monotonic to tell one act on a window from the next, and D-191's
//! `If-Match` needs something to compare an entity tag against. The window row
//! carried neither — unlike `pricing_price`, whose `row_version` is exactly what the
//! price routes' precondition compares.
//!
//! # It counts **acts**, not row writes, and that is load-bearing
//!
//! The two clock-driven edges of §4 — `inst-ws-activate` and `inst-ws-expire` —
//! leave the number alone; only an operator's act advances it (a schedule is born at
//! `0`, an `effectiveTo` adjustment and a cancellation each add one). That is not a
//! convenience and it is the one thing about this column a later group must not
//! "simplify":
//!
//! An act's identity is what an approval unit's subject is built from (D-184), and
//! the retry that follows an approve has to render the **same** subject the refused
//! attempt did. If the activation sweep advanced this counter, a window that reached
//! its `effective_from` between the refusal and the approved retry would make the
//! retry name a subject no unit was ever opened under — so it would find nothing,
//! open a second unit, and the approval loop would have no exit. That is precisely
//! the defect D-184 closed, arriving through the clock rather than through the
//! window id. `tests/sqlite_window_repo.rs::the_activation_sweep_does_not_advance_the_act_sequence`
//! is the pin, and its doc carries this argument where a reader of the sweep will
//! meet it.
//!
//! The cost of that choice is stated rather than hidden: as an entity tag this
//! number tracks the **acts** on a window and not its whole representation, so a
//! window that activated carries the tag it had while `scheduled`. Nothing reads a
//! window through a `GET` (there is none — D-191 clause (2)), so no cache validator
//! depends on it; and the precondition it does serve is not weakened, because the
//! writing transaction re-reads the row and judges the adjustment against the
//! **stored** state through `refuse_frozen_end` whatever the caller's tag said.
//!
//! # A sixth arm on the trigger, and no CHECK
//!
//! The whitelist arm of `m20260802_000016` freezes columns by naming them, so a new
//! column is mutable by default — and an unconstrained counter is one `UPDATE` away
//! from being a counter that goes backwards, which is the one thing a monotonic name
//! must not do. The sixth arm therefore admits exactly two shapes: unchanged (the
//! sweep's flips) or `OLD + 1` (an act). A decrement, a skip and a reset are all
//! refused.
//!
//! There is deliberately **no `CHECK (mutation_seq >= 0)`**, and the reason is
//! backend symmetry rather than indifference. Postgres would take one in an
//! `ALTER TABLE`; `SQLite` has no `ADD CONSTRAINT` at all, so the portable form is
//! `m20260802_000019`'s create-copy-drop-rename rebuild — five triggers, two indexes
//! and a foreign key restated by hand for a bound the store cannot reach anyway: the
//! only writer sets `0` or `OLD + 1`, and a negative value is refused at the
//! repository boundary, where [`WindowRecord`] converts the column to a `u64` and
//! answers [`RepoError::CorruptRow`]. A rebuild whose whole purpose is a constraint
//! against an unreachable state is a larger risk than the state.
//!
//! [`WindowRecord`]: crate::infra::storage::repo::window_repo::WindowRecord
//! [`RepoError::CorruptRow`]: crate::infra::storage::RepoError::CorruptRow
//!
//! # Why a migration of its own
//!
//! `m20260802_000018`'s reason, which is the chain's rule: `000016` is what the
//! window store was when it was created, and a reader asking when an act became
//! nameable gets a dated answer rather than a `git blame`. The `down` is the exact
//! inverse — the arm is restored to `000016`'s five-arm text, then the column goes —
//! and on `SQLite` the trigger is dropped **before** the column it reads, or the
//! `DROP COLUMN` is refused by the trigger that names it.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

/// `m20260802_000016`'s trigger function with the act-sequence arm appended.
///
/// The whole body is restated because `CREATE OR REPLACE FUNCTION` has no
/// incremental form; the five arms above the new one are verbatim from that
/// migration, in the same order, so a diff of the two files shows exactly one
/// addition.
const PG_FUNCTION_WITH_SEQUENCE: &str =
    "CREATE OR REPLACE FUNCTION bss.pricing_price_window_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_price_window: DELETE of window % is not permitted; cancel is a state, not a deletion',
              OLD.window_id;
          END IF;

          IF OLD.state IN ('expired','cancelled') THEN
            RAISE EXCEPTION
              'pricing_price_window: window % is %; an expired or cancelled window is immutable history',
              OLD.window_id, OLD.state;
          END IF;

          IF NEW.window_id      IS DISTINCT FROM OLD.window_id
          OR NEW.tenant_id      IS DISTINCT FROM OLD.tenant_id
          OR NEW.price_id       IS DISTINCT FROM OLD.price_id
          OR NEW.effective_from IS DISTINCT FROM OLD.effective_from
          OR NEW.reason_code    IS DISTINCT FROM OLD.reason_code
          OR NEW.created_by     IS DISTINCT FROM OLD.created_by
          OR NEW.created_at     IS DISTINCT FROM OLD.created_at THEN
            RAISE EXCEPTION
              'pricing_price_window: window % is bound to its price row and its start; only state, effective_to and the flip timestamps may move',
              OLD.window_id;
          END IF;

          IF NEW.state IS DISTINCT FROM OLD.state
             AND NOT (OLD.state = 'scheduled' AND NEW.state IN ('active','cancelled'))
             AND NOT (OLD.state = 'active'    AND NEW.state = 'expired') THEN
            RAISE EXCEPTION
              'pricing_price_window: state % -> % is not a sanctioned transition',
              OLD.state, NEW.state;
          END IF;

          IF NEW.effective_to IS DISTINCT FROM OLD.effective_to
             AND ((NEW.effective_to IS NOT NULL AND NEW.effective_to <= now())
               OR (OLD.effective_to IS NOT NULL AND OLD.effective_to <= now())) THEN
            RAISE EXCEPTION
              'pricing_price_window: the effective_to of window % may only be moved while it is in the future, and only to a future instant',
              OLD.window_id;
          END IF;

          IF NEW.mutation_seq IS DISTINCT FROM OLD.mutation_seq
             AND NEW.mutation_seq <> OLD.mutation_seq + 1 THEN
            RAISE EXCEPTION
              'pricing_price_window: the act sequence of window % moves by one act at a time, from % - it names an act and a name that can be reused or run backwards names nothing',
              OLD.window_id, OLD.mutation_seq;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql";

/// `m20260802_000016`'s function, exactly as that migration wrote it.
///
/// The `down` restores it rather than dropping it: the trigger created there still
/// references it, so a `DROP FUNCTION` would leave the table with a trigger pointing
/// at nothing.
const PG_FUNCTION_WITHOUT_SEQUENCE: &str =
    "CREATE OR REPLACE FUNCTION bss.pricing_price_window_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_price_window: DELETE of window % is not permitted; cancel is a state, not a deletion',
              OLD.window_id;
          END IF;

          IF OLD.state IN ('expired','cancelled') THEN
            RAISE EXCEPTION
              'pricing_price_window: window % is %; an expired or cancelled window is immutable history',
              OLD.window_id, OLD.state;
          END IF;

          IF NEW.window_id      IS DISTINCT FROM OLD.window_id
          OR NEW.tenant_id      IS DISTINCT FROM OLD.tenant_id
          OR NEW.price_id       IS DISTINCT FROM OLD.price_id
          OR NEW.effective_from IS DISTINCT FROM OLD.effective_from
          OR NEW.reason_code    IS DISTINCT FROM OLD.reason_code
          OR NEW.created_by     IS DISTINCT FROM OLD.created_by
          OR NEW.created_at     IS DISTINCT FROM OLD.created_at THEN
            RAISE EXCEPTION
              'pricing_price_window: window % is bound to its price row and its start; only state, effective_to and the flip timestamps may move',
              OLD.window_id;
          END IF;

          IF NEW.state IS DISTINCT FROM OLD.state
             AND NOT (OLD.state = 'scheduled' AND NEW.state IN ('active','cancelled'))
             AND NOT (OLD.state = 'active'    AND NEW.state = 'expired') THEN
            RAISE EXCEPTION
              'pricing_price_window: state % -> % is not a sanctioned transition',
              OLD.state, NEW.state;
          END IF;

          IF NEW.effective_to IS DISTINCT FROM OLD.effective_to
             AND ((NEW.effective_to IS NOT NULL AND NEW.effective_to <= now())
               OR (OLD.effective_to IS NOT NULL AND OLD.effective_to <= now())) THEN
            RAISE EXCEPTION
              'pricing_price_window: the effective_to of window % may only be moved while it is in the future, and only to a future instant',
              OLD.window_id;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql";

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price_window
        ADD COLUMN mutation_seq bigint NOT NULL DEFAULT 0",
    PG_FUNCTION_WITH_SEQUENCE,
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    PG_FUNCTION_WITHOUT_SEQUENCE,
    "ALTER TABLE bss.pricing_price_window DROP COLUMN mutation_seq",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transforms from the Postgres variant: no schema prefix, `bigint` ->
// `integer`, and the one PL/pgSQL arm becomes one `RAISE(ABORT, ...)` trigger with a
// fixed message (no procedural language, no interpolation) carrying the terminal
// exclusion its siblings carry, so exactly one arm fires per statement.
//
// `ADD COLUMN` is portable here and no rebuild is needed - the column takes no
// UNIQUE, no PRIMARY KEY and a constant default, which is the whole of what `SQLite`
// restricts. Compare `m20260802_000019`, which had no such luck: widening a CHECK
// has no `ALTER` form at all.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_price_window
        ADD COLUMN mutation_seq integer NOT NULL DEFAULT 0",
    "CREATE TRIGGER trg_pricing_price_window_act_sequence
        BEFORE UPDATE ON pricing_price_window
        FOR EACH ROW WHEN OLD.state NOT IN ('expired','cancelled')
          AND NEW.mutation_seq IS NOT OLD.mutation_seq
          AND NEW.mutation_seq <> OLD.mutation_seq + 1
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_window: the act sequence moves by one act at a time; it names an act, and a name that can be reused or run backwards names nothing');
        END",
];

// The trigger goes **before** the column it reads: `ALTER TABLE ... DROP COLUMN` is
// refused while a trigger names the column, so the reverse order would make the
// chain irreversible at this step and `down_then_up_round_trips` is what would say
// so.
const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_price_window_act_sequence",
    "ALTER TABLE pricing_price_window DROP COLUMN mutation_seq",
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
