//! `pricing_price`'s reserved-capacity attributes — the pair that makes a usage
//! row a *reserved* one (`design/10-advanced-primitives.md` §6, `inst-rv-attrs`,
//! A1/D-53/D-139).
//!
//! Two columns, and the design set is emphatic that it is two columns **on the
//! usage row** rather than a second row: A1 keeps `(meter, dimensionKey)`
//! injective, so a reservation is an attribute of the line it reserves and never
//! a line of its own. `reserved_rate_minor` is money in the row's currency;
//! `reservation_flavor` is `consumption | capacity`.
//!
//! # No `CHECK` on either engine — `m20260802_000050`'s decision, unchanged
//!
//! §6 states two constraints over this pair: the vocabulary
//! (`consumption | capacity`) and the pairing,
//! `CHECK (reservation_flavor IS NULL) = (reserved_rate_minor IS NULL)`.
//! Neither is expressed here, for that migration's reasons restated rather than
//! referenced:
//!
//! `SQLite` has no incremental form for a table-level `CHECK`. Adding one means
//! rebuilding `pricing_price` — a `CREATE`/`INSERT SELECT`/`DROP`/`RENAME` over
//! every column, index and trigger the chain has attached since
//! `m20260802_000002` — and adding it on Postgres alone would leave the two
//! engines' `EXPECTED_CHECKS` censuses stating different schemas, which is the
//! one thing those censuses exist to make impossible to do silently.
//!
//! So the **vocabulary** lives in the domain, where it is *unrepresentable*
//! rather than merely rejected:
//! [`ReservationFlavor`](crate::domain::price_row::ReservationFlavor) is an enum
//! and this column can only hold what one of its variants renders. That is the
//! argument `m20260802_000037` / `m20260802_000039` / `m20260802_000050` each
//! made for a `text` column whose value set is an enum.
//!
//! The **pairing** is what the domain cannot make unrepresentable while the two
//! halves stay two fields, and they stay two fields because §4.4 classifies them
//! differently: `reserved_rate_minor` is money and sits outside the
//! evaluation-policy roster, `reservation_flavor` decides how the billable
//! quantity is derived (`inst-rv-tier-q`, `inst-rv-level`) and is in it. A
//! single struct field would be one roster entry for two classifications. So the
//! pairing is a publish rule
//! ([`RESERVATION_PAIR_INCOMPLETE`](crate::domain::publish::rules::RESERVATION_PAIR_INCOMPLETE)),
//! which is where §6's other row-shape statements already live.
//!
//! # These two columns **are** in the frozen-column guard
//!
//! Unlike `m20260802_000050`, which declared the gap and left
//! `m20260802_000051` to pay it, the guard restatement lands beside this
//! migration as `m20260802_000055`. The argument for it is the sharpest this
//! table has had: `reserved_rate_minor` is a **rate**, it is authored draft
//! content a `PATCH` moves, and it is inside the approval content pin. A
//! published row whose reserved rate a writer outside this crate changed would
//! diverge from the pin that approved it, silently, and the divergence is money
//! per covered granule (D-139).
//!
//! It is still a migration of its own for `m20260802_000040`'s reason — the
//! Postgres half is a whole-function restatement of
//! `bss.pricing_price_append_only()` and deserves a diff a reviewer can read
//! alone.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price ADD COLUMN reserved_rate_minor bigint",
    "ALTER TABLE bss.pricing_price ADD COLUMN reservation_flavor text",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price DROP COLUMN reservation_flavor",
    "ALTER TABLE bss.pricing_price DROP COLUMN reserved_rate_minor",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_price ADD COLUMN reserved_rate_minor bigint",
    "ALTER TABLE pricing_price ADD COLUMN reservation_flavor text",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_price DROP COLUMN reservation_flavor",
    "ALTER TABLE pricing_price DROP COLUMN reserved_rate_minor",
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
