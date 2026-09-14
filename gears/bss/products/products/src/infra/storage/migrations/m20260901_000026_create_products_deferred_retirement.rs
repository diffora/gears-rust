//! Create `bss.products_deferred_retirement` — the leave-and-list snapshot of
//! a Product cascade that could not finish
//! (`design/04-lifecycle.md` §4; `features/lifecycle.md`
//! `dod-deferred-retirement-store`).
//!
//! # Guard family: never delete; freeze once resolved
//!
//! A deferred-retirement row is an **audit-continuity record**: resolved rows
//! flip `resolved_at` and **never delete**, so a second cascade on the same
//! Product can land without colliding on an unresolved corpse. The guard
//! therefore refuses every `DELETE` and any `UPDATE` whose `OLD.resolved_at`
//! is already set. Unresolved rows stay mutable so the resolution write can
//! land; a resolved row does not. This is the `identity_ref` / approval
//! posture applied to a nullable stamp rather than a state enum — **not** the
//! rebuildable-family exemption, which would admit truncation.
//!
//! # One live deferral per Product
//!
//! The partial `UNIQUE (tenant_id, product_id) WHERE resolved_at IS NULL` is
//! the physical floor under "at most one live deferral": a cancelled cascade
//! **MUST** resolve its row `cascade_cancelled`, which frees the slot. On a
//! bare composite key a cancelled cascade left an unresolved row forever and
//! a second cascade collided — the partial index is what closes that hole.
//!
//! # Key includes `cascade_ref`
//!
//! PK is `(tenant_id, product_id, cascade_ref)` with `cascade_ref` the
//! parent's `ScheduledTransition` id. The FK pins that reference so a
//! snapshot cannot name a transition that does not exist; the partial UNIQUE
//! above is what keeps live rows singular, not the PK.
//!
//! # Indexes lead with `tenant_id`
//!
//! The live-deferral unique index leads with `tenant_id`. A by-product look
//! without the tenant partition cannot serve a per-partition budget.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `timestamptz` becomes `text`, and the
//! `bss.` qualification is dropped. The CHECKs, the partial UNIQUE, the FK
//! and both halves of the guard are preserved on both sides; `SQLite` splits
//! the guard into per-op triggers.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-deferred-retirement-store:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_deferred_retirement (
            tenant_id           uuid        NOT NULL,
            product_id          uuid        NOT NULL,
            cascade_ref         uuid        NOT NULL,
            children_snapshot   text        NOT NULL,
            created_by          uuid        NOT NULL,
            resolved_at         timestamptz,
            resolution          text,
            created_at          timestamptz NOT NULL,
            CONSTRAINT products_deferred_retirement_pkey
                PRIMARY KEY (tenant_id, product_id, cascade_ref),
            CONSTRAINT chk_products_deferred_retirement_snapshot
                CHECK (children_snapshot <> ''),
            CONSTRAINT chk_products_deferred_retirement_resolution
                CHECK (
                    resolution IS NULL
                    OR resolution IN ('children_cleared', 'cascade_cancelled')
                ),
            CONSTRAINT chk_products_deferred_retirement_resolved_pair
                CHECK ((resolved_at IS NULL) = (resolution IS NULL)),
            CONSTRAINT fk_products_deferred_retirement_cascade
                FOREIGN KEY (cascade_ref)
                REFERENCES bss.products_scheduled_transition (transition_id)
        )",
    "CREATE UNIQUE INDEX uq_products_deferred_retirement_live
        ON bss.products_deferred_retirement USING btree (tenant_id, product_id)
        WHERE resolved_at IS NULL",
    "CREATE OR REPLACE FUNCTION bss.products_deferred_retirement_frozen() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'products_deferred_retirement rows are never deleted: DELETE is not permitted';
          END IF;
          IF OLD.resolved_at IS NOT NULL THEN
            RAISE EXCEPTION 'products_deferred_retirement: a resolved deferral is immutable';
          END IF;
          RETURN NEW;
        END;
        $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_deferred_retirement_frozen
        BEFORE DELETE OR UPDATE ON bss.products_deferred_retirement
        FOR EACH ROW EXECUTE FUNCTION bss.products_deferred_retirement_frozen()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_deferred_retirement_frozen ON bss.products_deferred_retirement",
    "DROP FUNCTION IF EXISTS bss.products_deferred_retirement_frozen",
    "DROP TABLE IF EXISTS bss.products_deferred_retirement",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_deferred_retirement (
            tenant_id           text NOT NULL,
            product_id          text NOT NULL,
            cascade_ref         text NOT NULL,
            children_snapshot   text NOT NULL,
            created_by          text NOT NULL,
            resolved_at         text,
            resolution          text,
            created_at          text NOT NULL,
            PRIMARY KEY (tenant_id, product_id, cascade_ref),
            CONSTRAINT chk_products_deferred_retirement_snapshot
                CHECK (children_snapshot <> ''),
            CONSTRAINT chk_products_deferred_retirement_resolution
                CHECK (
                    resolution IS NULL
                    OR resolution IN ('children_cleared', 'cascade_cancelled')
                ),
            CONSTRAINT chk_products_deferred_retirement_resolved_pair
                CHECK ((resolved_at IS NULL) = (resolution IS NULL)),
            CONSTRAINT fk_products_deferred_retirement_cascade
                FOREIGN KEY (cascade_ref)
                REFERENCES products_scheduled_transition (transition_id)
        )",
    "CREATE UNIQUE INDEX uq_products_deferred_retirement_live
        ON products_deferred_retirement (tenant_id, product_id)
        WHERE resolved_at IS NULL",
    "CREATE TRIGGER trg_products_deferred_retirement_no_delete
        BEFORE DELETE ON products_deferred_retirement
        BEGIN
          SELECT RAISE(ABORT, 'products_deferred_retirement rows are never deleted: DELETE is not permitted');
        END",
    "CREATE TRIGGER trg_products_deferred_retirement_frozen
        BEFORE UPDATE ON products_deferred_retirement
        WHEN OLD.resolved_at IS NOT NULL
        BEGIN
          SELECT RAISE(ABORT, 'products_deferred_retirement: a resolved deferral is immutable');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_deferred_retirement_frozen",
    "DROP TRIGGER IF EXISTS trg_products_deferred_retirement_no_delete",
    "DROP TABLE IF EXISTS products_deferred_retirement",
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
