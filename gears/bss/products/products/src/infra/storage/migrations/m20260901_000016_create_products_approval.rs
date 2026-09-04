//! Create `bss.products_approval` and `bss.products_approval_decision` — the
//! approval record and its decisions (`design/05-governance.md` §4;
//! **P-D-13**, **P-D-14**, **P-D-68**).
//!
//! # Two stored snapshots, and the reason they are stored
//!
//! `content_snapshot` and `quorum_descriptor` are **written at submission and
//! never re-derived**, and both for the same measured reason. §5 makes the
//! snapshot's probe the slice's flagship: *"submit -> edit the head -> the
//! superseded record's diff still renders the ORIGINAL submission against the
//! published version"* — a re-derived diff would show the draft against
//! itself, *"the exact pricing defect"*. The descriptor's case is the same
//! shape one field over: `configured_quorum` is *"the `N` in force at
//! submission"*, so deriving it from current policy *"would change a
//! **pending** record when the tenant edits `N`"*. A record that changes
//! after the fact is not a record.
//!
//! # One open approval per subject, and it is an index rather than a rule
//!
//! The partial `UNIQUE (tenant_id, subject_kind, subject_ref) WHERE state IN
//! ('pending', 'satisfied')` is §4's own shape: a subject may accumulate any
//! number of finalized approvals and hold **one** open one, so L-4's *"a new
//! submission explicitly supersedes the open one"* is enforced by the engine
//! rather than by a door's read-then-write. `subject_kind`'s roster is the
//! five §4 names plus **`materiality_policy`** (**P-D-120** row 38, edited
//! into this `CHECK` in place, 2026-09-04). The fifth, `bulk_batch`, is 09's
//! and its writer already ships (`products_bulk_batch.approval_ref` carries
//! no FK precisely because this table did not exist yet; it does now, and the
//! FK stays absent because the reference points the other way).
//!
//! # The revision floor is **zero**, not one
//!
//! `chk_products_approval_revision` read `>= 1` and **P-D-120** row 14
//! requires `0`: *"`internal_revision` is the op's own pin — the envelope's
//! revision where it has one, **`0` where the subject has no counter**"*,
//! because the column exists to detect a stale submission and an op with no
//! counter cannot go stale. The constraint as shipped made every non-entity
//! submission unwritable — measured 2026-09-04, when the submit door's first
//! `materiality_policy` record answered a 500 through the `CHECK`. The floor
//! is widened in place on both dialects; nothing wrote a zero before, so no
//! row moves.
//!
//! Zero is **not** a nullable revision, and P-D-120 says why it is a sentinel
//! rather than a `NULL`: a nullable column would put `NULL` into P-D-105's
//! equality clause, where it compares unequal to everything including itself.
//!
//! **Why the policy is a kind and not a `governed_live_op`.** `subject_kind`
//! names *what is approved*, which is P-D-14's own reason for making
//! `system_signal` a kind; a policy mutation is a thing approved, and it is
//! the one thing whose approval governs the count every other approval is
//! judged by. Folding it into `governed_live_op` would leave the record that
//! raised `N` indistinguishable from a taxonomy rename in the one column an
//! auditor filters on.
//!
//! # `N = 0` puts the acknowledgment on the record, not on a decision
//!
//! `author_override_ack` / `author_override_ack_at` are nullable and written
//! by the submit door **only when the effective quorum is zero** (**P-D-68**
//! arm 1). At `N = 0` no decision row can exist — the author is not an
//! approver and carries no verdict — and a synthetic decision row naming the
//! author would break the one-principal-one-decision UNIQUE below and the
//! two-person invariant it enforces. So a fact gets a column (the P-D-50
//! convention). A CHECK pins the pair: both columns or neither.
//!
//! # The decision UNIQUE is C2's physical floor
//!
//! `UNIQUE (approval_id, approver_principal)` is *"one principal, one
//! decision, whatever roles they hold"* — the floor under
//! distinctness-by-principal, so a human holding `CatalogAdmin` **and**
//! `FinanceReviewer` counts once no matter which role they decide under. The
//! principal is an `actor_ref`, pseudonymous from birth: these rows are
//! append-only, so **one raw identifier written is unreachable by erasure
//! forever**.
//!
//! # Append-only after finalization, and what that admits
//!
//! An approval is working state while `pending` and evidence once finalized,
//! so its guard is neither the head tables' whitelist nor the version tables'
//! unconditional refusal: it refuses any `UPDATE` whose **`OLD.state` is
//! already terminal** (`consumed`, `rejected`, `superseded`) and every
//! `DELETE`. `satisfied` stays mutable because `satisfied -> consumed` is the
//! one-shot consumption edge. Decision rows are append-only outright — a cast
//! verdict is not editable.
//!
//! # What is deliberately absent
//!
//! No `state` writer is installed here: §6's open item *"Which transaction
//! writes `state = satisfied`?"* is the doors' question, and a table that
//! pinned the answer would author it. The CHECK admits the five values; who
//! moves between them arrives with `dod-decide` and `dod-one-shot-consumption`.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `bigint` becomes `integer`,
//! `timestamptz` becomes `text`, and the `bss.` qualification is dropped.
//! Both partial indexes, every CHECK, both keys and both guards are preserved
//! on both sides; `SQLite` splits each guard into per-op triggers.
//!
//! **Neither `DoD` carries a marker here.** `dod-approval-store` waits on §7
//! rows 9, 11 and 14 and `dod-decision-store` on row 6 — all four about what
//! these columns MEAN for a non-entity subject, who writes `satisfied`, and
//! how approver refs meet `10`'s erasure path. The tables are complete and
//! usable; the questions are not this migration's to answer.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_approval (
            tenant_id             uuid        NOT NULL,
            approval_id           uuid        NOT NULL,
            subject_kind          text        NOT NULL,
            subject_ref           text        NOT NULL,
            internal_revision     bigint      NOT NULL,
            content_snapshot      text        NOT NULL,
            diff_basis            bigint,
            quorum_descriptor     text        NOT NULL,
            state                 text        NOT NULL,
            submitter             uuid        NOT NULL,
            author_override_ack   text,
            author_override_ack_at timestamptz,
            submitted_at          timestamptz NOT NULL,
            finalized_at          timestamptz,
            CONSTRAINT products_approval_pkey PRIMARY KEY (tenant_id, approval_id),
            CONSTRAINT chk_products_approval_subject_kind CHECK (subject_kind IN ('entity_publish', 'governed_live_op', 'system_signal', 'sku_correction', 'bulk_batch', 'materiality_policy')),
            CONSTRAINT chk_products_approval_subject_ref CHECK (subject_ref <> ''),
            CONSTRAINT chk_products_approval_state CHECK (state IN ('pending', 'satisfied', 'consumed', 'rejected', 'superseded')),
            CONSTRAINT chk_products_approval_revision CHECK (internal_revision >= 0),
            CONSTRAINT chk_products_approval_snapshot CHECK (content_snapshot <> ''),
            CONSTRAINT chk_products_approval_descriptor CHECK (quorum_descriptor <> ''),
            CONSTRAINT chk_products_approval_override_pair CHECK ((author_override_ack IS NULL) = (author_override_ack_at IS NULL)),
            CONSTRAINT chk_products_approval_finalized CHECK ((state IN ('pending', 'satisfied')) = (finalized_at IS NULL))
        )",
    "CREATE UNIQUE INDEX uq_products_approval_open ON bss.products_approval USING btree (tenant_id, subject_kind, subject_ref) WHERE state IN ('pending', 'satisfied')",
    "CREATE INDEX idx_products_approval_queue ON bss.products_approval USING btree (tenant_id, state, submitted_at)",
    "CREATE TABLE bss.products_approval_decision (
            tenant_id           uuid        NOT NULL,
            approval_id         uuid        NOT NULL,
            approver_principal  uuid        NOT NULL,
            verdict             text        NOT NULL,
            reason              text,
            override_acknowledgments text,
            decided_at          timestamptz NOT NULL,
            CONSTRAINT products_approval_decision_pkey PRIMARY KEY (tenant_id, approval_id, approver_principal),
            CONSTRAINT chk_products_approval_decision_verdict CHECK (verdict IN ('approved', 'rejected')),
            CONSTRAINT fk_products_approval_decision_approval FOREIGN KEY (tenant_id, approval_id)
                REFERENCES bss.products_approval (tenant_id, approval_id)
        )",
    "CREATE OR REPLACE FUNCTION bss.products_approval_frozen() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'products_approval is append-only evidence: DELETE is not permitted';
          END IF;
          IF OLD.state IN ('consumed', 'rejected', 'superseded') THEN
            RAISE EXCEPTION 'products_approval: a finalized approval is immutable';
          END IF;
          RETURN NEW;
        END;
        $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_approval_frozen
        BEFORE DELETE OR UPDATE ON bss.products_approval
        FOR EACH ROW EXECUTE FUNCTION bss.products_approval_frozen()",
    "CREATE OR REPLACE FUNCTION bss.products_approval_decision_frozen() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'UPDATE' THEN
            RAISE EXCEPTION 'products_approval_decision is frozen: a cast verdict is not editable';
          END IF;
          RAISE EXCEPTION 'products_approval_decision is frozen: DELETE is not permitted';
        END;
        $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_approval_decision_frozen
        BEFORE DELETE OR UPDATE ON bss.products_approval_decision
        FOR EACH ROW EXECUTE FUNCTION bss.products_approval_decision_frozen()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_approval_decision_frozen ON bss.products_approval_decision",
    "DROP FUNCTION IF EXISTS bss.products_approval_decision_frozen",
    "DROP TRIGGER IF EXISTS trg_products_approval_frozen ON bss.products_approval",
    "DROP FUNCTION IF EXISTS bss.products_approval_frozen",
    "DROP TABLE IF EXISTS bss.products_approval_decision",
    "DROP TABLE IF EXISTS bss.products_approval",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_approval (
            tenant_id             text    NOT NULL,
            approval_id           text    NOT NULL,
            subject_kind          text    NOT NULL,
            subject_ref           text    NOT NULL,
            internal_revision     integer NOT NULL,
            content_snapshot      text    NOT NULL,
            diff_basis            integer,
            quorum_descriptor     text    NOT NULL,
            state                 text    NOT NULL,
            submitter             text    NOT NULL,
            author_override_ack   text,
            author_override_ack_at text,
            submitted_at          text    NOT NULL,
            finalized_at          text,
            PRIMARY KEY (tenant_id, approval_id),
            CONSTRAINT chk_products_approval_subject_kind CHECK (subject_kind IN ('entity_publish', 'governed_live_op', 'system_signal', 'sku_correction', 'bulk_batch', 'materiality_policy')),
            CONSTRAINT chk_products_approval_subject_ref CHECK (subject_ref <> ''),
            CONSTRAINT chk_products_approval_state CHECK (state IN ('pending', 'satisfied', 'consumed', 'rejected', 'superseded')),
            CONSTRAINT chk_products_approval_revision CHECK (internal_revision >= 0),
            CONSTRAINT chk_products_approval_snapshot CHECK (content_snapshot <> ''),
            CONSTRAINT chk_products_approval_descriptor CHECK (quorum_descriptor <> ''),
            CONSTRAINT chk_products_approval_override_pair CHECK ((author_override_ack IS NULL) = (author_override_ack_at IS NULL)),
            CONSTRAINT chk_products_approval_finalized CHECK ((state IN ('pending', 'satisfied')) = (finalized_at IS NULL))
        )",
    "CREATE UNIQUE INDEX uq_products_approval_open ON products_approval (tenant_id, subject_kind, subject_ref) WHERE state IN ('pending', 'satisfied')",
    "CREATE INDEX idx_products_approval_queue ON products_approval (tenant_id, state, submitted_at)",
    "CREATE TABLE products_approval_decision (
            tenant_id           text    NOT NULL,
            approval_id         text    NOT NULL,
            approver_principal  text    NOT NULL,
            verdict             text    NOT NULL,
            reason              text,
            override_acknowledgments text,
            decided_at          text    NOT NULL,
            PRIMARY KEY (tenant_id, approval_id, approver_principal),
            CONSTRAINT chk_products_approval_decision_verdict CHECK (verdict IN ('approved', 'rejected')),
            CONSTRAINT fk_products_approval_decision_approval FOREIGN KEY (tenant_id, approval_id)
                REFERENCES products_approval (tenant_id, approval_id)
        )",
    "CREATE TRIGGER trg_products_approval_no_delete
        BEFORE DELETE ON products_approval
        BEGIN
          SELECT RAISE(ABORT, 'products_approval is append-only evidence: DELETE is not permitted');
        END",
    "CREATE TRIGGER trg_products_approval_frozen
        BEFORE UPDATE ON products_approval
        WHEN OLD.state IN ('consumed', 'rejected', 'superseded')
        BEGIN
          SELECT RAISE(ABORT, 'products_approval: a finalized approval is immutable');
        END",
    "CREATE TRIGGER trg_products_approval_decision_no_delete
        BEFORE DELETE ON products_approval_decision
        BEGIN
          SELECT RAISE(ABORT, 'products_approval_decision is frozen: DELETE is not permitted');
        END",
    "CREATE TRIGGER trg_products_approval_decision_no_update
        BEFORE UPDATE ON products_approval_decision
        BEGIN
          SELECT RAISE(ABORT, 'products_approval_decision is frozen: a cast verdict is not editable');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_approval_decision_no_update",
    "DROP TRIGGER IF EXISTS trg_products_approval_decision_no_delete",
    "DROP TRIGGER IF EXISTS trg_products_approval_frozen",
    "DROP TRIGGER IF EXISTS trg_products_approval_no_delete",
    "DROP TABLE IF EXISTS products_approval_decision",
    "DROP TABLE IF EXISTS products_approval",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(
            self.name(),
            manager,
            PG_DOWN_STATEMENTS,
            SQLITE_DOWN_STATEMENTS,
        )
        .await
    }
}
