//! Create `bss.products_bulk_batch` and `bss.products_bulk_row` — the batch
//! and its `RowLedger` (`design/09-bulk-promotion.md` §4, authored by
//! **P-D-61** from the values with a stated writer; the machine completed by
//! **P-D-54** and **P-D-69**).
//!
//! # The batch carries the seven-state machine and the worker's claim
//!
//! `state` is the P-D-54 six plus P-D-69's `abandoned`, CHECK-pinned because
//! the set is decided — `reported → abandoned` on the approval's rejection or
//! withdrawal, `staging|committing → failed` on the worker's attempt-budget
//! exhaustion, row failures never entering either. `batch_key` is the import
//! door's idempotency operand, UNIQUE per tenant; `mode` is P-D-69's
//! batch-level operand (`import` refuses a bound code as `DUPLICATE_CODE`,
//! only `promote` engages the resolver); `claimed_at`/`attempt` are the
//! P-D-54 batch worker's claim and lease. `approval_ref` is nullable and
//! carries **no FK**: the record it names is `05-governance`'s table, which
//! does not ship — the write path is 05's to supply
//! (`dod-batch-state-machine`).
//!
//! # The ledger is the row idempotency store, and rows freeze at their
//! # terminal state
//!
//! `(tenant_id, batch_id, row_key)` — row keys are **batch-scoped**, which is
//! what makes this ledger the row store rather than 01's endpoint store. The
//! `internal:bulk-row` lane's `client_key` is `row_id`, the ledger row's own
//! surrogate id (P-D-69), UNIQUE so the lane's key resolves one row.
//! `disposition` is nullable while the row is in flight and CHECK-pinned to
//! §1.7's terminal mix once written; **a row with a disposition is immutable**
//! — the guard refuses any UPDATE whose `OLD.disposition` is non-NULL, which
//! is `inst-bm-tables`' "immutable after their terminal state" made physical.
//! `staged_payload` is the row's imported content, canonically serialized
//! (**P-D-86**, `features/bulk-promotion.md` §7 row 30): the import door
//! writes it, the worker parses it, and `governed_live_op` keeps its own
//! stated meaning — a **live-entity** row's pending payload — so the two
//! row classes carry one column each rather than one column carrying an
//! overloaded meaning. A shape `CHECK` pins the pairing: a `product` or
//! `sku` row carries a payload, since a row the worker cannot stage should
//! never have been recorded.
//!
//! `reason` is a literal from a closed set, never operator text (**P-D-50**),
//! so the CHECK pins the one constant the design names today
//! (`batch-abandoned`); widening it is an in-place edit when a slice names a
//! second.
//!
//! # No batch guard trigger — the batch head is working state
//!
//! The worker flips `state`, stamps `claimed_at`, bumps `attempt`, writes
//! `approval_ref` at the report edge and `terminal_at` at the end: the batch
//! row is mutable by design, its discipline the CHECKs and the machine the
//! P-D-54/69 edges the worker walks. The **ledger** rows are where
//! immutability lives.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `bigint` becomes `integer`,
//! `timestamptz` becomes `text`, and the `bss.` qualification is dropped.
//! Every CHECK, both keys, the row FK and the frozen-row trigger are
//! preserved on both sides.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-bulk-tables:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_bulk_batch (
            tenant_id    uuid        NOT NULL,
            batch_id     uuid        NOT NULL,
            batch_key    text        NOT NULL,
            mode         text        NOT NULL,
            lane         text        NOT NULL,
            state        text        NOT NULL,
            operation_key text,
            approval_ref uuid,
            claimed_at   timestamptz,
            attempt      bigint      NOT NULL DEFAULT 0,
            created_at   timestamptz NOT NULL,
            terminal_at  timestamptz,
            CONSTRAINT products_bulk_batch_pkey PRIMARY KEY (tenant_id, batch_id),
            CONSTRAINT uq_products_bulk_batch_key UNIQUE (tenant_id, batch_key),
            CONSTRAINT chk_products_bulk_batch_mode CHECK (mode IN ('import', 'promote')),
            CONSTRAINT chk_products_bulk_batch_lane CHECK (lane IN ('import', 'lifecycle')),
            CONSTRAINT chk_products_bulk_batch_state CHECK (state IN ('staging', 'reported', 'approved', 'committing', 'completed', 'failed', 'abandoned')),
            CONSTRAINT chk_products_bulk_batch_attempt CHECK (attempt >= 0)
        )",
    "CREATE TABLE bss.products_bulk_row (
            tenant_id       uuid   NOT NULL,
            batch_id        uuid   NOT NULL,
            row_key         text   NOT NULL,
            row_id          uuid   NOT NULL,
            entity_kind     text   NOT NULL,
            entity_id       uuid,
            pinned_revision bigint,
            staged_payload  text,
            disposition     text,
            code            text,
            reason          text,
            governed_live_op text,
            override_acknowledged boolean NOT NULL DEFAULT false,
            terminal_at     timestamptz,
            CONSTRAINT products_bulk_row_pkey PRIMARY KEY (tenant_id, batch_id, row_key),
            CONSTRAINT uq_products_bulk_row_id UNIQUE (row_id),
            CONSTRAINT chk_products_bulk_row_key CHECK (row_key <> ''),
            CONSTRAINT chk_products_bulk_row_disposition CHECK (disposition IS NULL OR disposition IN ('published', 'applied', 'no_op', 'failed')),
            CONSTRAINT chk_products_bulk_row_reason CHECK (reason IS NULL OR reason IN ('batch-abandoned')),
            CONSTRAINT chk_products_bulk_row_terminal CHECK ((disposition IS NULL) = (terminal_at IS NULL)),
            CONSTRAINT chk_products_bulk_row_payload CHECK (entity_kind NOT IN ('product', 'sku') OR staged_payload IS NOT NULL),
            CONSTRAINT fk_products_bulk_row_batch FOREIGN KEY (tenant_id, batch_id)
                REFERENCES bss.products_bulk_batch (tenant_id, batch_id)
        )",
    "CREATE OR REPLACE FUNCTION bss.products_bulk_row_frozen() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'products_bulk_row is append-only evidence: DELETE is not permitted';
          END IF;
          IF OLD.disposition IS NOT NULL THEN
            RAISE EXCEPTION 'products_bulk_row: a row is immutable after its terminal state';
          END IF;
          RETURN NEW;
        END;
        $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_bulk_row_frozen
        BEFORE DELETE OR UPDATE ON bss.products_bulk_row
        FOR EACH ROW EXECUTE FUNCTION bss.products_bulk_row_frozen()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_bulk_row_frozen ON bss.products_bulk_row",
    "DROP FUNCTION IF EXISTS bss.products_bulk_row_frozen",
    "DROP TABLE IF EXISTS bss.products_bulk_row",
    "DROP TABLE IF EXISTS bss.products_bulk_batch",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_bulk_batch (
            tenant_id    text    NOT NULL,
            batch_id     text    NOT NULL,
            batch_key    text    NOT NULL,
            mode         text    NOT NULL,
            lane         text    NOT NULL,
            state        text    NOT NULL,
            operation_key text,
            approval_ref text,
            claimed_at   text,
            attempt      integer NOT NULL DEFAULT 0,
            created_at   text    NOT NULL,
            terminal_at  text,
            PRIMARY KEY (tenant_id, batch_id),
            CONSTRAINT uq_products_bulk_batch_key UNIQUE (tenant_id, batch_key),
            CONSTRAINT chk_products_bulk_batch_mode CHECK (mode IN ('import', 'promote')),
            CONSTRAINT chk_products_bulk_batch_lane CHECK (lane IN ('import', 'lifecycle')),
            CONSTRAINT chk_products_bulk_batch_state CHECK (state IN ('staging', 'reported', 'approved', 'committing', 'completed', 'failed', 'abandoned')),
            CONSTRAINT chk_products_bulk_batch_attempt CHECK (attempt >= 0)
        )",
    "CREATE TABLE products_bulk_row (
            tenant_id       text    NOT NULL,
            batch_id        text    NOT NULL,
            row_key         text    NOT NULL,
            row_id          text    NOT NULL,
            entity_kind     text    NOT NULL,
            entity_id       text,
            pinned_revision integer,
            staged_payload  text,
            disposition     text,
            code            text,
            reason          text,
            governed_live_op text,
            override_acknowledged integer NOT NULL DEFAULT 0,
            terminal_at     text,
            PRIMARY KEY (tenant_id, batch_id, row_key),
            CONSTRAINT uq_products_bulk_row_id UNIQUE (row_id),
            CONSTRAINT chk_products_bulk_row_key CHECK (row_key <> ''),
            CONSTRAINT chk_products_bulk_row_disposition CHECK (disposition IS NULL OR disposition IN ('published', 'applied', 'no_op', 'failed')),
            CONSTRAINT chk_products_bulk_row_reason CHECK (reason IS NULL OR reason IN ('batch-abandoned')),
            CONSTRAINT chk_products_bulk_row_terminal CHECK ((disposition IS NULL) = (terminal_at IS NULL)),
            CONSTRAINT chk_products_bulk_row_payload CHECK (entity_kind NOT IN ('product', 'sku') OR staged_payload IS NOT NULL),
            CONSTRAINT fk_products_bulk_row_batch FOREIGN KEY (tenant_id, batch_id)
                REFERENCES products_bulk_batch (tenant_id, batch_id)
        )",
    "CREATE TRIGGER trg_products_bulk_row_no_delete
        BEFORE DELETE ON products_bulk_row
        BEGIN
          SELECT RAISE(ABORT, 'products_bulk_row is append-only evidence: DELETE is not permitted');
        END",
    "CREATE TRIGGER trg_products_bulk_row_frozen
        BEFORE UPDATE ON products_bulk_row
        WHEN OLD.disposition IS NOT NULL
        BEGIN
          SELECT RAISE(ABORT, 'products_bulk_row: a row is immutable after its terminal state');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_bulk_row_frozen",
    "DROP TRIGGER IF EXISTS trg_products_bulk_row_no_delete",
    "DROP TABLE IF EXISTS products_bulk_row",
    "DROP TABLE IF EXISTS products_bulk_batch",
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
