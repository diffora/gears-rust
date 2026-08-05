//! `chk_pricing_approval_subject_kind` gains `'policy'` — the fourth member of
//! D-158's enumeration, arriving **with** its writer.
//!
//! `AuditSubjectKind` and this CHECK are one enumeration spelled in two places
//! (D-158, D-175), and the gear's standing rule is that a token with no writer is
//! not declared. `policy` had none until this change: `S5 §6` has listed it since
//! the store was designed, and G6 is the group that gives it one —
//! `ApprovalService::submit_threshold_policy`, the always-material D-10 unit a
//! `PUT /bss-pricing/v1/config/approval-threshold-policy` opens over a proposed
//! `pricing_approval_threshold` version. The Rust member, this token and that
//! writer land together or none of them lands.
//!
//! # Why a migration of its own rather than an amendment to `000015`
//!
//! `m20260802_000018`'s reason, restated because it is the chain's rule and not a
//! preference: the history stays legible. `000015` is what the approval store was
//! when it was created, and a reader asking when `policy` became storable gets a
//! dated answer instead of a `git blame`. The `down` is exactly the inverse, so the
//! chain is reversible at every point.
//!
//! # The Postgres half is two statements; the `SQLite` half is a table rebuild
//!
//! Postgres names constraints, so widening one is `DROP CONSTRAINT` +
//! `ADD CONSTRAINT` under the same name. `SQLite` has no `ALTER TABLE … DROP
//! CONSTRAINT` at all, so the portable form is `m20260802_000018`'s
//! create-copy-drop-rename dance — and this table is the harder instance of it,
//! which is why the list is written out rather than trusted to memory.
//!
//! **`pricing_approval` carries six triggers, from two migrations**, and `DROP
//! TABLE` takes every one of them with it:
//!
//! * `trg_pricing_approval_born_submitted`, `_no_delete`,
//!   `_immutable_once_decided`, `_pinned_columns`, `_flip_whitelist` — the five
//!   append-only arms `000015` created;
//! * `trg_pricing_approval_key_follow_state` — created by **`000017`**, on this
//!   table, and the one a rebuild is most likely to lose: it is the *other*
//!   migration's trigger, so a reader auditing this file against `000015` alone
//!   would find five and call the census complete. Without it a decided unit stops
//!   freeing the keys it held, `uq_pricing_approval_key_pending` keeps refusing a
//!   second unit on those keys forever, and no test that only decides an approval
//!   would notice.
//!
//! All six are re-created here, verbatim. `pricing_approval` carries **no inbound
//! foreign key** — `pricing_approval_key` reaches it through trigger sub-selects
//! rather than a `REFERENCES` clause, by `000017`'s own design — so the rebuild needs
//! no constraint work.
//!
//! **It carried no index either, and since `m20260802_000022` it does.** That
//! migration adds `uq_pricing_approval_policy_pending`, D-192's mint guard, and sorts
//! **after** this file, so this rebuild is still correct as it stands: on the way down
//! the guard's own `DROP INDEX` has already run, and on the way back up it is
//! re-created after this rebuild. What is no longer true is the general claim — a
//! rebuild of this table appended *after* `000022` would take the index with the
//! `DROP TABLE` and would have to restate it, the way the eight triggers below are
//! restated. `tests/sqlite_migrations.rs`'s index census is what says so.
//!
//! **And two more triggers have to move, on the other table.** The first draft of
//! this migration said the two `pricing_approval_key` triggers that sub-select this
//! table "are unaffected: they live on the *other* table". That was wrong and
//! `SQLite` said so, at the rename:
//!
//! ```text
//! error in trigger trg_pricing_approval_key_born_under_a_pending_unit:
//!   no such table: main.pricing_approval
//! ```
//!
//! `ALTER TABLE … RENAME TO` **re-parses the whole schema** (since 3.25, with
//! `legacy_alter_table` off), so a trigger left dangling by the `DROP TABLE` two
//! statements earlier fails the rename rather than waiting to fail at run time.
//! `trg_pricing_approval_key_born_under_a_pending_unit` and
//! `trg_pricing_approval_key_follows_its_unit` are those two — both on
//! `pricing_approval_key`, both reading `pricing_approval`'s `state` in a
//! sub-select — so both are dropped **before** the rebuild and re-created after it,
//! verbatim from `000017`. Eight triggers move here, across two tables, and the
//! re-parse is the reason a rebuild's census cannot stop at the table being rebuilt.
//!
//! `PRAGMA legacy_alter_table` was the other way out and is not taken: it would
//! leave the two dangling triggers in the schema to fail on the next `INSERT` into
//! the register instead, which is the same defect discovered later by a caller.
//!
//! `DROP TABLE` fires no row trigger in `SQLite`, so `trg_pricing_approval_no_delete`
//! does not refuse the rebuild's own drop — which is the one interaction that would
//! make this migration unrunnable rather than merely wrong.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_approval
        DROP CONSTRAINT chk_pricing_approval_subject_kind",
    "ALTER TABLE bss.pricing_approval
        ADD CONSTRAINT chk_pricing_approval_subject_kind CHECK (
            subject_kind IN ('plan_revision','price_unit','window','policy'))",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_approval
        DROP CONSTRAINT chk_pricing_approval_subject_kind",
    "ALTER TABLE bss.pricing_approval
        ADD CONSTRAINT chk_pricing_approval_subject_kind CHECK (
            subject_kind IN ('plan_revision','price_unit','window'))",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

/// The rebuild, parameterised on the CHECK body so `up` and `down` cannot drift.
///
/// Written as a function rather than two hand-copied statement lists because the
/// two directions differ in exactly four characters, and a hand-copied 60-line
/// rebuild is where the sixth trigger goes missing from one direction only.
macro_rules! sqlite_rebuild {
    ($kinds:literal) => {
        &[
            // The two `pricing_approval_key` triggers that sub-select this table.
            // Dropped first so the rename below can re-parse the schema; re-created
            // at the end, verbatim from `m20260802_000017`.
            "DROP TRIGGER IF EXISTS trg_pricing_approval_key_born_under_a_pending_unit",
            "DROP TRIGGER IF EXISTS trg_pricing_approval_key_follows_its_unit",
            concat!(
                "CREATE TABLE pricing_approval_rebuilt (
        approval_id         text NOT NULL PRIMARY KEY,
        tenant_id           text NOT NULL,
        subject_ref         text NOT NULL,
        subject_kind        text NOT NULL,
        content_hash        blob NOT NULL,
        state               text NOT NULL,
        submitter_principal text NOT NULL,
        approver_principal  text,
        reason              text,
        materiality         text NOT NULL,
        submitted_at        text NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        decided_at          text,
        CONSTRAINT chk_pricing_approval_distinct_principals CHECK (
            approver_principal IS NULL OR approver_principal <> submitter_principal),
        CONSTRAINT chk_pricing_approval_state CHECK (
            state IN ('submitted','approved','rejected','voided')),
        CONSTRAINT chk_pricing_approval_subject_kind CHECK (
            subject_kind IN (",
                $kinds,
                ")),
        CONSTRAINT chk_pricing_approval_decided_at CHECK (
            (state = 'submitted') = (decided_at IS NULL)),
        CONSTRAINT chk_pricing_approval_reason CHECK (
            state <> 'rejected' OR reason IS NOT NULL),
        CONSTRAINT chk_pricing_approval_approver CHECK (
            state IN ('submitted','voided') OR approver_principal IS NOT NULL)
    )"
            ),
            "INSERT INTO pricing_approval_rebuilt (
        approval_id, tenant_id, subject_ref, subject_kind, content_hash, state,
        submitter_principal, approver_principal, reason, materiality,
        submitted_at, decided_at)
     SELECT
        approval_id, tenant_id, subject_ref, subject_kind, content_hash, state,
        submitter_principal, approver_principal, reason, materiality,
        submitted_at, decided_at
     FROM pricing_approval",
            "DROP TABLE pricing_approval",
            "ALTER TABLE pricing_approval_rebuilt RENAME TO pricing_approval",
            // --- the five append-only arms of `m20260802_000015`, verbatim ---
            "CREATE TRIGGER trg_pricing_approval_born_submitted
        BEFORE INSERT ON pricing_approval
        FOR EACH ROW WHEN NEW.state <> 'submitted'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval: a record is born submitted');
        END",
            "CREATE TRIGGER trg_pricing_approval_no_delete
        BEFORE DELETE ON pricing_approval
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval: DELETE of an approval is not permitted; the record is the evidence');
        END",
            "CREATE TRIGGER trg_pricing_approval_immutable_once_decided
        BEFORE UPDATE ON pricing_approval
        FOR EACH ROW WHEN OLD.state <> 'submitted'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval: a decided record is immutable');
        END",
            "CREATE TRIGGER trg_pricing_approval_pinned_columns
        BEFORE UPDATE ON pricing_approval
        FOR EACH ROW WHEN OLD.state = 'submitted'
          AND (NEW.approval_id         IS NOT OLD.approval_id
            OR NEW.tenant_id           IS NOT OLD.tenant_id
            OR NEW.subject_ref         IS NOT OLD.subject_ref
            OR NEW.subject_kind        IS NOT OLD.subject_kind
            OR NEW.content_hash        IS NOT OLD.content_hash
            OR NEW.submitter_principal IS NOT OLD.submitter_principal
            OR NEW.materiality         IS NOT OLD.materiality
            OR NEW.submitted_at        IS NOT OLD.submitted_at)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval: the approval is pinned; only the decision columns may move');
        END",
            "CREATE TRIGGER trg_pricing_approval_flip_whitelist
        BEFORE UPDATE ON pricing_approval
        FOR EACH ROW WHEN OLD.state = 'submitted'
          AND NEW.state NOT IN ('approved','rejected','voided')
        BEGIN
          SELECT RAISE(ABORT, 'pricing_approval: state transition is not a sanctioned flip');
        END",
            // --- and the one `m20260802_000017` put here, verbatim ---
            "CREATE TRIGGER trg_pricing_approval_key_follow_state
        AFTER UPDATE OF state ON pricing_approval
        FOR EACH ROW WHEN NEW.state IS NOT OLD.state
        BEGIN
          UPDATE pricing_approval_key
             SET state = NEW.state
           WHERE approval_id = NEW.approval_id;
        END",
            // --- and the two on `pricing_approval_key`, back again ---
            "CREATE TRIGGER trg_pricing_approval_key_born_under_a_pending_unit
        BEFORE INSERT ON pricing_approval_key
        FOR EACH ROW WHEN (SELECT state FROM pricing_approval
                            WHERE approval_id = NEW.approval_id) IS NOT 'submitted'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval_key: a register row is born with a pending unit; this approval is missing or already decided');
        END",
            "CREATE TRIGGER trg_pricing_approval_key_follows_its_unit
        BEFORE UPDATE OF state ON pricing_approval_key
        FOR EACH ROW WHEN NEW.state IS NOT (SELECT state FROM pricing_approval
                                             WHERE approval_id = NEW.approval_id)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_approval_key: a register row follows its unit; its state cannot be moved on its own');
        END",
        ]
    };
}

const SQLITE_UP_STATEMENTS: &[&str] =
    sqlite_rebuild!("'plan_revision','price_unit','window','policy'");

const SQLITE_DOWN_STATEMENTS: &[&str] = sqlite_rebuild!("'plan_revision','price_unit','window'");

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
