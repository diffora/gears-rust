//! Create `bss.products_read_stamp` — the `StalenessStamp`'s per-tenant row
//! (**P-D-70** arm 6; `design/08-read-models.md` C3, `inst-rb-stamp`;
//! **P-D-07**).
//!
//! # One row per tenant, and the alternatives each fail a measured case
//!
//! P-D-70 arm 6 settles the shape: *"The `StalenessStamp` persists as one
//! per-tenant stamp row"*, carrying the last `catalog_version_id` and
//! `projectedAt`. Its two rejected arms are rejected for reasons the schema
//! has to honour — *"a column duplicated on every projection row cannot
//! answer an **empty** projection — the anchorless rebuild's own arm — and
//! derivation from the consumer checkpoint ties response metadata to broker
//! internals"*. So the key is `(tenant_id)` alone, and the table is
//! addressable with no projection row in existence.
//!
//! # `catalog_version_id` is nullable, and that is the anchorless arm
//!
//! A tenant that has published no catalog version has no anchor, and its
//! bootstrap is still an apply that stamps: `projectedAt` advances *"on every
//! projector apply, version or none"* (arm 3). A `NOT NULL` column would
//! force a sentinel, and `features/read-models.md` §6 makes the distinction
//! a probe — *"A zero-version tenant's response carries `asOfCatalogVersion =
//! null` **and** a `projectedAt`. A response omitting the field fails:
//! absence is indistinguishable from a dropped stamp."*
//!
//! # No FK on `catalog_version_id`, and no guard
//!
//! No FK: `products_catalog_version` is 06's and the stamp must be writable
//! during an anchorless rebuild, where the value is `NULL` and the reference
//! resolves to nothing by construction. No append-only guard: this is the
//! same rebuildable family as `products_read_entity` (§4 — *"rebuildable
//! state, not records"*), and the projector overwrites this row on every
//! apply. A guard here would refuse the table's only write.
//!
//! # The stamp is a floor, and the schema cannot say so — the code must
//!
//! **P-D-07** makes it a floor rather than a completeness claim, and no
//! column expresses that: a later entity event may add, change **or remove**
//! content relative to a stamp that does not move. `domain::read_model`'s
//! [`StalenessStamp`](crate::domain::read_model::StalenessStamp) carries the
//! reading, and the retirement-flip case is where the completeness reading
//! shows up as a false corruption alarm.
//!
//! # Backend differences
//!
//! `uuid` becomes `text`, `timestamptz` becomes `text`, and the `bss.`
//! qualification is dropped. The key is preserved on both sides.
//!
//! @cpt-cf-bss-products-dod-staleness-stamp

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &["CREATE TABLE bss.products_read_stamp (
            tenant_id          uuid        NOT NULL,
            catalog_version_id uuid,
            projected_at       timestamptz NOT NULL,
            CONSTRAINT products_read_stamp_pkey PRIMARY KEY (tenant_id)
        )"];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.products_read_stamp"];

const SQLITE_UP_STATEMENTS: &[&str] = &["CREATE TABLE products_read_stamp (
            tenant_id          text NOT NULL,
            catalog_version_id text,
            projected_at       text NOT NULL,
            PRIMARY KEY (tenant_id)
        )"];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_read_stamp"];

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
