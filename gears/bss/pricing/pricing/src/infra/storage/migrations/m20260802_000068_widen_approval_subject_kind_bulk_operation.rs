//! `chk_pricing_approval_subject_kind` gains `'bulk_operation'` — the sixth member
//! of D-158's enumeration, arriving with a writer on **one** of the two planes it
//! spans, exactly as `'overlay'` did at `m20260802_000035`.
//!
//! Same shape as that migration and for the same reason: `AuditSubjectKind` spells
//! **two** columns.
//!
//! * `pricing_audit_log.subject_kind` — the writer exists. The mass-repricing run's
//!   open (`api::rest::repricing_runs::open_run_in`) appends an audit record as of this
//!   change; before it, opening a run wrote no audit record at all — the debt that
//!   module's own doc named, because there was no `AuditSubjectKind` for a bulk
//!   operation to carry. That column carries no `CHECK` (see "Why the asymmetry stays"
//!   below), so nothing in the store was in the way of it — the whole obstacle was the
//!   enumeration having no token.
//! * `pricing_approval.subject_kind` — **no writer on this date**. `inst-bs-approval`'s
//!   batch approval, the unit `POST /repricing-runs`' `validating -> awaiting_approval`
//!   edge needs, is not wired in this change. It was wired since, by
//!   `api::rest::repricing_runs::advance_on_verdict`, so read this bullet as this
//!   migration's dated state; `approval_repo::SUBJECT_KINDS_WITH_A_WRITER` is the
//!   maintained roster of which kinds are written.
//!
//! D-158 requires the two stores to spell one enumeration and to be **extended
//! together**, and `sqlite_approval_repo::every_subject_kind_d158_declares_is_storable_on_the_mirror`
//! enforces it by opening a record of every `AuditSubjectKind::ALL` member against this
//! very constraint — which is also `tests/sqlite_approval_repo.rs`'s and
//! `tests/postgres_approval.rs`'s admission proof for this token, so no CHECK-roster
//! test lives in this file. So the token is widened here **because** the audit plane's
//! member landed, and it is the narrower claim of the two: the kind is *storable* and
//! not yet *stored*.
//!
//! # Why a migration of its own rather than an amendment to `000035`
//!
//! `000035`'s reason, unchanged: the history stays legible, and a reader asking when
//! `bulk_operation` became storable gets a dated answer instead of a `git blame`. The
//! `down` is exactly the inverse.
//!
//! # Why the asymmetry stays — `pricing_audit_log.subject_kind` still carries no `CHECK`
//! (review finding Z6-6, 2026-08-10)
//!
//! `pricing_approval.subject_kind` is `CHECK`-constrained and `pricing_audit_log`'s
//! twin column is not, though both are typed from `AuditSubjectKind` and D-158 asks for
//! one enumeration. `m20260802_000015`'s doc already gives the reason the asymmetry was
//! introduced: `pricing_audit_log` "types its own `subject_kind` as free `text` (the
//! column predates any declared vocabulary)" — `m20260802_000010` created it before
//! `AuditSubjectKind` existed to constrain it against. That reason is now nine
//! migrations and several months stale: the vocabulary has been stable since `000019`,
//! and every writer of `pricing_audit_log.subject_kind` is typed (`AuditAction::as_str`
//! / `AuditSubjectKind::as_str`, never a hand-written literal), so a `CHECK` would cost
//! nothing at the writers and would close the same "table written around" gap
//! `chk_pricing_approval_subject_kind` closes for its sibling.
//!
//! **This migration does not add it**, and the reason is scope rather than difficulty:
//! this file's one concern is D-158's roster gaining `bulk_operation`, on the plane
//! that already enforces it, in the shape `000019` and `000035` already established.
//! Bolting an unrelated first-time guard onto `pricing_audit_log` — a different table,
//! needing its own `SQLite` rebuild of its own two indexes and two triggers, none of
//! which this migration otherwise touches — is exactly the "two concerns, one
//! migration" shape the section above argues against for `000035` vs `000019`. Z6-6 is
//! filed Low, no writer can produce an unlisted token today, and the fix (a `CHECK` over
//! `AuditSubjectKind::ALL`, plus the `SQLite` rebuild that comes with any `CHECK` added
//! to an existing `SQLite` table) is independent of whether `bulk_operation` exists. It
//! is left as the open finding it already was rather than folded in here unremarked.
//!
//! # The `SQLite` half is `000035`'s rebuild, retokened
//!
//! Same nine objects move: the six triggers off `pricing_approval` itself (five from
//! `000015` plus `trg_pricing_approval_key_follow_state` from `000017`), the two on
//! `pricing_approval_key` that sub-select this table (dropped before the rename,
//! restored after — `ALTER TABLE … RENAME TO` re-parses the schema and a trigger left
//! dangling by the `DROP TABLE` fails the rename), and `uq_pricing_approval_policy_pending`
//! (D-192 clause (2)), restated **after** the copy so the copy cannot trip a guard the
//! rows it is copying already satisfy. Nothing has rebuilt `pricing_approval` between
//! `000035` and here, so this rebuild's object set is exactly `000035`'s.

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
            subject_kind IN ('plan_revision','price_unit','window','policy','overlay','bulk_operation'))",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_approval
        DROP CONSTRAINT chk_pricing_approval_subject_kind",
    "ALTER TABLE bss.pricing_approval
        ADD CONSTRAINT chk_pricing_approval_subject_kind CHECK (
            subject_kind IN ('plan_revision','price_unit','window','policy','overlay'))",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

/// The rebuild, parameterised on the CHECK body so `up` and `down` cannot drift.
///
/// `m20260802_000035`'s macro, retokened — see the module doc for why the object set
/// is unchanged from that migration's.
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
        state                text NOT NULL,
        submitter_principal text NOT NULL,
        approver_principal  text,
        reason              text,
        materiality          text NOT NULL,
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
            // --- the index `m20260802_000022` put here, verbatim, and after the
            // copy: the rows being copied already satisfy it, so creating it first
            // would only add a way for the rebuild to fail. `m20260802_000019`'s doc
            // predicted this statement.
            "CREATE UNIQUE INDEX uq_pricing_approval_policy_pending
        ON pricing_approval (tenant_id)
        WHERE subject_kind = 'policy' AND state = 'submitted'",
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
    sqlite_rebuild!("'plan_revision','price_unit','window','policy','overlay','bulk_operation'");

const SQLITE_DOWN_STATEMENTS: &[&str] =
    sqlite_rebuild!("'plan_revision','price_unit','window','policy','overlay'");

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
