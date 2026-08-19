//! `pricing_migration`'s primary key gains `tenant_id`.
//!
//! `migration_id` is **client-supplied** — `m20260802_000043`'s own module doc
//! opens with "`migration_id` is client-supplied, and that is the whole of M2" —
//! and it arrives straight off the request body. Until this migration it was also
//! the table's entire `PRIMARY KEY`, so the namespace of a client-chosen
//! identifier was the whole deployment rather than the tenant.
//!
//! # Two defects in one column, and the second has no remedy
//!
//! `migration_repo::insert_or_load` is `ON CONFLICT DO NOTHING`, then a
//! tenant-filtered `load`, then `ok_or_else(ConcurrentMutation)`. With a global
//! conflict target:
//!
//! - **An existence oracle on a live authenticated route.** Tenant B posting a
//!   `migration_id` tenant A already holds takes the `DO NOTHING` branch, so the
//!   `load` finds nothing under B and B is answered `CONCURRENT_MUTATION` (409);
//!   an unused id is answered 202. That difference is observable and is a fact
//!   about another tenant's rows.
//! - **A permanent cross-tenant denial.** The refusal says *retry*, and a retry
//!   collides identically, forever. Any tenant could reserve arbitrary migration
//!   ids against every other tenant, and the victim has no remedy at all:
//!   `trg_pricing_migration_no_delete` refuses the `DELETE` that would free it.
//!
//! It is filed High rather than Critical because no other tenant's *content*
//! becomes readable and nothing about financial integrity breaks; exploiting the
//! oracle also requires knowing or predicting the uuid, so with random ids it
//! amplifies an identifier leak rather than standing on its own.
//!
//! # Every sibling already did this
//!
//! `pricing_migration` was the last client-key store in the crate with a global
//! uniqueness namespace. `synthesis_repo` scopes its conflict target to
//! `(tenant_id, subscription_ref)` — with a doc block reasoning about *which*
//! target is right — `idempotency_repo` to `(tenant_id, operation, client_key)`,
//! and `bulk_repo`'s moved to `(tenant_id, kind, client_key)` under D-307, which
//! `m20260802_000064` performed one migration ago. So this is the same correction
//! applied to the one table that was missed, not a new position.
//!
//! # Why Postgres is two statements and `SQLite` is a rebuild
//!
//! `m20260802_000063`'s asymmetry exactly. Postgres names its primary key as a
//! droppable constraint; `SQLite` spells `PRIMARY KEY` inside the `CREATE TABLE`
//! and offers no `ALTER` that reaches it, so the table is rebuilt whole — and
//! `DROP TABLE` takes the indexes and the triggers with it, so all of them are
//! recreated after the rename.
//!
//! # `idx_pricing_migration_tenant` is dropped rather than recreated
//!
//! It indexed `(tenant_id, migration_id)`, which is now exactly the primary key's
//! own index on both engines. Keeping it would be a second index over one column
//! pair, maintained on every write, serving reads the key already serves. The
//! roster in `tests/sqlite_migrations.rs` moves with it — the index is gone, not
//! renamed, and a roster still naming it would be asserting a shape the schema no
//! longer has.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_migration DROP CONSTRAINT pricing_migration_pkey",
    "ALTER TABLE bss.pricing_migration
        ADD CONSTRAINT pricing_migration_pkey PRIMARY KEY (tenant_id, migration_id)",
    "DROP INDEX IF EXISTS bss.idx_pricing_migration_tenant",
];

const PG_DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE bss.pricing_migration DROP CONSTRAINT pricing_migration_pkey",
    "ALTER TABLE bss.pricing_migration
        ADD CONSTRAINT pricing_migration_pkey PRIMARY KEY (migration_id)",
    "CREATE INDEX idx_pricing_migration_tenant
        ON bss.pricing_migration (tenant_id, migration_id)",
];

/// The rebuild, parameterised by the primary key so `up` and `down` cannot drift
/// in any other respect — the columns, the eleven `CHECK`s, the three surviving
/// indexes and all **five** triggers are written once.
///
/// The index and trigger statements are the ones `m20260802_000043` created,
/// carried over character for character. That is not fussiness: retyping them
/// from a reading of the original dropped `trg_pricing_migration_flip_whitelist`
/// and `trg_pricing_migration_exclusion_replay` entirely and gave
/// `trg_pricing_migration_frozen_columns` a shorter refusal message — a rebuild
/// that silently sheds enforcement, which is the one failure mode this whole
/// exercise is about. `sqlite_migration_repo` caught it because it asserts the
/// refusal *text*.
macro_rules! sqlite_rebuild {
    ($pk:literal) => {
        &[
            concat!(
                "CREATE TABLE pricing_migration_rebuilt (
        migration_id       text NOT NULL,
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
        PRIMARY KEY (",
                $pk,
                "),
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
    )"
            ),
            "INSERT INTO pricing_migration_rebuilt (
        migration_id, tenant_id, source_plan_id, source_revision, target_plan_id,
        effective_at, announced_at, scope, state, delta_report, exclusion_snapshot,
        completion_record, created_by, created_at, started_at, completed_at, cancelled_at)
     SELECT
        migration_id, tenant_id, source_plan_id, source_revision, target_plan_id,
        effective_at, announced_at, scope, state, delta_report, exclusion_snapshot,
        completion_record, created_by, created_at, started_at, completed_at, cancelled_at
     FROM pricing_migration",
            "DROP TABLE pricing_migration",
            "ALTER TABLE pricing_migration_rebuilt RENAME TO pricing_migration",
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
        ]
    };
}

const SQLITE_UP_STATEMENTS: &[&str] = sqlite_rebuild!("tenant_id, migration_id");

const SQLITE_DOWN_STATEMENTS: &[&str] = sqlite_rebuild!("migration_id");

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(manager, PG_DOWN_STATEMENTS, SQLITE_DOWN_STATEMENTS).await
    }
}
