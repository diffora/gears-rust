//! Create `bss.pricing_approval` — the approval workflow's record
//! (`design/05-governance.md` §6, `cpt-cf-bss-pricing-state-approval`), the
//! store the two-person rule is decided in and the only thing that can hand
//! `PublishAuthorization::Approved` to a publish commit.
//!
//! The state machine of §4 lives in the constraints, not only in the domain:
//! `submitted -> approved | rejected | voided` and nothing else
//! (`inst-as-approve`, `inst-as-reject`, `inst-as-void`), a decided record
//! frozen forever (`inst-as-immutable`), a reject carrying its mandatory reason,
//! and an approver who is never the submitter (`inst-tp-distinct`). Each of
//! those is a rule the domain also states, and each is here for the reason
//! `pricing_plan`'s whitelist is: a rule that lives only in application code is
//! one ad-hoc `UPDATE` away from being bypassed, and what it would be bypassing
//! here is the evidence that a human other than the author agreed to the price
//! change.
//!
//! # `subject_kind` declares a **subset** of S5 §6's ten, and the CHECK is the roster
//!
//! D-158 requires `pricing_approval` and `pricing_audit_log` to spell **one**
//! enumeration and to be extended together, so that the approval record and the
//! audit record of one decision cannot disagree about what the decision was
//! about. What is declared is therefore whatever `AuditSubjectKind` declares, under
//! the same section's standing rule that a token with no writer is not declared —
//! and **the CHECK below is the roster; this paragraph deliberately does not repeat
//! its members.** It used to, and the count went stale the day `window` arrived:
//! a count in prose beside a roster in code leaves only one of the two true, and it
//! is never the prose. `AuditSubjectKind::ALL` and
//! `tests/sqlite_approval_repo.rs`'s
//! `every_subject_kind_d158_declares_is_storable_on_the_mirror` are what hold the
//! two sides equal, over `ALL` rather than over a written-out list.
//!
//! Declaring the members that have no writer would break D-158 in the direction it
//! exists to prevent — and would read as coverage to everyone who greps for it,
//! since nothing in this gear can open an approval over an overlay, a membership, a
//! bundle, a retirement, a policy, a historical import or a bulk batch.
//!
//! The `CHECK` is what makes that a declaration rather than a comment.
//! `pricing_audit_log` types its own `subject_kind` as free `text` (the column
//! predates any declared vocabulary); this table does not repeat that, because
//! S5 §6 types the column `enum` and the gear already spells the same
//! discriminator with a `CHECK` on `pricing_read_model` and
//! `pricing_catalog_version_ref`.
//!
//! # Two divergences from S5 §6's literal column notes, both reported
//!
//! **`approver_principal <> submitter_principal` needs an `IS NULL` arm.** The
//! table's note gives the bare comparison, and the same row says
//! `approver_principal` is "NULL until decided". A bare `<>` is NULL — hence
//! *satisfied* — on both engines when either side is NULL, so the literal form
//! happens to work; but writing it bare states an invariant the column's own
//! nullability contradicts, and a later reader tightening it to
//! `IS DISTINCT FROM` (the spelling that means what the sentence says) would
//! make **every open record unstorable** at a stroke. The arm is spelled out so
//! that reading is unavailable.
//!
//! **`subject_ref` is `text` and the principals are `uuid`.** S5 §6 types
//! `subject_ref` as `uuid` and both principals as `string`; this table inverts
//! both. A plan revision's durable name is `(plan_id, revision)` — rendered
//! `<plan_id>/<revision>` by `audit_repo::plan_revision_ref` and stored as
//! `text` in `pricing_audit_log.subject_ref` — so a `uuid` column could not hold
//! the one subject this phase writes, and D-158's "same enumeration" would be
//! paired with two incompatible reference types. The principals go the other
//! way for the mirror-image reason: `pricing_audit_log.actor_principal_id` is
//! `uuid`, and the two-person rule compares an approver against a submitter
//! whose identity the audit trail already holds in that type. Both are reported
//! rather than reconciled by editing the design set.
//!
//! # The trigger is the whitelist shape, and what it pins
//!
//! **A record is born `submitted`.** `INSERT` of any other state is refused
//! outright — §4 names `submitted` as the machine's initial state, and every
//! other rule below is written about a row that started there. This arm is a
//! correction: the trigger first guarded `UPDATE` and `DELETE` only, which left
//! a row free to be born `approved` with the whole decision plane bypassed
//! *because there was no `UPDATE` to bypass it on*. On a table whose entire
//! purpose is to be the evidence that a second human agreed, that is the
//! two-person rule defeated by one statement — and once publish reads
//! `PublishAuthorization` off this table, defeated silently. The four sibling
//! migrations `000011`-`000014` all guard `BEFORE INSERT OR UPDATE OR DELETE`;
//! this one now does too.
//!
//! `DELETE` is refused unconditionally: a decided record is the evidence, and a
//! `submitted` one is what `PENDING_CHANGE_UNIT_EXISTS` reads, so removing
//! either is removing the answer to a question rather than tidying up.
//!
//! An `UPDATE` of a record that is no longer `submitted` is refused outright —
//! `inst-as-immutable`, and a re-submit opens a **new** record. On the
//! `submitted` plane the eight non-decision columns are pinned, which is not
//! bookkeeping either: `content_hash` **is** the TOCTOU guard (`inst-ap-pin`),
//! and a hash that could be re-pinned in place would let the mutation the guard
//! exists to catch be laundered into an approval that verifies. Exactly four
//! columns may move, once: `state`, `approver_principal`, `reason`,
//! `decided_at`. Membership is tested rather than change, as
//! `pricing_plan_append_only` tests it and for the same reason — a
//! `NEW IS DISTINCT FROM OLD` conjunct would let the `SQLite` mirror accept a
//! no-op the Postgres branch refuses, and a backend divergence is worse than the
//! hole it would close.
//!
//! There is no `REVOKE`. It names a deployment role this migration does not own
//! and `SQLite` has no `GRANT`/`REVOKE` at all; the trigger is the portable half
//! of the discipline the design set calls "REVOKE + trigger"
//! (`m20260802_000001_create_pricing_plan.rs`).
//!
//! **Backend differences.** The systematic type mirror (`uuid` -> `text`,
//! `timestamptz` -> `text`, `jsonb` -> `text`, `bytea` -> `blob`), plus the
//! trigger split: Postgres carries one PL/pgSQL function interpolating the
//! offending values, while `SQLite` has no procedural language and
//! `RAISE(ABORT, ...)` takes a **literal** message only, so the same five rules
//! become five triggers with fixed messages and `IS DISTINCT FROM` written
//! `IS NOT`. The Postgres `down` drops the function as well as the table; the
//! `SQLite` one drops only the table, there being no function to drop.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_approval (
        approval_id         uuid        NOT NULL PRIMARY KEY,
        tenant_id           uuid        NOT NULL,
        subject_ref         text        NOT NULL,
        subject_kind        text        NOT NULL,
        content_hash        bytea       NOT NULL,
        state               text        NOT NULL,
        submitter_principal uuid        NOT NULL,
        approver_principal  uuid,
        reason              text,
        materiality         jsonb       NOT NULL,
        submitted_at        timestamptz NOT NULL DEFAULT now(),
        decided_at          timestamptz,
        -- The two-person rule at the storage layer (`inst-tp-distinct`). The
        -- `IS NULL` arm is deliberate and is a divergence from S5 6's literal
        -- note; see the module doc.
        CONSTRAINT chk_pricing_approval_distinct_principals CHECK (
            approver_principal IS NULL OR approver_principal <> submitter_principal),
        CONSTRAINT chk_pricing_approval_state CHECK (
            state IN ('submitted','approved','rejected','voided')),
        -- D-158's enumeration, exactly as `AuditSubjectKind` declares it, and the
        -- roster the module doc points at rather than restating. It is extended
        -- **with** its writer and never ahead of one: `window` joined both spellings
        -- in the change that mounted the three window surfaces, and acquired its
        -- approval-side writer -- `ApprovalService::submit_window_mutation` -- in the
        -- change that widened the pending register to keys. Every member S5 6 lists
        -- and this does not is a member with no writer here.
        CONSTRAINT chk_pricing_approval_subject_kind CHECK (
            subject_kind IN ('plan_revision','price_unit','window')),
        -- Pending and undecided are the same state, spelled once.
        CONSTRAINT chk_pricing_approval_decided_at CHECK (
            (state = 'submitted') = (decided_at IS NULL)),
        -- `REASON_REQUIRED` at the storage layer (`inst-as-reject`).
        CONSTRAINT chk_pricing_approval_reason CHECK (
            state <> 'rejected' OR reason IS NOT NULL),
        -- An approved or rejected record names who decided it. A voided one does
        -- not: a TOCTOU void has no human decider, and a withdraw's decider is
        -- the submitter, whom the distinctness CHECK above forbids in that
        -- column.
        CONSTRAINT chk_pricing_approval_approver CHECK (
            state IN ('submitted','voided') OR approver_principal IS NOT NULL)
    )",
    // --- append-only enforcement: born submitted, no delete, frozen once
    // --- decided, one flip
    "CREATE OR REPLACE FUNCTION bss.pricing_approval_append_only() RETURNS trigger AS $$
        BEGIN
          -- Born `submitted` or not born. Tested first because it is the only
          -- branch with no OLD row to read, and because every branch below is
          -- written about a record that started pending.
          IF TG_OP = 'INSERT' THEN
            IF NEW.state <> 'submitted' THEN
              RAISE EXCEPTION
                'pricing_approval: approval % arrives %; a record is born submitted',
                NEW.approval_id, NEW.state;
            END IF;
            RETURN NEW;
          END IF;

          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_approval: DELETE of approval % is not permitted; the record is the evidence',
              OLD.approval_id;
          END IF;

          IF OLD.state <> 'submitted' THEN
            RAISE EXCEPTION
              'pricing_approval: approval % is %; a decided record is immutable',
              OLD.approval_id, OLD.state;
          END IF;

          -- The submitted plane pins everything the decision does not touch.
          -- `content_hash` is the TOCTOU guard itself; re-pinning it in place
          -- would launder the very mutation the guard exists to catch.
          IF NEW.approval_id         IS DISTINCT FROM OLD.approval_id
          OR NEW.tenant_id           IS DISTINCT FROM OLD.tenant_id
          OR NEW.subject_ref         IS DISTINCT FROM OLD.subject_ref
          OR NEW.subject_kind        IS DISTINCT FROM OLD.subject_kind
          OR NEW.content_hash        IS DISTINCT FROM OLD.content_hash
          OR NEW.submitter_principal IS DISTINCT FROM OLD.submitter_principal
          OR NEW.materiality         IS DISTINCT FROM OLD.materiality
          OR NEW.submitted_at        IS DISTINCT FROM OLD.submitted_at THEN
            RAISE EXCEPTION
              'pricing_approval: approval % is pinned; only the decision columns may move',
              OLD.approval_id;
          END IF;

          IF NEW.state NOT IN ('approved','rejected','voided') THEN
            RAISE EXCEPTION 'pricing_approval: state % -> % is not a sanctioned flip',
              OLD.state, NEW.state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_approval_append_only
        BEFORE INSERT OR UPDATE OR DELETE ON bss.pricing_approval
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_approval_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_approval",
    "DROP FUNCTION IF EXISTS bss.pricing_approval_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transforms from the Postgres variant:
// * schema prefix `bss.` dropped (single namespace);
// * `uuid` -> `text`, `timestamptz` -> `text`, `jsonb` -> `text`,
//   `bytea` -> `blob`;
// * `now()` -> `(CURRENT_TIMESTAMP)`;
// * the single PL/pgSQL trigger becomes five `RAISE(ABORT, ...)` triggers
//   (SQLite has no `BEFORE INSERT OR UPDATE OR DELETE`, no procedural language
//   and no message interpolation), and `IS DISTINCT FROM` becomes `IS NOT`.
// Every CHECK is preserved, name for name.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_approval (
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
            subject_kind IN ('plan_revision','price_unit','window')),
        CONSTRAINT chk_pricing_approval_decided_at CHECK (
            (state = 'submitted') = (decided_at IS NULL)),
        CONSTRAINT chk_pricing_approval_reason CHECK (
            state <> 'rejected' OR reason IS NOT NULL),
        CONSTRAINT chk_pricing_approval_approver CHECK (
            state IN ('submitted','voided') OR approver_principal IS NOT NULL)
    )",
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
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_approval"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
