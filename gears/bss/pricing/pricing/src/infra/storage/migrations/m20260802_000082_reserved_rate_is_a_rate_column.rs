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

/// The rename, then the guard repair, then the rescale behind a stood-down guard.
///
/// **All three, and in that order, because the rescale cannot run otherwise**
/// (review P-2). `pricing_price_append_only()` freezes `reserved_rate_*` on every
/// row whose `lifecycle_state` is not `draft`, and `RENAME COLUMN` does not
/// rewrite `pg_proc.prosrc` — so on a database holding one published row with a
/// reserved rate, the `UPDATE` below used to raise
/// `42703 record "new" has no field "reserved_rate_minor"` and abort the whole
/// chain. The gear could not boot against such a database, and no test saw it:
/// every migration case in the suite applies the chain to an empty schema.
const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price
        RENAME COLUMN reserved_rate_minor TO reserved_rate_nano",
    // The guard now names a column that exists. Applied here rather than left to
    // `m20260802_000086`, which is four migrations too late to help this one.
    "",
    // Stood down for the rescale itself, which is a migration writing a frozen
    // column *on purpose*. `DISABLE TRIGGER` and not a `WHERE lifecycle_state =
    // 'draft'` filter: filtering would leave every published row a billion times
    // too small, which is the same corruption one step quieter.
    "ALTER TABLE bss.pricing_price DISABLE TRIGGER trg_pricing_price_append_only",
    "UPDATE bss.pricing_price
        SET reserved_rate_nano = reserved_rate_nano * 1000000000
        WHERE reserved_rate_nano IS NOT NULL",
    "ALTER TABLE bss.pricing_price ENABLE TRIGGER trg_pricing_price_append_only",
];

/// **`down` refuses rather than truncates** (review P-4).
///
/// `reserved_rate_nano / 1000000000` is integer division on both engines, and the
/// values it meets are precisely the ones this migration exists to make
/// representable: a rate of `$0.0000166667` per GB-second is `1_666_670`, and
/// `1666670 / 1000000000` is `0` — "$0.00 per unit". Re-applying `up` multiplies
/// that `0` back to `0`, so a `down`-then-`up` round trip silently zeroed every
/// sub-minor rate. `down_then_up_round_trips` could not see it: it round-trips an
/// empty schema.
///
/// The header of this migration argues at length that a rate "divided by 10^9 in
/// silence" is the failure it exists to prevent. Its own `down` performed exactly
/// that. Refusing is the honest reversal: a value that cannot survive the trip is
/// an operator decision, not something a migration may make quietly.
const PG_DOWN_STATEMENTS: &[&str] = &[
    "DO $$ BEGIN
        IF EXISTS (SELECT 1 FROM bss.pricing_price
                   WHERE reserved_rate_nano IS NOT NULL
                     AND reserved_rate_nano % 1000000000 <> 0) THEN
            RAISE EXCEPTION 'm20260802_000082 down would truncate a sub-minor reserved rate to zero; rescale or clear those rows before reversing';
        END IF;
    END $$",
    "ALTER TABLE bss.pricing_price DISABLE TRIGGER trg_pricing_price_append_only",
    "UPDATE bss.pricing_price
        SET reserved_rate_nano = reserved_rate_nano / 1000000000
        WHERE reserved_rate_nano IS NOT NULL",
    "ALTER TABLE bss.pricing_price ENABLE TRIGGER trg_pricing_price_append_only",
    "ALTER TABLE bss.pricing_price
        RENAME COLUMN reserved_rate_nano TO reserved_rate_minor",
    // Postgres does **not** rewrite a function body on a column rename, so the
    // guard left standing here would keep naming `reserved_rate_nano` and error
    // `record "new" has no field` on the next update of the table. That fails
    // closed rather than open, which is why it is less dangerous than the
    // `SQLite` half — and still wrong. The slot below is `m086`'s own body with
    // the column renamed in it; see `down`.
    "",
];

/// The mirror of [`PG_UP_STATEMENTS`], and the shape differs because the engines
/// do: `SQLite` has no `DISABLE TRIGGER`, so the guard is dropped and recreated.
///
/// `RENAME COLUMN` *does* rewrite a trigger body on `SQLite`, so the failure here
/// was not the missing field but the guard doing its job:
/// `trg_pricing_price_frozen_columns` refused the rescale with *"row … is
/// published; price, scope, model and entity-tag columns are immutable"*.
const SQLITE_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_price
        RENAME COLUMN reserved_rate_minor TO reserved_rate_nano",
    "DROP TRIGGER IF EXISTS trg_pricing_price_frozen_columns",
    "UPDATE pricing_price
        SET reserved_rate_nano = reserved_rate_nano * 1000000000
        WHERE reserved_rate_nano IS NOT NULL",
    // Recreated from `m20260802_000086`'s statements rather than from a copy of
    // the body here: it is fifty columns long, and a second copy would be free to
    // drift from the one the digest pin measures.
    "",
];

/// [`PG_DOWN_STATEMENTS`]' mirror. `SQLite` has no `RAISE` outside a trigger, so
/// the refusal is a `CHECK` the guard row cannot satisfy when any rate is
/// sub-minor — an abort with a named table rather than a silent truncation.
const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "CREATE TABLE m82_down_would_truncate (ok INTEGER NOT NULL CHECK (ok = 1))",
    "INSERT INTO m82_down_would_truncate (ok)
        SELECT CASE WHEN EXISTS (SELECT 1 FROM pricing_price
                                 WHERE reserved_rate_nano IS NOT NULL
                                   AND reserved_rate_nano % 1000000000 <> 0)
                    THEN 0 ELSE 1 END",
    "DROP TABLE m82_down_would_truncate",
    "DROP TRIGGER IF EXISTS trg_pricing_price_frozen_columns",
    "UPDATE pricing_price
        SET reserved_rate_nano = reserved_rate_nano / 1000000000
        WHERE reserved_rate_nano IS NOT NULL",
    // The guard goes back on **before** the rename, not after, and that ordering
    // is the whole trick: `RENAME COLUMN` rewrites a trigger body on `SQLite`
    // (this file's own `SQLITE_UP` doc records that as the thing that bit it), so
    // a guard recreated here naming `reserved_rate_nano` is rewritten to
    // `reserved_rate_minor` by the statement below. One source for a fifty-column
    // body, and no copy of it to drift.
    //
    // It was missing entirely until 2026-08-19, so every version from `m081` down
    // to `m070` had a `pricing_price` with no append-only guard at all — a
    // published row's price, scope and entity-tag columns mutable by any
    // statement. `down_then_up_round_trips` could not see it: it walks to the
    // bottom, where `m002` drops the table, so the window closed before anything
    // looked.
    "",
    "ALTER TABLE pricing_price
        RENAME COLUMN reserved_rate_nano TO reserved_rate_minor",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The empty slots above are where `m20260802_000086`'s guard repair goes:
        // on Postgres before the rescale (the body must stop naming the removed
        // column), on `SQLite` after it (the trigger is dropped for the write and
        // put back corrected). Spliced rather than copied - one source for a body
        // fifty columns long.
        let repair =
            super::m20260802_000086_guard_pricing_price_reserved_rate_nano::PG_UP_STATEMENTS;
        let mut pg: Vec<&str> = Vec::new();
        for statement in PG_UP_STATEMENTS {
            if statement.is_empty() {
                pg.extend(repair.iter().copied());
            } else {
                pg.push(statement);
            }
        }
        let repair =
            super::m20260802_000086_guard_pricing_price_reserved_rate_nano::SQLITE_UP_STATEMENTS;
        let mut sqlite: Vec<&str> = Vec::new();
        for statement in SQLITE_UP_STATEMENTS {
            if statement.is_empty() {
                sqlite.extend(repair.iter().copied());
            } else {
                sqlite.push(statement);
            }
        }
        super::exec_backend(manager, &pg, &sqlite).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The mirror of `up`'s splice, and it was missing: `down` passed the raw
        // arrays, so the guard this migration drops to make room for the rescale
        // was never put back on either engine.
        //
        // The two engines take different bodies for the reason the statement
        // comments give. `SQLite` gets `m086`'s verbatim and lets the rename that
        // follows rewrite it; Postgres gets the same body with the column renamed
        // in it, because a rename there leaves a function body alone.
        let pg_repair: Vec<String> =
            super::m20260802_000086_guard_pricing_price_reserved_rate_nano::PG_DOWN_STATEMENTS
                .iter()
                .map(|statement| statement.replace("reserved_rate_nano", "reserved_rate_minor"))
                .collect();
        let mut pg: Vec<&str> = Vec::new();
        for statement in PG_DOWN_STATEMENTS {
            if statement.is_empty() {
                pg.extend(pg_repair.iter().map(String::as_str));
            } else {
                pg.push(statement);
            }
        }
        let sqlite_repair =
            super::m20260802_000086_guard_pricing_price_reserved_rate_nano::SQLITE_DOWN_STATEMENTS;
        let mut sqlite: Vec<&str> = Vec::new();
        for statement in SQLITE_DOWN_STATEMENTS {
            if statement.is_empty() {
                sqlite.extend(sqlite_repair.iter().copied());
            } else {
                sqlite.push(statement);
            }
        }
        super::exec_backend(manager, &pg, &sqlite).await
    }
}
