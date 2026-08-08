//! `pricing_price`'s typed minimum-quantity floors and the `discountRef` day-1
//! hook (`design/10-advanced-primitives.md` §6, `inst-ft-typed`,
//! `inst-ft-fallback`, `inst-dr-referential`).
//!
//! Four columns for two unrelated primitives, in one migration because they are
//! four plain `ALTER`s on one table and splitting them would buy a reviewer
//! nothing: neither carries a constraint, a trigger or an index, and the guard
//! restatement they both need is `m20260802_000057`'s either way.
//!
//! # The floor is **two** columns, not one typed column
//!
//! §3's `inst-ft-typed` reads as though `minQtyThreshold` were one value with a
//! type beside it, and `inst-ft-both` settles that it is not: *"Both MAY be set
//! on one row (distinct fields)"*. A `purchase` floor is enforced by
//! Subscriptions at order time and a `usage` floor by Tariffs/Rating at
//! eligibility, so a row can legitimately carry both and they mean different
//! things. One column plus a type enum could hold only one of them, and the
//! authoring surface would have had to refuse the pair the design set permits.
//!
//! That is also what makes "untyped fails publish" (`FLOOR_TYPE_MISSING`)
//! **unrepresentable rather than rejected** for the shape it was written
//! against: there is no way to store a quantity without saying which floor it
//! is, because the column *is* the type. The code is still minted and still
//! carried, for a reason `inst-ft-fallback` supplies below.
//!
//! # `min_qty_usage_fallback` is a third column and the launch value set is one
//!
//! `inst-ft-fallback`: a `usage` floor MUST declare what happens beneath it, and
//! at launch the only supported value is `exception` — the below-floor line
//! fails closed into the rating exception path, "never silently zero-rated and
//! never silently charged". A one-value enum looks like a column that could be a
//! constant, and it is not: the point is that the author **declares** the
//! behaviour, so that the day a second fallback lands every already-published
//! row says which one it chose rather than inheriting a new default. It freezes
//! in the snapshot for the same reason.
//!
//! Its REQUIRED-ness is the rule `FLOOR_FALLBACK_MISSING` carries, and that one
//! is a genuine publish rule rather than a shape fact: a `bigint` and a `text`
//! are independently nullable and no type can pair them.
//!
//! # No `CHECK` on either engine
//!
//! `m20260802_000050`'s decision and `m20260802_000054`'s, unchanged and
//! restated rather than referenced: `SQLite` has no incremental form for a
//! table-level `CHECK`, and adding one on Postgres alone would leave the two
//! engines' `EXPECTED_CHECKS` censuses stating different schemas — the one thing
//! those censuses exist to make impossible to do silently. So the fallback
//! vocabulary lives in
//! [`MinQtyUsageFallback`](crate::domain::price_row::MinQtyUsageFallback), where
//! a column can only hold what a variant renders, and the pairing lives in the
//! publish path.
//!
//! # `discount_ref` validates nothing here, and that is `inst-dr-referential`
//!
//! The column is an opaque reference to an instrument **Promotions owns**. The
//! catalog does not author, evaluate or stack the discount (`inst-dr-boundary`),
//! so there is no vocabulary to constrain and no enum to hold it. A `text`
//! column is the whole of the storage story, and `inst-dr-boundary` is satisfied
//! by that alone: the ref round-trips and nothing in this gear reads it.
//!
//! **`inst-dr-referential` is not built, and it is not buildable here.** It asks
//! that the ref "resolve to a registered external instrument
//! (Promotions/Tariffs-owned)" and refuse publish otherwise
//! (`DISCOUNT_REF_UNRESOLVED`). There is no such registry: S10 §10 records that
//! *"the Promotions PRD still does not exist — `discountRef` is the committed
//! day-1 hook, the durable owner remains Future"*, and no gear in this workspace
//! publishes an instrument catalogue to resolve against.
//!
//! A rule with a stubbed resolver would be worse than its absence in both
//! directions it could be stubbed. Resolving everything makes
//! `DISCOUNT_REF_UNRESOLVED` unreachable while reading as coverage — the
//! vacuous pass this program has paid for repeatedly. Resolving nothing refuses
//! every `discountRef` ever authored, which deletes the day-1 hook the design
//! set committed to. So the column lands, the boundary half holds, and the
//! referential half is named as owed rather than synthesised — the posture
//! D-149 clause 3, D-161 clause 1, D-167 clause 3 and D-168 clause 1 already
//! take for an unlanded counterparty fact.
//!
//! **What that leaves unguarded is worth stating plainly**: a `discount_ref`
//! authored today freezes into an immutable version with nothing having checked
//! it. That is a smaller hazard than D-177's — the ref changes no charge this
//! gear computes, and a dangling one is what the `pricing.discount.ref_dangling`
//! alarm (§7, also unbuilt) exists to surface — but it is the same shape, and
//! the group that lands the resolver owes the backfill.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price ADD COLUMN min_qty_purchase bigint",
    "ALTER TABLE bss.pricing_price ADD COLUMN min_qty_usage bigint",
    "ALTER TABLE bss.pricing_price ADD COLUMN min_qty_usage_fallback text",
    "ALTER TABLE bss.pricing_price ADD COLUMN discount_ref text",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_price DROP COLUMN discount_ref",
    "ALTER TABLE bss.pricing_price DROP COLUMN min_qty_usage_fallback",
    "ALTER TABLE bss.pricing_price DROP COLUMN min_qty_usage",
    "ALTER TABLE bss.pricing_price DROP COLUMN min_qty_purchase",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_price ADD COLUMN min_qty_purchase bigint",
    "ALTER TABLE pricing_price ADD COLUMN min_qty_usage bigint",
    "ALTER TABLE pricing_price ADD COLUMN min_qty_usage_fallback text",
    "ALTER TABLE pricing_price ADD COLUMN discount_ref text",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE pricing_price DROP COLUMN discount_ref",
    "ALTER TABLE pricing_price DROP COLUMN min_qty_usage_fallback",
    "ALTER TABLE pricing_price DROP COLUMN min_qty_usage",
    "ALTER TABLE pricing_price DROP COLUMN min_qty_purchase",
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
