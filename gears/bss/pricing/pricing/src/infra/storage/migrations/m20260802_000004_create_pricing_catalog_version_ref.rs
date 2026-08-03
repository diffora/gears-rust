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
//! One guard. `chk_pricing_catalog_version_ref_commit` keeps the commit atomic
//! in the row: the version and the commit instant are set together or not at
//! all, so no row can claim a version with no record of when it was assigned.
//! It is what stops a half-finalize, and it is untouched.
//!
//! # The version index is **not** unique, and the unique one it replaces made
//! batching unbuildable (amended 2026-08-03, building the projector)
//!
//! This table carried
//! `uq_pricing_catalog_version_ref_version (tenant_id, catalog_version) WHERE
//! catalog_version IS NOT NULL`, on the argument that it "makes the mapping a
//! bijection per tenant — two pending handles resolving to one committed
//! version would make 'which publish produced this version' unanswerable".
//!
//! **Under batching that index refuses the normal case.** §4.2 step 5 and §3.6:
//! the registry is the sole incrementer and **batches approved publishes**
//! before emitting `CatalogVersionPublished`. `../PRD.md` says the consequence
//! without room for reading — "under batching a version bundles many
//! `PlanPublished` events, so there is no per-`PlanPublished` marker" — and
//! §4.4 and D-101 speak throughout of "**every** subject row that version
//! projects", plural, while D-157 has the projector arriving "holding
//! **committed refs**", plural. Two publishes of one tenant landing in one
//! batch is the case the whole D-47 batching model exists to serve, and the
//! unique index made the second finalize fail.
//!
//! The purpose it was defending is already served by something better. D-157
//! put `subject_kind` / `subject_ref` on this row, so "which publish produced
//! this version" is answered by the row itself — and the honest answer is
//! legitimately a **set**, which is exactly what the projected-row-set rule
//! requires it to be. What must stay one-way is the *finalize*, and that is a
//! property of the compare-and-swap in
//! [`catalog_version_ref_repo`](crate::infra::storage::repo::catalog_version_ref_repo)
//! plus the `CHECK` above, not of a uniqueness constraint over versions.
//!
//! `idx_pricing_catalog_version_ref_version` replaces it, non-unique, over the
//! same `(tenant_id, catalog_version)` columns and with no partial predicate:
//! the projector reads every ref of a version and the frontier walk reads the
//! smallest committed version above a watermark, so the index earns its place
//! on reads it is now the only support for.
//!
//! **Reported as a divergence, not resolved by editing the design set.** No
//! section states the bijection; the index asserted it, and the batching rule
//! two sections away contradicts it.
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
//! # The ref names the revision its publish judged (added 2026-08-03)
//!
//! `subject_revision` is here for the same shape of reason `subject_kind` and
//! `subject_ref` are, and it closes a defect that was reachable rather than
//! hypothetical. §4.4 says the projection source is the plan's **current**
//! revision, and the projector arrives up to the D-47 maximum batching delay —
//! **five minutes** — after the commit. A second publish of the same plan
//! inside that window makes its own revision current, so projecting from
//! "current" freezes content the earlier version's publish never judged. The
//! damage is permanent: a delta row is INSERT-only on the seven-year truth
//! horizon, in a store whose entire contract is that a completed version never
//! changes.
//!
//! The revision is a property of the **publish unit**, not of the subject's
//! identity, so carrying it here disturbs nothing about D-86/D-91's subject
//! typing — that keying is `pricing_read_model`'s, and it is untouched.
//!
//! Nullable, because only a revisioned subject kind has one; a `plan` subject
//! arriving without it is refused by the projector rather than defaulted, a
//! default being a guess about which content a frozen version froze.
//!
//! `subject_lifecycle_state` is the same fact one field over, and it closes the
//! residue pinning the revision left. Reading the pinned revision's state as it
//! **now** stands lets a third value into a delta: `superseded`, which
//! `plan_repo::load_current` could never return and which D-128 does not
//! contemplate for a projected subject — its clause names `published` **or**
//! `retired`, and D-90 calls "the published revision" the input to sellability
//! predicate (4), so a consumer coding that predicate as "is published" reads
//! the version as unsellable. Permanently, on an INSERT-only row.
//!
//! So the state the publish **judged** is frozen with the revision it judged,
//! and the `CHECK` admits exactly the two tokens D-128 sanctions. That is
//! coherent with the rest of the design rather than an exception to it:
//! retirement is a publish unit of its own (D-128) that re-projects the plan
//! subject, so a `retired` state arrives carrying **its own** version instead of
//! leaking backwards into an older one. Every unit that re-projects a plan
//! subject knows this value at commit — the publish commit after its own flip,
//! retirement after its own, a D-99 window mutation from the revision it read —
//! so there is no unit for which the column is unknowable.
//!
//! **Reported as a divergence.** §3.7 lists neither this column nor the two
//! D-157 added, and §4.4's "current revision" rule is what it contradicts.
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
//! backends express the version index and all four row `CHECK`s.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_catalog_version_ref (
        tenant_id       uuid        NOT NULL,
        pending_ref     text        NOT NULL,
        subject_kind    text        NOT NULL,
        subject_ref     text        NOT NULL,
        subject_revision bigint,
        subject_lifecycle_state text,
        catalog_version bigint,
        requested_at    timestamptz NOT NULL DEFAULT now(),
        committed_at    timestamptz,
        PRIMARY KEY (tenant_id, pending_ref),
        CONSTRAINT chk_pricing_catalog_version_ref_subject_kind CHECK (
            subject_kind IN ('plan','price_overlay','overlay_index','group_membership')),
        CONSTRAINT chk_pricing_catalog_version_ref_commit CHECK (
            (catalog_version IS NULL) = (committed_at IS NULL)),
        CONSTRAINT chk_pricing_catalog_version_ref_version CHECK (
            catalog_version IS NULL OR catalog_version >= 0),
        CONSTRAINT chk_pricing_catalog_version_ref_subject_revision CHECK (
            subject_revision IS NULL OR subject_revision >= 0),
        CONSTRAINT chk_pricing_catalog_version_ref_subject_lifecycle CHECK (
            subject_lifecycle_state IS NULL
            OR subject_lifecycle_state IN ('published','retired'))
    )",
    "CREATE INDEX idx_pricing_catalog_version_ref_version
        ON bss.pricing_catalog_version_ref (tenant_id, catalog_version)",
];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.pricing_catalog_version_ref"];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_catalog_version_ref (
        tenant_id       text   NOT NULL,
        pending_ref     text   NOT NULL,
        subject_kind    text   NOT NULL,
        subject_ref     text   NOT NULL,
        subject_revision bigint,
        subject_lifecycle_state text,
        catalog_version bigint,
        requested_at    text   NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        committed_at    text,
        PRIMARY KEY (tenant_id, pending_ref),
        CONSTRAINT chk_pricing_catalog_version_ref_subject_kind CHECK (
            subject_kind IN ('plan','price_overlay','overlay_index','group_membership')),
        CONSTRAINT chk_pricing_catalog_version_ref_commit CHECK (
            (catalog_version IS NULL) = (committed_at IS NULL)),
        CONSTRAINT chk_pricing_catalog_version_ref_version CHECK (
            catalog_version IS NULL OR catalog_version >= 0),
        CONSTRAINT chk_pricing_catalog_version_ref_subject_revision CHECK (
            subject_revision IS NULL OR subject_revision >= 0),
        CONSTRAINT chk_pricing_catalog_version_ref_subject_lifecycle CHECK (
            subject_lifecycle_state IS NULL
            OR subject_lifecycle_state IN ('published','retired'))
    )",
    "CREATE INDEX idx_pricing_catalog_version_ref_version
        ON pricing_catalog_version_ref (tenant_id, catalog_version)",
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
