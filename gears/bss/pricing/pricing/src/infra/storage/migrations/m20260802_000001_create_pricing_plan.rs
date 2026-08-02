//! Create `bss.pricing_plan` — the plan aggregate's **revision** rows
//! (`design/01-foundation.md` §3.7, D-56), keyed `(plan_id, revision)`. A plan
//! is not one row: it is a chain of revisions of which at most one is the
//! **current** one and at most one is an open `draft`.
//!
//! Two partial `UNIQUE` indexes carry that, and both are load-bearing.
//! `uq_pricing_plan_current` spans `lifecycle_state IN ('published','retired')`
//! — widened from `= 'published'` by **D-128**, because retirement flips the
//! only current revision and the narrower predicate then held no row at all,
//! leaving "the current revision" (the projection source, the sellability
//! predicate, every referential check) with no referent.
//! `uq_pricing_plan_open_draft` keeps a plan from having two concurrently
//! editable shapes.
//!
//! **A `revision` number is never freed** (D-145). A discarded draft revision is
//! flipped to the terminal `abandoned` state rather than deleted, so no DELETE
//! is permitted on this table at all — not even of a draft. Deleting one
//! returned `max(revision)` to its previous value, the next opened draft minted
//! the **same** number, and `(plan_id, revision)` — the name the grant table is
//! keyed by, every revision-scoped child copies under, and the audit trail
//! records — then denoted two different rows over a plan's lifetime, under which
//! a stale entity tag passes its precondition against the wrong one.
//! `abandoned` falls outside **both** partial predicates above, which is what
//! lets a tombstone accumulate without disturbing either uniqueness rule: a
//! replacement draft opens immediately, and "the current revision" is untouched.
//!
//! The physical guard is append-only enforcement with a **column whitelist**: a
//! published, retired or abandoned revision's content is frozen, and the only
//! sanctioned in-place mutations are the state-machine `lifecycle_state` flips
//! (`published -> superseded` when the successor revision commits,
//! `published -> retired` at retire, `draft -> published` at publish and
//! `draft -> abandoned` at discard). Draft revisions stay freely mutable **in
//! content**, and the guard restates the machine's edges on that plane too:
//! `draft -> superseded` and `draft -> retired` are refused here as they are in
//! `domain::lifecycle`, because a hand-run flip to `retired` mints a row that
//! satisfies `uq_pricing_plan_current` without ever having published — and the
//! projector sources a plan subject from exactly that row. Nothing on this table
//! is deletable. Without the guard an ad-hoc UPDATE would silently change a
//! frozen `CatalogVersion`'s content at the next warm re-drive, since the
//! projector reads truth rows (§4.4).
//!
//! **Backend differences.** Postgres raises through a PL/pgSQL
//! `RAISE EXCEPTION` carrying the offending value; `SQLite` has no PL/pgSQL and
//! `RAISE(ABORT, ...)` takes a **literal** message only, so the mirror splits
//! the same whitelist across three triggers with fixed messages and no
//! interpolated values. `REVOKE UPDATE, DELETE` is deliberately not issued on
//! either backend: it names a deployment role this migration does not own, and
//! `SQLite` has no `GRANT`/`REVOKE` at all — the trigger is the portable half of
//! the "REVOKE + trigger" discipline the design set describes.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    // First migration of the chain: make the schema exist. Idempotent with the
    // shared `coord` migration, which issues the same statement.
    "CREATE SCHEMA IF NOT EXISTS bss",
    "CREATE TABLE bss.pricing_plan (
        plan_id         uuid        NOT NULL,
        revision        bigint      NOT NULL,
        tenant_id       uuid        NOT NULL,
        sku_id          uuid,
        plan_tier       text,
        billing_cycle   text,
        lifecycle_state text        NOT NULL,
        available_from  timestamptz,
        available_to    timestamptz,
        created_by      uuid        NOT NULL,
        created_at_utc  timestamptz NOT NULL DEFAULT now(),
        row_version     bigint      NOT NULL DEFAULT 0,
        PRIMARY KEY (plan_id, revision),
        CONSTRAINT chk_pricing_plan_lifecycle_state CHECK (
            lifecycle_state IN ('draft','abandoned','published','superseded','retired')),
        CONSTRAINT chk_pricing_plan_revision CHECK (revision >= 0),
        CONSTRAINT chk_pricing_plan_availability CHECK (
            available_from IS NULL OR available_to IS NULL OR available_to > available_from)
    )",
    // At most one CURRENT revision per plan (D-128 widened the predicate).
    // `abandoned` is outside it: a tombstone never published, so it is not the
    // plan's current revision and a plan may hold any number of them.
    "CREATE UNIQUE INDEX uq_pricing_plan_current
        ON bss.pricing_plan (plan_id)
        WHERE lifecycle_state IN ('published','retired')",
    // At most one open draft revision per plan. `abandoned` is outside this one
    // too, which is what lets the replacement draft open in the same
    // transaction that discards its predecessor.
    "CREATE UNIQUE INDEX uq_pricing_plan_open_draft
        ON bss.pricing_plan (plan_id)
        WHERE lifecycle_state = 'draft'",
    "CREATE INDEX idx_pricing_plan_tenant
        ON bss.pricing_plan (tenant_id, plan_id, revision)",
    // --- append-only enforcement: column whitelist + sanctioned flips ---
    "CREATE OR REPLACE FUNCTION bss.pricing_plan_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_plan: DELETE of revision % of plan % is not permitted; a discarded draft revision is abandoned',
              OLD.revision, OLD.plan_id;
          END IF;

          -- The draft plane is where content moves, so its columns are
          -- unguarded - but its **exits** are not. A draft leaves by publishing
          -- or by being abandoned, and `NEW = draft` is the ordinary edit.
          -- Without the check a hand-run flip could mint a `retired` row that
          -- never published - one that satisfies the current-revision partial
          -- UNIQUE and is what the projector then sources a plan subject from.
          -- Membership is tested rather than change: a
          -- `NEW IS DISTINCT FROM OLD` conjunct would let the SQLite mirror
          -- accept a no-op UPDATE this branch refuses, and a backend divergence
          -- is worse than the hole it would close.
          IF OLD.lifecycle_state = 'draft' THEN
            IF NEW.lifecycle_state NOT IN ('draft','published','abandoned') THEN
              RAISE EXCEPTION 'pricing_plan: lifecycle_state % -> % is not a sanctioned flip',
                OLD.lifecycle_state, NEW.lifecycle_state;
            END IF;
            RETURN NEW;
          END IF;

          -- Past here the row is published, superseded, retired or abandoned.
          -- Once abandoned it is a tombstone: frozen in content by the whitelist
          -- below and left by no flip, so the number it consumed can never be
          -- attached to a different shape.

          IF NEW.plan_id        IS DISTINCT FROM OLD.plan_id
          OR NEW.revision       IS DISTINCT FROM OLD.revision
          OR NEW.tenant_id      IS DISTINCT FROM OLD.tenant_id
          OR NEW.sku_id         IS DISTINCT FROM OLD.sku_id
          OR NEW.plan_tier      IS DISTINCT FROM OLD.plan_tier
          OR NEW.billing_cycle  IS DISTINCT FROM OLD.billing_cycle
          OR NEW.available_from IS DISTINCT FROM OLD.available_from
          OR NEW.available_to   IS DISTINCT FROM OLD.available_to
          OR NEW.created_by     IS DISTINCT FROM OLD.created_by
          OR NEW.created_at_utc IS DISTINCT FROM OLD.created_at_utc
          OR NEW.row_version    IS DISTINCT FROM OLD.row_version THEN
            RAISE EXCEPTION
              'pricing_plan: revision % of plan % is frozen; only a sanctioned lifecycle_state flip is permitted',
              OLD.revision, OLD.plan_id;
          END IF;

          IF NOT (OLD.lifecycle_state = 'published'
                  AND NEW.lifecycle_state IN ('superseded','retired')) THEN
            RAISE EXCEPTION 'pricing_plan: lifecycle_state % -> % is not a sanctioned flip',
              OLD.lifecycle_state, NEW.lifecycle_state;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_plan_append_only
        BEFORE UPDATE OR DELETE ON bss.pricing_plan
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_plan_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_plan",
    "DROP FUNCTION IF EXISTS bss.pricing_plan_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transforms from the Postgres variant:
// * schema prefix `bss.` dropped (single namespace);
// * `uuid` -> `text`, `timestamptz` -> `text`;
// * `now()` -> `(CURRENT_TIMESTAMP)`;
// * the single PL/pgSQL trigger becomes four `RAISE(ABORT, ...)` triggers
//   (SQLite has no `BEFORE UPDATE OR DELETE`, no procedural language, and no
//   message interpolation), and `IS DISTINCT FROM` becomes `IS NOT`. The
//   fourth is the draft plane's exit whitelist, spelled as the same membership
//   test the Postgres branch uses so the two planes cannot drift apart.
// Every CHECK, index and PK is preserved.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_plan (
        plan_id         text    NOT NULL,
        revision        bigint  NOT NULL,
        tenant_id       text    NOT NULL,
        sku_id          text,
        plan_tier       text,
        billing_cycle   text,
        lifecycle_state text    NOT NULL,
        available_from  text,
        available_to    text,
        created_by      text    NOT NULL,
        created_at_utc  text    NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        row_version     bigint  NOT NULL DEFAULT 0,
        PRIMARY KEY (plan_id, revision),
        CONSTRAINT chk_pricing_plan_lifecycle_state CHECK (
            lifecycle_state IN ('draft','abandoned','published','superseded','retired')),
        CONSTRAINT chk_pricing_plan_revision CHECK (revision >= 0),
        CONSTRAINT chk_pricing_plan_availability CHECK (
            available_from IS NULL OR available_to IS NULL OR available_to > available_from)
    )",
    "CREATE UNIQUE INDEX uq_pricing_plan_current
        ON pricing_plan (plan_id)
        WHERE lifecycle_state IN ('published','retired')",
    "CREATE UNIQUE INDEX uq_pricing_plan_open_draft
        ON pricing_plan (plan_id)
        WHERE lifecycle_state = 'draft'",
    "CREATE INDEX idx_pricing_plan_tenant
        ON pricing_plan (tenant_id, plan_id, revision)",
    "CREATE TRIGGER trg_pricing_plan_frozen_columns
        BEFORE UPDATE ON pricing_plan
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND (NEW.plan_id        IS NOT OLD.plan_id
            OR NEW.revision       IS NOT OLD.revision
            OR NEW.tenant_id      IS NOT OLD.tenant_id
            OR NEW.sku_id         IS NOT OLD.sku_id
            OR NEW.plan_tier      IS NOT OLD.plan_tier
            OR NEW.billing_cycle  IS NOT OLD.billing_cycle
            OR NEW.available_from IS NOT OLD.available_from
            OR NEW.available_to   IS NOT OLD.available_to
            OR NEW.created_by     IS NOT OLD.created_by
            OR NEW.created_at_utc IS NOT OLD.created_at_utc
            OR NEW.row_version    IS NOT OLD.row_version)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_plan: revision is frozen; only a sanctioned lifecycle_state flip is permitted');
        END",
    "CREATE TRIGGER trg_pricing_plan_flip_whitelist
        BEFORE UPDATE ON pricing_plan
        FOR EACH ROW WHEN OLD.lifecycle_state <> 'draft'
          AND NOT (OLD.lifecycle_state = 'published'
                   AND NEW.lifecycle_state IN ('superseded','retired'))
        BEGIN
          SELECT RAISE(ABORT, 'pricing_plan: lifecycle_state transition is not a sanctioned flip');
        END",
    "CREATE TRIGGER trg_pricing_plan_draft_flip_whitelist
        BEFORE UPDATE ON pricing_plan
        FOR EACH ROW WHEN OLD.lifecycle_state = 'draft'
          AND NEW.lifecycle_state NOT IN ('draft','published','abandoned')
        BEGIN
          SELECT RAISE(ABORT, 'pricing_plan: lifecycle_state transition is not a sanctioned flip');
        END",
    "CREATE TRIGGER trg_pricing_plan_no_delete
        BEFORE DELETE ON pricing_plan
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_plan: DELETE of a revision is not permitted; a discarded draft revision is abandoned');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_plan"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
