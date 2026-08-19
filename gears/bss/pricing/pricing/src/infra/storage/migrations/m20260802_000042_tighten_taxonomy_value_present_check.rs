//! The taxonomy `value` CHECK rejects whitespace — D-242 (`T-6`).
//!
//! `m20260802_000028`…`000031` each gave their table
//! `CHECK (length(value) > 0)`. `'   '` satisfies that, so the store admitted a
//! value the domain refuses: `ScopeValue::new` **trims** before it decides.
//! Measured on both engines rather than reasoned — the register entry began as a
//! Postgres case asserting a refusal, and the insert landed.
//!
//! # The hole this closes is not the one the constraint was written for
//!
//! The constraint's stated purpose stands and is unchanged: it keeps the
//! classless-scope sentinel unforgeable, because `pricing_price_overlay` renders
//! the classless scope as the empty string and a declared `''` would validate
//! against a universe *and* read as classless. `'   '` was never that sentinel —
//! the empty string is rendered exactly — so `inst-plv-scope` was never exposed.
//!
//! What `'   '` cost is one level up. `TaxonomyRepo::list` maps a value
//! `ScopeValue` refuses to `RepoError::CorruptRow`, so **one** whitespace row
//! made `GET /config/taxonomies/{class}` fail for **every** value in that class,
//! and the only remedy was direct SQL: the `PUT` cannot round-trip a list it
//! cannot read. Tightening the CHECK is the only option of the three that stops
//! the row existing rather than coping with it, and it makes the store agree
//! with the domain type — which is the invariant that was intended.
//!
//! # Backend differences
//!
//! The trim function is **not** spelled the same on the two backends.
//! Postgres has `btrim(text)`; `SQLite` has no `btrim` at all and spells the
//! same operation `trim(X)`. Writing the decision's `btrim` into both arms would
//! give a `SQLite` chain that fails at `CREATE TABLE` with `no such function`,
//! so each arm uses its own engine's name for one operation.
//!
//! Postgres replaces a constraint in place — `DROP CONSTRAINT` then
//! `ADD CONSTRAINT` under the same name, so neither roster census moves. On
//! `SQLite` a CHECK cannot be altered, so each of the four tables is a
//! create-copy-drop-rename **rebuild**: four rebuilds, each restated by hand from
//! its creating migration.
//!
//! Each of the four was measured to be restated by nobody since — every one is
//! named in exactly one migration, its own. That is the **opposite** of the
//! answer next door on `pricing_policy_object`, where `m20260802_000018`'s
//! rebuild rather than the creating migration held the current text, which is
//! why the question was asked per table instead of assumed from the neighbour.
//!
//! None of the four carries a trigger, an index or an inbound foreign key, so
//! the rebuilds re-create nothing beyond their columns and CHECKs and move none
//! of `tests/sqlite_migrations.rs`' trigger-body digests.
//!
//! The `down` restores `length(value) > 0` on both backends — the looser
//! predicate, which every row admitted under the tighter one satisfies, so the
//! reversal cannot strand data.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_region_taxonomy
        DROP CONSTRAINT chk_pricing_region_taxonomy_value_present",
    "ALTER TABLE bss.pricing_region_taxonomy
        ADD CONSTRAINT chk_pricing_region_taxonomy_value_present CHECK (
            length(btrim(value)) > 0)",
    "ALTER TABLE bss.pricing_brand_taxonomy
        DROP CONSTRAINT chk_pricing_brand_taxonomy_value_present",
    "ALTER TABLE bss.pricing_brand_taxonomy
        ADD CONSTRAINT chk_pricing_brand_taxonomy_value_present CHECK (
            length(btrim(value)) > 0)",
    "ALTER TABLE bss.pricing_partner_taxonomy
        DROP CONSTRAINT chk_pricing_partner_taxonomy_value_present",
    "ALTER TABLE bss.pricing_partner_taxonomy
        ADD CONSTRAINT chk_pricing_partner_taxonomy_value_present CHECK (
            length(btrim(value)) > 0)",
    "ALTER TABLE bss.pricing_org_tier_taxonomy
        DROP CONSTRAINT chk_pricing_org_tier_taxonomy_value_present",
    "ALTER TABLE bss.pricing_org_tier_taxonomy
        ADD CONSTRAINT chk_pricing_org_tier_taxonomy_value_present CHECK (
            length(btrim(value)) > 0)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_region_taxonomy
        DROP CONSTRAINT chk_pricing_region_taxonomy_value_present",
    "ALTER TABLE bss.pricing_region_taxonomy
        ADD CONSTRAINT chk_pricing_region_taxonomy_value_present CHECK (
            length(value) > 0)",
    "ALTER TABLE bss.pricing_brand_taxonomy
        DROP CONSTRAINT chk_pricing_brand_taxonomy_value_present",
    "ALTER TABLE bss.pricing_brand_taxonomy
        ADD CONSTRAINT chk_pricing_brand_taxonomy_value_present CHECK (
            length(value) > 0)",
    "ALTER TABLE bss.pricing_partner_taxonomy
        DROP CONSTRAINT chk_pricing_partner_taxonomy_value_present",
    "ALTER TABLE bss.pricing_partner_taxonomy
        ADD CONSTRAINT chk_pricing_partner_taxonomy_value_present CHECK (
            length(value) > 0)",
    "ALTER TABLE bss.pricing_org_tier_taxonomy
        DROP CONSTRAINT chk_pricing_org_tier_taxonomy_value_present",
    "ALTER TABLE bss.pricing_org_tier_taxonomy
        ADD CONSTRAINT chk_pricing_org_tier_taxonomy_value_present CHECK (
            length(value) > 0)",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Four rebuilds. Each table's text is its creating migration's, restated by hand
// with the one predicate changed - `trim`, not `btrim`, which SQLite does not
// have.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_region_taxonomy_rebuilt (
        tenant_id        text    NOT NULL,
        value            text    NOT NULL,
        display_name     text    NOT NULL,
        state            text    NOT NULL DEFAULT 'active',
        tax_category     text,
        tax_rate_present boolean NOT NULL DEFAULT 0,
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_region_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_region_taxonomy_value_present CHECK (
            length(trim(value)) > 0)
    )",
    "INSERT INTO pricing_region_taxonomy_rebuilt (
        tenant_id, value, display_name, state, tax_category, tax_rate_present)
     SELECT
        tenant_id, value, display_name, state, tax_category, tax_rate_present
     FROM pricing_region_taxonomy",
    "DROP TABLE pricing_region_taxonomy",
    "ALTER TABLE pricing_region_taxonomy_rebuilt RENAME TO pricing_region_taxonomy",
    "CREATE TABLE pricing_brand_taxonomy_rebuilt (
        tenant_id    text NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_brand_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_brand_taxonomy_value_present CHECK (
            length(trim(value)) > 0)
    )",
    "INSERT INTO pricing_brand_taxonomy_rebuilt (
        tenant_id, value, display_name, state)
     SELECT
        tenant_id, value, display_name, state
     FROM pricing_brand_taxonomy",
    "DROP TABLE pricing_brand_taxonomy",
    "ALTER TABLE pricing_brand_taxonomy_rebuilt RENAME TO pricing_brand_taxonomy",
    "CREATE TABLE pricing_partner_taxonomy_rebuilt (
        tenant_id    text NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_partner_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_partner_taxonomy_value_present CHECK (
            length(trim(value)) > 0)
    )",
    "INSERT INTO pricing_partner_taxonomy_rebuilt (
        tenant_id, value, display_name, state)
     SELECT
        tenant_id, value, display_name, state
     FROM pricing_partner_taxonomy",
    "DROP TABLE pricing_partner_taxonomy",
    "ALTER TABLE pricing_partner_taxonomy_rebuilt RENAME TO pricing_partner_taxonomy",
    "CREATE TABLE pricing_org_tier_taxonomy_rebuilt (
        tenant_id    text NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_org_tier_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_org_tier_taxonomy_value_present CHECK (
            length(trim(value)) > 0)
    )",
    "INSERT INTO pricing_org_tier_taxonomy_rebuilt (
        tenant_id, value, display_name, state)
     SELECT
        tenant_id, value, display_name, state
     FROM pricing_org_tier_taxonomy",
    "DROP TABLE pricing_org_tier_taxonomy",
    "ALTER TABLE pricing_org_tier_taxonomy_rebuilt RENAME TO pricing_org_tier_taxonomy",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_region_taxonomy_rebuilt (
        tenant_id        text    NOT NULL,
        value            text    NOT NULL,
        display_name     text    NOT NULL,
        state            text    NOT NULL DEFAULT 'active',
        tax_category     text,
        tax_rate_present boolean NOT NULL DEFAULT 0,
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_region_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_region_taxonomy_value_present CHECK (
            length(value) > 0)
    )",
    "INSERT INTO pricing_region_taxonomy_rebuilt (
        tenant_id, value, display_name, state, tax_category, tax_rate_present)
     SELECT
        tenant_id, value, display_name, state, tax_category, tax_rate_present
     FROM pricing_region_taxonomy",
    "DROP TABLE pricing_region_taxonomy",
    "ALTER TABLE pricing_region_taxonomy_rebuilt RENAME TO pricing_region_taxonomy",
    "CREATE TABLE pricing_brand_taxonomy_rebuilt (
        tenant_id    text NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_brand_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_brand_taxonomy_value_present CHECK (
            length(value) > 0)
    )",
    "INSERT INTO pricing_brand_taxonomy_rebuilt (
        tenant_id, value, display_name, state)
     SELECT
        tenant_id, value, display_name, state
     FROM pricing_brand_taxonomy",
    "DROP TABLE pricing_brand_taxonomy",
    "ALTER TABLE pricing_brand_taxonomy_rebuilt RENAME TO pricing_brand_taxonomy",
    "CREATE TABLE pricing_partner_taxonomy_rebuilt (
        tenant_id    text NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_partner_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_partner_taxonomy_value_present CHECK (
            length(value) > 0)
    )",
    "INSERT INTO pricing_partner_taxonomy_rebuilt (
        tenant_id, value, display_name, state)
     SELECT
        tenant_id, value, display_name, state
     FROM pricing_partner_taxonomy",
    "DROP TABLE pricing_partner_taxonomy",
    "ALTER TABLE pricing_partner_taxonomy_rebuilt RENAME TO pricing_partner_taxonomy",
    "CREATE TABLE pricing_org_tier_taxonomy_rebuilt (
        tenant_id    text NOT NULL,
        value        text NOT NULL,
        display_name text NOT NULL,
        state        text NOT NULL DEFAULT 'active',
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_org_tier_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        CONSTRAINT chk_pricing_org_tier_taxonomy_value_present CHECK (
            length(value) > 0)
    )",
    "INSERT INTO pricing_org_tier_taxonomy_rebuilt (
        tenant_id, value, display_name, state)
     SELECT
        tenant_id, value, display_name, state
     FROM pricing_org_tier_taxonomy",
    "DROP TABLE pricing_org_tier_taxonomy",
    "ALTER TABLE pricing_org_tier_taxonomy_rebuilt RENAME TO pricing_org_tier_taxonomy",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
