//! Create `bss.products_category` and `bss.products_product_category` — the
//! governed tree and the assignment table
//! (`design/02-taxonomy-attributes.md` §4.1; **P-D-50**, **P-D-88**).
//!
//! # The tree's uniqueness is two indexes, and the second is P-D-88's
//!
//! §4.1 declares `UNIQUE (tenant_id, parent_id, name_normalized)`, and both
//! engines treat NULLs as distinct — so that index alone admits two **root**
//! categories with one name. The root half is its own partial
//! `UNIQUE (tenant_id, name_normalized) WHERE parent_id IS NULL`
//! (**P-D-88** arm 1): a sentinel parent cannot satisfy the
//! **self-referencing FK** without minting a fake category row per tenant,
//! and `NULLS NOT DISTINCT` is Postgres-15 syntax with no `SQLite`
//! equivalent, so partial indexes are the one candidate that holds
//! identically on both engines.
//!
//! # `mutation_seq` counts acts, not row writes
//!
//! The category live-value door's `If-Match` operand (**P-D-50**): the door
//! spends a `GovernedLiveOp`, and an approval subject built from an act
//! identity must render the same subject on the approved retry — a counter
//! advanced by non-operator writes would break that. It advances when a door
//! commits an act on the row, and for nothing else.
//!
//! # `name_normalized` is the Foundation's operand
//!
//! NFKC, then full casefold, then trim and collapse — computed
//! **application-side** by `domain::name::normalize`, the same function the
//! head tables' reservations ride, so one name normalizes identically at
//! every site.
//!
//! # Deletion is physical only through the retire guard
//!
//! `inst-tx-retire-guard`: retired + empty + unreferenced. The **children**
//! half is this FK — a parent with children cannot be deleted because the
//! child rows reference it — while "retired" and "unreferenced" are the
//! door's to prove (the assignment FK below covers product references).
//! Everything else is state flips, audited. No freeze guard: the tree is
//! working state, its discipline the door and the CHECKs.
//!
//! # The assignment table is the single source of truth
//!
//! 01 §4.1 carries no inline category columns, and this table is keyed
//! exactly as the `DoD` states: `(tenant_id, product_id, category_id, role)`,
//! with `UNIQUE (tenant_id, product_id, category_id)` so one product cannot
//! hold one category in two roles, and the partial
//! `UNIQUE (tenant_id, product_id) WHERE role = 'primary'` making
//! **at-most-one-primary an index rather than a convention**. The *required*
//! half — a published product must HAVE a primary — is deliberately not
//! here: it is `inst-tx-primary-at-publish`'s validator, because a draft
//! carrying none is legal (AC #5's "optional at draft").
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `bigint` becomes `integer`,
//! `timestamptz` becomes `text`, and the `bss.` qualification is dropped.
//! Both partial indexes, every CHECK, both FKs and both keys are preserved
//! on both sides.
//!
//! **`dod-category-assignment-table` carries no marker here, deliberately.**
//! The table ships complete, but §7 row 21 is live and is about exactly this
//! FK: *"no referential action is stated"*. This migration's FK takes the
//! default (no action), which refuses a category's deletion while ANY link
//! row exists — **including rows held by discarded and retired Products**,
//! which `inst-tx-retire-guard`'s *"unreferenced"* test does not count,
//! since it reads the Product's lifecycle state and never the link row. So
//! the DDL as written makes the guard's stated semantics unreachable in one
//! direction, and choosing between a cascade, a restrict, and the guard
//! clearing link rows in its own transaction is that row's call, co-owned
//! with the schema owner. The tick waits for it.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-category-table:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_category (
            tenant_id       uuid        NOT NULL,
            category_id     uuid        NOT NULL,
            parent_id       uuid,
            name            text        NOT NULL,
            name_normalized text        NOT NULL,
            state           text        NOT NULL,
            mutation_seq    bigint      NOT NULL DEFAULT 0,
            created_at      timestamptz NOT NULL,
            updated_at      timestamptz NOT NULL,
            CONSTRAINT products_category_pkey PRIMARY KEY (tenant_id, category_id),
            CONSTRAINT chk_products_category_name CHECK (name <> ''),
            CONSTRAINT chk_products_category_name_normalized CHECK (name_normalized <> ''),
            CONSTRAINT chk_products_category_state CHECK (state IN ('active', 'retired')),
            CONSTRAINT chk_products_category_mutation_seq CHECK (mutation_seq >= 0),
            CONSTRAINT chk_products_category_not_own_parent CHECK (parent_id IS NULL OR parent_id <> category_id),
            CONSTRAINT fk_products_category_parent FOREIGN KEY (tenant_id, parent_id)
                REFERENCES bss.products_category (tenant_id, category_id)
        )",
    "CREATE UNIQUE INDEX uq_products_category_name_in_parent ON bss.products_category USING btree (tenant_id, parent_id, name_normalized)",
    "CREATE UNIQUE INDEX uq_products_category_root_name ON bss.products_category USING btree (tenant_id, name_normalized) WHERE parent_id IS NULL",
    "CREATE TABLE bss.products_product_category (
            tenant_id   uuid NOT NULL,
            product_id  uuid NOT NULL,
            category_id uuid NOT NULL,
            role        text NOT NULL,
            assigned_at timestamptz NOT NULL,
            CONSTRAINT products_product_category_pkey PRIMARY KEY (tenant_id, product_id, category_id, role),
            CONSTRAINT chk_products_product_category_role CHECK (role IN ('primary', 'secondary')),
            CONSTRAINT uq_products_product_category UNIQUE (tenant_id, product_id, category_id),
            CONSTRAINT fk_products_product_category_product FOREIGN KEY (product_id)
                REFERENCES bss.products_product (product_id),
            CONSTRAINT fk_products_product_category_category FOREIGN KEY (tenant_id, category_id)
                REFERENCES bss.products_category (tenant_id, category_id)
        )",
    "CREATE UNIQUE INDEX uq_products_product_category_primary ON bss.products_product_category USING btree (tenant_id, product_id) WHERE role = 'primary'",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.products_product_category",
    "DROP TABLE IF EXISTS bss.products_category",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_category (
            tenant_id       text    NOT NULL,
            category_id     text    NOT NULL,
            parent_id       text,
            name            text    NOT NULL,
            name_normalized text    NOT NULL,
            state           text    NOT NULL,
            mutation_seq    integer NOT NULL DEFAULT 0,
            created_at      text    NOT NULL,
            updated_at      text    NOT NULL,
            PRIMARY KEY (tenant_id, category_id),
            CONSTRAINT chk_products_category_name CHECK (name <> ''),
            CONSTRAINT chk_products_category_name_normalized CHECK (name_normalized <> ''),
            CONSTRAINT chk_products_category_state CHECK (state IN ('active', 'retired')),
            CONSTRAINT chk_products_category_mutation_seq CHECK (mutation_seq >= 0),
            CONSTRAINT chk_products_category_not_own_parent CHECK (parent_id IS NULL OR parent_id <> category_id),
            CONSTRAINT fk_products_category_parent FOREIGN KEY (tenant_id, parent_id)
                REFERENCES products_category (tenant_id, category_id)
        )",
    "CREATE UNIQUE INDEX uq_products_category_name_in_parent ON products_category (tenant_id, parent_id, name_normalized)",
    "CREATE UNIQUE INDEX uq_products_category_root_name ON products_category (tenant_id, name_normalized) WHERE parent_id IS NULL",
    "CREATE TABLE products_product_category (
            tenant_id   text NOT NULL,
            product_id  text NOT NULL,
            category_id text NOT NULL,
            role        text NOT NULL,
            assigned_at text NOT NULL,
            PRIMARY KEY (tenant_id, product_id, category_id, role),
            CONSTRAINT chk_products_product_category_role CHECK (role IN ('primary', 'secondary')),
            CONSTRAINT uq_products_product_category UNIQUE (tenant_id, product_id, category_id),
            CONSTRAINT fk_products_product_category_product FOREIGN KEY (product_id)
                REFERENCES products_product (product_id),
            CONSTRAINT fk_products_product_category_category FOREIGN KEY (tenant_id, category_id)
                REFERENCES products_category (tenant_id, category_id)
        )",
    "CREATE UNIQUE INDEX uq_products_product_category_primary ON products_product_category (tenant_id, product_id) WHERE role = 'primary'",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS products_product_category",
    "DROP TABLE IF EXISTS products_category",
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
