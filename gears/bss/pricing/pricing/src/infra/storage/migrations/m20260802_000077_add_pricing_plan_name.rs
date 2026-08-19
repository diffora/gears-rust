//! `pricing_plan.plan_name` — the plan's human label (D-318).
//!
//! One nullable text column. Until now a plan had **no name at all**: the row
//! carries `plan_tier`, `sku_id` and a billing cycle, and every surface that had
//! to show a plan to a person fell back to the tier, or to the first eight
//! characters of a UUID when there was no tier. Two plans on one tier were then
//! indistinguishable in a list — which is not a hypothetical, it is what the
//! catalog on the stand looks like today.
//!
//! # Nullable, and it stays nullable
//!
//! Every plan already stored has no name, so `NOT NULL` would need a backfill
//! inventing one for each, and an invented name is worse than an absent one: a
//! reader cannot tell it from something an operator chose. `NULL` means "not
//! named" and the display fallback stays exactly where it is.
//!
//! **`NULL` and `''` must not both mean unnamed.** Two spellings of one state is
//! a defect this gear has hit before, so the empty string is refused at the
//! write stage (`PLAN_NAME_INVALID`) rather than normalised here — a `CHECK`
//! cannot be added to `pricing_plan` on `SQLite` without rebuilding the table
//! (`m20260802_000056`'s finding), and adding one on Postgres alone would leave
//! the two engines' censuses describing different schemas.
//!
//! # Frozen once published, and its guard is `m20260802_000078`
//!
//! A published revision is frozen in content, and a name is content: it is what
//! the catalog called this plan while that version was live, and a writer moving
//! it rewrites what an operator reading history would see the plan as having
//! been. So renaming a published plan is a new revision, like every other edit
//! to it — stated plainly in D-318 because it is the cost of the choice, not a
//! side effect of it.
//!
//! Since **D-263** the frozen-whitelist census reads the column list off the
//! table, so this column arriving without its guard reddens two suites rather
//! than shipping a quietly mutable field.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &["ALTER TABLE bss.pricing_plan ADD COLUMN plan_name text"];
const PG_DOWN_STATEMENTS: &[&str] = &["ALTER TABLE bss.pricing_plan DROP COLUMN plan_name"];

const SQLITE_UP_STATEMENTS: &[&str] = &["ALTER TABLE pricing_plan ADD COLUMN plan_name text"];
const SQLITE_DOWN_STATEMENTS: &[&str] = &["ALTER TABLE pricing_plan DROP COLUMN plan_name"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
