//! `pricing_migration.source_revision` and
//! `pricing_snapshot_provenance.source_revision` become `bigint` — review finding
//! **Z6-7**, the last two `integer` revision columns in the chain.
//!
//! # The outlier
//!
//! A plan revision is a `u64` everywhere it is a value: `pricing_plan.revision`
//! is `bigint` (`m20260802_000001`), and so is every column in the chain that
//! carries one — `subject_revision` on the ref tables, `plan_revision` on the
//! eleven revision-scoped children, `source_revision` on the outbox payload.
//! These two were `integer`, i.e. addressable up to 2^31-1 where the value's own
//! type reaches 2^64-1.
//!
//! It was **guarded rather than broken**, and that is why the review filed it Low:
//! `migration_repo::insert_or_load` and `synthesis_repo::freeze_or_load` each
//! narrowed with an `i32::try_from` and answered `CorruptRow` on failure, so the
//! consequence was a fail-closed refusal and never a truncated revision. The
//! narrowing is what goes away with the column — both sites now guard on
//! `i64::try_from`, which is the column's own range rather than a third one.
//!
//! # A rebuild has already passed over this without fixing it
//!
//! `m20260802_000065` rebuilt `pricing_migration` on `SQLite` to widen its key by
//! tenant, and **restated `source_revision integer NOT NULL` verbatim** in the
//! rebuilt table. That is the shape worth naming: a migration that re-types a
//! whole table is the cheapest possible moment to correct a column, and a
//! verbatim restatement carries the outlier forward invisibly. The entity offers
//! no clue either — `entity/migration.rs` said `i32` and a reader had to reach the
//! DDL to learn why.
//!
//! # Postgres does the work; `SQLite` has nothing to do, and that is not laziness
//!
//! On Postgres `integer` and `bigint` are different types with different widths,
//! and `ALTER COLUMN … TYPE bigint` is the whole change: the widening is
//! unconditionally safe (every `int4` is an `int8`), it rewrites the table but
//! validates nothing, and no CHECK, index or trigger on either table mentions the
//! column in a way a type change invalidates —
//! `chk_pricing_migration_source_revision` (`>= 0`) and
//! `chk_pricing_snapshot_provenance_revision` (`IS NULL OR >= 0`) hold over the
//! wider type unchanged, and `idx_pricing_migration_source` is rebuilt by the
//! `ALTER` itself.
//!
//! On `SQLite` the two spellings **are the same type**. Affinity is assigned by
//! substring: any declared type containing `INT` takes INTEGER affinity, and a
//! `SQLite` integer is stored in up to 8 bytes regardless of what the column was
//! declared as. So `integer` there already addressed the whole `u64` range that
//! fits in an `i64`, the narrowing never existed on that engine, and the only thing
//! a `SQLite` arm could change is the word in `sqlite_master`. That word would cost
//! a full table rebuild — `SQLite` cannot `ALTER COLUMN` — restating
//! `pricing_migration`'s three indexes and five triggers, which is real risk bought
//! for a cosmetic diff. The arm is therefore **empty on purpose**, and the claim is
//! measured rather than asserted:
//! `sqlite_migration_repo::a_revision_beyond_the_old_columns_range_round_trips` and
//! `sqlite_snapshot_provenance_store::a_revision_beyond_the_old_columns_range_round_trips`
//! write a revision above `i32::MAX` through the repositories and read it back, and
//! both were RED before this migration because the `i32::try_from` above refused
//! them.
//!
//! **This is the one place in the chain where an empty arm is right**, so it is
//! worth stating why the usual rule does not apply: elsewhere a missing `SQLite`
//! statement means the mirror drifts from the canonical schema and the fast tier
//! stops measuring what production runs. Here the mirror does not drift — the two
//! declarations denote one type on that engine — and what the fast tier measures is
//! the behaviour, which is identical before and after. The Postgres half of the
//! same claim is `postgres_migrations::every_revision_column_is_bigint`, which reads
//! `information_schema` over the whole schema rather than trusting this file — and
//! which is stated over every `…_revision` column rather than over these two,
//! because a spot check on the known outliers would be green against the next one.
//!
//! # No backfill, no roster line
//!
//! Widening a column changes no value, so there is nothing to backfill: every
//! existing `int4` is already a valid `int8` and the `ALTER` carries the rows across
//! itself. No constraint name and no index name changes, so neither engine's
//! roster in `tests/*_migrations.rs` gains or loses an entry.
//!
//! # The `down` narrows, and can fail
//!
//! `down` is `ALTER COLUMN … TYPE integer`, the exact inverse of `up` — and unlike
//! `up` it is not unconditionally safe: a row written above 2^31-1 while the column
//! was wide makes it fail, loudly, with Postgres' own out-of-range error. That is
//! the correct behaviour for a narrowing rollback and it is stated here rather than
//! worked around: a `down` that silently truncated a plan revision would corrupt
//! the migration schedule's source pointer, and `USING source_revision::integer`
//! would do exactly that on overflow. Nothing is added to make it succeed.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_migration
        ALTER COLUMN source_revision TYPE bigint",
    "ALTER TABLE bss.pricing_snapshot_provenance
        ALTER COLUMN source_revision TYPE bigint",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_snapshot_provenance
        ALTER COLUMN source_revision TYPE integer",
    "ALTER TABLE bss.pricing_migration
        ALTER COLUMN source_revision TYPE integer",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

/// Empty, and the module doc carries the argument: on `SQLite`, `integer` and
/// `bigint` denote one type — INTEGER affinity, up to 8 bytes — so there is no
/// narrowing on that engine to widen, and the two behavioural probes named in the
/// module doc measure that rather than assuming it.
const SQLITE_UP_STATEMENTS: &[&str] = &[];

/// Empty for `SQLITE_UP_STATEMENTS`' reason, which also makes the pair exact
/// inverses of each other on this engine.
const SQLITE_DOWN_STATEMENTS: &[&str] = &[];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
