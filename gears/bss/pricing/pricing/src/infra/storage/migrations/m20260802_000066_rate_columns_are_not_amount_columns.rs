//! Rates get their own columns and their own scale — D-311.
//!
//! `MinorAmount` is whole ISO-4217 minor units, and until this migration every
//! money column in the catalog was that: the sum a `flat` row charges, the price
//! of one unit on `per_unit`, the rate of a tier band, the price of a package
//! block. Two of those are **amounts** — they reach an invoice, and an invoice
//! cannot carry `$0.015`. Two are **rates**: multipliers, where what rounds to a
//! minor unit is the *product*, never the multiplier.
//!
//! Sub-minor-unit rates are the ordinary case in metered pricing — S3 at `$0.023`
//! per GB-month, Lambda at `$0.0000166667` per GB-second — and refusal was the
//! good half of the old behaviour. Truncation was worse: a `0.0150 / 0.0110`
//! ladder and a `0.0230 / 0.0120` ladder both collapsed to `0.01` at every band,
//! so two different tariffs became one, on rows that looked well-formed.
//!
//! # What moves
//!
//! - `pricing_price_tier_band.unit_price_minor` → **`unit_price_nano`**, the same
//!   `bigint` counting 10⁻⁹ minor units instead of 1.
//! - `pricing_price` gains **`unit_rate_nano`**, the `per_unit` rate, on its own
//!   rather than sharing `amount_minor` with `flat`.
//!
//! # Why `per_unit` gets a column of its own
//!
//! `amount_minor` documented itself as *"the single amount on `flat`, the unit
//! price on `per_unit`"* — one column already meaning two different things by
//! `model_kind`, which is the same defect D-311 names one level down. Splitting
//! it is what keeps "an amount column holds amounts" true; the alternative,
//! storing everything in the rate scale, would have put the invoice sum in the
//! rate type and dissolved the distinction the decision exists to draw.
//!
//! **Which rule follows the column, and which does not.** `amount_minor` carries
//! `chk_pricing_price_amount_non_negative` at the schema layer; the new column
//! gets its own `CHECK` here, on Postgres, for the same reason. What it does
//! *not* inherit is the placement rule — there is no schema constraint tying a
//! `model_kind` to the column it must price, on either engine. That rule lives in
//! `domain::rules` (`AMOUNT_PLACEMENT_INVALID`) and is unchanged by this
//! migration; saying otherwise here would have credited the schema with a guard
//! it does not hold.
//!
//! The frozen-column guard **is** a rule this column inherits, and it does not
//! arrive here: `m20260802_000069` restates both engines' guards to freeze
//! `unit_rate_nano` on a published row. Splitting a column splits its rules too,
//! and a price column outside the append-only guard is the one gap this pair
//! must not leave open.
//!
//! # Renamed rather than added-beside, and that is a decision
//!
//! There is no dual-write window and no back-fill: the column changes meaning by
//! a factor of 10⁹, and a period where both spellings existed would be a period
//! where two readers could disagree about the price of the same row. Nothing is
//! published on any stand — the two-person rule has held every publish — so the
//! rows that exist are drafts, and drafts are re-authorable. **The catalog on
//! benidorm is recreated rather than converted**, which is the decision that makes
//! a rename safe.
//!
//! `RENAME COLUMN` carries `CHECK` references with it on both engines, so the
//! constraint keeps its name and follows the column; no table rebuild is needed.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price_tier_band
        RENAME COLUMN unit_price_minor TO unit_price_nano",
    "ALTER TABLE bss.pricing_price ADD COLUMN unit_rate_nano bigint",
    "ALTER TABLE bss.pricing_price
        ADD CONSTRAINT chk_pricing_price_unit_rate_nano CHECK (unit_rate_nano >= 0)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price DROP CONSTRAINT IF EXISTS chk_pricing_price_unit_rate_nano",
    "ALTER TABLE bss.pricing_price DROP COLUMN IF EXISTS unit_rate_nano",
    "ALTER TABLE bss.pricing_price_tier_band
        RENAME COLUMN unit_price_nano TO unit_price_minor",
];

/// The half of the `SQLite` `up` that is safe to replay unconditionally — its own
/// `down` puts the name back, so the reverse walk leaves nothing for it to trip
/// over.
const SQLITE_UP_RENAME: &[&str] = &["ALTER TABLE pricing_price_tier_band
        RENAME COLUMN unit_price_minor TO unit_price_nano"];

/// The half that **cannot** be replayed blind (review F2, 2026-08-19).
///
/// `SQLite` cannot add a `CHECK` to an existing table, so this migration adds the
/// column without one and the constraint arrives with the next rebuild.
///
/// **That rebuild happened**: `m20260802_000076` rebuilt `pricing_price` ten
/// migrations later for the tier-aggregation window, and it now carries
/// `chk_pricing_price_unit_rate_nano` beside `amount_minor`'s. An earlier comment
/// here argued the asymmetry was permanent because rebuilding "would be a large
/// edit" — a reason that stopped being true the moment somebody paid that cost
/// for another reason. Kept as a record of why the gap existed, not as a live
/// claim.
///
/// The consequence is this constant's own: because `m076`'s `CHECK` names
/// `unit_rate_nano`, this migration's `down` can no longer drop the column, so a
/// partial rollback to below `m066` leaves it in place — and `ADD COLUMN` has no
/// `IF NOT EXISTS`. Rolling forward again therefore failed with `duplicate column
/// name` and left the database stuck at `m065` with a schema partly at `m066`.
/// The `up` asks first.
const SQLITE_UP_ADD_COLUMN: &str = "ALTER TABLE pricing_price ADD COLUMN unit_rate_nano bigint";

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    // **The column is left in place, deliberately.** `m20260802_000076` rebuilt
    // `pricing_price` ten migrations later and its rebuild carries
    // `chk_pricing_price_unit_rate_nano`; SQLite refuses to drop a column a CHECK
    // still names, and removing a CHECK there needs another whole-table rebuild.
    //
    // Dropping it here was already only half-true after that rebuild: this
    // migration's `up` adds a column to the table *it* knew, and by the time the
    // reverse walk arrives the table belongs to `m076`, whose own `down` is a
    // documented no-op for the same forward-only reason. The reverse walk still
    // reaches `m20260802_000002`, which drops the table outright, so nothing
    // survives a full rollback either way — what would not survive is the walk
    // itself, which is what this statement broke.
    "ALTER TABLE pricing_price_tier_band
        RENAME COLUMN unit_price_nano TO unit_price_minor",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_RENAME).await?;
        // **SQLite only, and the asymmetry is the point.** Postgres needs no
        // guard because its own `down` genuinely drops both the constraint and
        // the column (`PG_DOWN_STATEMENTS`), so a partial rollback there leaves
        // nothing behind to collide with. On SQLite the `down` cannot, so the
        // `up` asks. See `SQLITE_UP_ADD_COLUMN`.
        if manager.get_database_backend() == sea_orm::DatabaseBackend::Sqlite
            && !super::sqlite_column_exists(manager, "pricing_price", "unit_rate_nano").await?
        {
            super::exec_backend(manager, &[], &[SQLITE_UP_ADD_COLUMN]).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
