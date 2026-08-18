//! `pricing_plan.entitlement_grants` — the grant set Subscriptions provisions
//! from (`design/06-consumer-contracts.md` §6, `inst-gs-shape`, D-41).
//!
//! One `jsonb` column holding the §17.6 shape: `featureFlag: bool` entries,
//! `quotaKey: value` entries, optionally a `PlanTier` reference **plus** the set
//! it resolved to (`inst-gs-resolved`), and optionally a `perPhase` map keyed by
//! `phase_id` (D-41).
//!
//! # This is **not** `pricing_plan_grant`, and the difference is worth a
//! paragraph
//!
//! `pricing_plan_grant` (D-52) is **Slice 10's** table, and it holds a different
//! object entirely: the prepaid / promotional **credit** grant D-43 defines —
//! `category`, `applicability`, `drawdownPriority`, a `source ∈ {authored,
//! compiled_allowance}` lineage, and the compiled `carry` grants
//! `inst-ac-carry` produces per allowance-carrying usage row. Its home is
//! `design/10-advanced-primitives.md` §6.
//!
//! The Slice-6 **entitlement** grant set is D-41's, its home is
//! `06-consumer-contracts.md` §6, and that section declares it as **a column on
//! `pricing_plan`**, not a table. The two share the word "grant" and nothing
//! else: one is money a subscriber may spend, the other is what a subscriber is
//! entitled to use. Building the second on the first's table would put a
//! Slice-6 requirement inside a Slice-10 aggregate and give the `carry` compile
//! rows it must never see.
//!
//! # Why one column and not a `pricing_plan_entitlement_grant` table
//!
//! The set is read as a whole and written as a whole: Subscriptions resolves the
//! grant set for the active phase at `t` by a **single lookup**
//! (`inst-gs-perphase`), and an author submits the set they want the plan to
//! have. Nothing addresses one entry, so nothing needs one entry to have
//! identity — which is the same argument `PriceContent` makes for the band set,
//! and the reason D-52 needed a table where this does not: a compiled allowance
//! grant *does* have per-row lineage and a second grant to sit beside.
//!
//! Revision-scoped by construction, since `pricing_plan` rows are keyed
//! `(plan_id, revision)` (D-83).
//!
//! # `jsonb` on Postgres, `text` on `SQLite`
//!
//! `m20260802_000002`'s convention for `included_allowance`, followed rather
//! than re-decided.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.pricing_plan ADD COLUMN entitlement_grants jsonb"];

const PG_DOWN_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.pricing_plan DROP COLUMN entitlement_grants"];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE pricing_plan ADD COLUMN entitlement_grants text"];

const SQLITE_DOWN_STATEMENTS: &[&str] =
    &["ALTER TABLE pricing_plan DROP COLUMN entitlement_grants"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
