//! D-380's column: the tenant's approver count `N`, on the threshold policy
//! version.
//!
//! # Why it rides the version's rows rather than a table of its own
//!
//! `N` is a member of the **version**, not a per-tenant scalar: D-10 makes any
//! policy diff always material, so putting `N` inside the governed object is
//! what makes a change to it material under the then-current `N`, and what
//! gives it D-188's `effective_from` and D-186's tag for free. A table beside
//! the policy would be a second object with its own lifecycle, free to
//! disagree with the version an approver signed.
//!
//! So it goes on **every entry row** and on the **tombstone**, which is the
//! shape `effective_from` already has on these two tables: a per-version scalar
//! carried per row and derived back with disagreement refused as a corrupt row.
//! A tombstone is a version — it returns the tenant to the G1 fail-safe and
//! still says how many approvers a material change costs — so a tombstone
//! without an `N` would make the type partial.
//!
//! # `DEFAULT 1`, and why the default stays on the column
//!
//! One is the gear's shipped rule (`fr-approval-two-person`: submitter + 1
//! approver = two distinct principals), so the backfill leaves every stored
//! version meaning exactly what it meant. The default is **not** dropped after
//! the backfill: during a rolling deploy an older binary still writes rows
//! without the column, and they must land on the same value rather than fail.
//!
//! # The content pin moved in the same change
//!
//! An approver now signs the count, so `THRESHOLD_PIN_DOMAIN_SEP` went to `v2`.
//! A decision against a version pinned under `v1` answers
//! `APPROVAL_CONTENT_MISMATCH`; withdraw and resubmit it. That is the drain the
//! pin constants' docs prescribe, not an exception to them.
//!
//! # `SQLite` takes `ADD COLUMN` here without a table rebuild
//!
//! The frozen-chain rule about rebuilding a `SQLite` table is about **tightening
//! an existing** column. Adding one is admissible on both engines, so neither
//! side rebuilds and the append-only triggers on these two tables are
//! untouched.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_approval_threshold
        ADD COLUMN approver_count integer NOT NULL DEFAULT 1",
    "ALTER TABLE bss.pricing_approval_threshold
        ADD CONSTRAINT chk_pricing_approval_threshold_approver_count
        CHECK (approver_count >= 0)",
    "ALTER TABLE bss.pricing_approval_threshold_tombstone
        ADD COLUMN approver_count integer NOT NULL DEFAULT 1",
    "ALTER TABLE bss.pricing_approval_threshold_tombstone
        ADD CONSTRAINT chk_pricing_approval_threshold_tombstone_approver_count
        CHECK (approver_count >= 0)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_approval_threshold_tombstone
        DROP CONSTRAINT IF EXISTS chk_pricing_approval_threshold_tombstone_approver_count",
    "ALTER TABLE bss.pricing_approval_threshold_tombstone DROP COLUMN IF EXISTS approver_count",
    "ALTER TABLE bss.pricing_approval_threshold
        DROP CONSTRAINT IF EXISTS chk_pricing_approval_threshold_approver_count",
    "ALTER TABLE bss.pricing_approval_threshold DROP COLUMN IF EXISTS approver_count",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_approval_threshold
        ADD COLUMN approver_count integer NOT NULL DEFAULT 1
        CHECK (approver_count >= 0)",
    "ALTER TABLE pricing_approval_threshold_tombstone
        ADD COLUMN approver_count integer NOT NULL DEFAULT 1
        CHECK (approver_count >= 0)",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_approval_threshold_tombstone DROP COLUMN approver_count",
    "ALTER TABLE pricing_approval_threshold DROP COLUMN approver_count",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(
            self.name(),
            manager,
            PG_DOWN_STATEMENTS,
            SQLITE_DOWN_STATEMENTS,
        )
        .await
    }
}
