//! Create `bss.pricing_region_taxonomy` — the region value universe
//! (`design/04-currency-tax.md` §6, D-01).
//!
//! The first of the four scope-value taxonomies §6 declares. It is **Slice 4's
//! table**, created on the Slice 9 chain, and the reason is
//! `inst-plv-scope`: every non-`global` `PriceOverlay` scope value validates
//! against its declared universe, and until this migration none of the four
//! universes existed anywhere in the crate. A validation rule whose universe is
//! absent is the defect D-211 paid an entry for — `inst-bc-coverage` delegating
//! the currency axis to a `CurrencyBindingChecker` that no crate declares, so an
//! implementer either calls into thin air or silently duplicates the check.
//! Building the universe was the alternative to shipping that shape again.
//!
//! Slice 4 inherits this table and its three siblings unchanged. What Slice 4
//! still owes them is the `GET/PUT /bss-pricing/v1/config/*-taxonomy` surface
//! and the **price-row** arm of the retire guard; the overlay arm of that guard
//! is `overlay_repo`'s and lands with this slice.
//!
//! # `tax_category` and `tax_rate_present` are on this table alone
//!
//! §6 is explicit — *"the `tax_*` columns below are region-only"* (D-01) — and
//! the three sibling migrations therefore do not carry them. That is a fact
//! about the schema rather than about a comment, so
//! `sqlite_taxonomy_store::the_other_three_taxonomies_carry_no_tax_columns`
//! asserts the absence by selecting the columns and requiring the parser to
//! refuse: a column's absence is not observable through any constraint.
//!
//! `tax_rate_present` defaults to **false**, which is the fail-closed reading of
//! the MVP `RegionTaxReadiness` source: a region nobody has declared a rate for
//! is a region with no rate, not a region with an unknown one.
//!
//! # A blank value is refused, and that is not tidiness
//!
//! `pricing_price_overlay` renders the **absence** of a scope value as the empty
//! string — the `global` class carries `scope_value = ''` and every other class
//! carries a non-empty one, under one biconditional CHECK. A blank taxonomy
//! value would make that sentinel forgeable: a `brand` overlay carrying `''`
//! would validate against a declared universe *and* read as classless. The
//! empty string denotes "no scope value" and nothing else may render it, which
//! is `m20260802_000023`'s argument for `COALESCE(meter, '')` applied one table
//! over.
//!
//! # No index beyond the primary key
//!
//! The only read `inst-plv-scope` makes is a point read on `(tenant_id, value)`,
//! which the primary key serves, and enumeration is a full scan of a table with
//! one row per declared region. An index nothing takes is a second thing to keep
//! true.
//!
//! **Backend differences.** `uuid` becomes `text`, `boolean` keeps its name but
//! takes `0`/`1`, and the `bss.` qualification is dropped, as elsewhere in this
//! chain. Every `CHECK` and the primary key are preserved on both sides. No
//! append-only trigger: a taxonomy value is editable in place by design —
//! `retired -> active` re-activation is an explicitly legal audited move (§6).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &["CREATE TABLE bss.pricing_region_taxonomy (
        tenant_id        uuid    NOT NULL,
        value            text    NOT NULL,
        display_name     text    NOT NULL,
        state            text    NOT NULL DEFAULT 'active',
        tax_category     text,
        tax_rate_present boolean NOT NULL DEFAULT false,
        PRIMARY KEY (tenant_id, value),
        CONSTRAINT chk_pricing_region_taxonomy_state CHECK (
            state IN ('active', 'retired')),
        -- See the module doc: the empty string is `pricing_price_overlay`'s
        -- sentinel for the classless scope, so nothing else may render it.
        CONSTRAINT chk_pricing_region_taxonomy_value_present CHECK (
            length(value) > 0)
    )"];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.pricing_region_taxonomy"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &["CREATE TABLE pricing_region_taxonomy (
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
    )"];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_region_taxonomy"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
