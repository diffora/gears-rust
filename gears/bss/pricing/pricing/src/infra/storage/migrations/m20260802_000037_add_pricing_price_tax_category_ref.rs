//! `pricing_price.tax_category_ref` — the price row's tax category
//! (`design/04-currency-tax.md` §6, D-110).
//!
//! # The column the design set has assumed since D-110, and which did not exist
//!
//! §6 is unambiguous: *"`tax_category_ref` is the **source of truth** for a row's
//! tax category, and it is the **only** place a tax category lives (D-110)"*.
//! `inst-td-persist` builds on it, D-154's frozen resolved category is
//! `coalesce(row.tax_category_ref, readiness.taxCategory)`, and D-48's descriptor
//! contract carries `taxCategory` as one of its pinned v1 five *"now riding the
//! price row"*.
//!
//! **None of it had a column.** `plan_rules/descriptor_set.rs` says so in as many
//! words — *"has no `tax_category_ref` column at all (Slice 4 has not landed)"* —
//! and `materiality/delta.rs` records the same absence from the other side: a
//! comparison of a field with no column is a comparison that always answers
//! equal. Two slices' rules were therefore written against a fact the store could
//! not hold.
//!
//! # Nullable, and the absence is not "no category"
//!
//! `NULL` means **the row states no category of its own**, which under D-154
//! resolves to the region taxonomy's default. That is a different fact from a
//! region that declares none either — the pair is what
//! `inst-td-policy`'s coalesce is about, and collapsing them would make the
//! design set's own worked example (a row-level `tax_category_ref` satisfying the
//! check in a region with no default) unreachable.
//!
//! So there is deliberately **no `NOT NULL`** and no default. A `NOT NULL DEFAULT
//! ''` would give "unstated" two spellings, which is the mistake
//! `pricing_policy_object`'s descriptor extension column documents avoiding.
//!
//! # No `CHECK`, and that is a decision rather than an omission
//!
//! A tax category is an opaque token: D-01 leaves the universe tenant-declared,
//! no document of the set enumerates one, and the Tax Engine that will own the
//! vocabulary is post-MVP. A `CHECK` here would be this gear inventing a value
//! set the design set has not, and it would refuse the tenant categories the
//! region taxonomy's own `tax_category` column already accepts without one — the
//! two columns hold the same kind of value and must not disagree about what is
//! legal.
//!
//! Emptiness is refused at the repository boundary instead, where a blank token
//! is a caller mistake naming one field rather than a driver error and a 500.
//!
//! # Why the append-only trigger does not need an arm
//!
//! `m20260802_000002`'s freeze trigger whitelists the columns a **published** row
//! may not change by naming them, so a column added later is mutable by default
//! on that path. That is correct here and is not an oversight: the category is
//! **draft content**, edited through `PATCH …/prices/{priceId}` under the row's
//! entity tag like every other content field, and frozen — as every content field
//! is — by the row leaving `draft`. The state machine that already governs
//! `amount_minor` governs this identically, and a second mechanism for one column
//! would be a second answer to when content stops moving.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.pricing_price ADD COLUMN tax_category_ref text"];

const PG_DOWN_STATEMENTS: &[&str] = &["ALTER TABLE bss.pricing_price DROP COLUMN tax_category_ref"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

/// `ADD COLUMN` and not a table rebuild.
///
/// `m20260802_000019`'s create-copy-drop-rename is what a `CHECK` **widening**
/// costs on this engine, and it is expensive precisely because the rebuild takes
/// every trigger and index with it. Nothing here needs one: a nullable column
/// with no constraint is the one shape `SQLite`'s `ALTER TABLE` supports
/// natively, so the two backends' statements are the same statement.
const SQLITE_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE pricing_price ADD COLUMN tax_category_ref text"];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["ALTER TABLE pricing_price DROP COLUMN tax_category_ref"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
