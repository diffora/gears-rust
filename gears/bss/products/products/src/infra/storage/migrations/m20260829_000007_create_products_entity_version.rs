//! Create `bss.products_entity_version` — the frozen published-version
//! history, keyed `(tenant_id, entity_kind, entity_id, published_version)`
//! (`design/01-foundation.md` §4.3).
//!
//! # What a row freezes, and why it is one column rather than one per field
//!
//! §4.3 scopes a row's content as the publish-time entity **excluding the
//! metadata map** and excluding `lifecycle_state`, `deprecation_provenance`,
//! `replaced_by_sku_id` and `internal_revision` (**P-D-24**, extended by
//! **P-D-35**): those four move on transitions, which write no version row, so
//! freezing them would need the digest to change on a write that produces no
//! row to digest. They are read from the head row instead. **Content is not
//! state.**
//!
//! **This migration stores that content as one canonical-rendering column —
//! `content`, holding exactly the bytes that were digested, rather than a
//! column per content field. That is an implementation choice this file
//! makes; the design set scopes the content and pins the rendering, and does
//! not state the physical layout.** Two reasons, in order:
//!
//! 1. **The digest must be re-verifiable from the stored row.** §4.3 pins an
//!    engine-canonical serialization (`JSON`, keys sorted lexicographically,
//!    UTF-8 without BOM, absent values written `null` rather than omitted,
//!    numbers as bare decimal strings, timestamps RFC 3339 UTC at microsecond
//!    precision) and §5 asserts it byte-identical across engines. Slice 10's
//!    restore drill re-verifies sampled rows against `content_digest`
//!    **byte-for-byte**. A re-serialisation from typed columns cannot
//!    guarantee those bytes: it would depend on the driver's round-trip of
//!    every column type on every engine, and a drift there would silently
//!    fail the drill on rows that are in fact intact. Storing the digested
//!    bytes themselves removes the round-trip from the verification path
//!    entirely.
//! 2. **Content grows per slice, and a column-per-field table would need a
//!    migration per slice.** Slice 02 brings the category-assignment and
//!    attribute-value sets, slice 03 the metering shape. §4.3's own rule is
//!    that **adding a column to a frozen row's content is a digest-version
//!    bump, not a silent change** — so a widening is already a versioned,
//!    checkable event at the digest level, and a physical schema change per
//!    widening would buy nothing the digest version does not already carry.
//!
//! The cost is named rather than hidden: no `SQL` predicate can read a single
//! content field out of a frozen row without parsing the rendering. That is
//! acceptable here because §4.3 makes these rows the **only consumer-read
//! surface for entity content** and every such read projects the whole
//! content, never one field of it in a `WHERE` clause.
//!
//! # The Postgres type is `text`, not `json` and not `jsonb`
//!
//! Three types could hold a canonical rendering, and only one of them can
//! hold it safely. The chain is stated in full because the middle answer was
//! shipped here first, was wrong, and no test in this gear could see it.
//!
//! **`jsonb` is wrong because it does not store what it was given.** Postgres
//! stores `jsonb` decomposed and renders it back normalized: it **discards
//! insignificant whitespace, does not preserve object key order, and
//! normalizes numeric literals** (a trailing zero written is a trailing zero
//! lost). The whole reason this column exists is that `content_digest` must
//! be re-verifiable from the stored row **byte-for-byte**, so a type that
//! re-renders its input is the one type that cannot hold it — a row could be
//! perfectly intact and still fail slice 10's restore drill. The rendering
//! §4.3 pins already sorts keys and emits no insignificant whitespace, so
//! most of what `jsonb` would normalize is normalized before the write; that
//! is exactly why the difference is worth stating rather than leaving to
//! luck, since the guarantee must come from the storage type and not from the
//! writer happening to agree with it.
//!
//! **`json` was the first answer here, and it was wrong too — for a reason
//! that has nothing to do with what it stores.** `json` does keep an exact
//! copy of the input text, so it satisfies the byte-preservation argument
//! above. What it does not admit is the write. The entity field
//! `entity::entity_version::Model::content` is typed `String`, which
//! `DeriveEntityModel` infers as `ColumnType::String`, so the driver binds
//! the parameter with `OID` 25 — `text`. Postgres registers `text` to `json`
//! only at `COERCION_EXPLICIT`, while an `INSERT` target takes its argument
//! at `COERCION_ASSIGNMENT`. Every publish would therefore have failed on the
//! production engine with `column "content" is of type json but expression is
//! of type text` (`SQLSTATE` 42804) — both entity kinds, first publish and
//! re-publish alike. The read would have failed symmetrically: the driver
//! decodes `String` from `TEXT`, `VARCHAR`, `NAME`, `BPCHAR` and `UNKNOWN`,
//! never from `JSON`.
//!
//! **`text` is right because it is the only one of the three that both stores
//! its input verbatim and accepts a `text` parameter.** It is also what the
//! `SQLite` mirror already used, `SQLite` having no `JSON` type at all, so
//! the two engines now store the **identical** type — which is what §4.3's
//! byte-identity discipline wants, and one fewer place the engines can
//! diverge. The one thing `json` bought over `text` was a parse-validity
//! check at write time, and nothing is lost with it: `domain::canonical` is
//! this value's sole producer and always emits well-formed `JSON`. No `json`
//! operator is given up either, because no predicate in this gear reads a
//! field out of this column, per the paragraph above.
//!
//! **The `json` defect was caught by review, not by a test — and the reason
//! no test caught it is a standing limit worth recording here.** Every test
//! in this gear runs against an in-memory `SQLite`, where the mirror column
//! was already `text` and every publish therefore passed. Nothing short of a
//! Postgres tier can judge a Postgres-only statement, and this gear has none.
//! Until it does, the Postgres arm of every migration in this chain is
//! reviewed prose rather than measured behaviour: read the two statement
//! lists below as two claims, of which the suite checks one.
//!
//! # `digest_version` is stored per row so a later bump is checkable
//!
//! `digest_version` starts at `1` and is pinned as a code constant by §5's
//! golden vector rather than by configuration (**P-D-33**). It is stored **on
//! the row**, not deduced, for the reason **P-D-29** gives: the
//! "digest-version bump, not a silent change" rule is only checkable if the
//! version a row was computed under is stored on the row. Slice 10's restore
//! drill re-verifies sampled rows against it, and without it version-history
//! corruption is invisible to every checksum.
//!
//! # `approval_ref` stays nullable — decided, not owed (P-D-144)
//!
//! §4.3 names the column and `inst-fd-gate-verdict` says what it stores: on a
//! yes, the authorizing `ApprovalRecord`'s id. Slice 05's host is registered at
//! the publish door since P-D-142, and the tightening this header once owed to
//! that day is **declined**: a publish the materiality evaluator judges
//! `NonMaterial` runs ungoverned and has no record, so `NULL` is the honest value
//! for its frozen row — a placeholder would write a false authority into a
//! financial record. The nullability is a domain fact ("no record was needed"),
//! not a pre-05 accommodation, and the frozen-row whitelist keeps it immutable
//! either way.
//!
//! # The key is the primary key
//!
//! §4.3 states the key as a `UNIQUE` on
//! `(tenant_id, entity_kind, entity_id, published_version)`. It ships here as
//! the **primary key** over exactly those four columns: a primary key is a
//! unique key, and a separate unique index over the same four columns beside
//! a surrogate key would be a second structure enforcing the same rule. The
//! ordering also serves the one read shape this table has — every version of
//! one entity, in version order — so no further index is created.
//! `published_version >= 1` is checked: version `0` is the unpublished head's
//! counter value and has no frozen row by construction.
//!
//! # Append-only: no `UPDATE` path at all, ever
//!
//! §4.3: "Append-only, no `UPDATE` path at all; diffs are computed between
//! rows, never stored mutated." The guard below refuses every `UPDATE`
//! unconditionally on both engines. There is no row-image predicate and no
//! whitelist, because there is no admitted `UPDATE` for one to describe —
//! unlike the head tables one migration over, where the whitelist exists
//! precisely because some updates are legitimate.
//!
//! # `DELETE` runs under P-D-40's referential predicate — the owed arm, paid
//!
//! §4.3 admits **exactly one** `DELETE`, under a referential predicate
//! (**P-D-40**): a row may be deleted only when **no
//! `products_catalog_version_entry` references it**. Until `2026-09-01` this
//! file refused `DELETE` unconditionally, because writing the predicate
//! before that table existed would have been **fail-open** — the subquery
//! would have found nothing referencing any row and admitted every `DELETE`.
//! `m20260901_000013` landed the table, and this same file was edited **in
//! place** — this chain's own convention, no tightening chase — so the guard
//! below now carries the predicate on both engines, riding
//! `idx_products_catalog_version_entry_ref`, the index P-D-40 booked for
//! exactly this lookup. The only caller that legitimately issues the `DELETE`
//! is still slice 10's `inst-rt-gc`, which has no code yet; until it does,
//! nothing exercises the admitted arm in production, and the guard tests
//! exercise both arms in its stead.
//!
//! On `SQLite` the trigger references the entry table by name at fire time
//! (its `CREATE TRIGGER` does not resolve the name — measured), and every
//! fire happens after full-chain boot, so the reference always resolves.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-referential-delete-predicate:p1
//!
//! This is the discipline the head-row guard used one migration over: ship the
//! half that is checkable today, name the owed half and name the slice that
//! pays it. The difference in direction is deliberate — the head guard's owed
//! half would have *refused* writes it could not yet judge, so shipping
//! without it was fail-open in the permissive direction only for a bump no
//! door yet issued; here the owed half would *admit* deletes, so the safe
//! interim is the stricter rule, not the looser one.
//!
//! **§5's owed probe cannot be executed until slice 06.** §5 requires that
//! "deleting a `products_entity_version` row that a
//! `products_catalog_version_entry` still references must be refused by the
//! guard, not merely skipped by the GC — a probe that passes when the GC is
//! bypassed entirely." That probe needs a referencing row, which needs the
//! referencing table, which is slice 06's. This is a measurement, not an
//! omission: the probe's premise does not exist at this commit, and the
//! probe lands with the predicate it exercises. What *is* probed here is the
//! interim rule as written — an unconditional refusal — which is strictly
//! stronger than the predicate it stands in for and therefore cannot admit a
//! delete the final predicate would refuse.
//!
//! # The head-row guard's existence half is closed by this table
//!
//! `m20260829_000002_create_products_product` and
//! `m20260829_000003_create_products_sku` owed the `DoD`'s "only where the
//! matching frozen version row exists" half to this table. Both now read it
//! through a subquery against this table. See either file's module doc for the
//! clause and for why a subquery is compatible with **P-D-31**.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` for `tenant_id`, `entity_id`, `approval_ref` and
//! `actor_ref`; `content` is `text` on both engines (see above); `bytea` becomes `blob`
//! for `content_digest`; `timestamptz` becomes `text` for `published_at`; and
//! the `bss.` qualification is dropped. Every `CHECK` and the primary key are
//! preserved on both sides. Postgres raises through one `PL/pgSQL` function
//! branching on `TG_OP`, with one trigger firing `BEFORE DELETE OR UPDATE`;
//! `SQLite` has no procedural language and `RAISE(ABORT, ...)` takes a literal
//! message, so the mirror is two triggers carrying the two messages. The
//! Postgres `DOWN` drops the function as well as the table.
//!
//! @cpt-cf-bss-products-dod-version-history-table

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.products_entity_version (
            tenant_id         uuid        NOT NULL,
            entity_kind       text        NOT NULL,
            entity_id         uuid        NOT NULL,
            published_version bigint      NOT NULL,
            content           text        NOT NULL,
            content_digest    bytea       NOT NULL,
            digest_version    integer     NOT NULL,
            approval_ref      uuid,
            actor_ref         uuid        NOT NULL,
            published_at      timestamptz NOT NULL,
            CONSTRAINT products_entity_version_pkey PRIMARY KEY (tenant_id, entity_kind, entity_id, published_version),
            CONSTRAINT chk_products_entity_version_entity_kind CHECK (entity_kind IN ('product', 'sku')),
            CONSTRAINT chk_products_entity_version_published_version CHECK (published_version >= 1),
            CONSTRAINT chk_products_entity_version_digest_version CHECK (digest_version >= 1)
        )",
    "CREATE OR REPLACE FUNCTION bss.products_entity_version_frozen() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'UPDATE' THEN
            RAISE EXCEPTION 'products_entity_version is frozen: UPDATE is not permitted';
          END IF;
          IF EXISTS (
               SELECT 1 FROM bss.products_catalog_version_entry e
               WHERE e.tenant_id = OLD.tenant_id
                 AND e.entity_kind = OLD.entity_kind
                 AND e.entity_id = OLD.entity_id
                 AND e.published_version = OLD.published_version
             ) THEN
            RAISE EXCEPTION 'products_entity_version: DELETE is admitted only when no products_catalog_version_entry references the row (P-D-40)';
          END IF;
          RETURN OLD;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_products_entity_version_frozen BEFORE DELETE OR UPDATE ON bss.products_entity_version FOR EACH ROW EXECUTE FUNCTION bss.products_entity_version_frozen()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.products_entity_version",
    "DROP FUNCTION IF EXISTS bss.products_entity_version_frozen()",
];

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE products_entity_version (
            tenant_id         text   NOT NULL,
            entity_kind       text   NOT NULL,
            entity_id         text   NOT NULL,
            published_version bigint NOT NULL,
            content           text   NOT NULL,
            content_digest    blob   NOT NULL,
            digest_version    integer NOT NULL,
            approval_ref      text,
            actor_ref         text   NOT NULL,
            published_at      text   NOT NULL,
            PRIMARY KEY (tenant_id, entity_kind, entity_id, published_version),
            CONSTRAINT chk_products_entity_version_entity_kind CHECK (entity_kind IN ('product', 'sku')),
            CONSTRAINT chk_products_entity_version_published_version CHECK (published_version >= 1),
            CONSTRAINT chk_products_entity_version_digest_version CHECK (digest_version >= 1)
        )",
    "CREATE TRIGGER trg_products_entity_version_no_update BEFORE UPDATE ON products_entity_version FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'products_entity_version is frozen: UPDATE is not permitted'); END",
    "CREATE TRIGGER trg_products_entity_version_no_delete BEFORE DELETE ON products_entity_version FOR EACH ROW WHEN EXISTS (SELECT 1 FROM products_catalog_version_entry e WHERE e.tenant_id = OLD.tenant_id AND e.entity_kind = OLD.entity_kind AND e.entity_id = OLD.entity_id AND e.published_version = OLD.published_version) BEGIN SELECT RAISE(ABORT, 'products_entity_version: DELETE is admitted only when no products_catalog_version_entry references the row (P-D-40)'); END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_entity_version"];

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
