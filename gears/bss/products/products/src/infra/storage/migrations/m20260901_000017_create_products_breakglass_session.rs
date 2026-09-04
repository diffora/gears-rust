//! Create `bss.products_breakglass_session` — the elevation session
//! (`design/05-governance.md` §4; **P-D-68** arms 2 and 3).
//!
//! # The window is half-open, and the gate judges at admission
//!
//! `[valid_from, valid_until)` — half-open because expiry gates **admission**:
//! *"an elevated read admitted inside the window finishes"* (**P-D-68** arm
//! 2), so an instant equal to `valid_until` is already outside and no act is
//! cut in half. A CHECK pins the ordering, because a window whose end
//! precedes its start would admit nothing while looking open.
//!
//! # `expired_emitted` is a CAS stamp, not a flag someone reads
//!
//! `BreakGlassExpired` has **one** emitter: the first post-expiry act flips
//! this column by compare-and-swap in the same transaction as its own refusal
//! — the winner emits, a replay emits nothing (P-D-54's mechanism, reused).
//! The measured defect it repairs: the only named producer was a refused
//! call, so *"an untouched session emits nothing and a session called ten
//! times emits ten"*. A session never touched after expiry emits no event at
//! all, by design: its expiry is a **stored fact** (`valid_until` passed),
//! observable as a gauge with the alerting rule on top (P-D-59's mechanism),
//! which is also what the post-hoc review alert keys on.
//!
//! # The approval path is two columns, one ceremony, two timings
//!
//! Rule 1 admits an elevation *"two-person-approved **or**
//! post-hoc-reviewed"* with a fixed floor of two distinct platform
//! principals, and **P-D-68** arm 3 settles that these are not two ceremonies:
//! the review **is** the second principal's decision arriving after the fact.
//! So `posthoc_state` carries the enumerated `{pending, reviewed}` and
//! `reviewed_by` / `reviewed_at` are written when it flips — *"no new door or
//! grant is minted"*.
//!
//! **`two_person_approval_ref` carries no FK, and that is a recorded absence
//! rather than an oversight.** §6's open item asks whether a break-glass
//! two-person approval **is** an `ApprovalRecord` at all: `inst-bg-open`
//! requires two platform principals *"outside the tenant's configured `N`
//! entirely"*, §1.7 defines `required` only as `N` or `min(N, 1)` so no
//! writer can produce a fixed 2, and `inst-gv-scope` would refuse a platform
//! approver on another tenant's subject. P-D-68 arm 3 says so explicitly —
//! *"whether that decision's record is an `ApprovalRecord` stays its own open
//! §6 item, deliberately not presupposed here"*. A FK to
//! `products_approval` would presuppose it. The column is a nullable
//! reference and its referent is named by the door that first writes one,
//! exactly as `products_bulk_batch.approval_ref` shipped without one while
//! this slice's table did not exist.
//!
//! # `principal` is an `actor_ref`
//!
//! Pseudonymous from birth, like every actor-bearing store in this gear (M5
//! of the slice-10 review), and `reviewed_by` with it. Elevated audit rows
//! carry `session_id`, which is what makes *"every elevated read leaves an
//! audit row with the session id"* countable rather than sampled.
//!
//! # No tenant in the primary key, and that is deliberate
//!
//! A session's subject is `target_tenant`, but the session itself is a
//! **platform** record: the principal is outside the tenant, and a
//! tenant-scoped PK would imply a tenant owns its own elevation sessions.
//! `session_id` is therefore the whole key, with `target_tenant` an indexed
//! column the elevated reads narrow by.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `timestamptz` becomes `text`, `boolean`
//! becomes `integer`, and the `bss.` qualification is dropped. Every CHECK,
//! the key, the index and the guard are preserved on both sides.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-breakglass-store:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_breakglass_session (
            session_id              uuid        NOT NULL,
            principal               uuid        NOT NULL,
            target_tenant           uuid        NOT NULL,
            reason                  text        NOT NULL,
            valid_from              timestamptz NOT NULL,
            valid_until             timestamptz NOT NULL,
            two_person_approval_ref uuid,
            approver_a              uuid,
            approver_b              uuid,
            posthoc_state           text,
            reviewed_by             uuid,
            reviewed_at             timestamptz,
            expired_emitted         boolean     NOT NULL DEFAULT false,
            opened_at               timestamptz NOT NULL,
            CONSTRAINT products_breakglass_session_pkey PRIMARY KEY (session_id),
            CONSTRAINT chk_products_breakglass_reason CHECK (reason <> ''),
            CONSTRAINT chk_products_breakglass_window CHECK (valid_until > valid_from),
            CONSTRAINT chk_products_breakglass_posthoc_state CHECK (posthoc_state IS NULL OR posthoc_state IN ('pending', 'reviewed')),
            CONSTRAINT chk_products_breakglass_path CHECK ((two_person_approval_ref IS NULL) <> (posthoc_state IS NULL)),
            CONSTRAINT chk_products_breakglass_approvers CHECK ((approver_a IS NULL) = (two_person_approval_ref IS NULL) AND (approver_b IS NULL) = (two_person_approval_ref IS NULL)),
            CONSTRAINT chk_products_breakglass_approvers_distinct CHECK (approver_a IS NULL OR approver_a <> approver_b),
            CONSTRAINT chk_products_breakglass_review CHECK (
                (posthoc_state = 'reviewed' AND reviewed_by IS NOT NULL AND reviewed_at IS NOT NULL)
                OR (posthoc_state IS DISTINCT FROM 'reviewed' AND reviewed_by IS NULL AND reviewed_at IS NULL)
            )
        )",
    "CREATE INDEX idx_products_breakglass_session_tenant ON bss.products_breakglass_session USING btree (target_tenant, valid_until)",
    "CREATE OR REPLACE FUNCTION bss.products_breakglass_session_frozen() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'products_breakglass_session is append-only evidence: DELETE is not permitted';
          END IF;
          IF NEW.session_id <> OLD.session_id
             OR NEW.principal <> OLD.principal
             OR NEW.target_tenant <> OLD.target_tenant
             OR NEW.reason <> OLD.reason
             OR NEW.valid_from <> OLD.valid_from
             OR NEW.valid_until <> OLD.valid_until
             OR NEW.approver_a IS DISTINCT FROM OLD.approver_a
             OR NEW.approver_b IS DISTINCT FROM OLD.approver_b
             OR NEW.opened_at <> OLD.opened_at THEN
            RAISE EXCEPTION 'products_breakglass_session: an opened session''s terms are immutable';
          END IF;
          RETURN NEW;
        END;
        $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_breakglass_session_frozen
        BEFORE DELETE OR UPDATE ON bss.products_breakglass_session
        FOR EACH ROW EXECUTE FUNCTION bss.products_breakglass_session_frozen()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_breakglass_session_frozen ON bss.products_breakglass_session",
    "DROP FUNCTION IF EXISTS bss.products_breakglass_session_frozen",
    "DROP TABLE IF EXISTS bss.products_breakglass_session",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_breakglass_session (
            session_id              text    NOT NULL,
            principal               text    NOT NULL,
            target_tenant           text    NOT NULL,
            reason                  text    NOT NULL,
            valid_from              text    NOT NULL,
            valid_until             text    NOT NULL,
            two_person_approval_ref text,
            approver_a              text,
            approver_b              text,
            posthoc_state           text,
            reviewed_by             text,
            reviewed_at             text,
            expired_emitted         integer NOT NULL DEFAULT 0,
            opened_at               text    NOT NULL,
            PRIMARY KEY (session_id),
            CONSTRAINT chk_products_breakglass_reason CHECK (reason <> ''),
            CONSTRAINT chk_products_breakglass_window CHECK (valid_until > valid_from),
            CONSTRAINT chk_products_breakglass_posthoc_state CHECK (posthoc_state IS NULL OR posthoc_state IN ('pending', 'reviewed')),
            CONSTRAINT chk_products_breakglass_path CHECK ((two_person_approval_ref IS NULL) <> (posthoc_state IS NULL)),
            CONSTRAINT chk_products_breakglass_approvers CHECK ((approver_a IS NULL) = (two_person_approval_ref IS NULL) AND (approver_b IS NULL) = (two_person_approval_ref IS NULL)),
            CONSTRAINT chk_products_breakglass_approvers_distinct CHECK (approver_a IS NULL OR approver_a <> approver_b),
            CONSTRAINT chk_products_breakglass_review CHECK (
                (posthoc_state = 'reviewed' AND reviewed_by IS NOT NULL AND reviewed_at IS NOT NULL)
                OR ((posthoc_state IS NULL OR posthoc_state <> 'reviewed') AND reviewed_by IS NULL AND reviewed_at IS NULL)
            )
        )",
    "CREATE INDEX idx_products_breakglass_session_tenant ON products_breakglass_session (target_tenant, valid_until)",
    "CREATE TRIGGER trg_products_breakglass_session_no_delete
        BEFORE DELETE ON products_breakglass_session
        BEGIN
          SELECT RAISE(ABORT, 'products_breakglass_session is append-only evidence: DELETE is not permitted');
        END",
    "CREATE TRIGGER trg_products_breakglass_session_frozen
        BEFORE UPDATE ON products_breakglass_session
        WHEN NEW.session_id <> OLD.session_id
             OR NEW.principal <> OLD.principal
             OR NEW.target_tenant <> OLD.target_tenant
             OR NEW.reason <> OLD.reason
             OR NEW.valid_from <> OLD.valid_from
             OR NEW.valid_until <> OLD.valid_until
             OR NEW.approver_a IS NOT OLD.approver_a
             OR NEW.approver_b IS NOT OLD.approver_b
             OR NEW.opened_at <> OLD.opened_at
        BEGIN
          SELECT RAISE(ABORT, 'products_breakglass_session: an opened session''s terms are immutable');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_breakglass_session_frozen",
    "DROP TRIGGER IF EXISTS trg_products_breakglass_session_no_delete",
    "DROP TABLE IF EXISTS products_breakglass_session",
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
