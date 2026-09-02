//! Create `bss.products_scheduled_transition` — the persisted lifecycle
//! intent (`design/04-lifecycle.md` §4; `features/lifecycle.md`
//! `dod-scheduled-transition-store`; **P-D-46** for the two reason columns).
//!
//! # Guard family: freeze after terminal — not rebuildable, not a head whitelist
//!
//! A `ScheduledTransition` is a **record**, not rebuildable state: the runner
//! claims it, finishes it, and the audit plane reads it back. The matching
//! posture is `products_approval`'s — working state while live
//! (`pending`/`running`/`deferred`), evidence once terminal
//! (`applied`/`failed`/`superseded`). The guard therefore refuses every
//! `DELETE` and any `UPDATE` whose `OLD.state` is already terminal. Live rows
//! stay mutable so the claim CAS, lease reclaim, deferral and finish writes
//! can land; a terminal row does not. **Do not drop this for a "no guard"
//! reading of the bulk-batch table** — that table's immutability lives in its
//! ledger, and this table has no ledger beside itself.
//!
//! # Two reason columns, and why they stay separate (**P-D-46**)
//!
//! `retirement_reason` is the operator's text, written once at initiation.
//! `outcome_reason` is the runner's outcome text, written on
//! `applied|failed|deferred`. One column let a deferral's failure text
//! overwrite the operator's; the split is the floor under the lead-window
//! re-announcement, which must still read the operator's words after a hold.
//!
//! # One live intent per entity per kind
//!
//! The partial `UNIQUE (tenant_id, entity_kind, entity_id, kind) WHERE state
//! IN ('pending','running','deferred')` is the physical floor under "one live
//! intent": a re-schedule supersedes explicitly into a terminal state, which
//! frees the slot for a new row. Terminal states are outside the predicate on
//! purpose — a re-schedule after `applied` or `failed` is a **new** row.
//!
//! # Indexes lead with `tenant_id`
//!
//! The due-poll index is `(tenant_id, state, at)` — every claim reads a
//! tenant partition first. An index that did not lead with `tenant_id`
//! cannot serve a per-partition budget.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `integer` stays, `timestamptz` becomes
//! `text`, and the `bss.` qualification is dropped. The CHECKs, the partial
//! UNIQUE, the due index and both halves of the guard are preserved on both
//! sides; `SQLite` splits the guard into per-op triggers.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-scheduled-transition-store:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_scheduled_transition (
            transition_id       uuid        NOT NULL,
            tenant_id           uuid        NOT NULL,
            entity_kind         text        NOT NULL,
            entity_id           uuid        NOT NULL,
            kind                text        NOT NULL,
            at                  timestamptz NOT NULL,
            approval_ref        uuid        NOT NULL,
            state               text        NOT NULL,
            claimed_at          timestamptz,
            attempt             integer     NOT NULL DEFAULT 0,
            retirement_reason   text,
            outcome_reason      text,
            created_at          timestamptz NOT NULL,
            updated_at          timestamptz NOT NULL,
            CONSTRAINT products_scheduled_transition_pkey PRIMARY KEY (transition_id),
            CONSTRAINT chk_products_scheduled_transition_entity_kind
                CHECK (entity_kind IN ('product', 'sku')),
            CONSTRAINT chk_products_scheduled_transition_kind
                CHECK (kind IN ('publish', 'retire')),
            CONSTRAINT chk_products_scheduled_transition_state
                CHECK (state IN ('pending', 'running', 'applied', 'failed', 'deferred', 'superseded')),
            CONSTRAINT chk_products_scheduled_transition_attempt
                CHECK (attempt >= 0)
        )",
    "CREATE UNIQUE INDEX uq_products_scheduled_transition_live
        ON bss.products_scheduled_transition USING btree (tenant_id, entity_kind, entity_id, kind)
        WHERE state IN ('pending', 'running', 'deferred')",
    "CREATE INDEX idx_products_scheduled_transition_due
        ON bss.products_scheduled_transition USING btree (tenant_id, state, at)",
    "CREATE OR REPLACE FUNCTION bss.products_scheduled_transition_frozen() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'products_scheduled_transition is append-only evidence: DELETE is not permitted';
          END IF;
          IF OLD.state IN ('applied', 'failed', 'superseded') THEN
            RAISE EXCEPTION 'products_scheduled_transition: a terminal transition is immutable';
          END IF;
          RETURN NEW;
        END;
        $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_scheduled_transition_frozen
        BEFORE DELETE OR UPDATE ON bss.products_scheduled_transition
        FOR EACH ROW EXECUTE FUNCTION bss.products_scheduled_transition_frozen()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_scheduled_transition_frozen ON bss.products_scheduled_transition",
    "DROP FUNCTION IF EXISTS bss.products_scheduled_transition_frozen",
    "DROP TABLE IF EXISTS bss.products_scheduled_transition",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_scheduled_transition (
            transition_id       text    NOT NULL,
            tenant_id           text    NOT NULL,
            entity_kind         text    NOT NULL,
            entity_id           text    NOT NULL,
            kind                text    NOT NULL,
            at                  text    NOT NULL,
            approval_ref        text    NOT NULL,
            state               text    NOT NULL,
            claimed_at          text,
            attempt             integer NOT NULL DEFAULT 0,
            retirement_reason   text,
            outcome_reason      text,
            created_at          text    NOT NULL,
            updated_at          text    NOT NULL,
            PRIMARY KEY (transition_id),
            CONSTRAINT chk_products_scheduled_transition_entity_kind
                CHECK (entity_kind IN ('product', 'sku')),
            CONSTRAINT chk_products_scheduled_transition_kind
                CHECK (kind IN ('publish', 'retire')),
            CONSTRAINT chk_products_scheduled_transition_state
                CHECK (state IN ('pending', 'running', 'applied', 'failed', 'deferred', 'superseded')),
            CONSTRAINT chk_products_scheduled_transition_attempt
                CHECK (attempt >= 0)
        )",
    "CREATE UNIQUE INDEX uq_products_scheduled_transition_live
        ON products_scheduled_transition (tenant_id, entity_kind, entity_id, kind)
        WHERE state IN ('pending', 'running', 'deferred')",
    "CREATE INDEX idx_products_scheduled_transition_due
        ON products_scheduled_transition (tenant_id, state, at)",
    "CREATE TRIGGER trg_products_scheduled_transition_no_delete
        BEFORE DELETE ON products_scheduled_transition
        BEGIN
          SELECT RAISE(ABORT, 'products_scheduled_transition is append-only evidence: DELETE is not permitted');
        END",
    "CREATE TRIGGER trg_products_scheduled_transition_frozen
        BEFORE UPDATE ON products_scheduled_transition
        WHEN OLD.state IN ('applied', 'failed', 'superseded')
        BEGIN
          SELECT RAISE(ABORT, 'products_scheduled_transition: a terminal transition is immutable');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_scheduled_transition_frozen",
    "DROP TRIGGER IF EXISTS trg_products_scheduled_transition_no_delete",
    "DROP TABLE IF EXISTS products_scheduled_transition",
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
