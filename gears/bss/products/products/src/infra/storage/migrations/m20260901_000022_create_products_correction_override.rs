//! Create `bss.products_correction_override` — the break-glass correction's
//! **evidence** rows (`design/07-reference-signal.md` §4, `dod-override-table`).
//!
//! # The table IS the tripwire, and that is why there is no counter column
//!
//! `TripwireCounter` is *"a windowed count over this table — no separate
//! counter state to drift"*. So the rows carry the instant and the query
//! carries the window; a column would be a second piece of state that could
//! disagree with the evidence it summarises, and the evidence is the thing
//! an auditor reads.
//!
//! # One column per admitting arm's evidence, not one shared blob
//!
//! §4 asks for *"the arm's evidence — a per-producer unavailability snapshot
//! on arm (a), `unresolvable-target` on arm (b)"*. Those are two different
//! shapes: a snapshot is a rendered map of producers to their staleness, and
//! arm (b)'s evidence is the literal fact that the target could not be
//! resolved. `admitting_arm` names which arm admitted the override and a
//! `CHECK` pins the pair to it — arm `a` carries the snapshot and no
//! `unresolvable_target`, arm `b` the reverse. A single nullable blob would
//! let a row claim arm (a) while carrying arm (b)'s evidence, and nothing
//! would refuse it.
//!
//! # Append-only, unconditionally
//!
//! Evidential rows admit no `UPDATE` and no `DELETE`. The `UPDATE` half is
//! `m20260829_000007`'s unconditional shape rather than the head tables'
//! whitelist, because there is no admitted edit at all. The `DELETE` half is
//! **this file's own choice, not a copy**: `m20260829_000007`'s `DELETE` arm
//! is conditional, running P-D-40's referential predicate, and nothing here
//! has a referencing table to predicate on. Which collector may ever delete
//! from this table is registered as an open item — `10-retention-erasure`
//! `inst-rt-gc` lists correction overrides among the stores whose expiry
//! candidates it computes, and the audit plane resolved the same tension the
//! other way with a row-image retention arm (P-D-34). A correction that
//! turns out wrong is a **new** row, exactly as a mis-set identity is a new
//! entity: the trail is the product.
//!
//! # The SKU FK is one column, because the SKU's key is
//!
//! `products_sku`'s primary key is `(sku_id)` alone — identity is global and
//! `tenant_id` is the scope column beside it — so a two-column FK matches no
//! unique constraint and the migrator refuses its own DDL. The chain's own
//! test caught it; `m20260829_000003`'s FK to `products_product` has the
//! same single-column shape for the same reason. Tenant containment is
//! **the door's precondition, not the scope layer's**: a scope carrying
//! `OWNER_TENANT_ID In [A]` validates `tenant_id` alone and says nothing
//! about `sku_id`, so a writer must read the SKU under `(scope, tenant_id)`
//! before recording evidence against it — the shape `api::rest::skus`'
//! parent resolution already uses. Nothing writes this table yet; the
//! precondition arrives with the corrections door.
//!
//! # `ceremony_ref` carries no FK
//!
//! The record it names is `05-governance`'s approval table — which **does**
//! ship as of `m20260901_000016`, with a key an FK could target, so the
//! reason is not the one `m20260901_000014`'s `approval_ref` gave (*"the
//! record it names is 05-governance's table, which does not ship"*). The
//! reason here is the writer: no door records evidence yet, and an FK would
//! block the first evidence row on 05's write path landing first. The
//! `DoD`'s join — *"so the ceremony and the evidence are joinable from
//! either side"* — is owed on the audit side too: `products_audit_log`'s
//! roster carries no `ceremony_ref`, so the other half of the join arrives
//! with the corrections door and a column on that table.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `timestamptz` becomes `text`, and the
//! `bss.` qualification is dropped. Every `CHECK`, the key, the tripwire
//! index and the append-only guard are preserved on both sides.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-override-table:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_correction_override (
            tenant_id            uuid        NOT NULL,
            override_id          uuid        NOT NULL,
            sku_id               uuid        NOT NULL,
            field                text        NOT NULL,
            reason               text        NOT NULL,
            admitting_arm        text        NOT NULL,
            unavailability_snapshot text,
            unresolvable_target  text,
            ceremony_ref         uuid        NOT NULL,
            recorded_at          timestamptz NOT NULL,
            CONSTRAINT products_correction_override_pkey PRIMARY KEY (tenant_id, override_id),
            CONSTRAINT chk_products_correction_override_field CHECK (field <> ''),
            CONSTRAINT chk_products_correction_override_reason CHECK (reason <> ''),
            CONSTRAINT chk_products_correction_override_arm CHECK (admitting_arm IN ('producer_unavailable', 'unresolvable_target')),
            CONSTRAINT chk_products_correction_override_evidence CHECK (
                (admitting_arm = 'producer_unavailable' AND unavailability_snapshot IS NOT NULL AND unresolvable_target IS NULL)
                OR (admitting_arm = 'unresolvable_target' AND unresolvable_target IS NOT NULL AND unavailability_snapshot IS NULL)
            ),
            CONSTRAINT fk_products_correction_override_sku FOREIGN KEY (sku_id)
                REFERENCES bss.products_sku (sku_id)
        )",
    "CREATE INDEX idx_products_correction_override_window ON bss.products_correction_override USING btree (tenant_id, recorded_at)",
    "CREATE OR REPLACE FUNCTION bss.products_correction_override_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'UPDATE' THEN
            RAISE EXCEPTION 'products_correction_override is evidence: UPDATE is not permitted';
          END IF;
          RAISE EXCEPTION 'products_correction_override is evidence: DELETE is not permitted';
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_correction_override_append_only
        BEFORE DELETE OR UPDATE ON bss.products_correction_override
        FOR EACH ROW EXECUTE FUNCTION bss.products_correction_override_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_correction_override_append_only ON bss.products_correction_override",
    "DROP FUNCTION IF EXISTS bss.products_correction_override_append_only",
    "DROP TABLE IF EXISTS bss.products_correction_override",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_correction_override (
            tenant_id            text NOT NULL,
            override_id          text NOT NULL,
            sku_id               text NOT NULL,
            field                text NOT NULL,
            reason               text NOT NULL,
            admitting_arm        text NOT NULL,
            unavailability_snapshot text,
            unresolvable_target  text,
            ceremony_ref         text NOT NULL,
            recorded_at          text NOT NULL,
            PRIMARY KEY (tenant_id, override_id),
            CONSTRAINT chk_products_correction_override_field CHECK (field <> ''),
            CONSTRAINT chk_products_correction_override_reason CHECK (reason <> ''),
            CONSTRAINT chk_products_correction_override_arm CHECK (admitting_arm IN ('producer_unavailable', 'unresolvable_target')),
            CONSTRAINT chk_products_correction_override_evidence CHECK (
                (admitting_arm = 'producer_unavailable' AND unavailability_snapshot IS NOT NULL AND unresolvable_target IS NULL)
                OR (admitting_arm = 'unresolvable_target' AND unresolvable_target IS NOT NULL AND unavailability_snapshot IS NULL)
            ),
            CONSTRAINT fk_products_correction_override_sku FOREIGN KEY (sku_id)
                REFERENCES products_sku (sku_id)
        )",
    "CREATE INDEX idx_products_correction_override_window ON products_correction_override (tenant_id, recorded_at)",
    "CREATE TRIGGER trg_products_correction_override_no_update
        BEFORE UPDATE ON products_correction_override
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT, 'products_correction_override is evidence: UPDATE is not permitted');
        END",
    "CREATE TRIGGER trg_products_correction_override_no_delete
        BEFORE DELETE ON products_correction_override
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT, 'products_correction_override is evidence: DELETE is not permitted');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_correction_override_no_delete",
    "DROP TRIGGER IF EXISTS trg_products_correction_override_no_update",
    "DROP TABLE IF EXISTS products_correction_override",
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
