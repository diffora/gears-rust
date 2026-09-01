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
//! - **`published_version`** is admitted only unchanged or exactly `OLD + 1`,
//!   **and, when it moves at all, only where the matching frozen version row
//!   already exists** — a row of `products_entity_version` keyed
//!   `(NEW.tenant_id, 'sku', NEW.sku_id, NEW.published_version)`. Both halves
//!   of the `DoD`'s rule ship here; an unchanged `published_version` is
//!   admitted with no subquery. The existence half was owed to Phase 6 when
//!   this file was written and is paid now, for the identical reason and by
//!   the identical mechanism as the sibling table: a trigger body resolves
//!   table names at execution, not at creation, so this trigger may reference
//!   `m20260829_000007_create_products_entity_version`'s table even though
//!   that migration runs later in name order. `migrations_tests.rs` asserts
//!   that empirically rather than assuming it.
//!
//!   A subquery is compatible with **P-D-31**, whose objection was to a guard
//!   reading the **door** through a session variable that exists on one engine
//!   only: this subquery judges **data**, and both engines evaluate it.
//!
//!   Since Phase 6 a **third** half ships alongside them, identical to the
//!   sibling table's: a bump is refused outright when `OLD.lifecycle_state`
//!   is `retired` or `discarded`. No neighbouring clause reached that write
//!   — a publish of an already-terminal head writes no `lifecycle_state`, so
//!   the edge clause never fires — and
//!   `cpt-cf-bss-products-dod-transition-guard` requires refusing *"any head
//!   write on a `retired` or `discarded` row"* (P-D-25, P-D-32) under §1.6
//!   C5's physical append-only posture. It is gated on the counter
//!   **moving**, deliberately: it is not a ban on every `UPDATE` of a
//!   terminal row, because slice 04 writes `deprecation_provenance` and
//!   `replaced_by_sku_id` on terminal rows by design. Do not simplify it
//!   into a blanket ban; `migrations_tests.rs` pins both sides of that
//!   boundary.
//! - **Bucket-i** (`sku_code`, `product_id`) is admitted only while
//!   `OLD.published_version = 0` **and** `OLD.lifecycle_state` is
//!   non-terminal, never again after first publish.
//! - **Bucket-iii** (`region_scope`, `brand_scope` — this table has no
//!   `name`/`name_normalized`) is admitted while `OLD.lifecycle_state` is
//!   non-terminal.
//! - **`internal_revision`** must move by exactly `OLD + 1` on every admitted
//!   `UPDATE`, without exception.
//! - **`updated_at`** is admitted unconditionally.
//! - **`composition_pending`** is admitted **only in the same statement as a
//!   `published_version` bump** — a change to the flag where
//!   `published_version` does not also move is refused
//!   (`design/01-foundation.md` §4.2: the flag is *"changed only in the same
//!   statement as a `published_version` bump"*). That one predicate admits
//!   both writers without the guard knowing which door it was: the
//!   `PublishDoor`'s raise on an override-carrying `bundle` publish (P-D-30)
//!   and slice 06's clearing re-publish (`inst-cc-clear`), both of which are
//!   the publish door's own head-row `UPDATE` carrying `published_version
//!   += 1` (**P-D-32**: `inst-fd-save-txn` never touches `published_version`,
//!   so an ordinary operator save cannot move the flag — which is what makes
//!   it system-owned rather than a bucket-iii/iv column).
//! - **`tenant_id`, the primary key (`sku_id`) and `created_by`** are admitted
//!   in **no** update at all (P-D-34); neither is `created_at`.
//!
//! **Bucket-ii and bucket-iv have no members among today's columns.** The
//! remaining three not-yet-existing columns the sibling table's doc names are
//! owed here too, and they are **not all owed by the same slice**:
//! `cloned_from`, `deprecation_provenance` and `replaced_by_sku_id` arrive
//! with slice 03, while **`composition_pending` is this slice's own** — §1.5's
//! **In** list names *"the `PublishDoor`'s `composition_pending` write"* among
//! the guards that *"ride this slice's first migration and publish door"*, and
//! assigns only the composition *semantics* to slice 06.
//!
//! ## `composition_pending` ships here, its semantics do not
//!
//! The column ships in this migration with its **guard** (the clause above,
//! on both engines) and its **default**: `NOT NULL DEFAULT false` (**P-D-35**
//! — the create flow writes it nowhere and the publish door on a `bundle` is
//! its only raiser, so the default is the unraised state, and the first
//! migration needs no nullable third reading). It is a `products_sku` column
//! only: `bundle` is a value of the SKU-only `type` column, so
//! `products_product` carries none (§4.2).
//!
//! What remains **owed** is the *semantics*, not the schema:
//!
//! - slice 06's clear lane (`inst-cc-clear`) — the inbound composition signal,
//!   its deferred-on-a-dirty-head behaviour and the `SkuCompositionCleared`
//!   emission;
//! - slice 03's bundle-override condition (`inst-cl-bundle-override`) — the
//!   `BUNDLE_OVERRIDE_REQUIRED` refusal whose acknowledgment is the operand
//!   P-D-30 makes the door read;
//! - the `PublishDoor`'s **write** of the flag, a later wave of **this same
//!   phase** rather than a later slice's arrival.
//!
//! The guard is deliberately live ahead of all three: it judges data, so it
//! costs nothing while nothing writes the column, and it means the first
//! writer to arrive lands inside a predicate rather than beside one.
//!
//! The guard judges the data, never the door (P-D-31): every predicate reads
//! only `OLD`, `NEW` and — in the `published_version` clause alone —
//! committed data in another table, on both engines, for the identical reason
//! as the sibling table's guard.
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
//! @cpt-dod:cpt-cf-bss-products-dod-entity-tables:p1
//! @cpt-dod:cpt-cf-bss-products-dod-code-reservation:p1
//! @cpt-dod:cpt-cf-bss-products-dod-append-only-guard:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_sku (
            tenant_id           uuid        NOT NULL,
            sku_id              uuid        NOT NULL,
            product_id          uuid        NOT NULL,
            sku_code            text        NOT NULL,
            lifecycle_state     text        NOT NULL,
            internal_revision   bigint      NOT NULL,
            published_version   bigint      NOT NULL,
            composition_pending boolean     NOT NULL DEFAULT false,
            region_scope        text        NOT NULL DEFAULT '',
            brand_scope         text        NOT NULL DEFAULT '',
            created_by          text        NOT NULL,
            created_at          timestamptz NOT NULL,
            cloned_from         uuid,
            cloned_from_version bigint,
            updated_at          timestamptz NOT NULL,
            CONSTRAINT products_sku_pkey PRIMARY KEY (sku_id),
            CONSTRAINT fk_products_sku_product FOREIGN KEY (product_id) REFERENCES bss.products_product (product_id),
            CONSTRAINT chk_products_sku_lifecycle_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_sku_internal_revision CHECK (internal_revision >= 1),
            CONSTRAINT chk_products_sku_published_version CHECK (published_version >= 0),
            CONSTRAINT chk_products_sku_cloned_from_shape CHECK (cloned_from IS NOT NULL OR cloned_from_version IS NULL)
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
             OR NEW.cloned_from IS DISTINCT FROM OLD.cloned_from
             OR NEW.cloned_from_version IS DISTINCT FROM OLD.cloned_from_version
          THEN
            RAISE EXCEPTION 'products_sku: tenant_id, sku_id, created_by, created_at and the cloned_from pair are immutable';
          END IF;

          IF NEW.internal_revision IS DISTINCT FROM OLD.internal_revision + 1 THEN
            RAISE EXCEPTION 'products_sku: internal_revision must move by exactly one on every admitted update';
          END IF;

          IF NOT (NEW.published_version IS NOT DISTINCT FROM OLD.published_version
                  OR NEW.published_version IS NOT DISTINCT FROM OLD.published_version + 1)
          THEN
            RAISE EXCEPTION 'products_sku: published_version only moves by +1';
          END IF;

          IF NEW.published_version IS DISTINCT FROM OLD.published_version
             AND NOT EXISTS (
               SELECT 1 FROM bss.products_entity_version v
               WHERE v.tenant_id = NEW.tenant_id
                 AND v.entity_kind = 'sku'
                 AND v.entity_id = NEW.sku_id
                 AND v.published_version = NEW.published_version
             )
          THEN
            RAISE EXCEPTION 'products_sku: a published_version bump requires the matching products_entity_version row to exist';
          END IF;

          IF NEW.published_version IS DISTINCT FROM OLD.published_version
             AND OLD.lifecycle_state IN ('retired', 'discarded')
          THEN
            RAISE EXCEPTION 'products_sku: a published_version bump is not admitted on a terminal head';
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

          IF NEW.composition_pending IS DISTINCT FROM OLD.composition_pending
             AND NEW.published_version IS NOT DISTINCT FROM OLD.published_version
          THEN
            RAISE EXCEPTION 'products_sku: composition_pending is admitted only in the same statement as a published_version bump';
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
            tenant_id           text    NOT NULL,
            sku_id              text    NOT NULL,
            product_id          text    NOT NULL,
            sku_code            text    NOT NULL,
            lifecycle_state     text    NOT NULL,
            internal_revision   bigint  NOT NULL,
            published_version   bigint  NOT NULL,
            composition_pending boolean NOT NULL DEFAULT 0,
            region_scope        text    NOT NULL DEFAULT '',
            brand_scope         text    NOT NULL DEFAULT '',
            created_by          text    NOT NULL,
            created_at          text    NOT NULL,
            cloned_from         text,
            cloned_from_version integer,
            updated_at          text    NOT NULL,
            PRIMARY KEY (sku_id),
            CONSTRAINT fk_products_sku_product FOREIGN KEY (product_id) REFERENCES products_product (product_id),
            CONSTRAINT chk_products_sku_lifecycle_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_sku_internal_revision CHECK (internal_revision >= 1),
            CONSTRAINT chk_products_sku_published_version CHECK (published_version >= 0),
            CONSTRAINT chk_products_sku_cloned_from_shape CHECK (cloned_from IS NOT NULL OR cloned_from_version IS NULL)
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
            OR NEW.cloned_from IS NOT OLD.cloned_from
            OR NEW.cloned_from_version IS NOT OLD.cloned_from_version
        BEGIN SELECT RAISE(ABORT, 'products_sku: tenant_id, sku_id, created_by, created_at and the cloned_from pair are immutable'); END",
    "CREATE TRIGGER trg_products_sku_internal_revision BEFORE UPDATE ON products_sku FOR EACH ROW WHEN
            NEW.internal_revision IS NOT (OLD.internal_revision + 1)
        BEGIN SELECT RAISE(ABORT, 'products_sku: internal_revision must move by exactly one on every admitted update'); END",
    "CREATE TRIGGER trg_products_sku_published_version BEFORE UPDATE ON products_sku FOR EACH ROW WHEN NOT (
            NEW.published_version IS OLD.published_version
            OR NEW.published_version IS (OLD.published_version + 1)
        ) BEGIN SELECT RAISE(ABORT, 'products_sku: published_version only moves by +1'); END",
    "CREATE TRIGGER trg_products_sku_published_version_row BEFORE UPDATE ON products_sku FOR EACH ROW WHEN
            NEW.published_version IS NOT OLD.published_version
            AND NOT EXISTS (
                SELECT 1 FROM products_entity_version v
                WHERE v.tenant_id IS NEW.tenant_id
                  AND v.entity_kind = 'sku'
                  AND v.entity_id IS NEW.sku_id
                  AND v.published_version IS NEW.published_version
            )
        BEGIN SELECT RAISE(ABORT, 'products_sku: a published_version bump requires the matching products_entity_version row to exist'); END",
    "CREATE TRIGGER trg_products_sku_published_version_terminal BEFORE UPDATE ON products_sku FOR EACH ROW WHEN
            NEW.published_version IS NOT OLD.published_version
            AND OLD.lifecycle_state IN ('retired', 'discarded')
        BEGIN SELECT RAISE(ABORT, 'products_sku: a published_version bump is not admitted on a terminal head'); END",
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
    "CREATE TRIGGER trg_products_sku_composition_pending BEFORE UPDATE ON products_sku FOR EACH ROW WHEN
            NEW.composition_pending IS NOT OLD.composition_pending
            AND NEW.published_version IS OLD.published_version
        BEGIN SELECT RAISE(ABORT, 'products_sku: composition_pending is admitted only in the same statement as a published_version bump'); END",
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
