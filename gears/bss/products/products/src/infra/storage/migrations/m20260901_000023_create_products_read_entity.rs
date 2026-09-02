//! Create `bss.products_read_entity` — the browse projection's denormalized
//! serving rows (`design/08-read-models.md` §3.1 `inst-ps-shape`, §4;
//! **P-D-39**, **P-D-70**).
//!
//! # This table has NO guard, and the exemption is the point
//!
//! Every other table in this chain carries an append-only or whitelist
//! trigger. This one carries none, deliberately: §4 calls the family
//! *"rebuildable state, not records"* — *"no append-only guards, no audit
//! rows of their own (the audited truth lives upstream)"* — and records the
//! exemption from the published/history append-only guard as **the point**
//! rather than an oversight (L2, and 01 C5 carries the same exemption). A
//! projector rebuild replaces content wholesale, so a guard here would
//! refuse the operation the table exists to admit. **Do not add one**, and
//! read §4 before "fixing" its absence.
//!
//! Rebuilds go *"into a new projection cut over per `inst-rp-bootstrap` —
//! never dropped in place"*, so the absence of a guard is not a licence to
//! truncate the live rows either.
//!
//! # The columns are `inst-ps-shape`'s list, in its own order
//!
//! Identity, then state and flags, then the head-read fields C6 admits, then
//! the scope operands, then display, then paths and the version.
//!
//! `deprecated`, `composition_pending` and `sellable` are the three flags
//! `inst-ps-shape` names. `deprecation_provenance` and `replaced_by_sku_id`
//! are C6's head-read fields — carried here so a browse response can render
//! a deprecated row's successor without touching the head.
//!
//! **`region_scope` and `brand_scope` are the query-build operands**, and
//! their empty value means **unrestricted** (**P-D-39**), not "matches
//! nothing": a scope predicate matches a row whose set is empty *or*
//! contains the caller's claim. They are `text` for the same reason the head
//! tables' are — the containment rule is over restrictions, not raw sets, and
//! `domain::containment` owns it.
//!
//! `display_attributes` is the resolved per-locale rendering, materialized
//! for the tenant's active locales, and `category_paths` the assigned
//! categories' full paths — both canonical renderings, because a browse
//! response compares them and a re-serialization would make two equal
//! projections unequal.
//!
//! # `lifecycle_state`'s roster is the full five, and that is not the same
//! # as what a surface serves
//!
//! The `CHECK` admits all five states, including `draft` and `discarded`
//! which **no** surface serves. That is deliberate: the projector's job is to
//! reflect what happened, and `domain::read_model`'s `VisibilityFilter`
//! decides what a caller sees **at query build**. A `CHECK` narrowed to the
//! served three would make the projector unable to record a discard, and the
//! row would then linger as its last served state — served forever.
//!
//! # The index is the tenant partition
//!
//! NFR #1/#2's unit is the tenant partition (`inst-ps-shape`), so the primary
//! key leads with `tenant_id` and the browse index carries
//! `(tenant_id, lifecycle_state, entity_kind)` — the three columns every
//! `VisibilityFilter` predicate names, in the order it names them.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `bigint` becomes `integer`, `boolean`
//! becomes `integer`, `timestamptz` becomes `text`, and the `bss.`
//! qualification is dropped. Both `CHECK`s, the key and both indexes are
//! preserved on both sides.
//!
//! @cpt-cf-bss-products-dod-projection-table

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_read_entity (
            tenant_id          uuid        NOT NULL,
            entity_kind        text        NOT NULL,
            entity_id          uuid        NOT NULL,
            entity_code        text,
            name               text        NOT NULL,
            lifecycle_state    text        NOT NULL,
            deprecated         boolean     NOT NULL DEFAULT false,
            composition_pending boolean    NOT NULL DEFAULT false,
            sellable           boolean,
            deprecation_provenance text,
            replaced_by_sku_id uuid,
            region_scope       text        NOT NULL DEFAULT '',
            brand_scope        text        NOT NULL DEFAULT '',
            sku_type           text,
            plan_tier_label    text,
            metering_unit      text,
            display_attributes text,
            category_paths     text,
            published_version  bigint      NOT NULL,
            projected_at       timestamptz NOT NULL,
            CONSTRAINT products_read_entity_pkey PRIMARY KEY (tenant_id, entity_kind, entity_id),
            CONSTRAINT chk_products_read_entity_kind CHECK (entity_kind IN ('product', 'sku')),
            CONSTRAINT chk_products_read_entity_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_read_entity_version CHECK (published_version >= 0)
        )",
    "CREATE INDEX idx_products_read_entity_browse ON bss.products_read_entity USING btree (tenant_id, lifecycle_state, entity_kind)",
    "CREATE INDEX idx_products_read_entity_code ON bss.products_read_entity USING btree (tenant_id, entity_code)",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.products_read_entity"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_read_entity (
            tenant_id          text    NOT NULL,
            entity_kind        text    NOT NULL,
            entity_id          text    NOT NULL,
            entity_code        text,
            name               text    NOT NULL,
            lifecycle_state    text    NOT NULL,
            deprecated         integer NOT NULL DEFAULT 0,
            composition_pending integer NOT NULL DEFAULT 0,
            sellable           integer,
            deprecation_provenance text,
            replaced_by_sku_id text,
            region_scope       text    NOT NULL DEFAULT '',
            brand_scope        text    NOT NULL DEFAULT '',
            sku_type           text,
            plan_tier_label    text,
            metering_unit      text,
            display_attributes text,
            category_paths     text,
            published_version  integer NOT NULL,
            projected_at       text    NOT NULL,
            PRIMARY KEY (tenant_id, entity_kind, entity_id),
            CONSTRAINT chk_products_read_entity_kind CHECK (entity_kind IN ('product', 'sku')),
            CONSTRAINT chk_products_read_entity_state CHECK (lifecycle_state IN ('draft', 'published', 'deprecated', 'retired', 'discarded')),
            CONSTRAINT chk_products_read_entity_version CHECK (published_version >= 0)
        )",
    "CREATE INDEX idx_products_read_entity_browse ON products_read_entity (tenant_id, lifecycle_state, entity_kind)",
    "CREATE INDEX idx_products_read_entity_code ON products_read_entity (tenant_id, entity_code)",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_read_entity"];

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
