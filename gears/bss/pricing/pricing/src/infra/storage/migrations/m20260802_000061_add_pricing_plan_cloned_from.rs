//! `pricing_plan.cloned_from` — the provenance a cloned plan carries
//! (`design/12-operator-efficiency.md` §6, `inst-cl-copy`, D-19).
//!
//! One nullable column naming the plan a clone was copied from. `NULL` is the
//! ordinary case and means "authored, not cloned"; it is not a sentinel and
//! nothing reads it to decide behaviour — the clone is an **ordinary draft**
//! (`inst-cl-draft`), taking the full pipeline and an approval on its first
//! publish exactly as any other first publish does. The column is lineage an
//! operator can follow, and the `DoD`'s `clonedFrom` is this.
//!
//! # It is not a foreign key, and that is the parent's key shape rather than an
//! omission
//!
//! `pricing_plan`'s primary key is `(plan_id, revision)`, so `plan_id` alone is
//! not unique on the table and nothing exists for a bare `plan_id` to reference.
//! The alternatives are both worse than the gap: carrying `(cloned_from,
//! cloned_from_revision)` would key the clone to one *revision* of its source,
//! which is not what lineage means and would go stale the moment the source
//! re-publishes; and a `UNIQUE (plan_id)` on `pricing_plan` cannot exist at all,
//! the table holding one row per revision by construction.
//!
//! Stated here because `pricing_repricing_journal` shipped with two unkeyed id
//! columns whose module doc listed three deliberate omissions and was silent on
//! them (D-261) — an absence a reader cannot tell from an oversight is one they
//! will eventually re-litigate.
//!
//! # Frozen, and its guard is `m20260802_000062`
//!
//! Provenance, so it belongs beside `created_by` and `created_at_utc` in the
//! frozen-column whitelist: a writer moving it on a published revision rewrites
//! where the plan came from. That restatement is its own migration, per
//! `m20260802_000040`'s rule — and, since **D-263**, it is also no longer
//! possible to forget: the frozen-whitelist census reads the column list off the
//! table, so this column with no guard would redden two suites.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &["ALTER TABLE bss.pricing_plan ADD COLUMN cloned_from uuid"];
const PG_DOWN_STATEMENTS: &[&str] = &["ALTER TABLE bss.pricing_plan DROP COLUMN cloned_from"];

const SQLITE_UP_STATEMENTS: &[&str] = &["ALTER TABLE pricing_plan ADD COLUMN cloned_from text"];
const SQLITE_DOWN_STATEMENTS: &[&str] = &["ALTER TABLE pricing_plan DROP COLUMN cloned_from"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
