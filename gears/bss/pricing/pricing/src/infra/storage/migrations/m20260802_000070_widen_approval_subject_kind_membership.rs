//! `chk_pricing_approval_subject_kind` gains `'membership'` — the seventh member
//! of D-158's enumeration, arriving with a writer on **neither** of the two
//! planes it spans, unlike every migration that widened this CHECK before it.
//!
//! # Why this migration exists at all, given nothing opens a membership approval
//! yet
//!
//! `group_membership_repo` (Task 4 of the customer-group plane, 2026-08-11)
//! declared `AuditSubjectKind::Membership` and gave it an audit-plane writer
//! (`enroll` / `end_membership` append to `pricing_audit_log`); no approval-plane
//! writer exists — `infra::approval::re_derive` and `approval_repo::subject_aggregate`
//! both refuse this kind outright, by name, because `inst-mm-*`'s materiality is
//! unwired.
//!
//! That absence of a writer is **not** what obliges this migration.
//! `sqlite_approval_repo::every_subject_kind_d158_declares_is_storable_on_the_mirror`
//! iterates `AuditSubjectKind::ALL` — the *declaration*, not the set of kinds
//! anything currently writes — and opens a `pricing_approval` record of each
//! against this very CHECK, asserting every declared kind is admitted. It
//! reddened the moment `Membership` became a variant, with no route, no service
//! and no writer anywhere near `pricing_approval` having been touched. **This is
//! D-158's mechanism working as designed, not a gap it missed**: "the two stores
//! spell one enumeration, extended together" binds the *declaration*, and a kind
//! is declared the moment its Rust variant exists — `AuditSubjectKind::ALL` says
//! so unconditionally, with no side-channel for "declared, but not yet wired
//! anywhere, so this one doesn't count." A task review on the group-membership
//! change reasoned the opposite way — nothing writes an approval of this kind, so
//! the CHECK need not admit it — and that reasoning is exactly backwards: it is
//! true about *writers* and false about the *invariant* this CHECK enforces,
//! which is a fact about the enumeration and not about who currently exercises
//! it. **The next person adding a member to `AuditSubjectKind` should read this
//! paragraph before reaching for the test's green run as evidence that a widening
//! migration can wait for a writer.** It cannot: the roster test binds the
//! declaration, and the declaration is what obliges the CHECK, on the very next
//! `cargo test` after the variant exists — which is also why this migration is
//! numbered days after `AuditSubjectKind::Membership` landed rather than beside
//! it: the gap here is exactly the reddened window between "declared" and "this
//! migration merged."
//!
//! Same shape as `m20260802_000068` and `m20260802_000035` before it, and for the
//! same reason: `AuditSubjectKind` spells **two** columns.
//!
//! * `pricing_audit_log.subject_kind` — the writer exists, and always did as of
//!   the same change that declared the variant (see the module doc's own note:
//!   that column carries no `CHECK` at all, so nothing in the store was ever in
//!   the way of it).
//! * `pricing_approval.subject_kind` — **no writer**, and this migration does not
//!   add one. It only makes the kind *storable*, which is D-158's narrower claim
//!   and the one `chk_pricing_approval_subject_kind` is capable of enforcing; the
//!   approval-plane writer (wiring `inst-mm-*`'s material edge into
//!   `ApprovalService`) is a later task's, unaffected by whether this CHECK
//!   admits the token today.
//!
//! # Why a migration of its own rather than an amendment to `000068`
//!
//! `000068`'s reason, unchanged, `000035`'s before it: the history stays legible,
//! and a reader asking when `membership` became storable gets a dated answer
//! instead of a `git blame`. The `down` is exactly the inverse.
//!
//! # The `pricing_audit_log` asymmetry (review finding Z6-6) is inherited, not
//! addressed
//!
//! `m20260802_000068`'s module doc records the decision this migration follows
//! rather than differs from: `pricing_approval.subject_kind` is `CHECK`-constrained
//! and `pricing_audit_log`'s twin column is not, though both are typed from
//! `AuditSubjectKind`. Z6-6 is filed against that asymmetry and stays open here —
//! this migration's one concern is `chk_pricing_approval_subject_kind` gaining
//! `membership`, on the plane that already enforces a CHECK, in the shape `000019`,
//! `000035` and `000068` already established. Adding a first-time guard to
//! `pricing_audit_log` — a different table, needing its own `SQLite` rebuild of its
//! own indexes and triggers, none of which this migration otherwise touches — is
//! the "two concerns, one migration" shape `000068`'s doc already argues against.
//! Nothing about `membership` landing makes Z6-6 more or less true than it was the
//! day it was filed, so nothing about it is folded in here.
//!
//! # No new constraint name, so no new roster entry
//!
//! `chk_pricing_approval_subject_kind` is **widened**, not created:
//! `tests/postgres_migrations.rs`'s and `tests/sqlite_migrations.rs`'s constraint
//! censuses carry it by name already (from `000019`), and a name that does not
//! change needs no new roster line — unlike `000067`'s new table, which did add
//! entries. Checked rather than assumed, per this migration's own review round.
//!
//! # The `SQLite` half is `000068`'s rebuild, retokened
//!
//! Same nine objects move, `000068`'s doc's own accounting: the six triggers off
//! `pricing_approval` itself, the two on `pricing_approval_key` that sub-select
//! this table (dropped before the rename, restored after), and
//! `uq_pricing_approval_policy_pending` (restated after the copy). Nothing has
//! rebuilt `pricing_approval` between `000068` and here, so this rebuild's object
//! set is exactly `000068`'s.

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
            subject_kind IN ('plan_revision','price_unit','window','policy','overlay','bulk_operation','membership'))",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_approval
        DROP CONSTRAINT chk_pricing_approval_subject_kind",
    "ALTER TABLE bss.pricing_approval
        ADD CONSTRAINT chk_pricing_approval_subject_kind CHECK (
            subject_kind IN ('plan_revision','price_unit','window','policy','overlay','bulk_operation'))",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

/// The rebuild, parameterised on the CHECK body so `up` and `down` cannot drift.
///
/// `m20260802_000068`'s macro, retokened — see the module doc for why the object
/// set is unchanged from that migration's.
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

const SQLITE_UP_STATEMENTS: &[&str] = sqlite_rebuild!(
    "'plan_revision','price_unit','window','policy','overlay','bulk_operation','membership'"
);

const SQLITE_DOWN_STATEMENTS: &[&str] =
    sqlite_rebuild!("'plan_revision','price_unit','window','policy','overlay','bulk_operation'");

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
