//! Create `bss.products_catalog_version_entry` and
//! `bss.products_catalog_version_capture` — the manifest body, two tables
//! (`design/06-catalog-version.md` §4, **P-D-60**).
//!
//! # Two tables because one PK cannot express both keys
//!
//! The entry half is `(tenant_id, catalog_version_id, entity_kind,
//! entity_id) → published_version`, every row a reference into immutable
//! `products_entity_version`; the capture half is `(tenant_id,
//! catalog_version_id, capture_kind) → content`, a **stored canonical copy**
//! (H3: live content is copied, never referenced). A shared table would make
//! every column of both halves nullable — a row that is neither a valid
//! entry nor a valid capture — and P-D-40's predicate would judge a
//! population where capture rows can never match. The checksum covers both
//! halves, being computed over content rather than over a table.
//!
//! # `capture_kind` carries no roster CHECK — P-D-74
//!
//! The admitted set is contested (§4 lists seven kinds, the two §2 rules six
//! — `features/catalog-version.md` §7 row 49), so the DDL pins only
//! non-emptiness and the set stays the snapshot builder's to enforce once
//! that row resolves. Pinning either count here would author the answer; a
//! later pin is an in-place edit, this chain's own convention.
//!
//! # The additional index is P-D-40's, not a read of this slice's own
//!
//! The entry PK leads with `catalog_version_id` and is useless for the
//! retention DELETE's lookup, which asks *"does any entry reference this
//! entity version?"* by entity coordinates.
//! `idx_products_catalog_version_entry_ref` carries exactly that probe, and
//! the predicate installed one migration over (the edited
//! `m20260829_000007`) rides it.
//!
//! # Append-only, and the `DELETE` arm slice 10 landed
//!
//! Neither half has an admitted `UPDATE` — a manifest is immutable once
//! published — so both `UPDATE` guards are `m20260829_000007`'s unconditional
//! shape, not the head tables' whitelist.
//!
//! **`DELETE` runs under a referential predicate on the parent's release
//! stamp** (**P-D-137**, 2026-09-04): a row is deletable exactly when its
//! `products_catalog_version` carries `retention_released_at`. Until then the
//! guard refused unconditionally, *"with an interim message naming its future
//! admitter"* — slice 10's manifest retention — and that message was right:
//! strand D measured that a chain refusing every `DELETE` made P-D-118 item
//! 25's *"one catalog version at a time, whole"* a transaction that always
//! rolled back, and P-D-137 opened both arms rather than reclassifying the
//! chain as evidence.
//!
//! The predicate reads the **parent**, not this row, for the reason P-D-31
//! gives: a trigger can express properties of a row and never of the deleter,
//! and the parent's stamp is the one row-image fact the GC can make true. The
//! release is stamped first, then these rows go, then the parent — which the
//! FK requires in that order anyway, and which is exactly item 25's "whole".
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `bigint` becomes `integer`, and the
//! `bss.` qualification is dropped. Both guards split into per-op triggers on
//! `SQLite`; every CHECK, both FKs, both PKs and the P-D-40 index are
//! preserved on both sides.
//!
//! # Grandfathering holds by construction, and this table is half of it
//!
//! `dod-grandfathering` obliges that a frozen snapshot a grandfathered
//! consumer references is never mutated, **by construction rather than by a
//! check**. All three constructions ship: entity versions are append-only
//! under `m20260829_000007`'s guard, manifests are append-only under this
//! migration's own, and retirement and deprecation touch **head rows only**
//! (the head tables' lifecycle guards, whose edges never reach a version or
//! a manifest row). Eligibility policy stays plan-price's and
//! subscriptions-lifecycle's; the immutability is this gear's, and it is
//! these guards.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-version-entry-table:p1
//! @cpt-dod:cpt-cf-bss-products-dod-grandfathering:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_catalog_version_entry (
            tenant_id           uuid   NOT NULL,
            catalog_version_id  bigint NOT NULL,
            entity_kind         text   NOT NULL,
            entity_id           uuid   NOT NULL,
            published_version   bigint NOT NULL,
            CONSTRAINT products_catalog_version_entry_pkey PRIMARY KEY (tenant_id, catalog_version_id, entity_kind, entity_id),
            CONSTRAINT chk_products_catalog_version_entry_kind CHECK (entity_kind IN ('product', 'sku')),
            CONSTRAINT chk_products_catalog_version_entry_version CHECK (published_version >= 1),
            CONSTRAINT fk_products_catalog_version_entry_version FOREIGN KEY (tenant_id, catalog_version_id)
                REFERENCES bss.products_catalog_version (tenant_id, catalog_version_id)
        )",
    "CREATE INDEX idx_products_catalog_version_entry_ref ON bss.products_catalog_version_entry USING btree (tenant_id, entity_kind, entity_id, published_version)",
    "CREATE TABLE bss.products_catalog_version_capture (
            tenant_id           uuid   NOT NULL,
            catalog_version_id  bigint NOT NULL,
            capture_kind        text   NOT NULL,
            content             text   NOT NULL,
            CONSTRAINT products_catalog_version_capture_pkey PRIMARY KEY (tenant_id, catalog_version_id, capture_kind),
            CONSTRAINT chk_products_catalog_version_capture_kind CHECK (capture_kind <> ''),
            CONSTRAINT fk_products_catalog_version_capture_version FOREIGN KEY (tenant_id, catalog_version_id)
                REFERENCES bss.products_catalog_version (tenant_id, catalog_version_id)
        )",
    "CREATE OR REPLACE FUNCTION bss.products_catalog_version_entry_frozen() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'UPDATE' THEN
            RAISE EXCEPTION 'products_catalog_version_entry is frozen: UPDATE is not permitted';
          END IF;
          IF NOT EXISTS (SELECT 1 FROM bss.products_catalog_version v
                         WHERE v.tenant_id = OLD.tenant_id
                           AND v.catalog_version_id = OLD.catalog_version_id
                           AND v.retention_released_at IS NOT NULL) THEN
            RAISE EXCEPTION 'products_catalog_version_entry: DELETE is admitted only when its catalog version carries retention_released_at (P-D-137)';
          END IF;
          RETURN OLD;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_catalog_version_entry_frozen BEFORE DELETE OR UPDATE ON bss.products_catalog_version_entry FOR EACH ROW EXECUTE FUNCTION bss.products_catalog_version_entry_frozen()",
    "CREATE OR REPLACE FUNCTION bss.products_catalog_version_capture_frozen() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'UPDATE' THEN
            RAISE EXCEPTION 'products_catalog_version_capture is frozen: UPDATE is not permitted';
          END IF;
          IF NOT EXISTS (SELECT 1 FROM bss.products_catalog_version v
                         WHERE v.tenant_id = OLD.tenant_id
                           AND v.catalog_version_id = OLD.catalog_version_id
                           AND v.retention_released_at IS NOT NULL) THEN
            RAISE EXCEPTION 'products_catalog_version_capture: DELETE is admitted only when its catalog version carries retention_released_at (P-D-137)';
          END IF;
          RETURN OLD;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_catalog_version_capture_frozen BEFORE DELETE OR UPDATE ON bss.products_catalog_version_capture FOR EACH ROW EXECUTE FUNCTION bss.products_catalog_version_capture_frozen()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_catalog_version_capture_frozen ON bss.products_catalog_version_capture",
    "DROP FUNCTION IF EXISTS bss.products_catalog_version_capture_frozen",
    "DROP TRIGGER IF EXISTS trg_products_catalog_version_entry_frozen ON bss.products_catalog_version_entry",
    "DROP FUNCTION IF EXISTS bss.products_catalog_version_entry_frozen",
    "DROP TABLE IF EXISTS bss.products_catalog_version_capture",
    "DROP TABLE IF EXISTS bss.products_catalog_version_entry",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_catalog_version_entry (
            tenant_id           text    NOT NULL,
            catalog_version_id  integer NOT NULL,
            entity_kind         text    NOT NULL,
            entity_id           text    NOT NULL,
            published_version   integer NOT NULL,
            PRIMARY KEY (tenant_id, catalog_version_id, entity_kind, entity_id),
            CONSTRAINT chk_products_catalog_version_entry_kind CHECK (entity_kind IN ('product', 'sku')),
            CONSTRAINT chk_products_catalog_version_entry_version CHECK (published_version >= 1),
            CONSTRAINT fk_products_catalog_version_entry_version FOREIGN KEY (tenant_id, catalog_version_id)
                REFERENCES products_catalog_version (tenant_id, catalog_version_id)
        )",
    "CREATE INDEX idx_products_catalog_version_entry_ref ON products_catalog_version_entry (tenant_id, entity_kind, entity_id, published_version)",
    "CREATE TABLE products_catalog_version_capture (
            tenant_id           text    NOT NULL,
            catalog_version_id  integer NOT NULL,
            capture_kind        text    NOT NULL,
            content             text    NOT NULL,
            PRIMARY KEY (tenant_id, catalog_version_id, capture_kind),
            CONSTRAINT chk_products_catalog_version_capture_kind CHECK (capture_kind <> ''),
            CONSTRAINT fk_products_catalog_version_capture_version FOREIGN KEY (tenant_id, catalog_version_id)
                REFERENCES products_catalog_version (tenant_id, catalog_version_id)
        )",
    "CREATE TRIGGER trg_products_catalog_version_entry_no_update BEFORE UPDATE ON products_catalog_version_entry FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'products_catalog_version_entry is frozen: UPDATE is not permitted'); END",
    "CREATE TRIGGER trg_products_catalog_version_entry_no_delete BEFORE DELETE ON products_catalog_version_entry FOR EACH ROW WHEN NOT EXISTS (SELECT 1 FROM products_catalog_version v WHERE v.tenant_id = OLD.tenant_id AND v.catalog_version_id = OLD.catalog_version_id AND v.retention_released_at IS NOT NULL) BEGIN SELECT RAISE(ABORT, 'products_catalog_version_entry: DELETE is admitted only when its catalog version carries retention_released_at (P-D-137)'); END",
    "CREATE TRIGGER trg_products_catalog_version_capture_no_update BEFORE UPDATE ON products_catalog_version_capture FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'products_catalog_version_capture is frozen: UPDATE is not permitted'); END",
    "CREATE TRIGGER trg_products_catalog_version_capture_no_delete BEFORE DELETE ON products_catalog_version_capture FOR EACH ROW WHEN NOT EXISTS (SELECT 1 FROM products_catalog_version v WHERE v.tenant_id = OLD.tenant_id AND v.catalog_version_id = OLD.catalog_version_id AND v.retention_released_at IS NOT NULL) BEGIN SELECT RAISE(ABORT, 'products_catalog_version_capture: DELETE is admitted only when its catalog version carries retention_released_at (P-D-137)'); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_catalog_version_capture_no_delete",
    "DROP TRIGGER IF EXISTS trg_products_catalog_version_capture_no_update",
    "DROP TRIGGER IF EXISTS trg_products_catalog_version_entry_no_delete",
    "DROP TRIGGER IF EXISTS trg_products_catalog_version_entry_no_update",
    "DROP TABLE IF EXISTS products_catalog_version_capture",
    "DROP TABLE IF EXISTS products_catalog_version_entry",
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
