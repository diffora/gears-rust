//! Create `bss.pricing_audit_log` — the append-only, hash-chained actor /
//! before-after / approval trail (`design/01-foundation.md` §3.7; normative in
//! `design/05-governance.md`), retained for at least seven years.
//!
//! The chain is **segmented per `(tenant_id, chain_id)`** where `chain_id` is
//! the audited subject's aggregate — plan, overlay, payer, policy, bulk
//! operation (**D-135**). A single per-tenant chain would have serialized
//! *every* mutation of a tenant behind one head, inside the mutation
//! transaction, which the >= 50 rows/s repricing SLO never accounted for.
//! Tamper-evidence across segments is restored by a **periodic per-tenant
//! roll-up row** (`entry_kind = 'rollup'`) whose `segment_heads` payload chains
//! the segment heads together.
//!
//! **The actor is a pseudonymous principal id, never a display name or an email
//! address** (D-61, `inst-au-pii`). The column is typed `uuid` rather than
//! `text` precisely so that constraint is physical: a retention horizon of
//! seven-plus years must not accumulate directly identifying operator PII, and
//! a `text` column would eventually be handed one. Resolving a principal id to
//! a human is the identity plane's job, subject to its own access control.
//!
//! Two physical guards. The append-only trigger rejects **every** UPDATE and
//! **every** DELETE unconditionally — unlike `pricing_plan` and `pricing_price`
//! there is no whitelist here, because there is no sanctioned in-place mutation
//! of an audit record at all; a hash chain whose links can be rewritten is not
//! evidence. `chk_pricing_audit_log_rollup` ties `entry_kind` to
//! `segment_heads`, so a roll-up row cannot exist without the heads it claims
//! to chain, nor a mutation row carry heads it never computed.
//!
//! **Backend differences.** Postgres raises through PL/pgSQL with the offending
//! `TG_OP` interpolated; `SQLite` has no procedural language and
//! `RAISE(ABORT, ...)` takes a literal message, so the mirror is two triggers
//! with fixed messages. `bytea` becomes `blob` and `jsonb` becomes `text`. As
//! elsewhere in this chain, `REVOKE UPDATE, DELETE` is not issued: it names a
//! deployment role this migration does not own, and `SQLite` has no
//! `GRANT`/`REVOKE`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_audit_log (
        tenant_id          uuid        NOT NULL,
        chain_id           uuid        NOT NULL,
        seq                bigint      NOT NULL,
        entry_kind         text        NOT NULL DEFAULT 'mutation',
        recorded_at        timestamptz NOT NULL DEFAULT now(),
        actor_principal_id uuid        NOT NULL,
        action             text        NOT NULL,
        subject_kind       text        NOT NULL,
        subject_ref        text        NOT NULL,
        before_state       jsonb,
        after_state        jsonb,
        approval_ref       uuid,
        correlation_id     uuid,
        segment_heads      jsonb,
        prev_hash          bytea,
        row_hash           bytea       NOT NULL,
        PRIMARY KEY (tenant_id, chain_id, seq),
        CONSTRAINT chk_pricing_audit_log_seq CHECK (seq >= 0),
        CONSTRAINT chk_pricing_audit_log_entry_kind CHECK (
            entry_kind IN ('mutation','rollup')),
        CONSTRAINT chk_pricing_audit_log_rollup CHECK (
            (entry_kind = 'rollup') = (segment_heads IS NOT NULL))
    )",
    "CREATE INDEX idx_pricing_audit_log_recorded
        ON bss.pricing_audit_log (tenant_id, recorded_at)",
    "CREATE INDEX idx_pricing_audit_log_subject
        ON bss.pricing_audit_log (tenant_id, subject_kind, subject_ref, recorded_at)",
    "CREATE OR REPLACE FUNCTION bss.pricing_audit_log_append_only() RETURNS trigger AS $$
        BEGIN
          RAISE EXCEPTION 'pricing_audit_log is append-only: % is not permitted', TG_OP;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_audit_log_append_only
        BEFORE UPDATE OR DELETE ON bss.pricing_audit_log
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_audit_log_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_audit_log",
    "DROP FUNCTION IF EXISTS bss.pricing_audit_log_append_only()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_audit_log (
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
            entry_kind IN ('mutation','rollup')),
        CONSTRAINT chk_pricing_audit_log_rollup CHECK (
            (entry_kind = 'rollup') = (segment_heads IS NOT NULL))
    )",
    "CREATE INDEX idx_pricing_audit_log_recorded
        ON pricing_audit_log (tenant_id, recorded_at)",
    "CREATE INDEX idx_pricing_audit_log_subject
        ON pricing_audit_log (tenant_id, subject_kind, subject_ref, recorded_at)",
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
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_audit_log"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
