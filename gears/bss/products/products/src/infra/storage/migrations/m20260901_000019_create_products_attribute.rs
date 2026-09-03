//! Create `bss.products_attribute_definition` and
//! `bss.products_attribute_value` — the governed definition roster and the
//! value plane (`design/02-taxonomy-attributes.md` §4.1; **P-D-47**,
//! **P-D-88** arm 2).
//!
//! # A removal is a state flip, and the guard makes that physical
//!
//! `state ∈ {active, deprecated, removed}` and **`removed` is reachable only
//! as a flip** (**P-D-47**, the rule 03 §3.1 states for every
//! `RecognizedSet`): the row survives as a tombstone outside the set, so a
//! value on a terminal head keeps resolving and no `products_attribute_value`
//! row is ever orphaned, and `removed → active` re-lists the same identity
//! through the same `GovernedLiveOp` — the identity never having changed. The
//! `DoD` requires that *"no migration or door may delete a row"*, so the
//! guard refuses `DELETE` unconditionally rather than trusting the doors.
//!
//! # `value_type` carries no roster CHECK, and that is P-D-74's shape
//!
//! **No document enumerates the admitted value types.** §4.2's seeds imply
//! three shapes (a localized string, a URI string, a localized string list)
//! but no closed set is declared anywhere in the set. The chain's own
//! convention for exactly this situation is `products_catalog_version_capture`
//! (**P-D-74**): the DDL pins only non-emptiness and the roster stays the
//! door's to enforce once it is decided — that entry pins no `capture_kind`
//! roster for exactly this reason, leaving the set to the snapshot builder.
//! A later pin is an in-place edit. The question is
//! registered in `design/02` §6 rather than answered here.
//!
//! # The coordinates are `NOT NULL` with a stated absence — P-D-88 arm 2
//!
//! §4.1 writes the locale coordinates `(locale?, region?, brand?)`, and both
//! engines treat NULLs as distinct — so a nullable tuple would leave the
//! **`global`** coordinate, the one `inst-av-default-locale` makes mandatory,
//! unconstrained by the very `UNIQUE` declared to constrain it. All three ship
//! `NOT NULL` with the empty string as the **stated absence value** (P-D-39's
//! convention, which the open item itself named), so the `UNIQUE` is total and
//! `global` is spelled `('', '', '')`.
//!
//! **That is the spelling only.** What `global` means to the resolver, which
//! of the eight presence combinations a door admits, and where a brand-scoped
//! default lives are three separate §6 open items, untouched by this table.
//!
//! # No FK to the owning entity, and the reason is the polymorphism
//!
//! `entity_kind ∈ {product, sku, category}` — the third is H2's fix: category
//! display values are **live-entity content**, so for those rows this table
//! **is** the live state with no freeze-copy, while Product and SKU rows hold
//! the current head state only and history lives in the frozen version rows.
//! Three kinds live in three tables, so no single FK can cover the
//! coordinate; the owning door proves existence. The **definition** FK is
//! real, because there is one definition table.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `boolean` becomes `integer`,
//! `timestamptz` becomes `text`, and the `bss.` qualification is dropped.
//! Every CHECK, both keys, the definition FK and the no-delete guard are
//! preserved on both sides.
//!
//! # Neither `DoD` is ticked, and both reasons are live §7 rows
//!
//! `dod-attribute-definition-table` waits on **row 13**: *"Where does a
//! definition's display label live? Label edits are a named non-material op
//! and the definition roster carries no label column, so the op has no
//! target."* This roster carries none either — adding one would author the
//! answer to the row that asks for it.
//!
//! `dod-attribute-value-table`'s row 20 is **answered, and the guard is now a
//! set.** **P-D-108** arm 3 closed the roster at four — `product`, `sku`,
//! `category` and `attribute_definition`, the last being where a definition's
//! display label lives under arm 2 — so
//! `chk_products_attribute_value_entity_kind` is tightened here **in place**,
//! from `entity_kind <> ''` to that enumeration, on both engines.
//!
//! The old form was an **open complement**: it named the one value it refused
//! and admitted every other string, which is the shape that cannot be
//! probed. A closed set can be, and the `CorruptRow` case it makes testable —
//! a row whose `entity_kind` the reader cannot parse — is exactly what the
//! open guard denied, because no such row could be written to read back.
//!
//! The migration is edited rather than repaired by a successor, which is this
//! gear's rule.

//! @cpt-dod:cpt-cf-bss-products-dod-attribute-definition-table:p1
//! @cpt-dod:cpt-cf-bss-products-dod-attribute-value-table:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_attribute_definition (
            tenant_id     uuid        NOT NULL,
            definition_id uuid        NOT NULL,
            key           text        NOT NULL,
            value_type    text        NOT NULL,
            localized     boolean     NOT NULL DEFAULT false,
            region_scope  text        NOT NULL DEFAULT '',
            brand_scope   text        NOT NULL DEFAULT '',
            state         text        NOT NULL,
            seeded_by     text,
            created_at    timestamptz NOT NULL,
            updated_at    timestamptz NOT NULL,
            CONSTRAINT products_attribute_definition_pkey PRIMARY KEY (tenant_id, definition_id),
            CONSTRAINT uq_products_attribute_definition_key UNIQUE (tenant_id, key),
            CONSTRAINT chk_products_attribute_definition_key CHECK (key <> ''),
            CONSTRAINT chk_products_attribute_definition_value_type CHECK (value_type <> ''),
            CONSTRAINT chk_products_attribute_definition_state CHECK (state IN ('active', 'deprecated', 'removed'))
        )",
    "CREATE OR REPLACE FUNCTION bss.products_attribute_definition_no_delete() RETURNS trigger AS $$
        BEGIN
          RAISE EXCEPTION 'products_attribute_definition: a removal is the removed state, never a DELETE';
        END;
        $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_attribute_definition_no_delete
        BEFORE DELETE ON bss.products_attribute_definition
        FOR EACH ROW EXECUTE FUNCTION bss.products_attribute_definition_no_delete()",
    "CREATE TABLE bss.products_attribute_value (
            tenant_id     uuid        NOT NULL,
            entity_kind   text        NOT NULL,
            entity_id     uuid        NOT NULL,
            definition_id uuid        NOT NULL,
            locale        text        NOT NULL DEFAULT '',
            region        text        NOT NULL DEFAULT '',
            brand         text        NOT NULL DEFAULT '',
            value         text        NOT NULL,
            updated_at    timestamptz NOT NULL,
            CONSTRAINT products_attribute_value_pkey PRIMARY KEY (tenant_id, entity_kind, entity_id, definition_id, locale, region, brand),
            CONSTRAINT chk_products_attribute_value_entity_kind CHECK (entity_kind IN ('product', 'sku', 'category', 'attribute_definition')),
            CONSTRAINT fk_products_attribute_value_definition FOREIGN KEY (tenant_id, definition_id)
                REFERENCES bss.products_attribute_definition (tenant_id, definition_id)
        )",
    "CREATE INDEX idx_products_attribute_value_definition ON bss.products_attribute_value USING btree (tenant_id, definition_id)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.products_attribute_value",
    "DROP TRIGGER IF EXISTS trg_products_attribute_definition_no_delete ON bss.products_attribute_definition",
    "DROP FUNCTION IF EXISTS bss.products_attribute_definition_no_delete",
    "DROP TABLE IF EXISTS bss.products_attribute_definition",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_attribute_definition (
            tenant_id     text    NOT NULL,
            definition_id text    NOT NULL,
            key           text    NOT NULL,
            value_type    text    NOT NULL,
            localized     integer NOT NULL DEFAULT 0,
            region_scope  text    NOT NULL DEFAULT '',
            brand_scope   text    NOT NULL DEFAULT '',
            state         text    NOT NULL,
            seeded_by     text,
            created_at    text    NOT NULL,
            updated_at    text    NOT NULL,
            PRIMARY KEY (tenant_id, definition_id),
            CONSTRAINT uq_products_attribute_definition_key UNIQUE (tenant_id, key),
            CONSTRAINT chk_products_attribute_definition_key CHECK (key <> ''),
            CONSTRAINT chk_products_attribute_definition_value_type CHECK (value_type <> ''),
            CONSTRAINT chk_products_attribute_definition_state CHECK (state IN ('active', 'deprecated', 'removed'))
        )",
    "CREATE TRIGGER trg_products_attribute_definition_no_delete
        BEFORE DELETE ON products_attribute_definition
        BEGIN
          SELECT RAISE(ABORT, 'products_attribute_definition: a removal is the removed state, never a DELETE');
        END",
    "CREATE TABLE products_attribute_value (
            tenant_id     text NOT NULL,
            entity_kind   text NOT NULL,
            entity_id     text NOT NULL,
            definition_id text NOT NULL,
            locale        text NOT NULL DEFAULT '',
            region        text NOT NULL DEFAULT '',
            brand         text NOT NULL DEFAULT '',
            value         text NOT NULL,
            updated_at    text NOT NULL,
            PRIMARY KEY (tenant_id, entity_kind, entity_id, definition_id, locale, region, brand),
            CONSTRAINT chk_products_attribute_value_entity_kind CHECK (entity_kind IN ('product', 'sku', 'category', 'attribute_definition')),
            CONSTRAINT fk_products_attribute_value_definition FOREIGN KEY (tenant_id, definition_id)
                REFERENCES products_attribute_definition (tenant_id, definition_id)
        )",
    "CREATE INDEX idx_products_attribute_value_definition ON products_attribute_value (tenant_id, definition_id)",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS products_attribute_value",
    "DROP TRIGGER IF EXISTS trg_products_attribute_definition_no_delete",
    "DROP TABLE IF EXISTS products_attribute_definition",
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
