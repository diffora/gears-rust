//! Create `bss.products_recognized_set` — the generic set table behind all
//! four recognized sets (`design/03-sku-classification.md` §3.1 and §4;
//! **P-D-47**, **P-D-92**).
//!
//! # One table, four sets, and the roster is deliberately not pinned
//!
//! The `DoD` and §4 both name the domain — `metering_unit`, `tax_category`,
//! `gl_code`, `plan_tier` — and **neither demands a `CHECK` over it**, which
//! is what **P-D-92** turns on: §7 row 5 asks whether `tax_category_ref` and
//! `gl_code_ref` belong to this registry at all, and its answer *"may delete
//! ... its two `set_kind` values"*. A `CHECK` enumerating the four would be
//! that row's answer written by a migration, so `set_kind` is pinned
//! **non-empty only** and the admitted set is the membership door's to
//! enforce. This is the third application of that form in this chain, after
//! `capture_kind` (P-D-74) and `entity_kind`/`value_type` — a later pin is an
//! in-place edit rather than a redesign.
//!
//! # `removed` is a tombstone, so no `DELETE` is ever admitted
//!
//! **P-D-47**: the row survives outside the set, which is what lets a value
//! on a terminal head keep resolving and keeps `removed → active` a
//! re-listing of *the same identity* rather than a new member. A `DELETE`
//! would break both, so the guard refuses it unconditionally.
//!
//! # The whitelist admits exactly two columns, and this one IS a whitelist
//!
//! Unlike the head tables — whose guards name the **immutable** columns and
//! admit the rest — §4 states this one from the other side: *"trigger
//! whitelist admits `state` and `display_label` only"*. So `member_code`,
//! `set_kind`, `tenant_id` and `seeded_by` are refused on every `UPDATE`,
//! and a column added later is refused until someone adds it here
//! deliberately. The asymmetry is the design's, and it fits: a head row is
//! an authoring surface, while a set member is a governed identity whose
//! only mutable facts are its lifecycle and its label.
//!
//! # `display_label` is `plan_tier`'s and ignored elsewhere
//!
//! Not a per-kind table split, and not a `CHECK` either: §4 says *"used by
//! `plan_tier` (and ignored elsewhere)"*, and a constraint forbidding it on
//! the other three would refuse a harmless write the design merely ignores.
//! `dod-plantier-governance` is where the label's own rules live — tier
//! identity is the stable code with no update path, the label carried
//! separately.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `timestamptz` becomes `text`, and the
//! `bss.` qualification is dropped. Every CHECK, the key and both guard arms
//! are preserved on both sides; `SQLite` splits the guard into per-op
//! triggers.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-recognized-set-table:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_recognized_set (
            tenant_id     uuid        NOT NULL,
            set_kind      text        NOT NULL,
            member_code   text        NOT NULL,
            display_label text,
            state         text        NOT NULL,
            seeded_by     text,
            created_at    timestamptz NOT NULL,
            updated_at    timestamptz NOT NULL,
            CONSTRAINT products_recognized_set_pkey PRIMARY KEY (tenant_id, set_kind, member_code),
            CONSTRAINT chk_products_recognized_set_kind CHECK (set_kind <> ''),
            CONSTRAINT chk_products_recognized_set_member CHECK (member_code <> ''),
            CONSTRAINT chk_products_recognized_set_state CHECK (state IN ('active', 'deprecated', 'removed'))
        )",
    "CREATE INDEX idx_products_recognized_set_state ON bss.products_recognized_set USING btree (tenant_id, set_kind, state)",
    "CREATE OR REPLACE FUNCTION bss.products_recognized_set_guard() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'products_recognized_set: a removal is the removed state, never a DELETE';
          END IF;
          IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
             OR NEW.set_kind IS DISTINCT FROM OLD.set_kind
             OR NEW.member_code IS DISTINCT FROM OLD.member_code
             OR NEW.seeded_by IS DISTINCT FROM OLD.seeded_by
             OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
            RAISE EXCEPTION 'products_recognized_set: only state and display_label are writable';
          END IF;
          RETURN NEW;
        END;
        $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_recognized_set_guard
        BEFORE DELETE OR UPDATE ON bss.products_recognized_set
        FOR EACH ROW EXECUTE FUNCTION bss.products_recognized_set_guard()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_recognized_set_guard ON bss.products_recognized_set",
    "DROP FUNCTION IF EXISTS bss.products_recognized_set_guard",
    "DROP TABLE IF EXISTS bss.products_recognized_set",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_recognized_set (
            tenant_id     text NOT NULL,
            set_kind      text NOT NULL,
            member_code   text NOT NULL,
            display_label text,
            state         text NOT NULL,
            seeded_by     text,
            created_at    text NOT NULL,
            updated_at    text NOT NULL,
            PRIMARY KEY (tenant_id, set_kind, member_code),
            CONSTRAINT chk_products_recognized_set_kind CHECK (set_kind <> ''),
            CONSTRAINT chk_products_recognized_set_member CHECK (member_code <> ''),
            CONSTRAINT chk_products_recognized_set_state CHECK (state IN ('active', 'deprecated', 'removed'))
        )",
    "CREATE INDEX idx_products_recognized_set_state ON products_recognized_set (tenant_id, set_kind, state)",
    "CREATE TRIGGER trg_products_recognized_set_no_delete
        BEFORE DELETE ON products_recognized_set
        BEGIN
          SELECT RAISE(ABORT, 'products_recognized_set: a removal is the removed state, never a DELETE');
        END",
    "CREATE TRIGGER trg_products_recognized_set_guard
        BEFORE UPDATE ON products_recognized_set
        WHEN NEW.tenant_id IS NOT OLD.tenant_id
             OR NEW.set_kind IS NOT OLD.set_kind
             OR NEW.member_code IS NOT OLD.member_code
             OR NEW.seeded_by IS NOT OLD.seeded_by
             OR NEW.created_at IS NOT OLD.created_at
        BEGIN
          SELECT RAISE(ABORT, 'products_recognized_set: only state and display_label are writable');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_recognized_set_guard",
    "DROP TRIGGER IF EXISTS trg_products_recognized_set_no_delete",
    "DROP TABLE IF EXISTS products_recognized_set",
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
