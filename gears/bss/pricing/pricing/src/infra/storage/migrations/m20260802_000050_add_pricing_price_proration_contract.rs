//! `pricing_price`'s proration input contract — the three fields Subscriptions
//! computes its proration from (`design/06-consumer-contracts.md` §6,
//! `inst-pi-required`).
//!
//! Four columns, because `billingAnchorPolicy` is two: §6's table gives it as an
//! enum "`calendar_month | subscription_start | fixed_day`; + `anchor_day`
//! (`int`, for `fixed_day`)". A `fixed_day` with no day is not a policy and a
//! day beside `calendar_month` is a number nothing reads, so the pair is one
//! fact in two columns — the shape `chk_pricing_plan_custom_interval_pairing`
//! already keeps for `customEveryN`.
//!
//! # No `CHECK` on either engine, and that is a decision with a precedent
//!
//! §6 states the value sets (K1's five proration bases, K2's three anchor
//! policies) and `anchor_day BETWEEN 1 AND 31`. None of them is expressed here.
//!
//! `SQLite` has no incremental form for a table-level `CHECK`: adding one means
//! rebuilding the table, which for `pricing_price` is a `CREATE`/`INSERT
//! SELECT`/`DROP`/`RENAME` over every column, every index and every trigger the
//! chain has attached to it since `m20260802_000002` — the shape
//! `m20260802_000042` had to take for four small taxonomy tables. Adding the
//! constraint on Postgres alone would leave the two engines' `EXPECTED_CHECKS`
//! censuses stating different schemas, which is the one thing those censuses
//! exist to make impossible to do silently.
//!
//! So the value sets live in the domain, where they are **unrepresentable**
//! rather than merely rejected: `BillingAnchorPolicy` and `ProrationBasis` are
//! enums, and a column can only hold what one of them renders. That is the
//! argument `publish::rules` already makes for the money rules — "a
//! [`PriceRow`](crate::domain::price_row::PriceRow) cannot hold a violation of
//! any of them, so a rule here would be a rule with nothing to reject". The
//! precedent for the migration shape is Slice 4's: `m20260802_000037`
//! (`tax_category_ref`) and `m20260802_000039` (`resolved_tax_category`) both
//! add a plain `text` column and leave the vocabulary to the domain.
//!
//! What the domain cannot hold is the **pairing** and the **market
//! uniformity** — both are statements about a set of rows, not about a value —
//! and both are publish-path rules (`inst-pi-required`, `inst-pi-anchor`,
//! `inst-pi-uniform`).
//!
//! # These four columns are **not** in the frozen-column guard yet
//!
//! Named here rather than left for a reader to find, because it is the same gap
//! `m20260802_000039` declared and `m20260802_000040` paid: they are authored
//! draft content, they are inside the approval content pin, and a writer outside
//! this crate could therefore move a published row's proration contract away
//! from the pin that approved it — silently. The guard is a **whole-function
//! restatement** on Postgres (`bss.pricing_price_append_only()` carries the
//! DELETE guard and three lifecycle guards in one body) and a trigger recreate
//! on `SQLite`, so it is its own migration for `m20260802_000040`'s reason: a
//! hand-restated guard that drops one of its other arms is a worse outcome than
//! the gap it closes, and it deserves a diff a reviewer can read on its own.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price ADD COLUMN billing_anchor_policy text",
    "ALTER TABLE bss.pricing_price ADD COLUMN anchor_day integer",
    "ALTER TABLE bss.pricing_price ADD COLUMN proration_basis text",
    "ALTER TABLE bss.pricing_price ADD COLUMN credit_on_downgrade boolean",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price DROP COLUMN credit_on_downgrade",
    "ALTER TABLE bss.pricing_price DROP COLUMN proration_basis",
    "ALTER TABLE bss.pricing_price DROP COLUMN anchor_day",
    "ALTER TABLE bss.pricing_price DROP COLUMN billing_anchor_policy",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_price ADD COLUMN billing_anchor_policy text",
    "ALTER TABLE pricing_price ADD COLUMN anchor_day integer",
    "ALTER TABLE pricing_price ADD COLUMN proration_basis text",
    "ALTER TABLE pricing_price ADD COLUMN credit_on_downgrade boolean",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_price DROP COLUMN credit_on_downgrade",
    "ALTER TABLE pricing_price DROP COLUMN proration_basis",
    "ALTER TABLE pricing_price DROP COLUMN anchor_day",
    "ALTER TABLE pricing_price DROP COLUMN billing_anchor_policy",
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
