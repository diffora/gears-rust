//! `reserved_rate_minor` becomes `reserved_rate_nano`, and its values are rescaled.
//!
//! The third column D-311 should have moved and did not. That decision split
//! rates out of amounts because *"a metered rate routinely prices below the
//! currency's minor unit"*, and truncation *"collapsed a `0.0150 / 0.0110` ladder
//! and a `0.0230 / 0.0120` ladder both to `0.01` at every band, so two different
//! tariffs became the same tariff on rows that looked well-formed."* It moved
//! `pricing_price_tier_band.unit_price_minor` and added `pricing_price
//! .unit_rate_nano` (`m20260802_000066`), and it left this one alone — because its
//! census enumerated references to `unit_price_minor`, and this column is not one
//! of them. That is the wrong-operand census D-323 already records as having cost
//! one site; this is the second.
//!
//! **That `reservedRate` is a rate is not an inference.** PRD §2674 calls it a
//! committed *unit price*; the `capacity` flavor accrues it **per covered
//! granule**; the conformance oracle multiplies it by a granule count. The
//! migration that created it (`m20260802_000054`) says so too, and contradicts
//! itself doing it — its opening paragraph calls the column *"money in the row's
//! currency"* and its §4.4 paragraph calls it *"a **rate**"*. The second reading
//! is the right one and this migration settles the disagreement in the schema.
//!
//! **What the old type made impossible.** In whole minor units the smallest
//! expressible non-zero value is one minor unit — one cent on USD. A reserved
//! capacity billed per second (`max_hold_granules`' own doc names `per_second` as
//! the granularity that motivated widening *that* column to `bigint`) at
//! `$0.0000166667` per GB-second is `0.00166667` minor units, so the author could
//! submit `0` or `1`, the latter being 600x the intended rate. Not truncated:
//! unrepresentable.
//!
//! # Why this rescales and `m20260802_000066` did not
//!
//! The sibling rename carried no `UPDATE`: it renamed `unit_price_minor` to
//! `unit_price_nano` and let the reinterpretation stand, which is sound when the
//! column holds nothing. This one multiplies, because the column has been
//! writable since `m20260802_000054` and a stand carrying a single authored
//! reservation would otherwise have its rate divided by 10^9 in silence — the
//! failure mode is a reserved rate that reads as zero, which is exactly the
//! shape a review would not catch by looking at a plausible number. `bigint`
//! holds the product for any minor value below 9.2e9, far past any rate.
//!
//! `RENAME COLUMN` carries `CHECK` and trigger references with it on both
//! engines, which is the property `m20260802_000066` relies on and
//! `sqlite_migrations` re-pins by digest — so the four frozen-column guards that
//! name this column (`m20260802_000055`, `_000057`, `_000069`, `_000076`) keep
//! working under the new name without being restated.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price
        RENAME COLUMN reserved_rate_minor TO reserved_rate_nano",
    "UPDATE bss.pricing_price
        SET reserved_rate_nano = reserved_rate_nano * 1000000000
        WHERE reserved_rate_nano IS NOT NULL",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "UPDATE bss.pricing_price
        SET reserved_rate_nano = reserved_rate_nano / 1000000000
        WHERE reserved_rate_nano IS NOT NULL",
    "ALTER TABLE bss.pricing_price
        RENAME COLUMN reserved_rate_nano TO reserved_rate_minor",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_price
        RENAME COLUMN reserved_rate_minor TO reserved_rate_nano",
    "UPDATE pricing_price
        SET reserved_rate_nano = reserved_rate_nano * 1000000000
        WHERE reserved_rate_nano IS NOT NULL",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "UPDATE pricing_price
        SET reserved_rate_nano = reserved_rate_nano / 1000000000
        WHERE reserved_rate_nano IS NOT NULL",
    "ALTER TABLE pricing_price
        RENAME COLUMN reserved_rate_nano TO reserved_rate_minor",
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
