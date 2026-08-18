//! Create `bss.pricing_approval_key` — the set of canonical scope keys a
//! `submitted` approval unit **holds** (`design/07-pricewindow-linkage.md`
//! `inst-co-single-pending`).
//!
//! The rule: *"at most one pending approval unit **of any kind** may hold a
//! canonical scope key … A second submit touching a held key while one is
//! `submitted` returns 409 (`PENDING_CHANGE_UNIT_EXISTS`)"*, and §5's gloss on
//! that code says the same thing from the wire side — *"a pending unit already
//! holds one of the touched keys"*. Both sentences are about a **set of keys**,
//! and until this table there was nowhere to put one.
//!
//! # What stood here before, and why it was not this rule
//!
//! `approval_repo::find_pending_for_plan` matched `subject_ref LIKE '<plan_id>/%'`
//! over `subject_kind = 'plan_revision'`. That is a **plan-revision** lock: it
//! refuses a second unit over the same plan, which is a *consequence* of the rule
//! on a plan whose whole key set one unit holds, and is neither necessary nor
//! sufficient for it. It refuses too much (two units on disjoint keys of one plan
//! are legal and it refused them) and too little in the direction that matters —
//! it cannot see a unit of any other `subject_kind` at all, so a window unit and a
//! plan unit over one key both opened, leaving two always-material units approvable
//! and the final state commit-order-dependent, which is the hazard the rule's own
//! 2026-07-31d fix note (C-3) records.
//!
//! # Shape (a): a child table with a partial unique index, and why the alternative
//! lost
//!
//! The rule could be a **check** — a key set on the parent record, compared inside
//! the submitting transaction — or a **constraint**. It is a constraint, because a
//! check here is defeatable by the writer it exists to stop.
//!
//! `ApprovalService::submit` reads before it inserts, inside the transaction that
//! inserts, and its own doc argued the residual race away: *"the residual race —
//! both reading before either writes, under an isolation level that permits it — is
//! decided by the primary key rather than left open"*. **That premise is false, and
//! it is why this table exists.** `pricing_approval`'s primary key is
//! `approval_id`, which the *caller* mints — `api::rest::publish` calls
//! `Uuid::now_v7()` per request — so two concurrent submits over one plan carry two
//! different primary keys and collide on nothing. Under `READ COMMITTED` both read
//! a store with no pending unit, both insert, and both commit. The primary key
//! decides only the *retry of one submit*, which is a different question and the
//! one that sentence was true about.
//!
//! So the register is a **partial unique index** —
//! `UNIQUE (tenant_id, scope_key) WHERE state = 'submitted'` — and the loser of
//! that race is refused by the server rather than by a comparison it raced. The
//! in-transaction check stays: it is what produces a 409 that names the unit
//! holding the key, which an index violation cannot. The two are not redundant, and
//! neither is the other's test — the check is the ordinary answer, the index is the
//! invariant.
//!
//! # `state` is denormalised onto the register row, and a trigger keeps it
//!
//! A partial index on `state = 'submitted'` needs `state` in the indexed table.
//! Three ways to get it there, and the middle one is a trap:
//!
//! * **Delete the register rows when the unit is decided.** Refused: the register
//!   is the record of what a unit held, and this store's whole discipline is that a
//!   decided record is evidence. `DELETE` is refused here as it is on the parent.
//! * **Have each decision path update the register too** — `decide`,
//!   `void_pending_for_plan`, `void_pending_for_subject`. Refused: that is three
//!   call sites which must each remember, and a forgotten one leaves a key held
//!   **forever** by a unit nobody can decide again. The failure mode is a tenant
//!   that cannot publish a plan and no row anywhere saying why.
//! * **A trigger on the parent.** Taken. `state` follows the unit automatically, so
//!   a decision path cannot forget it and a fourth decision path added later
//!   inherits it. The register's own guard admits exactly that one column moving,
//!   once, out of `submitted` — **and only to the state its unit is actually in**,
//!   which is what makes an ad-hoc `UPDATE` unable to free a key.
//!
//! **That last clause was false when it was first written, and the correction is a
//! trigger rather than a retraction.** It claimed "the same whitelist shape the parent
//! carries, so an ad-hoc `UPDATE` cannot free a key either", and the parent's shape is
//! not the same: `pricing_approval`'s function *whitelists the destination* (`NEW.state
//! NOT IN ('approved','rejected','voided')`), while the register's three arms only
//! pinned the other columns and refused a second move. So
//! `UPDATE pricing_approval_key SET state = 'voided' WHERE scope_key = '<key>'` left the
//! pinned columns untouched, satisfied `OLD.state = 'submitted'`, and landed — measured
//! on the mirror, not inferred: the child read `voided`, the parent still read
//! `submitted`, `uq_pricing_approval_key_pending` then admitted a **second holder**, and
//! two units held one key while the parent was still approvable.
//!
//! `trg_pricing_approval_key_follows_its_unit` (Postgres: the same clause inside the
//! append-only function) closes it by making the destination the **parent's current
//! state**, so the only statement that can move a register row is the one the parent's
//! own transition makes. The parent's transition satisfies it by construction —
//! `follow_state` fires `AFTER UPDATE`, so the row it reads is already the new one.
//!
//! **And a row is born under a pending unit**, which is the same hole at the other end:
//! there is no foreign key here, so a row inserted under a missing or already-decided
//! approval held its key **forever** — `follow_state` fires only `AFTER UPDATE`, the
//! parent refuses every UPDATE once decided, and `find_pending_key_holder` answers
//! `CorruptRow` (a 500, not a refusal). `approval_repo::open` cannot reach that state
//! today, both inserts being one transaction, so it was an unenforced invariant rather
//! than a live defect; it is enforced because the table's other hole was live.
//!
//! # The scope key is `text`, and it is the canonical rendering
//!
//! The same ten-axis string `ScopeKey::to_string` produces and the same string a
//! publish refusal names a key by — not the ten columns spread out. The register
//! answers set membership and nothing else: it is never joined to `pricing_price`,
//! never filtered per axis, and never read to reconstruct a key. Spreading it would
//! buy a query nobody makes and would put a second canonical rendering in the
//! schema for the axes to drift between.
//!
//! # There is no `REVOKE`
//!
//! For `m20260802_000001_create_pricing_plan.rs`'s reason: it names a deployment
//! role this chain does not own and `SQLite` has no `GRANT`/`REVOKE`. The triggers
//! are the portable half.
//!
//! **Backend differences.** The systematic type mirror (`uuid` -> `text`), plus the
//! trigger split: Postgres carries two PL/pgSQL functions, while `SQLite` has no
//! procedural language and `RAISE(ABORT, ...)` takes a **literal** message, so each of
//! the register's rules becomes its own trigger with a fixed message and
//! `IS DISTINCT FROM` is written `IS NOT`. The state-follow trigger is one statement
//! on both. The roster is the `SQLITE_UP_STATEMENTS` list below and
//! `tests/sqlite_migrations.rs`'s census, which is what stops a mirror trigger being
//! dropped silently — before that census, deleting any of them left the whole suite
//! green, because the contention tests are answered by the *application* read
//! `find_pending_key_holder` and never touch the index or the triggers.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_approval_key (
        approval_id  uuid NOT NULL,
        tenant_id    uuid NOT NULL,
        -- The canonical rendering, exactly as a publish refusal names a key.
        scope_key    text NOT NULL,
        -- Denormalised from the parent and maintained by
        -- `trg_pricing_approval_key_follow_state`; see the module doc.
        state        text NOT NULL,
        PRIMARY KEY (approval_id, scope_key),
        CONSTRAINT chk_pricing_approval_key_state CHECK (
            state IN ('submitted','approved','rejected','voided'))
    )",
    // `inst-co-single-pending` as a constraint. The predicate **is** the rule: a
    // decided or voided unit holds nothing, which is what makes the withdraw of
    // `inst-as-void` an escape from the pin rather than a second way to spell it.
    "CREATE UNIQUE INDEX uq_pricing_approval_key_pending
        ON bss.pricing_approval_key (tenant_id, scope_key)
        WHERE state = 'submitted'",
    // Reading the register by unit - what a decided unit held, and the join a
    // 409's message walks to name the holder.
    "CREATE INDEX idx_pricing_approval_key_approval
        ON bss.pricing_approval_key (approval_id)",
    "CREATE OR REPLACE FUNCTION bss.pricing_approval_key_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_approval_key: DELETE of key % held by approval % is not permitted; the register is the record of what the unit held',
              OLD.scope_key, OLD.approval_id;
          END IF;

          IF TG_OP = 'INSERT' THEN
            IF NEW.state <> 'submitted' THEN
              RAISE EXCEPTION
                'pricing_approval_key: key % arrives %; a register row is born submitted with its unit',
                NEW.scope_key, NEW.state;
            END IF;
            -- **And it is born with a unit that is pending.** The foreign key and
            -- the parent-state check in one clause, because the failure they close
            -- is one failure: a row born under a unit that is missing or already
            -- decided holds its key **forever** - `follow_state` fires only
            -- `AFTER UPDATE` and the parent refuses every UPDATE once decided, so
            -- nothing can ever move it, and `find_pending_key_holder` answers
            -- `CorruptRow` (a 500) rather than a refusal an operator can act on.
            IF (SELECT state FROM bss.pricing_approval WHERE approval_id = NEW.approval_id)
               IS DISTINCT FROM 'submitted' THEN
              RAISE EXCEPTION
                'pricing_approval_key: approval % is not a pending unit; a register row is born with one',
                NEW.approval_id;
            END IF;
            RETURN NEW;
          END IF;

          -- Only `state` moves, and only off `submitted`. Everything else is what
          -- the unit held, which is not editable after the fact.
          IF NEW.approval_id IS DISTINCT FROM OLD.approval_id
          OR NEW.tenant_id   IS DISTINCT FROM OLD.tenant_id
          OR NEW.scope_key   IS DISTINCT FROM OLD.scope_key THEN
            RAISE EXCEPTION
              'pricing_approval_key: the register row of approval % is pinned; only state follows the unit',
              OLD.approval_id;
          END IF;

          IF OLD.state <> 'submitted' THEN
            RAISE EXCEPTION
              'pricing_approval_key: the register row of approval % is already %; it follows its unit once',
              OLD.approval_id, OLD.state;
          END IF;

          -- **The direction whitelist**: the state a register row moves to is the
          -- state its unit is *in*, so the only statement that can move it is the
          -- one the parent's own transition makes. See the module doc.
          IF NEW.state IS DISTINCT FROM
             (SELECT state FROM bss.pricing_approval WHERE approval_id = NEW.approval_id) THEN
            RAISE EXCEPTION
              'pricing_approval_key: approval % is %; a register row follows its unit and cannot be moved to % on its own',
              NEW.approval_id,
              (SELECT state FROM bss.pricing_approval WHERE approval_id = NEW.approval_id),
              NEW.state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_approval_key_append_only
        BEFORE INSERT OR UPDATE OR DELETE ON bss.pricing_approval_key
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_approval_key_append_only()",
    // The register follows its unit out of `submitted`, so a decided unit frees
    // every key it held and no decision path has to remember to say so.
    "CREATE OR REPLACE FUNCTION bss.pricing_approval_key_follow_state() RETURNS trigger AS $$
        BEGIN
          IF NEW.state IS DISTINCT FROM OLD.state THEN
            UPDATE bss.pricing_approval_key
               SET state = NEW.state
             WHERE approval_id = NEW.approval_id;
          END IF;
          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_approval_key_follow_state
        AFTER UPDATE ON bss.pricing_approval
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_approval_key_follow_state()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_approval_key_follow_state ON bss.pricing_approval",
    "DROP TABLE IF EXISTS bss.pricing_approval_key",
    "DROP FUNCTION IF EXISTS bss.pricing_approval_key_append_only()",
    "DROP FUNCTION IF EXISTS bss.pricing_approval_key_follow_state()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transforms from the Postgres variant:
// * schema prefix `bss.` dropped (single namespace);
// * `uuid` -> `text`;
// * the single PL/pgSQL append-only function becomes four `RAISE(ABORT, ...)`
//   triggers, and `IS DISTINCT FROM` becomes `IS NOT`.
// Every CHECK and both indexes are preserved, name for name.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_approval_key (
        approval_id  text NOT NULL,
        tenant_id    text NOT NULL,
        scope_key    text NOT NULL,
        state        text NOT NULL,
        PRIMARY KEY (approval_id, scope_key),
        CONSTRAINT chk_pricing_approval_key_state CHECK (
            state IN ('submitted','approved','rejected','voided'))
    )",
    "CREATE UNIQUE INDEX uq_pricing_approval_key_pending
        ON pricing_approval_key (tenant_id, scope_key)
        WHERE state = 'submitted'",
    "CREATE INDEX idx_pricing_approval_key_approval
        ON pricing_approval_key (approval_id)",
    "CREATE TRIGGER trg_pricing_approval_key_no_delete
        BEFORE DELETE ON pricing_approval_key
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval_key: DELETE of a held key is not permitted; the register is the record of what the unit held');
        END",
    "CREATE TRIGGER trg_pricing_approval_key_born_submitted
        BEFORE INSERT ON pricing_approval_key
        FOR EACH ROW WHEN NEW.state <> 'submitted'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval_key: a register row is born submitted with its unit');
        END",
    // The foreign key and the parent-state check in one trigger, for the reason the
    // Postgres branch gives: a row born under a missing or already-decided unit is a
    // permanent phantom hold, and the two ways to be born wrong have one remedy.
    "CREATE TRIGGER trg_pricing_approval_key_born_under_a_pending_unit
        BEFORE INSERT ON pricing_approval_key
        FOR EACH ROW WHEN (SELECT state FROM pricing_approval
                            WHERE approval_id = NEW.approval_id) IS NOT 'submitted'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval_key: a register row is born with a pending unit; this approval is missing or already decided');
        END",
    "CREATE TRIGGER trg_pricing_approval_key_pinned_columns
        BEFORE UPDATE ON pricing_approval_key
        FOR EACH ROW WHEN (NEW.approval_id IS NOT OLD.approval_id
                        OR NEW.tenant_id   IS NOT OLD.tenant_id
                        OR NEW.scope_key   IS NOT OLD.scope_key)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval_key: the register row is pinned; only state follows the unit');
        END",
    "CREATE TRIGGER trg_pricing_approval_key_follows_once
        BEFORE UPDATE ON pricing_approval_key
        FOR EACH ROW WHEN OLD.state <> 'submitted'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval_key: the register row already followed its unit once');
        END",
    // The direction whitelist. Without it the three arms above admit
    // `UPDATE pricing_approval_key SET state = 'voided' WHERE scope_key = '<key>'`
    // — pinned columns untouched, `OLD.state = 'submitted'` — which frees the key
    // while its unit is still submitted and approvable. Measured before it existed:
    // the statement landed and a second holder then took the key.
    "CREATE TRIGGER trg_pricing_approval_key_follows_its_unit
        BEFORE UPDATE OF state ON pricing_approval_key
        FOR EACH ROW WHEN NEW.state IS NOT (SELECT state FROM pricing_approval
                                             WHERE approval_id = NEW.approval_id)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval_key: a register row follows its unit; its state cannot be moved on its own');
        END",
    "CREATE TRIGGER trg_pricing_approval_key_follow_state
        AFTER UPDATE OF state ON pricing_approval
        FOR EACH ROW WHEN NEW.state IS NOT OLD.state
        BEGIN
          UPDATE pricing_approval_key
             SET state = NEW.state
           WHERE approval_id = NEW.approval_id;
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_pricing_approval_key_follow_state",
    "DROP TABLE IF EXISTS pricing_approval_key",
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
