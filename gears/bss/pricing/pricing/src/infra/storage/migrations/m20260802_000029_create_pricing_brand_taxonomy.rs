//! Create `bss.pricing_brand_taxonomy` — the brand value universe
//! (`design/04-currency-tax.md` §6).
//!
//! The second of the four scope-value taxonomies, and the one the design set
//! already referenced from Slice 9: `inst-plv-scope`'s first clause sends
//! `brand` to *"the Slice 4 brand taxonomy"*. Until this migration that sentence
//! named nothing. `m20260802_000028`'s module doc carries the whole argument for
//! why the four are built here; this file states only what is its own.
//!
//! # It carries no `tax_*` columns, and the absence is the point
//!
//! §6 declares the four tables together and then narrows: *"the `tax_*` columns
//! below are region-only"* (D-01). A brand is a commercial label and has no
//! default tax category — a tax category rides the price row's
//! `tax_category_ref` (D-110) or the region's readiness, and a third place for
//! one to live is the cardinality error D-110 removed from
//! `pricing_plan_descriptor_set`.
//!
//! Everything else — the `(tenant_id, value)` key, the `active | retired` state,
//! the non-blank value, the absence of an append-only trigger — is
//! `m20260802_000028`'s and is stated there. `sqlite_taxonomy_store` proves each
//! rule against **all four** tables in one loop rather than asserting it of one
//! and assuming it of the rest, which is what keeps four near-identical
//! migrations from drifting.
//!
//! **Backend differences.** As `m20260802_000028`: `uuid` becomes `text` and the
//! `bss.` qualification is dropped. Every `CHECK` and the primary key are
//! preserved on both sides.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &["CREATE TABLE bss.pricing_brand_taxonomy (
        tenant_id    uuid NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_brand_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_brand_taxonomy_value_present CHECK (
            length(value) > 0)
    )"];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.pricing_brand_taxonomy"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &["CREATE TABLE pricing_brand_taxonomy (
        tenant_id    text NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_brand_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_brand_taxonomy_value_present CHECK (
            length(value) > 0)
    )"];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_brand_taxonomy"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
