//! `pricing_plan`'s plan-change contract — the edges a self-service change may
//! travel and the facts that classify one (`design/06-consumer-contracts.md`
//! §6, `inst-pc-targets` / `inst-pc-rank` / `inst-pc-counter-carry`).
//!
//! Three columns:
//!
//! - **`allowed_change_targets`** — an explicit list of published `planId`s.
//!   Rule-based targets are **not authorable at launch** (D-23: a rule resolves
//!   only at read time, defeating every publish-time guarantee the contract
//!   rests on), so the column holds identities and never a selector. `NULL` is
//!   the fail-safe and it means **no self-service change** (`inst-pc-failsafe`)
//!   — never "any to any" and never "unknown".
//! - **`comparability_rank`** — a tenant-wide scale, required of any plan
//!   participating in self-service change (K4). Higher is an upgrade, lower a
//!   downgrade, equal a switch; Subscriptions derives the proration sign from
//!   it.
//! - **`usage_counter_on_plan_change`** — D-113's snapshot-frozen tier-`Q`
//!   continuity flag. `reset` is the default and **absence is `reset`**, so an
//!   old snapshot without the field is a reset rather than a rating failure.
//!
//! # `jsonb` on Postgres, `text` on `SQLite`
//!
//! §6 names the column `jsonb`. `SQLite` has no such type and stores JSON in
//! `text`; `m20260802_000002` already made that choice for
//! `included_allowance`, so this follows it rather than inventing a second
//! convention for the same problem.
//!
//! # What is **not** stamped here, and why the absence is the decision
//!
//! There is no `in_place` / `cancel_plus_new` column, and D-93 is the reason
//! (`inst-pc-boundary`). The classification is computed **at change time by
//! Subscriptions** from both plans' published facts at its pinned version. The
//! pre-D-93 publish-time stamp promised re-computation "on either side's
//! re-publish", but a target's publish unit warms only the target's own delta
//! (D-86/D-91) and the source's published revision is immutable — so the
//! mechanism never existed, and a stale `in_place` would have run an in-place
//! change across a currency or frequency boundary, which is the wrong proration.
//! The catalog publishes the **inputs**; a column here would be a frozen verdict
//! nothing could ever correct.
//!
//! # No `CHECK` on the two token columns
//!
//! `m20260802_000050`'s argument, unchanged: `SQLite` cannot add a table-level
//! `CHECK` without rebuilding `pricing_plan`, and adding one on Postgres alone
//! would leave the two engines' censuses stating different schemas. The
//! vocabulary is [`UsageCounterOnPlanChange`](crate::domain::contracts::UsageCounterOnPlanChange),
//! which a column can only ever hold a rendering of.
//!
//! `comparability_rank` is a plain `int` with no bound, deliberately: K4 makes it
//! a *tenant-wide scale* and no document names a range. A bound invented here
//! would be an authoring limit the design never ratified.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_plan ADD COLUMN allowed_change_targets jsonb",
    "ALTER TABLE bss.pricing_plan ADD COLUMN comparability_rank integer",
    "ALTER TABLE bss.pricing_plan ADD COLUMN usage_counter_on_plan_change text",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_plan DROP COLUMN usage_counter_on_plan_change",
    "ALTER TABLE bss.pricing_plan DROP COLUMN comparability_rank",
    "ALTER TABLE bss.pricing_plan DROP COLUMN allowed_change_targets",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_plan ADD COLUMN allowed_change_targets text",
    "ALTER TABLE pricing_plan ADD COLUMN comparability_rank integer",
    "ALTER TABLE pricing_plan ADD COLUMN usage_counter_on_plan_change text",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_plan DROP COLUMN usage_counter_on_plan_change",
    "ALTER TABLE pricing_plan DROP COLUMN comparability_rank",
    "ALTER TABLE pricing_plan DROP COLUMN allowed_change_targets",
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
