//! `pricing_price.resolved_tax_category` — D-154's frozen effective category,
//! written at the publish commit (`design/04-currency-tax.md` §6,
//! `inst-td-persist`).
//!
//! # Why the projector could not be the one to resolve it
//!
//! `inst-td-persist` is explicit about the actor and the moment: *"**publish**
//! resolves `coalesce(row.tax_category_ref, readiness.taxCategory)` and freezes
//! the **result**"*. It was first built resolving in the **projector** instead,
//! and the gap between the two is up to D-47's five-minute batching maximum —
//! during which the region taxonomy is a mutable, tenant-declared table anyone
//! holding `config × write` may re-declare.
//!
//! The failure that opens is not theoretical. A row with no `tax_category_ref`,
//! in a region defaulting to `standard`, publishes at T0 — `TaxBasisComplete`
//! passes because the coalesce resolves. At T0+1min an admin `PUT`s the region
//! taxonomy omitting `taxCategory`; that `PUT` is a whole-set replacement and
//! writes the markers unconditionally, so the column becomes NULL. At T0+3min the
//! sweep freezes `resolvedTaxCategory: null` into an INSERT-only delta on the
//! seven-year horizon, and Billing receives a descriptor set missing a pinned
//! D-48 v1 element — the exact outcome D-154 exists to prevent. The mirror case
//! freezes a category no rule ever judged.
//!
//! So the value is resolved and persisted **inside the publish transaction**,
//! against the same readiness the rule set judged the row with, and the projector
//! reads this column instead of the taxonomy. Found by review; register `T-13`.
//!
//! # The column is **not** added to either engine's frozen-column guard
//!
//! Stated plainly because it is a gap rather than an oversight, and because the
//! reason is a real asymmetry rather than laziness.
//!
//! `trg_pricing_price_frozen_columns` fires `WHEN OLD.lifecycle_state <> 'draft'`,
//! so the publish transition itself may write this column and every later update
//! *ought* to be refused. Adding it to that guard costs one line on `SQLite`,
//! where the frozen-column check is a trigger of its own — and on Postgres the
//! same check is one arm of a **single large PL/pgSQL function** that also
//! carries the DELETE guard and both lifecycle-transition guards
//! (`m20260802_000002`). Neither engine has an incremental form, so the Postgres
//! half means restating that whole function by hand, and a hand-restated guard
//! that silently drops one of its other three arms is a worse outcome than the
//! gap it closes: this program has twice had a scripted replacement do something
//! other than intended, once deleting whole function bodies.
//!
//! What the gap costs is bounded and worth naming: the column is written by
//! exactly one statement in this crate (`price_repo::publish_rows`) and read by
//! the projector, so no path here can mutate it after publish. The guard's job is
//! to hold against a writer that is **not** this crate, and for this column that
//! protection is missing. Register `T-18` carries it, with the whole-function
//! restatement as the remedy.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.pricing_price ADD COLUMN resolved_tax_category text"];

const PG_DOWN_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.pricing_price DROP COLUMN resolved_tax_category"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE pricing_price ADD COLUMN resolved_tax_category text"];

const SQLITE_DOWN_STATEMENTS: &[&str] =
    &["ALTER TABLE pricing_price DROP COLUMN resolved_tax_category"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
