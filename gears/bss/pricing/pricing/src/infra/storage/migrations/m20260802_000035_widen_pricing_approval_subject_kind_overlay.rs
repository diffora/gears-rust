//! `chk_pricing_approval_subject_kind` gains `'overlay'` — the fifth member of
//! D-158's enumeration, arriving with a writer on **one** of the two planes it spans.
//!
//! `m20260802_000019`'s sibling, and the difference from it is the part worth
//! reading. That migration added `'policy'` and its module doc states the gear's
//! standing rule in full: the Rust member, this token and the writer *"land together
//! or none of them lands"*. Here they do not, and the reason is that
//! `AuditSubjectKind` spells **two** columns:
//!
//! * `pricing_audit_log.subject_kind` — the writer exists. `OverlayRepo`'s four
//!   mutations append an audit record as of this change; before it the overlay plane
//!   wrote none at all, which was a plain regression against D-14 on an
//!   always-material subject (Slice 9's register, **O-3**). That column carries no
//!   `CHECK`, so nothing in the store was in the way of it — the whole obstacle was
//!   the enumeration having no token.
//! * `pricing_approval.subject_kind` — **no writer on this date**. D-50 makes every
//!   overlay mutation an approval subject, but the unit that would open one is
//!   Slice 9's **O-7** and was unwired; `infra::approval.rs` and `infra::publish.rs`
//!   were on the overlay strand's forbidden list, which is exactly why it is owed
//!   here. D-225 wired it since, so read this bullet — and the refusals cited two
//!   paragraphs down — as this migration's dated state and not as a live claim;
//!   `approval_repo::SUBJECT_KINDS_WITH_A_WRITER` is the maintained roster.
//!
//! D-158 requires the two stores to spell one enumeration and to be **extended
//! together**, and `sqlite_approval_repo::every_subject_kind_d158_declares_is_storable_on_the_mirror`
//! enforces it by opening a record of every `AuditSubjectKind::ALL` member against
//! this very constraint. So the token is widened here **because** the audit plane's
//! member landed, and it is the narrower claim of the two: the kind is *storable* and
//! not yet *stored*. On this date `approval_repo::subject_aggregate` and
//! `infra::approval::re_derive`'s overlay arms both refused a record carrying it, and
//! said in as many words that one appearing did not come from this crate — which is
//! what kept "storable" from being read as "resolvable". Both arms resolve it now
//! (D-225); the distinction the sentence draws is what survives, not its example.
//!
//! # Why a migration of its own rather than an amendment to `000019`
//!
//! `000018`'s reason, which is the chain's rule: the history stays legible, and a
//! reader asking when `price_overlay` became storable gets a dated answer instead of
//! a `git blame`. The `down` is exactly the inverse.
//!
//! # The `SQLite` half is a table rebuild, and it moves **nine** objects, not eight
//!
//! `SQLite` has no `ALTER TABLE … DROP CONSTRAINT`, so widening a CHECK is
//! `000018`'s create-copy-drop-rename dance. `000019` did this once and its module
//! doc enumerates the eight triggers that move with it — six on `pricing_approval`
//! (five from `000015` plus `trg_pricing_approval_key_follow_state` from `000017`,
//! the one a reader auditing against `000015` alone would miss) and two on
//! `pricing_approval_key` that sub-select this table and must be dropped **before**
//! the rename, because `ALTER TABLE … RENAME TO` re-parses the whole schema and a
//! trigger left dangling by the `DROP TABLE` fails the rename rather than waiting to
//! fail at run time.
//!
//! **And `000019` names the ninth in advance.** Its doc records that it carried no
//! index *"and since `m20260802_000022` it does"*, and states the consequence
//! precisely: *"a rebuild of this table appended **after** `000022` would take the
//! index with the `DROP TABLE` and would have to restate it, the way the eight
//! triggers below are restated."* This is that rebuild.
//! `uq_pricing_approval_policy_pending` — D-192 clause (2)'s "one open policy
//! proposal per tenant" — is therefore re-created here, verbatim from `000022`, and
//! **after** the copy, so the copy cannot trip a guard the rows it is copying already
//! satisfy. Losing it would not fail anything at migration time; it would let a
//! second policy proposal open, silently, forever.
//!
//! Nothing else attached to `pricing_approval` between `000022` and here: the only
//! other migration whose triggers *name* it is `000020`, whose two are on
//! `pricing_approval_threshold_tombstone` and sub-select nothing —
//! `pricing_approval` appears in that file's prose alone. `pricing_approval` still
//! carries no inbound foreign key (`000017`'s design: the register reaches it through
//! trigger sub-selects), so the rebuild needs no constraint work.
//!
//! `DROP TABLE` fires no row trigger in `SQLite`, so `trg_pricing_approval_no_delete`
//! does not refuse the rebuild's own drop.

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
            subject_kind IN ('plan_revision','price_unit','window','policy','overlay'))",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_approval
        DROP CONSTRAINT chk_pricing_approval_subject_kind",
    "ALTER TABLE bss.pricing_approval
        ADD CONSTRAINT chk_pricing_approval_subject_kind CHECK (
            subject_kind IN ('plan_revision','price_unit','window','policy'))",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

/// The rebuild, parameterised on the CHECK body so `up` and `down` cannot drift.
///
/// `m20260802_000019`'s macro with one statement added — the index. Written as a
/// function of the token list for its reason: the two directions differ in one
/// literal, and a hand-copied seventy-line rebuild is where an object goes missing
/// from one direction only.
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
    sqlite_rebuild!("'plan_revision','price_unit','window','policy','overlay'");

const SQLITE_DOWN_STATEMENTS: &[&str] =
    sqlite_rebuild!("'plan_revision','price_unit','window','policy'");

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
