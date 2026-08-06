//! Create `bss.pricing_price_overlay` — the overlay header and its revision
//! chain (`design/09-price-overlays.md` §6, D-42, D-92, D-107).
//!
//! An overlay is a **line container**, not an adjustment (D-42): this row is the
//! header — scope, precedence, dating, tax basis, disclosure — and the lines it
//! contains live in `m20260802_000033`'s table. The pre-D-42 single-adjustment
//! overlay is the degenerate one-line case, so nothing about this shape loses it.
//!
//! # The precedence index is **partial**, and D-107 is why
//!
//! §6 spells it `UNIQUE (tenant_id, scope_class, precedence) WHERE
//! lifecycle_state = 'published'`, and the predicate is load-bearing rather than
//! an optimisation. D-92 gave overlays coexisting `draft` / `published` /
//! `superseded` revision rows on one `price_overlay_id`; an unqualified index
//! makes a draft revision of a published overlay collide **with itself**, so
//! every edit of a live overlay fails `PRECEDENCE_DUPLICATE` and an overlay is
//! authorable exactly once. `sqlite_overlay_store`'s
//! `a_draft_revision_may_reuse_its_own_published_revisions_precedence` is the
//! case that fails the moment the `WHERE` is dropped, and its sibling
//! `two_published_overlays_of_one_class_may_not_share_a_precedence` is the case
//! that fails if the index is dropped instead — partial does not mean absent.
//!
//! This is the same treatment the price rows and plan revisions already carry
//! (`uq_pricing_price_scope_key_current`, `uq_pricing_plan_current`).
//!
//! # `scope_value` renders the classless scope as the empty string
//!
//! §6 says `scope_value` is *"taxonomy-validated per class"*, and the `global`
//! class has no taxonomy and no value. The two representable choices are a NULL
//! and a sentinel, and this table takes the sentinel under one biconditional
//! `CHECK`: `(scope_class = 'global') = (length(scope_value) = 0)`.
//!
//! A NULL would have been the obvious reading and is the worse one here. Every
//! index and every comparison this column takes part in — the scope enumeration
//! index, `inst-plv-dating`'s per-line-key overlap walk, `inst-plv-scope`'s
//! taxonomy lookup — would then need a null-safe form, and `Column::X.eq(None)`
//! renders `X = NULL` and matches nothing, which `SeaORM` builds happily. The
//! empty string makes the column total and the comparisons ordinary. It is safe
//! as a sentinel for exactly `m20260802_000023`'s reason about `COALESCE(meter,
//! '')`: **a taxonomy value may not be blank** — all four taxonomy tables carry
//! `chk_*_value_present` — so `''` denotes "no scope value" and nothing else can
//! render it.
//!
//! # `row_version` is an addition to §6's column list, and it is not decoration
//!
//! §6 lists no concurrency column. Without one the `PATCH` half of
//! `POST/PATCH /bss-pricing/v1/price-overlays` has no entity tag to answer
//! `If-Match` with, and two authors editing one draft revision would both
//! satisfy their precondition — the lost update `fr-concurrent-edit` exists to
//! refuse. It is the `pricing_plan` revision's column under the same name and
//! the same D-170 tag shape (`"<revision>-<version>"`), so the overlay plane
//! answers preconditions the way every other authoring plane in this gear does.
//! Reported in the owed register rather than smoothed over.
//!
//! # What is deliberately **not** here
//!
//! No `created_by` / `created_at_utc`. `pricing_plan` carries them and this
//! table does not, because §6 lists neither and `pricing_audit_log` already
//! holds the actor, the instant and the before/after of every overlay mutation
//! (`inst-rb-audit`). A second copy of the actor on the row is a second thing to
//! keep true, and it is the copy that goes stale.
//!
//! No `overlay_index` projection column and no read-model coupling: D-112 and
//! D-133's sharded index is read-model territory and carries an open question of
//! its own (the `global` sentinel is one shard for every classless overlay, so
//! the sharding divides by one there).
//!
//! # The append-only trigger
//!
//! A published revision row is immutable in content (D-92): the projector — warm
//! and re-drive alike — reads the **published revision's** rows, so a draft edit
//! that could reach a published row would re-materialize a frozen
//! `CatalogVersion` at different content. The trigger freezes every content
//! column once the row leaves `draft`, and admits exactly two flips:
//! `draft -> published` (the submit) and `published -> superseded` (the same
//! submit, on the predecessor, in one commit).
//!
//! **DELETE is refused off the draft plane and permitted on it.** The overlay's
//! three states carry no `abandoned` tombstone the way `pricing_plan` does
//! (§6 names three states, not four), so a discarded draft revision leaves by
//! DELETE and there is no other way for it to leave. What must never be
//! deletable is a revision some `CatalogVersion` froze, and that is what the
//! guard says.
//!
//! **Backend differences.** Postgres carries the rule as one PL/pgSQL trigger
//! function with the offending state interpolated; `SQLite` has no procedural
//! language and `RAISE(ABORT, ...)` takes a literal message, so the same rule
//! becomes four fixed-message triggers. `uuid` becomes `text`, `timestamptz`
//! becomes `text`, `jsonb` becomes `text`, and the `bss.` qualification is
//! dropped. Every `CHECK`, index and the primary key are preserved on both
//! sides.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_price_overlay (
        price_overlay_id uuid        NOT NULL,
        revision         bigint      NOT NULL,
        tenant_id        uuid        NOT NULL,
        lifecycle_state  text        NOT NULL,
        scope_class      text        NOT NULL,
        scope_value      text        NOT NULL,
        precedence       int         NOT NULL,
        effective_from   timestamptz,
        effective_to     timestamptz,
        tax_basis        text        NOT NULL,
        disclosure       text        NOT NULL DEFAULT 'restricted',
        target_ref       jsonb       NOT NULL DEFAULT '{}'::jsonb,
        row_version      bigint      NOT NULL DEFAULT 0,
        PRIMARY KEY (price_overlay_id, revision),
        CONSTRAINT chk_pricing_price_overlay_lifecycle_state CHECK (
            lifecycle_state IN ('draft', 'published', 'superseded')),
        CONSTRAINT chk_pricing_price_overlay_revision CHECK (revision >= 0),
        -- The six classes of 1.1, spelled snake_case as the wire is.
        CONSTRAINT chk_pricing_price_overlay_scope_class CHECK (
            scope_class IN (
                'partner', 'org_tier', 'brand', 'region', 'customer_group', 'global')),
        -- The classless sentinel, both ways. See the module doc.
        CONSTRAINT chk_pricing_price_overlay_scope_value CHECK (
            (scope_class = 'global') = (length(scope_value) = 0)),
        -- L5: the basis MUST be declared, and NOT NULL is what `silence fails`
        -- means physically.
        CONSTRAINT chk_pricing_price_overlay_tax_basis CHECK (
            tax_basis IN ('inclusive', 'exclusive', 'delegated_tariffs')),
        -- L6: fail-closed default, so an overlay nobody classified is one
        -- nobody may see.
        CONSTRAINT chk_pricing_price_overlay_disclosure CHECK (
            disclosure IN ('restricted', 'public')),
        CONSTRAINT chk_pricing_price_overlay_interval CHECK (
            effective_from IS NULL OR effective_to IS NULL
            OR effective_to > effective_from)
    )",
    // D-107's partial predicate. Without it every edit of a live overlay fails.
    "CREATE UNIQUE INDEX uq_pricing_price_overlay_precedence
        ON bss.pricing_price_overlay (tenant_id, scope_class, precedence)
        WHERE lifecycle_state = 'published'",
    // At most one open draft revision per overlay (D-92), the plan revision
    // chain's `uq_pricing_plan_open_draft` one table over.
    "CREATE UNIQUE INDEX uq_pricing_price_overlay_open_draft
        ON bss.pricing_price_overlay (price_overlay_id)
        WHERE lifecycle_state = 'draft'",
    // `inst-plv-scope`'s taxonomy walk and `inst-plv-dating`'s collision domain
    // both range over one tenant's overlays of one scope.
    "CREATE INDEX idx_pricing_price_overlay_scope
        ON bss.pricing_price_overlay (tenant_id, scope_class, scope_value, lifecycle_state)",
    "CREATE OR REPLACE FUNCTION bss.pricing_price_overlay_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            IF OLD.lifecycle_state <> 'draft' THEN
              RAISE EXCEPTION
                'pricing_price_overlay: DELETE of revision % of overlay % is not permitted; it is %',
                OLD.revision, OLD.price_overlay_id, OLD.lifecycle_state;
            END IF;
            RETURN OLD;
          END IF;

          -- The draft plane is where content moves, so its columns are
          -- unguarded - but its exits are not. A draft leaves by publishing and
          -- by nothing else; `draft -> superseded` would mint a superseded row
          -- that never published, which the projector would then source from.
          IF OLD.lifecycle_state = 'draft' THEN
            IF NEW.lifecycle_state NOT IN ('draft', 'published') THEN
              RAISE EXCEPTION
                'pricing_price_overlay: lifecycle_state % -> % is not a sanctioned flip',
                OLD.lifecycle_state, NEW.lifecycle_state;
            END IF;
            RETURN NEW;
          END IF;

          IF NEW.price_overlay_id IS DISTINCT FROM OLD.price_overlay_id
          OR NEW.revision         IS DISTINCT FROM OLD.revision
          OR NEW.tenant_id        IS DISTINCT FROM OLD.tenant_id
          OR NEW.scope_class      IS DISTINCT FROM OLD.scope_class
          OR NEW.scope_value      IS DISTINCT FROM OLD.scope_value
          OR NEW.precedence       IS DISTINCT FROM OLD.precedence
          OR NEW.effective_from   IS DISTINCT FROM OLD.effective_from
          OR NEW.effective_to     IS DISTINCT FROM OLD.effective_to
          OR NEW.tax_basis        IS DISTINCT FROM OLD.tax_basis
          OR NEW.disclosure       IS DISTINCT FROM OLD.disclosure
          OR NEW.target_ref       IS DISTINCT FROM OLD.target_ref
          OR NEW.row_version      IS DISTINCT FROM OLD.row_version THEN
            RAISE EXCEPTION
              'pricing_price_overlay: revision % of overlay % is frozen; only a sanctioned lifecycle_state flip is permitted',
              OLD.revision, OLD.price_overlay_id;
          END IF;

          IF NEW.lifecycle_state IS DISTINCT FROM OLD.lifecycle_state
             AND NOT (OLD.lifecycle_state = 'published'
                      AND NEW.lifecycle_state = 'superseded') THEN
            RAISE EXCEPTION
              'pricing_price_overlay: lifecycle_state % -> % is not a sanctioned flip',
              OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_price_overlay_append_only
        BEFORE UPDATE OR DELETE ON bss.pricing_price_overlay
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_price_overlay_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_price_overlay",
    "DROP FUNCTION IF EXISTS bss.pricing_price_overlay_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_price_overlay (
        price_overlay_id text   NOT NULL,
        revision         bigint NOT NULL,
        tenant_id        text   NOT NULL,
        lifecycle_state  text   NOT NULL,
        scope_class      text   NOT NULL,
        scope_value      text   NOT NULL,
        precedence       int    NOT NULL,
        effective_from   text,
        effective_to     text,
        tax_basis        text   NOT NULL,
        disclosure       text   NOT NULL DEFAULT 'restricted',
        target_ref       text   NOT NULL DEFAULT '{}',
        row_version      bigint NOT NULL DEFAULT 0,
        PRIMARY KEY (price_overlay_id, revision),
        CONSTRAINT chk_pricing_price_overlay_lifecycle_state CHECK (
            lifecycle_state IN ('draft', 'published', 'superseded')),
        CONSTRAINT chk_pricing_price_overlay_revision CHECK (revision >= 0),
        CONSTRAINT chk_pricing_price_overlay_scope_class CHECK (
            scope_class IN (
                'partner', 'org_tier', 'brand', 'region', 'customer_group', 'global')),
        CONSTRAINT chk_pricing_price_overlay_scope_value CHECK (
            (scope_class = 'global') = (length(scope_value) = 0)),
        CONSTRAINT chk_pricing_price_overlay_tax_basis CHECK (
            tax_basis IN ('inclusive', 'exclusive', 'delegated_tariffs')),
        CONSTRAINT chk_pricing_price_overlay_disclosure CHECK (
            disclosure IN ('restricted', 'public')),
        CONSTRAINT chk_pricing_price_overlay_interval CHECK (
            effective_from IS NULL OR effective_to IS NULL
            OR effective_to > effective_from)
    )",
    "CREATE UNIQUE INDEX uq_pricing_price_overlay_precedence
        ON pricing_price_overlay (tenant_id, scope_class, precedence)
        WHERE lifecycle_state = 'published'",
    "CREATE UNIQUE INDEX uq_pricing_price_overlay_open_draft
        ON pricing_price_overlay (price_overlay_id)
        WHERE lifecycle_state = 'draft'",
    "CREATE INDEX idx_pricing_price_overlay_scope
        ON pricing_price_overlay (tenant_id, scope_class, scope_value, lifecycle_state)",
    "CREATE TRIGGER trg_pricing_price_overlay_frozen_columns
        BEFORE UPDATE ON pricing_price_overlay
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND (NEW.price_overlay_id IS NOT OLD.price_overlay_id
            OR NEW.revision         IS NOT OLD.revision
            OR NEW.tenant_id        IS NOT OLD.tenant_id
            OR NEW.scope_class      IS NOT OLD.scope_class
            OR NEW.scope_value      IS NOT OLD.scope_value
            OR NEW.precedence       IS NOT OLD.precedence
            OR NEW.effective_from   IS NOT OLD.effective_from
            OR NEW.effective_to     IS NOT OLD.effective_to
            OR NEW.tax_basis        IS NOT OLD.tax_basis
            OR NEW.disclosure       IS NOT OLD.disclosure
            OR NEW.target_ref       IS NOT OLD.target_ref
            OR NEW.row_version      IS NOT OLD.row_version)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay: the revision is frozen; only a sanctioned lifecycle_state flip is permitted');
        END",
    "CREATE TRIGGER trg_pricing_price_overlay_draft_exit
        BEFORE UPDATE ON pricing_price_overlay
        FOR EACH ROW WHEN OLD.lifecycle_state = 'draft'
          AND NEW.lifecycle_state NOT IN ('draft', 'published')
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay: that lifecycle_state move is not a sanctioned flip');
        END",
    "CREATE TRIGGER trg_pricing_price_overlay_frozen_flip
        BEFORE UPDATE ON pricing_price_overlay
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND NEW.lifecycle_state IS NOT OLD.lifecycle_state
          AND NOT (OLD.lifecycle_state = 'published'
                   AND NEW.lifecycle_state = 'superseded')
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay: that lifecycle_state move is not a sanctioned flip');
        END",
    "CREATE TRIGGER trg_pricing_price_overlay_no_delete
        BEFORE DELETE ON pricing_price_overlay
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_price_overlay: DELETE of a revision that is not a draft is not permitted');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_price_overlay"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
