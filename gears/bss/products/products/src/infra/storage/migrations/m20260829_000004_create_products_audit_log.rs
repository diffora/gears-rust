//! Create `bss.products_audit_log` — the append-only trail for every act that
//! emits no broker event (`design/01-foundation.md` §4.4): a refusal, a read
//! under elevation, and a committed act the design declares eventless.
//!
//! It also carries the **reserved platform-sealing seam** (P-D-08) from this
//! first migration and never seals it here: `seal_state`, `chain_id`, `seq`,
//! `prev_hash` and `row_hash`. `seal_state` is written `unsealed` at INSERT,
//! always, so the unproven era is queryable rather than inferred from a
//! deployment date. This gear computes no hash and runs no verification job —
//! that is the platform capability's job, subject to P-D-08 S1–S9.
//!
//! # `audit_id` is a surrogate uuid primary key, not `(tenant_id, chain_id, seq)`
//!
//! The sibling pricing gear's audit log keys on `(tenant_id, chain_id, seq)`,
//! because every row it ever writes already has all three. This table cannot
//! do that: the sealing seam's one-way `UPDATE` has to address a row that is
//! **not yet sealed**, and `seq` is null until it is (owner's call,
//! 2026-08-27, **P-D-28**). A key built from a column that is null on every
//! unsealed row cannot address that row, so the key has to be independent of
//! the chain's ordering altogether. This is the single largest departure from
//! the donor.
//!
//! # `actor_ref` is `uuid`, for the donor's own stated reason
//!
//! Pseudonymous by construction, never a display name or an email address,
//! typed `uuid` rather than `text` so that constraint is physical: a `text`
//! column retained for years would eventually be handed directly-identifying
//! operator PII. The identity-reference map that resolves it to a human is
//! slice 10's `products_identity_ref`, built in a later slice — this
//! migration writes no foreign key to it.
//!
//! # `subject_id` and `subject_revision` are nullable
//!
//! A refusal raised before the mint has no id to carry — it carries
//! `attempted_key` (the attempted `name`, `sku_code` or `product_code`)
//! instead. An audit row must never name an id that identifies nothing.
//! `chk_products_audit_log_subject_ref` is the "every row is identifiable by
//! something" rule: a row carries a `subject_id`, an `attempted_key`, or a
//! `session_id`. The third arm is not decoration — v1 elevation is
//! audit-export only (`design/01-foundation.md` §4.4), so an elevated read
//! routinely names **no** subject at all, and a two-arm constraint refused
//! every such row outright. The constraint is this file's own invention
//! rather than the design set's: §4.4 makes `subject_id`, `attempted_key`
//! and `session_id` each independently nullable and never requires one of
//! them, so the rule is a floor this migration adds, and its arms must cover
//! every class the gear actually writes.
//!
//! A refusal raised before the mint carries the attempted key and no id; a
//! refusal after it carries the id. The gear's own writer never populates
//! both on one row — `RefusalSubject` is a sum type with no both-variant —
//! so the disjunction is wider than any current writer needs and is left
//! that way deliberately: a later door that resolves an id *after* raising
//! on the attempted key would otherwise need this migration edited.
//!
//! `error_code` is a column rather than free text because §3.1 makes the code
//! the attribution channel; it is null on the classes that are not refusals.
//! `written_at` is the operand slice 10's `RetentionClock` reads.
//! `session_id` is present on the elevation class only.
//!
//! # No vocabulary `CHECK` on `action` or `subject_kind` — an owed debt
//!
//! The donor (`pricing_audit_log`) constrains both columns against its
//! `domain::audit::AuditSubjectKind` and `domain::audit::AuditAction` enums,
//! and argues at length in its own module doc that the constraint should be
//! physical even though the only writer is typed: a hash-chained record
//! retained for years, immutable by trigger, is the last place a token should
//! arrive unspelled and the one place a wrong one cannot be corrected
//! afterwards. Products has no such domain enum yet, and the design set does
//! not enumerate a products roster for either column — inventing one here
//! would put a guessed token into exactly the table the donor's argument says
//! must never hold one. This is recorded as an explicit, owed debt: the two
//! vocabulary `CHECK`s (`chk_products_audit_log_action` and
//! `chk_products_audit_log_subject_kind`, mirroring the donor's names) are to
//! be added **to this migration file in place**, once the domain vocabulary
//! exists — this chain edits migrations in place and takes no follow-up
//! tightening migration.
//!
//! # The append-only trigger guard
//!
//! **DELETE is refused unconditionally on both engines**, as the donor's is.
//! Design §4.4 / P-D-34 describes a retention DELETE arm as a row-image
//! predicate — a row whose `written_at` is older than its class's retention
//! window — but that window is Legal/Finance's call and `PRD` §15 currently
//! leaves it undecided. A trigger cannot read configuration, so there is no
//! predicate to write here — and **P-D-118** (2026-09-03) rules that there
//! never will be: the window is **configuration** (`retention_days_audit`,
//! interim 3650 days, Legal and Finance's to narrow per jurisdiction), and a
//! DDL constant could not be set per jurisdiction as `PRD` §15 says it must.
//! So this trigger and the window are **two different guards** the earlier
//! text ran together. The trigger's job is to refuse **unauthorised**
//! deletion — anything that is not the GC. The window is the GC's own
//! predicate, read from configuration, and the GC is the only authorised
//! deleter. No `OLD.written_at < <cutoff>` arm is written in this file; when
//! slice 10's `inst-rt-gc` lands, the DELETE arm this trigger admits is the
//! GC's identity, not a date.
//!
//! **UPDATE admits exactly one transition**: `unsealed` to `sealed`, one-way,
//! supplying `chain_id`, `seq`, `prev_hash` and `row_hash` together in the same
//! statement, with every **record** column unchanged from `OLD`.
//!
//! `prev_hash` is one of the four the seal supplies, not one of the columns
//! held unchanged — it is the link to the previous row in the segment. It is
//! the only one of the four that may stay `NULL`, and a `NULL` one means this
//! row is the segment head. Requiring it unchanged would pin it at the `NULL`
//! every `unsealed` row carries, so every sealed row would be a segment head
//! and the chain could never link — which is the whole of what a hash chain
//! is. Nothing can rewrite it afterwards either: an already-`sealed` row
//! matches no admitted transition, since the arm requires `OLD.seal_state` to
//! be `unsealed`. Without this arm P-D-08's
//! sealing capability computes the seal asynchronously over rows already
//! immutable by trigger, and a whitelist that admitted no column at all would
//! refuse that write — precisely the migration the reserved seam exists to
//! avoid. The sealer's identity is an application and grant guarantee, not
//! something the trigger reads: the session variable that would carry it
//! exists on Postgres and not on `SQLite`, so neither trigger reads one.
//!
//! `REVOKE UPDATE, DELETE` is not issued, as the donor declines it in both
//! engine tiers: P-D-46 withdrew that arm, it names a deployment role this
//! migration does not own, and `SQLite` has no `GRANT`/`REVOKE`.
//!
//! # Backend differences
//!
//! Postgres raises through one `PL/pgSQL` function branching on `TG_OP`, with
//! one trigger firing `BEFORE DELETE OR UPDATE`; its `DOWN` drops the function
//! as well as the table. `SQLite` has no procedural language and
//! `RAISE(ABORT, ...)` takes a literal message, so the mirror is three
//! triggers with fixed messages and `WHEN` clauses carrying the predicates:
//! one refusing every DELETE, one refusing every UPDATE that is not the
//! admitted sealing transition, and one refusing a sealing UPDATE that also
//! changes a record column. Postgres compares `NEW` against `OLD` with `IS
//! DISTINCT FROM` so a `NULL`-to-`NULL` comparison behaves; `SQLite` uses `IS NOT`,
//! its own null-safe form. `uuid` becomes `text`, `bytea` becomes `blob`,
//! `timestamptz` becomes `text`, and the `bss.` qualification is dropped.
//! Every `CHECK` and index is preserved on both sides.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-audit-table:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_audit_log (
            audit_id          uuid        NOT NULL,
            tenant_id         uuid        NOT NULL,
            actor_ref         uuid        NOT NULL,
            action            text        NOT NULL,
            subject_kind      text        NOT NULL,
            subject_id        uuid,
            subject_revision  bigint,
            error_code        text,
            attempted_key     text,
            reason            text,
            correlation_id    uuid,
            written_at        timestamptz NOT NULL,
            session_id        uuid,
            seal_state        text        NOT NULL,
            chain_id          uuid,
            seq               bigint,
            prev_hash         bytea,
            row_hash          bytea,
            CONSTRAINT products_audit_log_pkey PRIMARY KEY (audit_id),
            CONSTRAINT chk_products_audit_log_seal_state CHECK (seal_state IN ('unsealed', 'sealed')),
            CONSTRAINT chk_products_audit_log_seal_group CHECK (
                (seal_state = 'unsealed' AND chain_id IS NULL AND seq IS NULL AND prev_hash IS NULL AND row_hash IS NULL)
                OR
                (seal_state = 'sealed' AND chain_id IS NOT NULL AND seq IS NOT NULL AND row_hash IS NOT NULL)
            ),
            CONSTRAINT chk_products_audit_log_seq CHECK (seq IS NULL OR seq >= 0),
            CONSTRAINT chk_products_audit_log_subject_ref CHECK (subject_id IS NOT NULL OR attempted_key IS NOT NULL OR session_id IS NOT NULL)
        )",
    "CREATE INDEX idx_products_audit_log_tenant_time ON bss.products_audit_log USING btree (tenant_id, written_at)",
    "CREATE INDEX idx_products_audit_log_subject ON bss.products_audit_log USING btree (tenant_id, subject_kind, subject_id, written_at)",
    "CREATE INDEX idx_products_audit_log_actor ON bss.products_audit_log USING btree (tenant_id, actor_ref, written_at)",
    "CREATE OR REPLACE FUNCTION bss.products_audit_log_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'products_audit_log is append-only: DELETE is not permitted';
          END IF;

          IF OLD.seal_state = 'unsealed'
             AND NEW.seal_state = 'sealed'
             AND NEW.chain_id IS NOT NULL
             AND NEW.seq IS NOT NULL
             AND NEW.row_hash IS NOT NULL
             AND NEW.audit_id IS NOT DISTINCT FROM OLD.audit_id
             AND NEW.tenant_id IS NOT DISTINCT FROM OLD.tenant_id
             AND NEW.actor_ref IS NOT DISTINCT FROM OLD.actor_ref
             AND NEW.action IS NOT DISTINCT FROM OLD.action
             AND NEW.subject_kind IS NOT DISTINCT FROM OLD.subject_kind
             AND NEW.subject_id IS NOT DISTINCT FROM OLD.subject_id
             AND NEW.subject_revision IS NOT DISTINCT FROM OLD.subject_revision
             AND NEW.error_code IS NOT DISTINCT FROM OLD.error_code
             AND NEW.attempted_key IS NOT DISTINCT FROM OLD.attempted_key
             AND NEW.reason IS NOT DISTINCT FROM OLD.reason
             AND NEW.correlation_id IS NOT DISTINCT FROM OLD.correlation_id
             AND NEW.written_at IS NOT DISTINCT FROM OLD.written_at
             AND NEW.session_id IS NOT DISTINCT FROM OLD.session_id
          THEN
            RETURN NEW;
          END IF;

          RAISE EXCEPTION 'products_audit_log is append-only: % is not permitted', TG_OP;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_audit_log_append_only BEFORE DELETE OR UPDATE ON bss.products_audit_log FOR EACH ROW EXECUTE FUNCTION bss.products_audit_log_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.products_audit_log",
    "DROP FUNCTION IF EXISTS bss.products_audit_log_append_only()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_audit_log (
            audit_id          text   NOT NULL,
            tenant_id         text   NOT NULL,
            actor_ref         text   NOT NULL,
            action            text   NOT NULL,
            subject_kind      text   NOT NULL,
            subject_id        text,
            subject_revision  bigint,
            error_code        text,
            attempted_key     text,
            reason            text,
            correlation_id    text,
            written_at        text   NOT NULL,
            session_id        text,
            seal_state        text   NOT NULL,
            chain_id          text,
            seq               bigint,
            prev_hash         blob,
            row_hash          blob,
            PRIMARY KEY (audit_id),
            CONSTRAINT chk_products_audit_log_seal_state CHECK (seal_state IN ('unsealed', 'sealed')),
            CONSTRAINT chk_products_audit_log_seal_group CHECK (
                (seal_state = 'unsealed' AND chain_id IS NULL AND seq IS NULL AND prev_hash IS NULL AND row_hash IS NULL)
                OR
                (seal_state = 'sealed' AND chain_id IS NOT NULL AND seq IS NOT NULL AND row_hash IS NOT NULL)
            ),
            CONSTRAINT chk_products_audit_log_seq CHECK (seq IS NULL OR seq >= 0),
            CONSTRAINT chk_products_audit_log_subject_ref CHECK (subject_id IS NOT NULL OR attempted_key IS NOT NULL OR session_id IS NOT NULL)
        )",
    "CREATE INDEX idx_products_audit_log_tenant_time ON products_audit_log (tenant_id, written_at)",
    "CREATE INDEX idx_products_audit_log_subject ON products_audit_log (tenant_id, subject_kind, subject_id, written_at)",
    "CREATE INDEX idx_products_audit_log_actor ON products_audit_log (tenant_id, actor_ref, written_at)",
    "CREATE TRIGGER trg_products_audit_log_no_delete BEFORE DELETE ON products_audit_log FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'products_audit_log is append-only: DELETE is not permitted'); END",
    "CREATE TRIGGER trg_products_audit_log_no_update BEFORE UPDATE ON products_audit_log FOR EACH ROW WHEN NOT (
            OLD.seal_state IS 'unsealed'
            AND NEW.seal_state IS 'sealed'
            AND NEW.chain_id IS NOT NULL
            AND NEW.seq IS NOT NULL
            AND NEW.row_hash IS NOT NULL
        ) BEGIN SELECT RAISE(ABORT, 'products_audit_log is append-only: UPDATE is not permitted'); END",
    "CREATE TRIGGER trg_products_audit_log_seal_unchanged BEFORE UPDATE ON products_audit_log FOR EACH ROW WHEN (
            OLD.seal_state IS 'unsealed'
            AND NEW.seal_state IS 'sealed'
            AND NEW.chain_id IS NOT NULL
            AND NEW.seq IS NOT NULL
            AND NEW.row_hash IS NOT NULL
        ) AND NOT (
            NEW.audit_id IS OLD.audit_id
            AND NEW.tenant_id IS OLD.tenant_id
            AND NEW.actor_ref IS OLD.actor_ref
            AND NEW.action IS OLD.action
            AND NEW.subject_kind IS OLD.subject_kind
            AND NEW.subject_id IS OLD.subject_id
            AND NEW.subject_revision IS OLD.subject_revision
            AND NEW.error_code IS OLD.error_code
            AND NEW.attempted_key IS OLD.attempted_key
            AND NEW.reason IS OLD.reason
            AND NEW.correlation_id IS OLD.correlation_id
            AND NEW.written_at IS OLD.written_at
            AND NEW.session_id IS OLD.session_id
        ) BEGIN SELECT RAISE(ABORT, 'products_audit_log is append-only: UPDATE is not permitted'); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_audit_log"];

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
