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
//! # The ref names its subject, and it has to (added 2026-08-03, building the
//! publish commit)
//!
//! `subject_kind` and `subject_ref` are on this table because the projector
//! cannot do its job without them and nothing else can carry them. §4.4 requires
//! the projection at `CatalogVersionPublished` to write "exactly the subjects of
//! the publish units that produced" a version (D-86/D-91) — and the projector
//! arrives holding a batch of **committed refs**. Without a subject on the ref
//! row there is no path at all from a `pending_ref` back to what it published.
//!
//! **Reported as a divergence, not resolved by editing the design set.** §3.7
//! describes this table as "`pending` vs `committed` version linkage per
//! publish" and lists neither column, while the requirement that forces them
//! lives a section away in §4.4's projected-row-set rule.
//!
//! The rejected alternative was the **outbox row**: `pricing_outbox.aggregate_id`
//! is the plan id, its payload carries the pending ref, and nothing deletes
//! outbox rows today. That makes a delivery queue the projector's durable index
//! — one table with two unrelated contracts — and the first compaction of
//! delivered history would silently remove the projector's input. The ref row is
//! the projector's own; nothing drains it.
//!
//! `subject_kind`'s `CHECK` is **the same four tokens** `pricing_read_model`
//! carries, deliberately: the ref names the subject the projector will write,
//! so two vocabularies would be two answers to one question. Both are rendered
//! from `domain::read_model::SubjectKind::as_str`.
//!
//! **One pair, and the multi-subject unit is owed.** A plan publish projects
//! exactly one subject. An overlay publish unit projects **two** — the overlay
//! document and the D-112/D-133 `overlay_index` shard — and one pair of columns
//! cannot hold that. Generalizing today would be building for a unit no code in
//! this repository can produce (there is no overlay store), so the plan case is
//! recorded as one pair and the widening is stated here as owed. While the chain
//! is greenfield that widening is another in-place amendment, which is exactly
//! the situation this file is already an instance of.
//!
//! **Amended in place rather than fixed up.** The chain has never been deployed
//! and every environment and every test creates it from scratch, so a trailing
//! `ALTER TABLE` migration would add a step that only exists to describe a
//! history nothing lived through.
//!
//! **Backend differences.** None beyond the systematic type mirror; both
//! backends express the partial `UNIQUE` and both row `CHECK`s.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_catalog_version_ref (
        tenant_id       uuid        NOT NULL,
        pending_ref     text        NOT NULL,
        subject_kind    text        NOT NULL,
        subject_ref     text        NOT NULL,
        catalog_version bigint,
        requested_at    timestamptz NOT NULL DEFAULT now(),
        committed_at    timestamptz,
        PRIMARY KEY (tenant_id, pending_ref),
        CONSTRAINT chk_pricing_catalog_version_ref_subject_kind CHECK (
            subject_kind IN ('plan','price_overlay','overlay_index','group_membership')),
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
        subject_kind    text   NOT NULL,
        subject_ref     text   NOT NULL,
        catalog_version bigint,
        requested_at    text   NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        committed_at    text,
        PRIMARY KEY (tenant_id, pending_ref),
        CONSTRAINT chk_pricing_catalog_version_ref_subject_kind CHECK (
            subject_kind IN ('plan','price_overlay','overlay_index','group_membership')),
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
