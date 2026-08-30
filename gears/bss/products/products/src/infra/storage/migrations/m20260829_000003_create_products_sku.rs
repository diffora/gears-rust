//! Create `bss.products_sku` — a SKU's identity, its parent link, its lifecycle
//! and the two version counters (`design/01-foundation.md` §4.2).
//!
//! # `ReservationIndex`
//!
//! `uq_products_sku_code` is the reservation itself: a partial unique index on
//! `(tenant_id, sku_code)` where the row is not discarded. The **insert** is
//! what reserves, which is why the loser of a concurrent race is refused by the
//! index rather than by a read-then-act check that would lose the race it
//! exists to decide. Discard releases; first publish makes the column immutable,
//! and that half is the head-row guard's, below.
//!
//! # `product_id` carries a real foreign key
//!
//! Unlike the sibling pricing gear's bundle table, this reference **can** be a
//! foreign key: `products_product` is keyed on `product_id` alone and carries
//! total uniqueness on it, so Postgres accepts it as a referent. The parent's
//! *state* checks — terminal, retire-intent — are the door's, because a foreign
//! key can express existence and not lifecycle.
//!
//! # Backend differences
//!
//! `uuid` becomes `text`, the `bss.` qualification is dropped, and timestamps
//! become `text`. Every `CHECK`, index, the primary key and the foreign key are
//! preserved on both sides.
//!
//! # The append-only head-row guard (`cpt-cf-bss-products-dod-append-only-guard`)
//!
//! The `products_sku` half of the guard mirrors `products_product`'s clause
//! for clause — same edge list, same bucket gating, same immutable set — with
//! the shape difference §4.2 gives this table: no `brand_id` or `name`/
//! `name_normalized` columns, and a parent link, `product_id`, that this
//! table's own passage assigns to **bucket-i** rather than to name/brand
//! (`design/01-foundation.md` §4.1, "A SKU's parent link `product_id` is
//! **bucket-i**"). See the sibling migration's module doc for the full
//! per-clause rationale; this file states only the SKU-specific facts.
//!
//! ## What the whitelist admits, clause by clause
//!
//! - **`lifecycle_state`** moves only along the same five-edge state machine
//!   `features/foundation.md` §4 declares — the machine is shared by Product
//!   and SKU (`design/01-foundation.md` §4, "The machine is shared by
//!   `Product` and `SKU`"). `retired` and `discarded` are terminal.
//! - **`published_version`** is admitted only unchanged or exactly `OLD + 1`;
//!   the "matching frozen version row exists" half is **owed to Phase 6**'s
//!   `products_entity_version`, for the identical reason as the sibling table.
//! - **Bucket-i** (`sku_code`, `product_id`) is admitted only while
//!   `OLD.published_version = 0` **and** `OLD.lifecycle_state` is
//!   non-terminal, never again after first publish.
//! - **Bucket-iii** (`region_scope`, `brand_scope` — this table has no
//!   `name`/`name_normalized`) is admitted while `OLD.lifecycle_state` is
//!   non-terminal.
//! - **`internal_revision`** must move by exactly `OLD + 1` on every admitted
//!   `UPDATE`, without exception.
//! - **`updated_at`** is admitted unconditionally.
//! - **`tenant_id`, the primary key (`sku_id`) and `created_by`** are admitted
//!   in **no** update at all (P-D-34); neither is `created_at`.
//!
//! **Bucket-ii and bucket-iv have no members among today's columns.** The
//! same four not-yet-existing columns the sibling table's doc names —
//! `cloned_from`, `deprecation_provenance`, `replaced_by_sku_id` (slice 03)
//! and `composition_pending` (slice 07) — are owed here too; this migration
//! guards what exists today.
//!
//! The guard judges the data, never the door (P-D-31): every predicate reads
//! only `OLD` and `NEW`, on both engines, for the identical reason as the
//! sibling table's guard.
//!
//! **DELETE is refused unconditionally, on both engines** — the same C5
//! append-only posture as `products_product`, `products_audit_log` and history
//! rows. `REVOKE UPDATE, DELETE` is not issued (P-D-46; the deployment role
//! this migration does not own; `SQLite` has no `GRANT`/`REVOKE`).
//!
//! Postgres raises through one `PL/pgSQL` function branching on `TG_OP`;
//! `SQLite` mirrors it as one no-delete trigger plus one `WHEN`-guarded
//! trigger per column class, exactly as the sibling table does.
//!
//! @cpt-cf-bss-products-fr-skucode-reservation-concurrency
//! @cpt-cf-bss-products-dod-entity-tables
//! @cpt-cf-bss-products-dod-code-reservation
//! @cpt-cf-bss-products-dod-append-only-guard

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_sku (
            tenant_id         uuid        NOT NULL,
            sku_id            uuid        NOT NULL,
            product_id        uuid        NOT NULL,
            sku_code          text        NOT NULL,
            lifecycle_state   text        NOT NULL,
            internal_revision bigint      NOT NULL,
            published_version bigint      NOT NULL,
            region_scope      text        NOT NULL DEFAULT '',
            brand_scope       text        NOT NULL DEFAULT '',
            created_by        text        NOT NULL,
            created_at        timestamptz NOT NULL,
            updated_at        timestamptz NOT NULL,
            CONSTRAINT products_sku_pkey PRIMARY KEY (sku_id),
            CONSTRAINT fk_products_sku_product FOREIGN KEY (product_id) REFERENCES bss.products_product (product_id),
            CONSTRAINT chk_products_sku_lifecycle_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_sku_internal_revision CHECK (internal_revision >= 1),
            CONSTRAINT chk_products_sku_published_version CHECK (published_version >= 0)
        )",
    "CREATE INDEX idx_products_sku_tenant ON bss.products_sku USING btree (tenant_id, sku_id)",
    "CREATE INDEX idx_products_sku_parent ON bss.products_sku USING btree (tenant_id, product_id)",
    "CREATE UNIQUE INDEX uq_products_sku_code ON bss.products_sku USING btree (tenant_id, sku_code) WHERE lifecycle_state <> 'discarded'",
    "CREATE OR REPLACE FUNCTION bss.products_sku_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'products_sku is append-only: DELETE is not permitted';
          END IF;

          IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
             OR NEW.sku_id IS DISTINCT FROM OLD.sku_id
             OR NEW.created_by IS DISTINCT FROM OLD.created_by
             OR NEW.created_at IS DISTINCT FROM OLD.created_at
          THEN
            RAISE EXCEPTION 'products_sku: tenant_id, sku_id, created_by and created_at are immutable';
          END IF;

          IF NEW.internal_revision IS DISTINCT FROM OLD.internal_revision + 1 THEN
            RAISE EXCEPTION 'products_sku: internal_revision must move by exactly one on every admitted update';
          END IF;

          IF NOT (NEW.published_version IS NOT DISTINCT FROM OLD.published_version
                  OR NEW.published_version IS NOT DISTINCT FROM OLD.published_version + 1)
          THEN
            RAISE EXCEPTION 'products_sku: published_version only moves by +1';
          END IF;

          IF NEW.lifecycle_state IS DISTINCT FROM OLD.lifecycle_state
             AND NOT (
               (OLD.lifecycle_state = 'draft' AND NEW.lifecycle_state = 'published')
               OR (OLD.lifecycle_state = 'draft' AND NEW.lifecycle_state = 'discarded')
               OR (OLD.lifecycle_state = 'published' AND NEW.lifecycle_state = 'deprecated')
               OR (OLD.lifecycle_state = 'deprecated' AND NEW.lifecycle_state = 'published')
               OR (OLD.lifecycle_state = 'deprecated' AND NEW.lifecycle_state = 'retired')
             )
          THEN
            RAISE EXCEPTION 'products_sku: lifecycle_state % -> % is not an admitted edge', OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          IF (NEW.sku_code IS DISTINCT FROM OLD.sku_code OR NEW.product_id IS DISTINCT FROM OLD.product_id)
             AND NOT (OLD.published_version = 0 AND OLD.lifecycle_state NOT IN ('retired', 'discarded'))
          THEN
            RAISE EXCEPTION 'products_sku: bucket-i columns are admitted only before first publish, on a non-terminal head';
          END IF;

          IF (NEW.region_scope IS DISTINCT FROM OLD.region_scope
              OR NEW.brand_scope IS DISTINCT FROM OLD.brand_scope)
             AND OLD.lifecycle_state IN ('retired', 'discarded')
          THEN
            RAISE EXCEPTION 'products_sku: bucket-iii columns are admitted only while the head is non-terminal';
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_sku_append_only BEFORE DELETE OR UPDATE ON bss.products_sku FOR EACH ROW EXECUTE FUNCTION bss.products_sku_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.products_sku",
    "DROP FUNCTION IF EXISTS bss.products_sku_append_only()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_sku (
            tenant_id         text   NOT NULL,
            sku_id            text   NOT NULL,
            product_id        text   NOT NULL,
            sku_code          text   NOT NULL,
            lifecycle_state   text   NOT NULL,
            internal_revision bigint NOT NULL,
            published_version bigint NOT NULL,
            region_scope      text   NOT NULL DEFAULT '',
            brand_scope       text   NOT NULL DEFAULT '',
            created_by        text   NOT NULL,
            created_at        text   NOT NULL,
            updated_at        text   NOT NULL,
            PRIMARY KEY (sku_id),
            CONSTRAINT fk_products_sku_product FOREIGN KEY (product_id) REFERENCES products_product (product_id),
            CONSTRAINT chk_products_sku_lifecycle_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_sku_internal_revision CHECK (internal_revision >= 1),
            CONSTRAINT chk_products_sku_published_version CHECK (published_version >= 0)
        )",
    "CREATE INDEX idx_products_sku_tenant ON products_sku (tenant_id, sku_id)",
    "CREATE INDEX idx_products_sku_parent ON products_sku (tenant_id, product_id)",
    "CREATE UNIQUE INDEX uq_products_sku_code ON products_sku (tenant_id, sku_code) WHERE lifecycle_state <> 'discarded'",
    "CREATE TRIGGER trg_products_sku_no_delete BEFORE DELETE ON products_sku FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'products_sku is append-only: DELETE is not permitted'); END",
    "CREATE TRIGGER trg_products_sku_immutable_columns BEFORE UPDATE ON products_sku FOR EACH ROW WHEN
            NEW.tenant_id IS NOT OLD.tenant_id
            OR NEW.sku_id IS NOT OLD.sku_id
            OR NEW.created_by IS NOT OLD.created_by
            OR NEW.created_at IS NOT OLD.created_at
        BEGIN SELECT RAISE(ABORT, 'products_sku: tenant_id, sku_id, created_by and created_at are immutable'); END",
    "CREATE TRIGGER trg_products_sku_internal_revision BEFORE UPDATE ON products_sku FOR EACH ROW WHEN
            NEW.internal_revision IS NOT (OLD.internal_revision + 1)
        BEGIN SELECT RAISE(ABORT, 'products_sku: internal_revision must move by exactly one on every admitted update'); END",
    "CREATE TRIGGER trg_products_sku_published_version BEFORE UPDATE ON products_sku FOR EACH ROW WHEN NOT (
            NEW.published_version IS OLD.published_version
            OR NEW.published_version IS (OLD.published_version + 1)
        ) BEGIN SELECT RAISE(ABORT, 'products_sku: published_version only moves by +1'); END",
    "CREATE TRIGGER trg_products_sku_lifecycle_edge BEFORE UPDATE ON products_sku FOR EACH ROW WHEN
            NEW.lifecycle_state IS NOT OLD.lifecycle_state
            AND NOT (
                (OLD.lifecycle_state IS 'draft' AND NEW.lifecycle_state IS 'published')
                OR (OLD.lifecycle_state IS 'draft' AND NEW.lifecycle_state IS 'discarded')
                OR (OLD.lifecycle_state IS 'published' AND NEW.lifecycle_state IS 'deprecated')
                OR (OLD.lifecycle_state IS 'deprecated' AND NEW.lifecycle_state IS 'published')
                OR (OLD.lifecycle_state IS 'deprecated' AND NEW.lifecycle_state IS 'retired')
            )
        BEGIN SELECT RAISE(ABORT, 'products_sku: lifecycle_state transition is not an admitted edge'); END",
    "CREATE TRIGGER trg_products_sku_bucket_i BEFORE UPDATE ON products_sku FOR EACH ROW WHEN (
            NEW.sku_code IS NOT OLD.sku_code OR NEW.product_id IS NOT OLD.product_id
        ) AND NOT (
            OLD.published_version = 0 AND OLD.lifecycle_state NOT IN ('retired', 'discarded')
        ) BEGIN SELECT RAISE(ABORT, 'products_sku: bucket-i columns are admitted only before first publish, on a non-terminal head'); END",
    "CREATE TRIGGER trg_products_sku_bucket_iii BEFORE UPDATE ON products_sku FOR EACH ROW WHEN (
            NEW.region_scope IS NOT OLD.region_scope
            OR NEW.brand_scope IS NOT OLD.brand_scope
        ) AND OLD.lifecycle_state IN ('retired', 'discarded')
        BEGIN SELECT RAISE(ABORT, 'products_sku: bucket-iii columns are admitted only while the head is non-terminal'); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_sku"];

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
