//! `pricing_catalog_version_ref`'s primary key gains the subject — one pending
//! handle, every subject the publish unit that requested it projects (D-234).
//!
//! `m20260802_000004` keyed this table `(tenant_id, pending_ref)`, which is one row
//! per handle, and its own module doc records why that is not the general shape:
//!
//! > **One pair, and the multi-subject unit is owed.** A plan publish projects
//! > exactly one subject. An overlay publish unit projects **two** — the overlay
//! > document and the D-112/D-133 `overlay_index` shard — and one pair of columns
//! > cannot hold that.
//!
//! It is three when a revision moves the scope value: D-133 says a commit rewrites
//! *"exactly one shard (two when a revision moves the scope value)"*, beside the
//! overlay document itself.
//!
//! # Two handles is not the way out, and the reason is a pin
//!
//! The obvious alternative is one handle per subject — no key change, two registry
//! requests, two rows. It fails on the thing the index exists for. D-112 added the
//! `overlay_index` subject as evaluation's **enumeration access path**, and the
//! registry batches (D-47), so two handles of one act may resolve to two versions.
//! A pin landing between them resolves the overlay document as live while the shard
//! at or below the pin still does not list it: enumeration and resolution
//! disagreeing at one pin, permanently, on INSERT-only rows. One act, one registry
//! request, one handle, N subject rows.
//!
//! # What the widened key still refuses, unchanged
//!
//! `record_pending`'s contract is that a second record of one assignment is refused
//! rather than upserted — "a handle arriving twice means two publish transactions
//! believe they own the same assignment, and silently overwriting the first would
//! hand one publish's subject to the other's version". That protection is about a
//! **subject** claiming a handle twice, and the widened key keeps it at exactly that
//! granularity: `(tenant_id, pending_ref, subject_kind, subject_ref)` still refuses
//! the duplicate and now admits the sibling.
//!
//! # A migration of its own rather than an amendment to `000004`
//!
//! `000004`'s module doc says it was "amended in place rather than fixed up", which
//! was the practice before the chain stated a rule. `m20260802_000035` states it —
//! *"`000018`'s reason, which is the chain's rule: the history stays legible, and a
//! reader asking when X became storable gets a dated answer instead of a
//! `git blame`"* — and this file follows the rule rather than the older file's
//! practice. `000004`'s "owed" paragraph is what a reader greps; it now has a dated
//! answer.
//!
//! # The `SQLite` half is a table rebuild, and it moves exactly **one** object
//!
//! `SQLite` has no `ALTER TABLE … DROP CONSTRAINT`, so changing a primary key is
//! `000018`'s create-copy-drop-rename dance, which takes every trigger and index on
//! the table with it. This table carries **no trigger at all** and exactly one index
//! — `idx_pricing_catalog_version_ref_version`, from `000004` — and nothing else in
//! the chain attaches to it: `000015` names it in prose only, and it carries no
//! inbound foreign key. So the count here is one, and it is stated as a count rather
//! than left to be inferred, because `000019` and `000035` both turned on somebody
//! having enumerated it.
//!
//! The index is re-created **after** the copy, for `000035`'s reason: the rows being
//! copied already satisfy it, so creating it first only adds a way for the rebuild
//! to fail.
//!
//! **Backend differences.** Postgres alters the key in place, naming the implicit
//! constraint `pricing_catalog_version_ref_pkey`; `SQLite` rebuilds. The resulting
//! schemas are the same one.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_catalog_version_ref
        DROP CONSTRAINT pricing_catalog_version_ref_pkey",
    "ALTER TABLE bss.pricing_catalog_version_ref
        ADD PRIMARY KEY (tenant_id, pending_ref, subject_kind, subject_ref)",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_catalog_version_ref
        DROP CONSTRAINT pricing_catalog_version_ref_pkey",
    "ALTER TABLE bss.pricing_catalog_version_ref
        ADD PRIMARY KEY (tenant_id, pending_ref)",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

/// The rebuild, parameterised on the key columns so `up` and `down` cannot drift.
///
/// `m20260802_000035`'s macro with the CHECK list frozen and the key list moving —
/// the two directions differ in one literal, and a hand-copied rebuild is where an
/// object goes missing from one direction only.
macro_rules! sqlite_rebuild {
    ($key:literal) => {
        &[
            concat!(
                "CREATE TABLE pricing_catalog_version_ref_rebuilt (
        tenant_id       text   NOT NULL,
        pending_ref     text   NOT NULL,
        subject_kind    text   NOT NULL,
        subject_ref     text   NOT NULL,
        subject_revision bigint,
        subject_lifecycle_state text,
        catalog_version bigint,
        requested_at    text   NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        commit_observed_at text,
        committed_at    text,
        PRIMARY KEY (",
                $key,
                "),
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
    )"
            ),
            "INSERT INTO pricing_catalog_version_ref_rebuilt (
        tenant_id, pending_ref, subject_kind, subject_ref, subject_revision,
        subject_lifecycle_state, catalog_version, requested_at, commit_observed_at,
        committed_at)
     SELECT
        tenant_id, pending_ref, subject_kind, subject_ref, subject_revision,
        subject_lifecycle_state, catalog_version, requested_at, commit_observed_at,
        committed_at
     FROM pricing_catalog_version_ref",
            "DROP TABLE pricing_catalog_version_ref",
            "ALTER TABLE pricing_catalog_version_ref_rebuilt
                RENAME TO pricing_catalog_version_ref",
            // --- the one index `m20260802_000004` put here, verbatim, and after
            // the copy ---
            "CREATE INDEX idx_pricing_catalog_version_ref_version
        ON pricing_catalog_version_ref (tenant_id, catalog_version)",
        ]
    };
}

const SQLITE_UP_STATEMENTS: &[&str] =
    sqlite_rebuild!("tenant_id, pending_ref, subject_kind, subject_ref");

const SQLITE_DOWN_STATEMENTS: &[&str] = sqlite_rebuild!("tenant_id, pending_ref");

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
