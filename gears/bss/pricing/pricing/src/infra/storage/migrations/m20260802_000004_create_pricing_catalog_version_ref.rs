//! Create `bss.pricing_catalog_version_ref` — the **pending vs committed**
//! `CatalogVersion` linkage, one row per publish
//! (`design/01-foundation.md` §3.7).
//!
//! The registry (Product & SKU) is the sole incrementer and batches approved
//! publishes (D-47), so between the publish commit and
//! `CatalogVersionPublished` a publish genuinely has an identity — the
//! registry's **pending handle** — with no version number yet. This table is
//! where that handle lives and where it is later resolved to the committed
//! version, which is what lets `pricingSnapshotRef` be stamped at publish and
//! finalized afterwards.
//!
//! Two guards. `chk_pricing_catalog_version_ref_commit` keeps the commit
//! atomic in the row: the version and the commit instant are set together or
//! not at all, so no row can claim a version with no record of when it was
//! assigned. `uq_pricing_catalog_version_ref_version` makes the mapping a
//! bijection per tenant — two pending handles resolving to one committed
//! version would make "which publish produced this version" unanswerable, and
//! finalization is one-way precisely so an already-posted period's pin cannot
//! be re-pointed.
//!
//! **Backend differences.** None beyond the systematic type mirror; both
//! backends express the partial `UNIQUE` and the row `CHECK`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_catalog_version_ref (
        tenant_id       uuid        NOT NULL,
        pending_ref     text        NOT NULL,
        catalog_version bigint,
        requested_at    timestamptz NOT NULL DEFAULT now(),
        committed_at    timestamptz,
        PRIMARY KEY (tenant_id, pending_ref),
        CONSTRAINT chk_pricing_catalog_version_ref_commit CHECK (
            (catalog_version IS NULL) = (committed_at IS NULL)),
        CONSTRAINT chk_pricing_catalog_version_ref_version CHECK (
            catalog_version IS NULL OR catalog_version >= 0)
    )",
    "CREATE UNIQUE INDEX uq_pricing_catalog_version_ref_version
        ON bss.pricing_catalog_version_ref (tenant_id, catalog_version)
        WHERE catalog_version IS NOT NULL",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.pricing_catalog_version_ref"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_catalog_version_ref (
        tenant_id       text   NOT NULL,
        pending_ref     text   NOT NULL,
        catalog_version bigint,
        requested_at    text   NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        committed_at    text,
        PRIMARY KEY (tenant_id, pending_ref),
        CONSTRAINT chk_pricing_catalog_version_ref_commit CHECK (
            (catalog_version IS NULL) = (committed_at IS NULL)),
        CONSTRAINT chk_pricing_catalog_version_ref_version CHECK (
            catalog_version IS NULL OR catalog_version >= 0)
    )",
    "CREATE UNIQUE INDEX uq_pricing_catalog_version_ref_version
        ON pricing_catalog_version_ref (tenant_id, catalog_version)
        WHERE catalog_version IS NOT NULL",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_catalog_version_ref"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
