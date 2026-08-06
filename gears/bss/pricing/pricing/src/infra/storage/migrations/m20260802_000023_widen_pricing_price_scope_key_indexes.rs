//! The two scope-key partial `UNIQUE`s gain the usage line — D-196 clause (2).
//!
//! D-103 is normative and was confirmed per-item: *"a `PaaS` plan pricing cloudlets,
//! storage and egress is one plan, not three"*. The canonical scope key carried no
//! `meter` and no `dimensionKey`, so every usage line of one plan in one
//! `(currency, region, phase, priceEligibility, cohort)` rendered **one** key and
//! the second line was refused `DUPLICATE_SCOPE_KEY` at save. `m20260802_000002`'s
//! own module doc records the collision from the other side — its
//! `postgres_schema_price` fixture gives its second `egress` row a different
//! `region` precisely so it will not collide — so the shape was known at the point
//! of writing that test and reached no decision until D-196.
//!
//! Foundation §4.1 now carries the axes; this migration is the physical half.
//!
//! # The column cannot simply be listed, and the naive form fails the wrong way
//!
//! `meter` is **nullable** (`dimension_key` is already `NOT NULL DEFAULT ''`), and
//! inside a `UNIQUE` index NULLs are **distinct**. Adding the bare column would not
//! merely leave usage rows under-constrained — it would **remove the uniqueness
//! these two indexes have today on every non-usage key**, because every recurring,
//! one-time and setup row carries `meter IS NULL` and no two NULLs ever conflict.
//!
//! Measured rather than reasoned about (2026-08-06, SQLite): a naive
//! `UNIQUE (a, meter, dim)` admitted **two** rows on one key with `meter` NULL on
//! both, and the same index over `COALESCE(meter, '')` refused the second while
//! still admitting `cloudlets` and `egress_gb` as two keys. The Postgres proof is
//! `postgres_schema_price::two_meterless_usage_rows_on_one_key_still_collide` and
//! its draft-plane sibling, which exist because a measurement on one engine is not
//! a fact about the other.
//!
//! `''` is safe as the sentinel only because a meter may not *be* blank:
//! `domain::scope_key::Meter::new` refuses a blank value, so the empty string
//! denotes "no meter" and nothing else can render it.
//!
//! # One index per plane, not two split by charge kind
//!
//! The alternative was to split each index in two — the eight axes
//! `WHERE charge_kind <> 'usage'`, the ten `WHERE charge_kind = 'usage'` — which
//! would have left four partial indexes where the design set describes two, and
//! four predicates to keep disjoint instead of two. Under the sentinel a non-usage
//! row keys on `('', '')` and the index degenerates to exactly today's behaviour
//! for it, which is what makes the single form correct rather than merely shorter.
//!
//! # `uq_pricing_price_meter_line_current` is untouched, and is not subsumed
//!
//! That index deliberately excludes `charge_kind` (D-43/D-103's per-line reading),
//! so it forbids one `(meter, dimensionKey)` line being priced twice across
//! *different* charge kinds — which the scope key, carrying `charge_kind`, still
//! permits. Its `WHERE meter IS NOT NULL` conjunct is also the one place a NULL
//! `meter` carries meaning: "not a metered line" as against "a line with no meter".
//! Folding the two would destroy that distinction, so the two overlap without
//! either implying the other.
//!
//! # Why a migration of its own
//!
//! The chain's rule, as `m20260802_000018` and `m20260802_000021` state it:
//! `m20260802_000002` is what the price store was when it was created, and a reader
//! asking when a usage line became a key gets a dated answer rather than a
//! `git blame`. The `down` restores `m20260802_000002`'s exact index text, so the
//! round trip is byte-for-byte that migration's.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------
//
// An index cannot be altered into a different column list, so each is dropped and
// recreated. The drop names the schema; the create does not, because the index
// inherits the schema of the table it is on.

const PG_UP_STATEMENTS: &[&str] = &[
    "DROP INDEX bss.uq_pricing_price_scope_key_current",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current
        ON bss.pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort,
            COALESCE(meter, ''), dimension_key)
        WHERE lifecycle_state = 'published'",
    "DROP INDEX bss.uq_pricing_price_scope_key_draft",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_draft
        ON bss.pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort,
            COALESCE(meter, ''), dimension_key)
        WHERE lifecycle_state = 'draft'",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX bss.uq_pricing_price_scope_key_current",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current
        ON bss.pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort)
        WHERE lifecycle_state = 'published'",
    "DROP INDEX bss.uq_pricing_price_scope_key_draft",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_draft
        ON bss.pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort)
        WHERE lifecycle_state = 'draft'",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// The systematic transform is the usual one: no schema prefix. Expression indexes
// and partial indexes are both supported here, so the two forms differ in nothing
// else - which matters, because the fast suite is the gate that runs on every
// increment and it would otherwise be testing a different guarantee from the one
// production has.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "DROP INDEX uq_pricing_price_scope_key_current",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current
        ON pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort,
            COALESCE(meter, ''), dimension_key)
        WHERE lifecycle_state = 'published'",
    "DROP INDEX uq_pricing_price_scope_key_draft",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_draft
        ON pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort,
            COALESCE(meter, ''), dimension_key)
        WHERE lifecycle_state = 'draft'",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX uq_pricing_price_scope_key_current",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_current
        ON pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort)
        WHERE lifecycle_state = 'published'",
    "DROP INDEX uq_pricing_price_scope_key_draft",
    "CREATE UNIQUE INDEX uq_pricing_price_scope_key_draft
        ON pricing_price (
            tenant_id, plan_id, currency, region, price_overlay,
            phase, price_eligibility, charge_kind, cohort)
        WHERE lifecycle_state = 'draft'",
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
