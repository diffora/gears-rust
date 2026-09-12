//! Create `bss.products_catalog_version` — the version row
//! (`design/06-catalog-version.md` §4, unblocked by **P-D-73**).
//!
//! # The column set is exactly §4's, after its three answers
//!
//! `checksum` with its **`digest_version`** companion (P-D-73 arm 1 — the
//! `products_entity_version` convention: without the column, slice 10's drill
//! cannot re-verify a sampled manifest against the rule its digest was
//! actually computed under); `published_at` (`staged_at` was struck by
//! P-D-67: no writer, no reader); `participant_set_snapshot`, a **derived
//! cache** whose authoritative copy is the capture store's and is the one
//! inside the checksum (P-D-67); and `freeze_state`, the ledger's derived
//! cache, refreshed in-transaction by the three acts that change the ledger —
//! ack, release, force-completion (P-D-73 arm 2). *"The manifest header"* was
//! struck (P-D-73 arm 3): no field set, no writer, no reader anywhere in the
//! tree.
//!
//! # Append-only on the whitelist discipline, not the unconditional refusal
//!
//! `m20260829_000007`'s guard refuses every `UPDATE` because that table has
//! none to describe. **This table has exactly one admitted update**:
//! `freeze_state` — force-completion must land `complete(forced)` and the
//! last ack must land `complete` (`dod-catalog-version-table`,
//! `dod-force-completion`, P-D-73). So the model is
//! `m20260829_000002`'s head-row guard: on Postgres one `PL/pgSQL` function
//! branching on `TG_OP` behind a single trigger firing
//! `BEFORE DELETE OR UPDATE`, comparing `NEW` against `OLD` with
//! `IS DISTINCT FROM`; on `SQLite`, one no-delete trigger and one
//! `WHEN`-guarded trigger over the frozen column class, using `IS NOT`. Every
//! column but `freeze_state` is refused by name — the byte-identity flagship
//! rests on them.
//!
//! # `DELETE` runs under the release stamp (P-D-137)
//!
//! A catalog version is a **financial record with a statutory window**
//! (`PRD` §330: *"Snapshots are financial records"*), not evidence — so
//! unlike the approval, decision, break-glass and correction-override tables
//! (P-D-136) this one is collectable, and the arm that admits the collector
//! had to be expressible without the two channels earlier decisions closed:
//! **P-D-31** removed the session variable that would carry the deleter's
//! identity, and **P-D-118** removed the date constant that would carry the
//! window. What is left is a property of the row itself, which the GC can
//! make true: **`retention_released_at`**.
//!
//! Nullable, and the `UPDATE` whitelist admits it moving **exactly once**,
//! `NULL` to a value — the sealing arm's shape one table over. A value may
//! not be changed and may not be cleared, so a stamp is not a toggle a
//! caller can flip back and forth around a delete. `DELETE` is then admitted
//! for a row whose stamp is set, and refused for one whose stamp is `NULL`.
//!
//! **The stamp is not an authorisation and does not pretend to be.** Any
//! caller who may `UPDATE` this table may stamp it; what the arm buys is that
//! a deletion is always a *deliberate two-step*, recorded in the row itself,
//! rather than a single statement a mistaken `WHERE` can perform. The
//! application-side guarantee that only the GC stamps is a **writer count**
//! (`lib_tests::every_writer_of_a_release_stamp_is_counted`, P-D-105's
//! pattern), which is where an invariant no schema can hold belongs.
//!
//! The two refusal messages are distinct on purpose: a body that lost its
//! `UPDATE` branch would still refuse an update, but with the delete
//! message — same outcome, different guard, and only the text tells them
//! apart (`dod-catalog-version-table`'s own assertion requirement).
//!
//! # `freeze_state`'s roster is a CHECK, and the id is gapless by contract
//!
//! `chk_products_catalog_version_freeze_state` pins the three-value roster
//! §4 states. `catalog_version_id >= 1` pins the P-D-67 counter start; the
//! gapless walk itself is the allocator's contract
//! (`m20260901_000008`), not a constraint a single row can carry.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `bigint` becomes `integer`,
//! `timestamptz` becomes `text`, and the `bss.` qualification is dropped.
//! Every CHECK, the primary key and both guard halves are preserved on both
//! sides.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-catalog-version-table:p1

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_catalog_version (
            tenant_id                 uuid        NOT NULL,
            catalog_version_id        bigint      NOT NULL,
            checksum                  text        NOT NULL,
            digest_version            integer     NOT NULL,
            published_at              timestamptz NOT NULL,
            participant_set_snapshot  text        NOT NULL,
            freeze_state              text        NOT NULL,
            retention_released_at     timestamptz,
            CONSTRAINT products_catalog_version_pkey PRIMARY KEY (tenant_id, catalog_version_id),
            CONSTRAINT chk_products_catalog_version_id_floor CHECK (catalog_version_id >= 1),
            CONSTRAINT chk_products_catalog_version_freeze_state CHECK (freeze_state IN ('open', 'complete', 'complete(forced)')),
            CONSTRAINT chk_products_catalog_version_digest CHECK (digest_version >= 1)
        )",
    "CREATE OR REPLACE FUNCTION bss.products_catalog_version_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            IF OLD.retention_released_at IS NULL THEN
              RAISE EXCEPTION 'products_catalog_version: DELETE is admitted only for a version whose retention_released_at is stamped (P-D-137)';
            END IF;
            RETURN OLD;
          END IF;
          IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
             OR NEW.catalog_version_id IS DISTINCT FROM OLD.catalog_version_id
             OR NEW.checksum IS DISTINCT FROM OLD.checksum
             OR NEW.digest_version IS DISTINCT FROM OLD.digest_version
             OR NEW.published_at IS DISTINCT FROM OLD.published_at
             OR NEW.participant_set_snapshot IS DISTINCT FROM OLD.participant_set_snapshot THEN
            RAISE EXCEPTION 'products_catalog_version: freeze_state and retention_released_at are the only columns the UPDATE arm admits';
          END IF;
          IF NEW.retention_released_at IS DISTINCT FROM OLD.retention_released_at
             AND NOT (OLD.retention_released_at IS NULL AND NEW.retention_released_at IS NOT NULL) THEN
            RAISE EXCEPTION 'products_catalog_version: retention_released_at is stamped once and never moved (P-D-137)';
          END IF;
          RETURN NEW;
        END;
        $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_catalog_version_append_only
        BEFORE DELETE OR UPDATE ON bss.products_catalog_version
        FOR EACH ROW EXECUTE FUNCTION bss.products_catalog_version_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_catalog_version_append_only ON bss.products_catalog_version",
    "DROP FUNCTION IF EXISTS bss.products_catalog_version_append_only",
    "DROP TABLE IF EXISTS bss.products_catalog_version",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_catalog_version (
            tenant_id                 text    NOT NULL,
            catalog_version_id        integer NOT NULL,
            checksum                  text    NOT NULL,
            digest_version            integer NOT NULL,
            published_at              text    NOT NULL,
            participant_set_snapshot  text    NOT NULL,
            freeze_state              text    NOT NULL,
            retention_released_at     text,
            PRIMARY KEY (tenant_id, catalog_version_id),
            CONSTRAINT chk_products_catalog_version_id_floor CHECK (catalog_version_id >= 1),
            CONSTRAINT chk_products_catalog_version_freeze_state CHECK (freeze_state IN ('open', 'complete', 'complete(forced)')),
            CONSTRAINT chk_products_catalog_version_digest CHECK (digest_version >= 1)
        )",
    "CREATE TRIGGER trg_products_catalog_version_no_delete
        BEFORE DELETE ON products_catalog_version
        WHEN OLD.retention_released_at IS NULL
        BEGIN
          SELECT RAISE(ABORT, 'products_catalog_version: DELETE is admitted only for a version whose retention_released_at is stamped (P-D-137)');
        END",
    "CREATE TRIGGER trg_products_catalog_version_release_once
        BEFORE UPDATE ON products_catalog_version
        WHEN NEW.retention_released_at IS NOT OLD.retention_released_at
          AND NOT (OLD.retention_released_at IS NULL AND NEW.retention_released_at IS NOT NULL)
        BEGIN
          SELECT RAISE(ABORT, 'products_catalog_version: retention_released_at is stamped once and never moved (P-D-137)');
        END",
    "CREATE TRIGGER trg_products_catalog_version_frozen_columns
        BEFORE UPDATE ON products_catalog_version
        WHEN NEW.tenant_id IS NOT OLD.tenant_id
          OR NEW.catalog_version_id IS NOT OLD.catalog_version_id
          OR NEW.checksum IS NOT OLD.checksum
          OR NEW.digest_version IS NOT OLD.digest_version
          OR NEW.published_at IS NOT OLD.published_at
          OR NEW.participant_set_snapshot IS NOT OLD.participant_set_snapshot
        BEGIN
          SELECT RAISE(ABORT, 'products_catalog_version: freeze_state and retention_released_at are the only columns the UPDATE arm admits');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS trg_products_catalog_version_release_once",
    "DROP TRIGGER IF EXISTS trg_products_catalog_version_frozen_columns",
    "DROP TRIGGER IF EXISTS trg_products_catalog_version_no_delete",
    "DROP TABLE IF EXISTS products_catalog_version",
];

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
