//! `pricing_audit_log.subject_kind` gains the CHECK its `pricing_approval`
//! sibling has carried since `m20260802_000015` — review finding **Z6-6**.
//!
//! # The asymmetry this closes
//!
//! Both columns hold **one** vocabulary from **one** Rust enum,
//! `domain::audit::AuditSubjectKind`, and D-158 requires the two stores to spell
//! it identically and to extend it in step — `entity/approval.rs` states that as a
//! rule. Only one of the two was held to it. `chk_pricing_approval_subject_kind`
//! has existed since the approval table was created and has been widened four
//! times (`000019`, `000035`, `000068`, `000070`); `pricing_audit_log.subject_kind`
//! was `text NOT NULL` and nothing more, on either engine, and
//! `m20260802_000010`'s module doc enumerates the table's guards ("Two physical
//! guards") without mentioning the absence.
//!
//! **No live break, and that is not the argument for waiting.** Every writer of
//! the column is typed — `audit_repo::append` is the only site that builds an
//! `audit_log::ActiveModel`, and it writes `entry.subject_kind.as_str()` off the
//! enum — so no unspelled token can arrive through the crate as it now stands. The
//! reason to make the constraint physical anyway is what the table *is*: a
//! hash-chained record retained for seven-plus years, whose rows are immutable by
//! two triggers and by design. It is the last place a token should be able to
//! arrive unspelled and the one place a wrong token cannot be corrected afterwards
//! — a `subject_kind` fixed by an `UPDATE` is a broken chain, and one fixed by a
//! `DELETE` plus re-insert is a re-written one.
//!
//! `m20260802_000070`'s module doc argued Z6-6 belonged in a migration of its own
//! rather than folded into a widening of the approval CHECK — "a different table,
//! needing its own `SQLite` rebuild of its own indexes and triggers". This is that
//! migration.
//!
//! # `action` is deliberately left free-form
//!
//! The same review line notes `action` on this table is also unconstrained. It is
//! left so, and that is a decision rather than an omission: `action` has no
//! enumeration anywhere — `NewAuditEntry.action` is a `String` its callers spell
//! per site ("publish", "abandon", "window.cancel", …) — so there is no roster to
//! hold it to, and a CHECK minted from today's call sites would be a second,
//! narrower vocabulary that the next audited act would have to migrate. A CHECK
//! over a set nothing declares is a guess. `subject_kind` is the opposite case:
//! `AuditSubjectKind::ALL` is the declared roster, and the constraint below is
//! exactly it.
//!
//! # The seven tokens, and how they stay in step
//!
//! The `IN` list is `AuditSubjectKind::ALL`'s seven `as_str` tokens, in the enum's
//! own order. Keeping them in step is not this file's promise but a test's:
//! `sqlite_audit_chain::every_subject_kind_d158_declares_is_storable_in_the_trail`
//! iterates `AuditSubjectKind::ALL` and inserts one row of each kind against this
//! very CHECK, so an eighth variant reddens on the next `cargo test` with nothing
//! else touched — the mechanism `m20260802_000070`'s doc describes at length for
//! the approval plane, now covering both planes the enum spells.
//!
//! # No backfill, and why none is owed
//!
//! Every row already in the table was written by the one typed writer above, so
//! every existing `subject_kind` is one of the seven and the constraint admits the
//! table as it stands. That is a claim about the writer rather than about the data,
//! which is why the `SQLite` arm copies the rows through the new CHECK rather than
//! adding it afterwards: on that engine the copy **is** the verification, and a row
//! carrying an unspelled token would fail the rebuild loudly instead of leaving a
//! constraint that lies about the rows beneath it. Postgres verifies the existing
//! rows for the same reason — `ADD CONSTRAINT` without `NOT VALID` scans the table.
//!
//! # The `SQLite` rebuild
//!
//! `SQLite` cannot `ALTER TABLE … ADD CONSTRAINT`, so the arm rebuilds the table.
//! Four objects move and they are all this table's own: the two indexes
//! `m20260802_000010` created (`idx_pricing_audit_log_recorded`,
//! `idx_pricing_audit_log_subject`) and the two `RAISE(ABORT)` triggers that make
//! it append-only. **No trigger anywhere else sub-selects `pricing_audit_log`** —
//! checked against every migration in the chain, not assumed, because
//! `ALTER TABLE … RENAME TO` re-parses the whole schema and a sibling trigger left
//! standing is what `m20260802_000063` had to drop first. The copy runs while the
//! table carries no triggers at all, and `DROP TABLE` fires none, so the
//! append-only pair never has to be worked around.
//!
//! The rebuild is parameterised on the CHECK clause so `up` and `down` cannot
//! drift: `down` is the same rebuild with the constraint absent, which is an exact
//! inverse rather than a hand-written second table.
//!
//! # One new constraint name, so two roster lines
//!
//! Unlike the four widenings, this **creates** a name:
//! `chk_pricing_audit_log_subject_kind` joins `EXPECTED_CHECKS` in
//! `tests/sqlite_migrations.rs` and in `tests/postgres_migrations.rs`. Both rosters
//! are asserted in both directions, so the entry is owed on both engines.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

// The token list is `AuditSubjectKind::ALL`'s seven `as_str` values in the enum's
// own order, and it appears twice in this file — once per engine's `up`. Neither
// `down` restates it (the Postgres one drops a constraint by name and the `SQLite`
// one rebuilds without it), so there is no third copy to keep true, and the test
// named in the module doc is what binds both copies to the enum.
const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_audit_log
        ADD CONSTRAINT chk_pricing_audit_log_subject_kind CHECK (
            subject_kind IN ('plan_revision','price_unit','window','policy','overlay','bulk_operation','membership'))",
];

const PG_DOWN_STATEMENTS: &[&str] = &["ALTER TABLE bss.pricing_audit_log
        DROP CONSTRAINT chk_pricing_audit_log_subject_kind"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

/// The rebuild, parameterised on the constraint clause so `up` and `down` cannot
/// drift — `m20260802_000070`'s macro, over this table's four objects.
macro_rules! sqlite_rebuild {
    ($subject_kind_check:literal) => {
        &[
            concat!(
                "CREATE TABLE pricing_audit_log_rebuilt (
        tenant_id          text   NOT NULL,
        chain_id           text   NOT NULL,
        seq                bigint NOT NULL,
        entry_kind         text   NOT NULL DEFAULT 'mutation',
        recorded_at        text   NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        actor_principal_id text   NOT NULL,
        action             text   NOT NULL,
        subject_kind       text   NOT NULL,
        subject_ref        text   NOT NULL,
        before_state       text,
        after_state        text,
        approval_ref       text,
        correlation_id     text,
        segment_heads      text,
        prev_hash          blob,
        row_hash           blob   NOT NULL,
        PRIMARY KEY (tenant_id, chain_id, seq),
        CONSTRAINT chk_pricing_audit_log_seq CHECK (seq >= 0),
        CONSTRAINT chk_pricing_audit_log_entry_kind CHECK (
            entry_kind IN ('mutation','rollup')),",
                $subject_kind_check,
                "
        CONSTRAINT chk_pricing_audit_log_rollup CHECK (
            (entry_kind = 'rollup') = (segment_heads IS NOT NULL))
    )"
            ),
            // The copy runs before any trigger exists on the rebuilt table, and
            // `DROP TABLE` below fires none on the old one, so the append-only pair
            // needs no working around.
            "INSERT INTO pricing_audit_log_rebuilt (
        tenant_id, chain_id, seq, entry_kind, recorded_at, actor_principal_id,
        action, subject_kind, subject_ref, before_state, after_state,
        approval_ref, correlation_id, segment_heads, prev_hash, row_hash)
     SELECT
        tenant_id, chain_id, seq, entry_kind, recorded_at, actor_principal_id,
        action, subject_kind, subject_ref, before_state, after_state,
        approval_ref, correlation_id, segment_heads, prev_hash, row_hash
     FROM pricing_audit_log",
            "DROP TABLE pricing_audit_log",
            "ALTER TABLE pricing_audit_log_rebuilt RENAME TO pricing_audit_log",
            // --- the two indexes `m20260802_000010` created, verbatim ---
            "CREATE INDEX idx_pricing_audit_log_recorded
        ON pricing_audit_log (tenant_id, recorded_at)",
            "CREATE INDEX idx_pricing_audit_log_subject
        ON pricing_audit_log (tenant_id, subject_kind, subject_ref, recorded_at)",
            // --- and its two append-only arms, verbatim ---
            "CREATE TRIGGER trg_pricing_audit_log_no_update
        BEFORE UPDATE ON pricing_audit_log
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT, 'pricing_audit_log is append-only: UPDATE is not permitted');
        END",
            "CREATE TRIGGER trg_pricing_audit_log_no_delete
        BEFORE DELETE ON pricing_audit_log
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT, 'pricing_audit_log is append-only: DELETE is not permitted');
        END",
        ]
    };
}

const SQLITE_UP_STATEMENTS: &[&str] = sqlite_rebuild!(
    "
        CONSTRAINT chk_pricing_audit_log_subject_kind CHECK (
            subject_kind IN ('plan_revision','price_unit','window','policy','overlay','bulk_operation','membership')),"
);

/// The inverse: the same table without the constraint this migration adds.
const SQLITE_DOWN_STATEMENTS: &[&str] = sqlite_rebuild!("");

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
