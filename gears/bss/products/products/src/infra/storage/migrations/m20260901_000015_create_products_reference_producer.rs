//! Create `bss.products_reference_producer` — the registered producer set
//! (`design/07-reference-signal.md` §4, **P-D-03**, **P-D-87**).
//!
//! # The row is the predicate's quantifier and the capture store's ride
//!
//! `(tenant_id, producer)` → a `state` of `registered | retired`, the
//! registration instant, the ceremony that admitted it, and the
//! **declaration payload** — the reserved field where Contracts' own
//! draft-and-quote-counting answer lands at its registration (`PRD` §15).
//! The reference predicate runs over every **registered** row of the
//! tenant, and `06-catalog-version`'s capture store snapshots that set per
//! version so a historical verdict is judged against the then-registered
//! set.
//!
//! # Why the state is a column and not a deletion
//!
//! **P-D-87 arm 2** clears a retired producer's **watermark and member
//! rows** in the retirement transaction — that is what makes
//! *"a registering producer's first watermark starts `never-received`, so
//! onboarding can only tighten"* true rather than merely stated — but the
//! producer row itself stays, its `state` moving to `retired`, so the
//! registration history and the ceremony that admitted it are not lost.
//!
//! # No `pricing` seed here
//!
//! P-D-03 fixes the v1 registered set at `{pricing}`, and a migration is
//! the wrong writer for it: the set is **per tenant** and this gear learns
//! of a tenant when one acts, not when it migrates. The registration door
//! is the writer; a deployment onboarding a tenant registers `pricing`
//! through it.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite` and `timestamptz` becomes `text`, and
//! the `bss.` qualification is dropped. Both CHECKs and the primary key are
//! preserved on both sides.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-producer-table:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_reference_producer (
            tenant_id           uuid        NOT NULL,
            producer            text        NOT NULL,
            state               text        NOT NULL,
            registered_at       timestamptz NOT NULL,
            ceremony_ref        uuid,
            declaration_payload text,
            CONSTRAINT products_reference_producer_pkey PRIMARY KEY (tenant_id, producer),
            CONSTRAINT chk_products_reference_producer_name CHECK (producer <> ''),
            CONSTRAINT chk_products_reference_producer_state CHECK (state IN ('registered', 'retired'))
        )",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.products_reference_producer"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_reference_producer (
            tenant_id           text NOT NULL,
            producer            text NOT NULL,
            state               text NOT NULL,
            registered_at       text NOT NULL,
            ceremony_ref        text,
            declaration_payload text,
            PRIMARY KEY (tenant_id, producer),
            CONSTRAINT chk_products_reference_producer_name CHECK (producer <> ''),
            CONSTRAINT chk_products_reference_producer_state CHECK (state IN ('registered', 'retired'))
        )",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_reference_producer"];

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
