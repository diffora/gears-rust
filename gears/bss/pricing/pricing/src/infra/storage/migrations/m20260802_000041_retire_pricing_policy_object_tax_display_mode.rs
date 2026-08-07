//! `pricing_policy_object.tax_display_mode` is retired — D-240 (`T-2`).
//!
//! The column was created by `m20260802_000006` as
//! `text NOT NULL DEFAULT 'tax_exclusive'` under
//! `chk_pricing_policy_object_tax_display`, and it is an implementation
//! invention: **nothing in `src/` reads it** — no accessor on `AuthoringPolicy`,
//! no domain type, no rule — and `01-foundation.md` §3.7 never declares it.
//!
//! What decided its retirement is the *name* rather than the disuse. §6 spends
//! `tax display` on an enforcement **mode**, and Slice 4 built exactly that
//! beside it: `tax_display_policy_mode` (`fail_closed | warn`,
//! `m20260802_000038`). Leaving the older column in place leaves a reader two
//! adjacent names for two different facts, one of which is dead — a second
//! thing every future reader must learn is not the switch.
//!
//! # The table this rebuilds from, which is not the one that created it
//!
//! `pricing_policy_object` has been restated **once** since `m20260802_000006`:
//! `m20260802_000018` moved the approval-threshold pair out to
//! `pricing_approval_threshold` and, because `SQLite` refuses to drop a column a
//! CHECK names, did it as a create-copy-drop-rename. So `000018`'s rebuild is
//! the current text of this table on `SQLite`, **plus** `000038`'s
//! `ALTER TABLE … ADD COLUMN`, which lands after `updated_by`.
//!
//! Both halves matter. A rebuild written from `000006` would resurrect the
//! threshold pair `000018` retired; a rebuild written from `000018` alone would
//! silently drop C4's fail-closed switch and still look complete, because
//! nothing in the chain would name what went missing. The neighbouring question
//! on `pricing_price` had the opposite answer — `m20260802_000002` was never
//! restated — so the shape of this answer is measured here, not inherited.
//!
//! `000038`'s column is restated in the form it actually holds: a column-level
//! `CONSTRAINT … CHECK`, not a table-level one. That is what `000038`'s own
//! `down` drops it through, and rewriting it as a table constraint would be a
//! change this migration has no reason to make.
//!
//! # Why four statements are enough
//!
//! `pricing_policy_object` carries **no index, no trigger and no inbound foreign
//! key** — verified against the whole chain rather than taken from `000018`'s
//! doc, which is where that sentence was first written. So the rebuild has
//! nothing to re-create beyond its columns and CHECKs, and
//! `tests/sqlite_migrations.rs`' four censuses plus its trigger-body digests are
//! what would catch a lost arm: this table contributes no digest, so a faithful
//! rebuild moves **none** of them.
//!
//! # Backend differences
//!
//! Postgres drops a column and the CHECKs naming it in one statement, so the
//! `up` is a single `ALTER TABLE` there and a rebuild here — the same asymmetry
//! `m20260802_000018` carries, for the same reason.
//!
//! The `down` restores the column with its CHECK on both backends. It is a
//! restoration of *shape*, not of data: the values are gone with the column, and
//! every row takes the `'tax_exclusive'` default that `000006` declared. Nothing
//! reads the column on either side of the move, which is what makes that
//! acceptable rather than lossy.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

// Dropping the column drops `chk_pricing_policy_object_tax_display` with it;
// Postgres does that itself, which is why the CHECK is not dropped by name.
const PG_UP_STATEMENTS: &[&str] =
    &["ALTER TABLE bss.pricing_policy_object DROP COLUMN tax_display_mode"];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_policy_object
        ADD COLUMN tax_display_mode text NOT NULL DEFAULT 'tax_exclusive'",
    "ALTER TABLE bss.pricing_policy_object
        ADD CONSTRAINT chk_pricing_policy_object_tax_display CHECK (
            tax_display_mode IN ('tax_inclusive','tax_exclusive'))",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

// The column drop, as a rebuild. See the module doc: the source text is
// `m20260802_000018`'s rebuild plus `m20260802_000038`'s appended column, and
// the appended column keeps its column-level constraint.
const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_policy_object_rebuilt (
        tenant_id                       text    NOT NULL PRIMARY KEY,
        default_rounding_policy_ref     text,
        enforced_migration_notice_days  integer NOT NULL DEFAULT 60,
        max_tier_bands_per_row          integer,
        max_price_rows_per_plan         integer,
        max_custom_interval_days        integer,
        max_custom_interval_months      integer,
        additional_required_descriptors text    NOT NULL DEFAULT '[]',
        updated_at_utc                  text    NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        updated_by                      text    NOT NULL,
        tax_display_policy_mode         text    NOT NULL DEFAULT 'fail_closed'
            CONSTRAINT chk_pricing_policy_object_tax_display_policy
            CHECK (tax_display_policy_mode IN ('fail_closed', 'warn')),
        CONSTRAINT chk_pricing_policy_object_notice_floor CHECK (
            enforced_migration_notice_days >= 60),
        CONSTRAINT chk_pricing_policy_object_tier_band_cap CHECK (
            max_tier_bands_per_row IS NULL OR max_tier_bands_per_row > 0),
        CONSTRAINT chk_pricing_policy_object_price_row_cap CHECK (
            max_price_rows_per_plan IS NULL OR max_price_rows_per_plan > 0),
        CONSTRAINT chk_pricing_policy_object_interval_days_cap CHECK (
            max_custom_interval_days IS NULL OR max_custom_interval_days > 0),
        CONSTRAINT chk_pricing_policy_object_interval_months_cap CHECK (
            max_custom_interval_months IS NULL OR max_custom_interval_months > 0)
    )",
    "INSERT INTO pricing_policy_object_rebuilt (
        tenant_id, default_rounding_policy_ref,
        enforced_migration_notice_days, max_tier_bands_per_row,
        max_price_rows_per_plan, max_custom_interval_days,
        max_custom_interval_months, additional_required_descriptors,
        updated_at_utc, updated_by, tax_display_policy_mode)
     SELECT
        tenant_id, default_rounding_policy_ref,
        enforced_migration_notice_days, max_tier_bands_per_row,
        max_price_rows_per_plan, max_custom_interval_days,
        max_custom_interval_months, additional_required_descriptors,
        updated_at_utc, updated_by, tax_display_policy_mode
     FROM pricing_policy_object",
    "DROP TABLE pricing_policy_object",
    "ALTER TABLE pricing_policy_object_rebuilt RENAME TO pricing_policy_object",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_policy_object_rebuilt (
        tenant_id                       text    NOT NULL PRIMARY KEY,
        tax_display_mode                text    NOT NULL DEFAULT 'tax_exclusive',
        default_rounding_policy_ref     text,
        enforced_migration_notice_days  integer NOT NULL DEFAULT 60,
        max_tier_bands_per_row          integer,
        max_price_rows_per_plan         integer,
        max_custom_interval_days        integer,
        max_custom_interval_months      integer,
        additional_required_descriptors text    NOT NULL DEFAULT '[]',
        updated_at_utc                  text    NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        updated_by                      text    NOT NULL,
        tax_display_policy_mode         text    NOT NULL DEFAULT 'fail_closed'
            CONSTRAINT chk_pricing_policy_object_tax_display_policy
            CHECK (tax_display_policy_mode IN ('fail_closed', 'warn')),
        CONSTRAINT chk_pricing_policy_object_tax_display CHECK (
            tax_display_mode IN ('tax_inclusive','tax_exclusive')),
        CONSTRAINT chk_pricing_policy_object_notice_floor CHECK (
            enforced_migration_notice_days >= 60),
        CONSTRAINT chk_pricing_policy_object_tier_band_cap CHECK (
            max_tier_bands_per_row IS NULL OR max_tier_bands_per_row > 0),
        CONSTRAINT chk_pricing_policy_object_price_row_cap CHECK (
            max_price_rows_per_plan IS NULL OR max_price_rows_per_plan > 0),
        CONSTRAINT chk_pricing_policy_object_interval_days_cap CHECK (
            max_custom_interval_days IS NULL OR max_custom_interval_days > 0),
        CONSTRAINT chk_pricing_policy_object_interval_months_cap CHECK (
            max_custom_interval_months IS NULL OR max_custom_interval_months > 0)
    )",
    "INSERT INTO pricing_policy_object_rebuilt (
        tenant_id, default_rounding_policy_ref,
        enforced_migration_notice_days, max_tier_bands_per_row,
        max_price_rows_per_plan, max_custom_interval_days,
        max_custom_interval_months, additional_required_descriptors,
        updated_at_utc, updated_by, tax_display_policy_mode)
     SELECT
        tenant_id, default_rounding_policy_ref,
        enforced_migration_notice_days, max_tier_bands_per_row,
        max_price_rows_per_plan, max_custom_interval_days,
        max_custom_interval_months, additional_required_descriptors,
        updated_at_utc, updated_by, tax_display_policy_mode
     FROM pricing_policy_object",
    "DROP TABLE pricing_policy_object",
    "ALTER TABLE pricing_policy_object_rebuilt RENAME TO pricing_policy_object",
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
