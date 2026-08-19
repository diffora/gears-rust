//! Create `bss.pricing_customer_group_taxonomy` — the BSS customer-group value
//! universe (`design/09-price-overlays.md` §3 `inst-cg-taxonomy`, §6).
//!
//! # This is Slice 4's four shape, on Slice 9's own route
//!
//! `inst-cg-taxonomy` states the rule in one line: *"`GroupTaxonomy` is BSS-owned
//! and governed like region/brand (values validated at authoring; retire guarded
//! by referential checks)"*. The table this migration creates is therefore the
//! four Slice 4 taxonomies' shape exactly — `m20260802_000031`'s, which is
//! `orgTier`'s and carries no `tax_*` columns for `m20260802_000029`'s reason.
//!
//! What is **not** shared is the route. `design/09-price-overlays.md` §5 gives
//! this taxonomy its own pair, `GET/PUT /bss-pricing/v1/customer-groups/taxonomy`,
//! and `design/05-governance.md` (around the endpoint-mapping table) is explicit
//! that it is **not** filed under `config × write/read` with its four siblings:
//! *"the customer-group taxonomy is **not** here: it lives at
//! `/bss-pricing/v1/customer-groups/taxonomy` under `customer_group` (more
//! sensitive)"*. Per-payer membership is payer-level commercial data, and a
//! table addressable through the shared `{class}` route would be reachable by
//! every holder of `config × write` — exactly the widening the design set
//! segregates against. So `TaxonomyClass` (`crate::domain::taxonomy`) gains no
//! fifth arm; this table is validated and written through its own repository
//! arm and its own route (`crate::api::rest::customer_groups`).
//!
//! `ScopeClass::CustomerGroup` (`crate::domain::overlay`) already names this
//! table as its taxonomy (`ScopeClass::taxonomy_table`), which is what
//! `inst-plv-scope`'s validation and `inst-tx-mutation`'s retire guard both key
//! on once this migration lands. Before this, the class had no universe at all
//! and `overlay_repo::taxonomy_declares` answered `false` unconditionally for it
//! (D-223) — wiring that read to this table is a later change; this migration
//! only gives the class somewhere to read.
//!
//! **Backend differences.** As `m20260802_000028`: `bss.`-qualified DDL on
//! Postgres, unqualified on `SQLite`, both `CREATE TABLE` bodies otherwise
//! identical.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &["CREATE TABLE bss.pricing_customer_group_taxonomy (
        tenant_id    uuid NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_customer_group_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_customer_group_taxonomy_value_present CHECK (
            length(value) > 0)
    )"];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.pricing_customer_group_taxonomy"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &["CREATE TABLE pricing_customer_group_taxonomy (
        tenant_id    text NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_customer_group_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_customer_group_taxonomy_value_present CHECK (
            length(value) > 0)
    )"];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_customer_group_taxonomy"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
