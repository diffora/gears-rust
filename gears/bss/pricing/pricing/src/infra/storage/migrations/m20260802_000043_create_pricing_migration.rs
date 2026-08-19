//! Create `bss.pricing_migration` — one scheduled plan migration
//! (`design/11-lifecycle.md` §6, `cpt-cf-bss-pricing-state-migration`).
//!
//! One row is one migration: a source plan revision, a published target, an
//! effective date, a scope, and the §4 state machine that carries it from
//! `scheduled` to a terminal state. The catalog never mutates a subscription and
//! never touches a posted invoice (M1) — this row is a **schedule**, and
//! Subscriptions is what executes against it.
//!
//! # `migration_id` is client-supplied, and that is the whole of M2
//!
//! `inst-ms-api` makes the create idempotent on a **client-supplied**
//! `migration_id`, mirroring Slice 12's `run_id`: a timed-out client retry
//! returns the original schedule, never a second one. So the primary key is not a
//! minted surrogate, and the idempotency is the key rather than a dedup table
//! beside it. A second `POST` carrying a `migration_id` that already exists is
//! answered from the stored row; there is no path by which two rows can describe
//! one migration, because the store will not hold two.
//!
//! # The §4 state machine lives in the constraints, not only in the domain
//!
//! `pricing_price_window`'s reason, one table over: a rule that lives only in
//! application code is one ad-hoc `UPDATE` away from being bypassed, and what it
//! would bypass here is whether an executor that has already re-bound a thousand
//! subscriptions is allowed to be told the run was never real. Four edges, and
//! nothing else:
//!
//! ```text
//! scheduled   -> in_progress   (`inst-mst-start`, and only via POST .../start)
//! scheduled   -> cancelled     (`inst-mst-cancel`, M3 — nothing executed yet)
//! in_progress -> cancelled     (`inst-mst-cancel-inflight`, D-34)
//! in_progress -> completed     (`inst-mst-complete`)
//! ```
//!
//! **`completed` is terminal and uncancellable, and that is D-34's own sentence
//! made physical.** There is no `completed -> cancelled` edge, so the 409 the
//! route answers (`MIGRATION_COMPLETED`) is guarded at two layers rather than
//! one. `in_progress -> cancelled` **is** an edge, which is the part of D-34 that
//! changed on 2026-08-07: the stop-the-bleeding control halts further `PlanLink`
//! processing, and already-migrated subscriptions are unaffected because the
//! catalog never held their state to begin with.
//!
//! # `DELETE` is refused, and the REST `DELETE` is not a deletion
//!
//! `DELETE /bss-pricing/v1/migrations/{id}` **cancels**; it does not remove the
//! row. The trigger refuses the statement outright, in `pricing_price_window`'s
//! words ("cancel is a state, not a deletion") and for a sharper reason: the
//! schedule is the record an executor re-reads before each batch
//! (`inst-mg-cancel`'s state handshake), so a deleted row would read as "no such
//! migration" to a party whose correct behaviour is to **stop**. Absence and
//! cancellation must not be the same answer on that lane.
//!
//! # The flip timestamps, and the one biconditional that is deliberately not one
//!
//! `completed_at` and `cancelled_at` are plain biconditionals against their
//! states, in `chk_pricing_approval_decided_at`'s idiom. `started_at` is **not**,
//! and the asymmetry is a fact about §4 rather than an omission: `cancelled` is
//! reachable from *both* `scheduled` (never started) and `in_progress` (started),
//! so a biconditional would have to refuse one of D-34's two cancel edges. It is
//! written as the two implications that are actually true —
//! `in_progress`/`completed` **require** a start, and `scheduled` **forbids** one
//! — leaving `cancelled` admitting either, which is exactly the reachable set.
//!
//! # `exclusion_snapshot` is co-nullable with `started_at`, and that is D-65
//!
//! D-65 (sharpened 2026-07-31) makes `POST .../start` **persist-and-replay**: the
//! exclusion set is computed once at the first call, persisted, and repeat calls
//! return that stored snapshot verbatim without re-transitioning and without
//! re-running the D-36 re-validation, because a recompute could differ from the
//! set the executor already honoured. The biconditional
//! `(started_at IS NOT NULL) = (exclusion_snapshot IS NOT NULL)` makes the
//! "persist" half physical: a row cannot be `in_progress` without the snapshot
//! its executor was handed, so there is no state in which a replay has nothing to
//! replay.
//!
//! # `delta_report` is frozen and `exclusion_snapshot` is not, on purpose
//!
//! §6 calls `delta_report` the deltas "**at schedule time**", so it is in the
//! frozen-column whitelist: it is evidence of what the operator confirmed
//! against, and an operator who approved a schedule against one report must not
//! find a different one on the row afterwards. D-36's execution-time
//! re-resolution deliberately lands in a **different** column, so "what was known
//! when this was scheduled" and "what was true when it ran" are two facts the row
//! holds separately rather than one the second overwrites.
//!
//! # `announced_at`, and the half of D-49 a row-local `CHECK` can carry
//!
//! D-49 validates `effective_at >= announcement + the tenant's configured notice
//! period`, floor 60 days, and the notice value lives in
//! `pricing_policy_object.enforced_migration_notice_days` — **another table**, so
//! no `CHECK` here can state the rule. What is row-local is its weakest half,
//! `announced_at <= effective_at`, and that is the one written: whatever the
//! configured notice was, the row cannot record a migration that took effect
//! before it was announced. `m20260802_000016`'s split, verbatim in shape — the
//! durable half is here and the configured half is
//! [`domain::migration`](crate::domain::migration)'s, which is also where the
//! `MIGRATION_NOTICE_TOO_SHORT` refusal is produced.
//!
//! # No foreign keys, for `m20260802_000024`'s reason
//!
//! `source_plan_id` and `target_plan_id` name plans, and `pricing_plan` is keyed
//! `(plan_id, revision)` (D-56) with uniqueness on `plan_id` alone only in two
//! **partial** indexes. Postgres refuses a partial index as a foreign key's
//! referent, so neither reference is expressible, exactly as `pricing_bundle`
//! found. The references are enforced one layer up instead: scheduling resolves
//! both plans through `plan_repo::load_current` and refuses a target that is not
//! published (`MIGRATION_TARGET_INVALID`), which is a stronger predicate than a
//! foreign key could state anyway — an FK would admit a `draft` target.
//!
//! # An addition §6 does not state: `source_plan_id <> target_plan_id`
//!
//! Reported as an addition rather than presented as transcription. §6 lists the
//! two columns and says nothing about their relationship, but a migration from a
//! plan to itself is not a degenerate case with a sensible answer: it would emit
//! `PlanMigrationScheduled` asking Subscriptions to create `PlanLink`s onto the
//! plan every subscriber is already on. It is refused here because the refusal is
//! total and row-local; nothing in the design set contradicts it.
//!
//! **Backend differences.** The systematic mirror of this chain: `bss.` dropped,
//! `uuid` -> `text`, `timestamptz` -> `text`, `jsonb` -> `text`, `now()` ->
//! `(CURRENT_TIMESTAMP)`, and the single PL/pgSQL trigger split into four
//! `RAISE(ABORT, ...)` triggers, since `SQLite` has no procedural language, no
//! `BEFORE UPDATE OR DELETE` and no message interpolation. Each arm below the
//! terminal one repeats that arm's exclusion in its `WHEN`, so exactly one fires
//! on any one statement, in the Postgres order. Every `CHECK`, index and the
//! primary key are preserved name for name. The Postgres `down` drops the
//! function as well as the table; the `SQLite` one drops only the table.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// Postgres variant - canonical production schema (bss-qualified DDL).
// ---------------------------------------------------------------------------

const PG_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE bss.pricing_migration (
        migration_id       uuid        NOT NULL PRIMARY KEY,
        tenant_id          uuid        NOT NULL,
        source_plan_id     uuid        NOT NULL,
        source_revision    integer     NOT NULL,
        target_plan_id     uuid        NOT NULL,
        effective_at       timestamptz NOT NULL,
        announced_at       timestamptz NOT NULL,
        scope              jsonb       NOT NULL,
        state              text        NOT NULL,
        delta_report       jsonb       NOT NULL,
        exclusion_snapshot jsonb,
        completion_record  jsonb,
        created_by         uuid        NOT NULL,
        created_at         timestamptz NOT NULL DEFAULT now(),
        started_at         timestamptz,
        completed_at       timestamptz,
        cancelled_at       timestamptz,
        CONSTRAINT chk_pricing_migration_state CHECK (
            state IN ('scheduled','in_progress','completed','cancelled')),
        -- A revision number is an ordinal, and 0 is the first one a plan mints.
        CONSTRAINT chk_pricing_migration_source_revision CHECK (
            source_revision >= 0),
        -- See the module doc: an addition to Section 6, not a transcription of it.
        CONSTRAINT chk_pricing_migration_distinct_plans CHECK (
            source_plan_id <> target_plan_id),
        -- D-49's row-local half. The configured notice period lives in
        -- pricing_policy_object, so the rule itself is the domain's; what cannot
        -- be true whatever that value is, is an effect preceding its announcement.
        CONSTRAINT chk_pricing_migration_announced_before_effective CHECK (
            announced_at <= effective_at),
        -- Section 4's reachable set. `cancelled` is reachable from both
        -- `scheduled` and `in_progress` (D-34), so it admits either, and these are
        -- the two implications that hold rather than a biconditional that does not.
        CONSTRAINT chk_pricing_migration_started_required CHECK (
            state NOT IN ('in_progress','completed') OR started_at IS NOT NULL),
        CONSTRAINT chk_pricing_migration_scheduled_unstarted CHECK (
            state <> 'scheduled' OR started_at IS NULL),
        CONSTRAINT chk_pricing_migration_completed_at CHECK (
            (state = 'completed') = (completed_at IS NOT NULL)),
        CONSTRAINT chk_pricing_migration_cancelled_at CHECK (
            (state = 'cancelled') = (cancelled_at IS NOT NULL)),
        -- D-65's persist half: there is no started run without the exclusion set
        -- its executor was handed, so a replay always has something to replay.
        CONSTRAINT chk_pricing_migration_exclusion_snapshot CHECK (
            (started_at IS NOT NULL) = (exclusion_snapshot IS NOT NULL)),
        -- The flip instants cannot precede the row they belong to.
        CONSTRAINT chk_pricing_migration_started_order CHECK (
            started_at IS NULL OR started_at >= created_at),
        CONSTRAINT chk_pricing_migration_completed_order CHECK (
            completed_at IS NULL OR completed_at >= started_at),
        CONSTRAINT chk_pricing_migration_cancelled_order CHECK (
            cancelled_at IS NULL OR cancelled_at >= created_at)
    )",
    // The tenant-scoped read every operator surface makes.
    "CREATE INDEX idx_pricing_migration_tenant
        ON bss.pricing_migration (tenant_id, migration_id)",
    // `RETIRE_TARGET_OF_MIGRATION`: retirement asks whether an in-flight
    // migration targets the plan being retired, which is a probe on the target.
    "CREATE INDEX idx_pricing_migration_target
        ON bss.pricing_migration (tenant_id, target_plan_id)",
    // Listing a source plan's schedules, and the retirement pairing Section 3
    // step 4 recommends.
    //
    // **The reader is owed and does not exist** (review Z1-3):
    // `grep -rn "Column::SourcePlanId" src/infra/storage/repo/` returns 0. The
    // column is written, read back into DTOs and echoed to callers, but nothing
    // ever *filters* on it, so this index is never the access path — unlike
    // `idx_pricing_migration_target` one line up, whose `RETIRE_TARGET_OF_MIGRATION`
    // reader is real. The owed surface is "list the migrations off a plan". Named
    // here rather than left to be discovered, which is `entity/policy_object`'s
    // treatment of the same situation; the cost until then is write amplification
    // on an append-only table and nothing else.
    // Section 10's `pricing.migration.stalled` alarm scans `in_progress` rows
    // past an expected completion horizon.
    "CREATE INDEX idx_pricing_migration_due
        ON bss.pricing_migration (state, effective_at)",
    "CREATE OR REPLACE FUNCTION bss.pricing_migration_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION
              'pricing_migration: DELETE of migration % is not permitted; cancel is a state, not a deletion, and an executor must be able to tell a cancelled run from an absent one',
              OLD.migration_id;
          END IF;

          IF OLD.state IN ('completed','cancelled') THEN
            RAISE EXCEPTION
              'pricing_migration: migration % is %; a completed or cancelled run is immutable history',
              OLD.migration_id, OLD.state;
          END IF;

          IF NEW.migration_id    IS DISTINCT FROM OLD.migration_id
          OR NEW.tenant_id       IS DISTINCT FROM OLD.tenant_id
          OR NEW.source_plan_id  IS DISTINCT FROM OLD.source_plan_id
          OR NEW.source_revision IS DISTINCT FROM OLD.source_revision
          OR NEW.target_plan_id  IS DISTINCT FROM OLD.target_plan_id
          OR NEW.effective_at    IS DISTINCT FROM OLD.effective_at
          OR NEW.announced_at    IS DISTINCT FROM OLD.announced_at
          OR NEW.scope           IS DISTINCT FROM OLD.scope
          OR NEW.delta_report    IS DISTINCT FROM OLD.delta_report
          OR NEW.created_by      IS DISTINCT FROM OLD.created_by
          OR NEW.created_at      IS DISTINCT FROM OLD.created_at THEN
            RAISE EXCEPTION
              'pricing_migration: migration % is bound to its source, target, effective date, scope and schedule-time delta report; only state, the execution records and the flip timestamps may move',
              OLD.migration_id;
          END IF;

          IF NEW.state IS DISTINCT FROM OLD.state
             AND NOT (OLD.state = 'scheduled'   AND NEW.state IN ('in_progress','cancelled'))
             AND NOT (OLD.state = 'in_progress' AND NEW.state IN ('completed','cancelled')) THEN
            RAISE EXCEPTION
              'pricing_migration: state % -> % is not a sanctioned transition',
              OLD.state, NEW.state;
          END IF;

          -- D-65's replay half: once persisted, the exclusion set an executor was
          -- handed is what every repeat call must be answered with.
          IF OLD.exclusion_snapshot IS NOT NULL
             AND NEW.exclusion_snapshot IS DISTINCT FROM OLD.exclusion_snapshot THEN
            RAISE EXCEPTION
              'pricing_migration: the exclusion set of migration % is computed once and replayed verbatim; a recompute could differ from the set the executor already honoured',
              OLD.migration_id;
          END IF;

          RETURN NEW;
        END;
     $$ LANGUAGE plpgsql",
    "CREATE TRIGGER trg_pricing_migration_append_only
        BEFORE UPDATE OR DELETE ON bss.pricing_migration
        FOR EACH ROW EXECUTE FUNCTION bss.pricing_migration_append_only()",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "DROP TABLE IF EXISTS bss.pricing_migration",
    "DROP FUNCTION IF EXISTS bss.pricing_migration_append_only()",
];

// ---------------------------------------------------------------------------
// SQLite variant - non-production schema for fast tests / dev.
// ---------------------------------------------------------------------------
//
// Systematic transforms from the Postgres variant:
// * schema prefix `bss.` dropped (single namespace);
// * `uuid` -> `text`, `timestamptz` -> `text`, `jsonb` -> `text`;
// * `now()` -> `(CURRENT_TIMESTAMP)`;
// * the single PL/pgSQL trigger becomes four `RAISE(ABORT, ...)` triggers, and
//   every arm below the terminal one repeats that arm's exclusion in its `WHEN`
//   so one statement fires one trigger, in the Postgres order.
// Every CHECK, the primary key and all four indexes are preserved name for name.
// All instant comparisons here are between two STORED instants, which this gear's
// writers render identically (RFC 3339, `+00:00`), so lexicographic comparison is
// exact and no `datetime(...)` normalization is wanted - unlike
// `m20260802_000016`'s arm 5, whose other side is the clock. Nothing in this
// table compares against `CURRENT_TIMESTAMP`.

const SQLITE_UP_STATEMENTS: &[&str] = &[
    "CREATE TABLE pricing_migration (
        migration_id       text NOT NULL PRIMARY KEY,
        tenant_id          text NOT NULL,
        source_plan_id     text NOT NULL,
        source_revision    integer NOT NULL,
        target_plan_id     text NOT NULL,
        effective_at       text NOT NULL,
        announced_at       text NOT NULL,
        scope              text NOT NULL,
        state              text NOT NULL,
        delta_report       text NOT NULL,
        exclusion_snapshot text,
        completion_record  text,
        created_by         text NOT NULL,
        created_at         text NOT NULL DEFAULT (CURRENT_TIMESTAMP),
        started_at         text,
        completed_at       text,
        cancelled_at       text,
        CONSTRAINT chk_pricing_migration_state CHECK (
            state IN ('scheduled','in_progress','completed','cancelled')),
        CONSTRAINT chk_pricing_migration_source_revision CHECK (
            source_revision >= 0),
        CONSTRAINT chk_pricing_migration_distinct_plans CHECK (
            source_plan_id <> target_plan_id),
        CONSTRAINT chk_pricing_migration_announced_before_effective CHECK (
            announced_at <= effective_at),
        CONSTRAINT chk_pricing_migration_started_required CHECK (
            state NOT IN ('in_progress','completed') OR started_at IS NOT NULL),
        CONSTRAINT chk_pricing_migration_scheduled_unstarted CHECK (
            state <> 'scheduled' OR started_at IS NULL),
        CONSTRAINT chk_pricing_migration_completed_at CHECK (
            (state = 'completed') = (completed_at IS NOT NULL)),
        CONSTRAINT chk_pricing_migration_cancelled_at CHECK (
            (state = 'cancelled') = (cancelled_at IS NOT NULL)),
        CONSTRAINT chk_pricing_migration_exclusion_snapshot CHECK (
            (started_at IS NOT NULL) = (exclusion_snapshot IS NOT NULL)),
        CONSTRAINT chk_pricing_migration_started_order CHECK (
            started_at IS NULL OR started_at >= created_at),
        CONSTRAINT chk_pricing_migration_completed_order CHECK (
            completed_at IS NULL OR completed_at >= started_at),
        CONSTRAINT chk_pricing_migration_cancelled_order CHECK (
            cancelled_at IS NULL OR cancelled_at >= created_at)
    )",
    "CREATE INDEX idx_pricing_migration_tenant
        ON pricing_migration (tenant_id, migration_id)",
    "CREATE INDEX idx_pricing_migration_target
        ON pricing_migration (tenant_id, target_plan_id)",
    "CREATE INDEX idx_pricing_migration_source
        ON pricing_migration (tenant_id, source_plan_id)",
    "CREATE INDEX idx_pricing_migration_due
        ON pricing_migration (state, effective_at)",
    "CREATE TRIGGER trg_pricing_migration_no_delete
        BEFORE DELETE ON pricing_migration
        FOR EACH ROW
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_migration: DELETE of a migration is not permitted; cancel is a state, not a deletion, and an executor must be able to tell a cancelled run from an absent one');
        END",
    "CREATE TRIGGER trg_pricing_migration_immutable_history
        BEFORE UPDATE ON pricing_migration
        FOR EACH ROW WHEN OLD.state IN ('completed','cancelled')
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_migration: a completed or cancelled run is immutable history');
        END",
    "CREATE TRIGGER trg_pricing_migration_frozen_columns
        BEFORE UPDATE ON pricing_migration
        FOR EACH ROW WHEN OLD.state NOT IN ('completed','cancelled')
          AND (NEW.migration_id    IS NOT OLD.migration_id
            OR NEW.tenant_id       IS NOT OLD.tenant_id
            OR NEW.source_plan_id  IS NOT OLD.source_plan_id
            OR NEW.source_revision IS NOT OLD.source_revision
            OR NEW.target_plan_id  IS NOT OLD.target_plan_id
            OR NEW.effective_at    IS NOT OLD.effective_at
            OR NEW.announced_at    IS NOT OLD.announced_at
            OR NEW.scope           IS NOT OLD.scope
            OR NEW.delta_report    IS NOT OLD.delta_report
            OR NEW.created_by      IS NOT OLD.created_by
            OR NEW.created_at      IS NOT OLD.created_at)
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_migration: the migration is bound to its source, target, effective date, scope and schedule-time delta report; only state, the execution records and the flip timestamps may move');
        END",
    "CREATE TRIGGER trg_pricing_migration_flip_whitelist
        BEFORE UPDATE ON pricing_migration
        FOR EACH ROW WHEN OLD.state NOT IN ('completed','cancelled')
          AND NEW.state IS NOT OLD.state
          AND NOT (OLD.state = 'scheduled'   AND NEW.state IN ('in_progress','cancelled'))
          AND NOT (OLD.state = 'in_progress' AND NEW.state IN ('completed','cancelled'))
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_migration: state transition is not a sanctioned one');
        END",
    "CREATE TRIGGER trg_pricing_migration_exclusion_replay
        BEFORE UPDATE ON pricing_migration
        FOR EACH ROW WHEN OLD.state NOT IN ('completed','cancelled')
          AND OLD.exclusion_snapshot IS NOT NULL
          AND NEW.exclusion_snapshot IS NOT OLD.exclusion_snapshot
        BEGIN
          SELECT RAISE(ABORT,
            'pricing_migration: the exclusion set is computed once and replayed verbatim; a recompute could differ from the set the executor already honoured');
        END",
];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS pricing_migration"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
