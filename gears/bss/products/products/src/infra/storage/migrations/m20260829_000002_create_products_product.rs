//! Create `bss.products_product` — a Product's identity, lifecycle and the two
//! version counters (`design/01-foundation.md` §4.1).
//!
//! # The two partial unique indexes, and why both are partial
//!
//! `uq_products_product_name` admits one non-discarded holder per
//! `(tenant_id, brand_id, name_normalized)`. It is **partial on
//! `lifecycle_state <> 'discarded'`** because discard releases the name exactly
//! as it releases codes: holding it would let one typo in a never-published
//! draft burn a name forever. The asymmetry with `retired`, which *does* hold
//! its name, is the intended one — a discarded draft was never published and a
//! retired entity was.
//!
//! `uq_products_product_code` is partial on the same predicate **and** on the
//! column being set, since `product_code` is optional and a NULL is not a
//! reservation.
//!
//! Region scope plays no part in either. That is P-D-04, and it is the reason
//! the name index is described as **absolute** uniqueness rather than
//! scope-relative.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` and the `bss.` qualification is dropped. Every
//! `CHECK`, index and the primary key are preserved on both sides. Timestamps
//! are `timestamptz` on Postgres and `text` on `SQLite`, which is what the
//! driver pair stores a `DateTime<Utc>` as.
//!
//! # The append-only head-row guard (`cpt-cf-bss-products-dod-append-only-guard`)
//!
//! Landed in this file rather than a follow-up migration, per this chain's own
//! rule of editing a migration in place. The whitelist below is deliberately
//! structured so a later slice adding a column to a bucket only ever touches
//! this file's `WHEN`/`IF` predicates, never a new migration.
//!
//! ## What the whitelist admits, clause by clause
//!
//! - **`lifecycle_state`** moves only along the five-edge state machine
//!   `features/foundation.md` §4 declares (`draft → published`,
//!   `draft → discarded`, `published → deprecated`, `deprecated → published`,
//!   `deprecated → retired`); `retired` and `discarded` are terminal, so no
//!   edge admits a write out of either. A same-value write (`NEW = OLD`) is
//!   not a transition and is never blocked by this clause.
//! - **`published_version`** is admitted only unchanged or exactly `OLD + 1`.
//!   The `DoD` also requires the write to land "only where the matching frozen
//!   version row exists"; that half is **owed to Phase 6**, whose
//!   `products_entity_version` table this migration predates and cannot read
//!   from a trigger before it exists. Only the row-image `+1` half ships here.
//! - **Bucket-i** (`product_code`, `brand_id` — `design/01-foundation.md` §4.1,
//!   "Bucket assignment for the Foundation-owned columns") is admitted only
//!   while `OLD.published_version = 0` **and** `OLD.lifecycle_state` is
//!   non-terminal, and never again once either condition lapses — "never
//!   after first publish" in the `DoD`'s own words.
//! - **Bucket-iii** (`name`, `name_normalized`, `region_scope`, `brand_scope`
//!   — same design passage) is admitted while `OLD.lifecycle_state` is
//!   non-terminal, with no `published_version` gate: a published or
//!   deprecated Product may still be renamed or rescoped under governance.
//! - **`internal_revision`** must move by exactly `OLD + 1` on every admitted
//!   `UPDATE`, without exception — there is no carve-out clause for it, unlike
//!   every other column class.
//! - **`updated_at`** is admitted unconditionally.
//! - **`tenant_id`, the primary key (`product_id`) and `created_by`** are
//!   admitted in **no** update at all (P-D-34); neither is `created_at`,
//!   which the `DoD` does not name but which sits with them for the same
//!   reason — none of the four is ever supplied by an admitted `UPDATE`, only
//!   by the `INSERT`.
//!
//! **Bucket-ii and bucket-iv have no members among today's columns.** Slice 03
//! (`cloned_from`, `deprecation_provenance`, `replaced_by_sku_id`) and slice 07
//! (`composition_pending`) bring the columns those clauses govern; this
//! migration guards what exists today and is silent — by measurement, not by
//! oversight — on the four columns it does not yet have. When those columns
//! land, their clauses join this same file's whitelist rather than a
//! follow-up migration.
//!
//! ## The guard judges the data, never the door (P-D-31)
//!
//! Every predicate above reads only `OLD` and `NEW` — the row image — never
//! who is writing. Postgres has a session variable that could carry a door's
//! identity; `SQLite` has none. Reading one on Postgres and not on `SQLite`
//! would make the two engines enforce different rules, so neither trigger
//! reads one, exactly as the sibling `products_audit_log` guard and the donor
//! pricing gear's guard do not.
//!
//! ## DELETE is refused unconditionally, on both engines
//!
//! The `DoD`'s own title is "append-only," and C5
//! (`design/01-foundation.md` §1.6) puts head rows under the same physical
//! append-only posture as history rows and `products_audit_log` — "not just
//! conventionally." A head row is retired from use through `lifecycle_state`
//! (`discarded`, `retired`), never removed; there is no row-image predicate
//! under which a head-row DELETE is ever legitimate, unlike the audit log's
//! owed retention arm. `REVOKE UPDATE, DELETE` is **not** issued: P-D-46
//! withdrew that arm, it names a deployment role this migration does not own,
//! and `SQLite` has no `GRANT`/`REVOKE`.
//!
//! ## Backend differences (the guard)
//!
//! Postgres raises through one `PL/pgSQL` function branching on `TG_OP`, with
//! one trigger firing `BEFORE DELETE OR UPDATE`; its `DOWN` drops the function
//! as well as the table. `SQLite` has no procedural language and
//! `RAISE(ABORT, ...)` takes a literal message, so the mirror splits the same
//! whitelist across one no-delete trigger and one `WHEN`-guarded trigger per
//! column class. Postgres compares `NEW` against `OLD` with `IS DISTINCT
//! FROM` so a `NULL`-to-`NULL` comparison behaves; `SQLite` uses `IS`/`IS NOT`,
//! its own null-safe form.
//!
//! @cpt-cf-bss-products-fr-create-product
//! @cpt-cf-bss-products-dod-entity-tables
//! @cpt-cf-bss-products-dod-name-uniqueness
//! @cpt-cf-bss-products-dod-append-only-guard

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_product (
            tenant_id         uuid        NOT NULL,
            product_id        uuid        NOT NULL,
            brand_id          uuid        NOT NULL,
            name              text        NOT NULL,
            name_normalized   text        NOT NULL,
            product_code      text,
            lifecycle_state   text        NOT NULL,
            internal_revision bigint      NOT NULL,
            published_version bigint      NOT NULL,
            region_scope      text        NOT NULL DEFAULT '',
            brand_scope       text        NOT NULL DEFAULT '',
            created_by        text        NOT NULL,
            created_at        timestamptz NOT NULL,
            updated_at        timestamptz NOT NULL,
            CONSTRAINT products_product_pkey PRIMARY KEY (product_id),
            CONSTRAINT chk_products_product_lifecycle_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_product_internal_revision CHECK (internal_revision >= 1),
            CONSTRAINT chk_products_product_published_version CHECK (published_version >= 0)
        )",
    "CREATE INDEX idx_products_product_tenant ON bss.products_product USING btree (tenant_id, product_id)",
    "CREATE UNIQUE INDEX uq_products_product_name ON bss.products_product USING btree (tenant_id, brand_id, name_normalized) WHERE lifecycle_state <> 'discarded'",
    "CREATE UNIQUE INDEX uq_products_product_code ON bss.products_product USING btree (tenant_id, product_code) WHERE product_code IS NOT NULL AND lifecycle_state <> 'discarded'",
    "CREATE OR REPLACE FUNCTION bss.products_product_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'products_product is append-only: DELETE is not permitted';
          END IF;

          IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
             OR NEW.product_id IS DISTINCT FROM OLD.product_id
             OR NEW.created_by IS DISTINCT FROM OLD.created_by
             OR NEW.created_at IS DISTINCT FROM OLD.created_at
          THEN
            RAISE EXCEPTION 'products_product: tenant_id, product_id, created_by and created_at are immutable';
          END IF;

          IF NEW.internal_revision IS DISTINCT FROM OLD.internal_revision + 1 THEN
            RAISE EXCEPTION 'products_product: internal_revision must move by exactly one on every admitted update';
          END IF;

          IF NOT (NEW.published_version IS NOT DISTINCT FROM OLD.published_version
                  OR NEW.published_version IS NOT DISTINCT FROM OLD.published_version + 1)
          THEN
            RAISE EXCEPTION 'products_product: published_version only moves by +1';
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
            RAISE EXCEPTION 'products_product: lifecycle_state % -> % is not an admitted edge', OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          IF (NEW.product_code IS DISTINCT FROM OLD.product_code OR NEW.brand_id IS DISTINCT FROM OLD.brand_id)
             AND NOT (OLD.published_version = 0 AND OLD.lifecycle_state NOT IN ('retired', 'discarded'))
          THEN
            RAISE EXCEPTION 'products_product: bucket-i columns are admitted only before first publish, on a non-terminal head';
          END IF;

          IF (NEW.name IS DISTINCT FROM OLD.name
              OR NEW.name_normalized IS DISTINCT FROM OLD.name_normalized
              OR NEW.region_scope IS DISTINCT FROM OLD.region_scope
              OR NEW.brand_scope IS DISTINCT FROM OLD.brand_scope)
             AND OLD.lifecycle_state IN ('retired', 'discarded')
          THEN
            RAISE EXCEPTION 'products_product: bucket-iii columns are admitted only while the head is non-terminal';
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_product_append_only BEFORE DELETE OR UPDATE ON bss.products_product FOR EACH ROW EXECUTE FUNCTION bss.products_product_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.products_product",
    "DROP FUNCTION IF EXISTS bss.products_product_append_only()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_product (
            tenant_id         text   NOT NULL,
            product_id        text   NOT NULL,
            brand_id          text   NOT NULL,
            name              text   NOT NULL,
            name_normalized   text   NOT NULL,
            product_code      text,
            lifecycle_state   text   NOT NULL,
            internal_revision bigint NOT NULL,
            published_version bigint NOT NULL,
            region_scope      text   NOT NULL DEFAULT '',
            brand_scope       text   NOT NULL DEFAULT '',
            created_by        text   NOT NULL,
            created_at        text   NOT NULL,
            updated_at        text   NOT NULL,
            PRIMARY KEY (product_id),
            CONSTRAINT chk_products_product_lifecycle_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_product_internal_revision CHECK (internal_revision >= 1),
            CONSTRAINT chk_products_product_published_version CHECK (published_version >= 0)
        )",
    "CREATE INDEX idx_products_product_tenant ON products_product (tenant_id, product_id)",
    "CREATE UNIQUE INDEX uq_products_product_name ON products_product (tenant_id, brand_id, name_normalized) WHERE lifecycle_state <> 'discarded'",
    "CREATE UNIQUE INDEX uq_products_product_code ON products_product (tenant_id, product_code) WHERE product_code IS NOT NULL AND lifecycle_state <> 'discarded'",
    "CREATE TRIGGER trg_products_product_no_delete BEFORE DELETE ON products_product FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'products_product is append-only: DELETE is not permitted'); END",
    "CREATE TRIGGER trg_products_product_immutable_columns BEFORE UPDATE ON products_product FOR EACH ROW WHEN
            NEW.tenant_id IS NOT OLD.tenant_id
            OR NEW.product_id IS NOT OLD.product_id
            OR NEW.created_by IS NOT OLD.created_by
            OR NEW.created_at IS NOT OLD.created_at
        BEGIN SELECT RAISE(ABORT, 'products_product: tenant_id, product_id, created_by and created_at are immutable'); END",
    "CREATE TRIGGER trg_products_product_internal_revision BEFORE UPDATE ON products_product FOR EACH ROW WHEN
            NEW.internal_revision IS NOT (OLD.internal_revision + 1)
        BEGIN SELECT RAISE(ABORT, 'products_product: internal_revision must move by exactly one on every admitted update'); END",
    "CREATE TRIGGER trg_products_product_published_version BEFORE UPDATE ON products_product FOR EACH ROW WHEN NOT (
            NEW.published_version IS OLD.published_version
            OR NEW.published_version IS (OLD.published_version + 1)
        ) BEGIN SELECT RAISE(ABORT, 'products_product: published_version only moves by +1'); END",
    "CREATE TRIGGER trg_products_product_lifecycle_edge BEFORE UPDATE ON products_product FOR EACH ROW WHEN
            NEW.lifecycle_state IS NOT OLD.lifecycle_state
            AND NOT (
                (OLD.lifecycle_state IS 'draft' AND NEW.lifecycle_state IS 'published')
                OR (OLD.lifecycle_state IS 'draft' AND NEW.lifecycle_state IS 'discarded')
                OR (OLD.lifecycle_state IS 'published' AND NEW.lifecycle_state IS 'deprecated')
                OR (OLD.lifecycle_state IS 'deprecated' AND NEW.lifecycle_state IS 'published')
                OR (OLD.lifecycle_state IS 'deprecated' AND NEW.lifecycle_state IS 'retired')
            )
        BEGIN SELECT RAISE(ABORT, 'products_product: lifecycle_state transition is not an admitted edge'); END",
    "CREATE TRIGGER trg_products_product_bucket_i BEFORE UPDATE ON products_product FOR EACH ROW WHEN (
            NEW.product_code IS NOT OLD.product_code OR NEW.brand_id IS NOT OLD.brand_id
        ) AND NOT (
            OLD.published_version = 0 AND OLD.lifecycle_state NOT IN ('retired', 'discarded')
        ) BEGIN SELECT RAISE(ABORT, 'products_product: bucket-i columns are admitted only before first publish, on a non-terminal head'); END",
    "CREATE TRIGGER trg_products_product_bucket_iii BEFORE UPDATE ON products_product FOR EACH ROW WHEN (
            NEW.name IS NOT OLD.name
            OR NEW.name_normalized IS NOT OLD.name_normalized
            OR NEW.region_scope IS NOT OLD.region_scope
            OR NEW.brand_scope IS NOT OLD.brand_scope
        ) AND OLD.lifecycle_state IN ('retired', 'discarded')
        BEGIN SELECT RAISE(ABORT, 'products_product: bucket-iii columns are admitted only while the head is non-terminal'); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_product"];

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
