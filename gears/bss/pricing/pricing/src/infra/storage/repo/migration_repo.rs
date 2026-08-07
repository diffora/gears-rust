//! The migration schedule's storage half (`inst-ms-api`, `inst-mg-idem`,
//! `inst-mst-start`, `inst-mst-complete`, `inst-mst-cancel`,
//! `inst-mst-cancel-inflight`, D-34, D-65).
//!
//! A runner-taking repository rather than a provider-holding one, for
//! [`super::audit_repo`]'s reason: every write here belongs to a transaction
//! that also emits an event or appends an audit record, and a schedule that
//! could commit separately from the `PlanMigrationScheduled` announcing it is a
//! schedule Subscriptions may never hear about.
//!
//! # M2 is the primary key, and [`insert_or_load`] is the whole of it
//!
//! `inst-ms-api` makes the create idempotent on a **client-supplied**
//! `migration_id`. The implementation is an `INSERT ... ON CONFLICT DO NOTHING`
//! followed by a load — [`super::idempotency_repo`]'s shape, and for the same
//! reason it gives there: the at-most-once guarantee **is** the statement, so two
//! concurrent creates of one `migration_id` cannot both succeed however they
//! interleave. A read-then-insert would be a race with a window exactly as wide
//! as the round trip.
//!
//! **A conflicting retry returns the stored row rather than an error.** That is
//! `inst-ms-api`'s own sentence — "a timed-out client retry returns the original
//! schedule, never a second one" — and it is why this function's name says
//! `or_load` rather than `or_fail`. The caller learns which happened from
//! [`Scheduled::created`], because a route that answered `202 Created` to a
//! replay would tell a client it had just scheduled something it scheduled an
//! hour ago.
//!
//! # The flips are compare-and-swaps on the state they leave
//!
//! `plan_repo::retire_revision`'s shape. Each `UPDATE` carries the **expected
//! current state** in its `WHERE`, so a concurrent writer that moved the row
//! first loses rather than overwrites: `rows_affected == 0` is a lost race and is
//! reported as [`RepoError::ConcurrentMutation`], never as a silent success. The
//! table's trigger refuses the same edges independently — the machine is stated
//! in `domain::migration`, in the `CHECK`s and here, and this layer is the one
//! that makes the refusal a typed error rather than a driver string.
//!
//! # D-65's replay is a read, not a second write
//!
//! [`start`] is the only flip that can be called twice in the ordinary course of
//! events, because Subscriptions retries. It resolves that **before** attempting
//! the `UPDATE`: an already-`in_progress` row returns its persisted
//! `exclusion_snapshot` verbatim and re-transitions nothing. That ordering is the
//! rule rather than an optimisation — a recompute could differ from the set the
//! executor already honoured, and the table's `trg_pricing_migration_exclusion_replay`
//! arm refuses the write that would install a different one.

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, OnConflict, SimpleExpr};
use sea_orm::{ColumnTrait, Condition, DbErr, EntityTrait};
use serde_json::Value as JsonValue;
use toolkit_db::secure::{
    AccessScope, DBRunner, ScopeError, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use crate::domain::migration::MigrationState;
use crate::domain::scope_key::PlanId;
use crate::infra::storage::entity::migration;
use crate::infra::storage::{RepoError, contention_or_db};

/// A migration schedule as this gear holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationRecord {
    /// The client-supplied id, and the idempotency key.
    pub migration_id: Uuid,
    /// The retiring side.
    pub source_plan_id: PlanId,
    /// The source plan's revision at schedule time.
    pub source_revision: u64,
    /// The published target.
    pub target_plan_id: PlanId,
    /// When the migration takes effect.
    pub effective_at: DateTime<Utc>,
    /// The instant D-49's notice period is measured from.
    pub announced_at: DateTime<Utc>,
    /// `all` or a subscription filter.
    pub scope: JsonValue,
    /// Where the run stands in §4's machine.
    pub state: MigrationState,
    /// The deltas as they were at schedule time.
    pub delta_report: JsonValue,
    /// D-65's persisted exclusion set; `None` until the first `start`.
    pub exclusion_snapshot: Option<JsonValue>,
    /// The completion or partial-cancel record.
    pub completion_record: Option<JsonValue>,
}

/// The row a schedule is created from.
#[derive(Clone, Debug)]
pub struct NewMigration {
    /// Client-supplied (`inst-ms-api`).
    pub migration_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The retiring side.
    pub source_plan_id: PlanId,
    /// The source plan's revision at schedule time.
    pub source_revision: u64,
    /// The published target.
    pub target_plan_id: PlanId,
    /// When the migration takes effect.
    pub effective_at: DateTime<Utc>,
    /// D-49's announcement instant — the scheduling commit.
    pub announced_at: DateTime<Utc>,
    /// `all` or a subscription filter.
    pub scope: JsonValue,
    /// The schedule-time delta report.
    pub delta_report: JsonValue,
    /// The principal scheduling it.
    pub created_by: Uuid,
    /// The commit instant.
    pub created_at: DateTime<Utc>,
}

/// What [`insert_or_load`] answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheduled {
    /// The schedule, whether it was just created or already existed.
    pub record: MigrationRecord,
    /// `false` when this call was a replay of an earlier create (M2).
    pub created: bool,
}

/// Create a schedule, or return the one this `migration_id` already names
/// (`inst-ms-api`, `inst-mg-idem`, M2).
///
/// # Errors
/// [`RepoError::Db`] on a storage failure; [`RepoError::CorruptRow`] when the
/// stored row cannot be read back into the domain's vocabulary;
/// [`RepoError::ConcurrentMutation`] when the insert is refused and the row it
/// conflicted with has vanished, which the table's `DELETE` ban makes
/// unreachable in practice and which is reported rather than unwrapped.
pub async fn insert_or_load(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewMigration,
) -> Result<Scheduled, RepoError> {
    let Ok(revision) = i32::try_from(new.source_revision) else {
        return Err(RepoError::CorruptRow(format!(
            "plan {} stands at a revision {} no column can address",
            new.source_plan_id, new.source_revision
        )));
    };

    let model = migration::ActiveModel {
        migration_id: Set(new.migration_id),
        tenant_id: Set(new.tenant_id),
        source_plan_id: Set(new.source_plan_id.get()),
        source_revision: Set(revision),
        target_plan_id: Set(new.target_plan_id.get()),
        effective_at: Set(new.effective_at),
        announced_at: Set(new.announced_at),
        scope: Set(new.scope),
        state: Set(MigrationState::Scheduled.as_str().to_owned()),
        delta_report: Set(new.delta_report),
        exclusion_snapshot: Set(None),
        completion_record: Set(None),
        created_by: Set(new.created_by),
        created_at: Set(new.created_at),
        started_at: Set(None),
        completed_at: Set(None),
        cancelled_at: Set(None),
    };

    let on_conflict = OnConflict::column(migration::Column::MigrationId)
        .do_nothing()
        .to_owned();

    // The at-most-once guarantee **is** this statement; see the module doc.
    // `RecordNotInserted` is the conflict swallowing the insert, which is
    // `idempotency_repo::claim`'s signal and means the same thing here: this
    // `migration_id` was scheduled before, so the create is a replay.
    let created = match migration::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| RepoError::Db(format!("migration schedule scope: {e}")))?
        .on_conflict_raw(on_conflict)
        .exec(runner)
        .await
    {
        Ok(_) => true,
        Err(ScopeError::Db(DbErr::RecordNotInserted)) => false,
        Err(e) => {
            return Err(RepoError::Db(format!(
                "schedule migration {}: {e}",
                new.migration_id
            )));
        }
    };

    let record = load(runner, scope, new.tenant_id, new.migration_id)
        .await?
        .ok_or_else(|| RepoError::ConcurrentMutation {
            aggregate: format!("migration {}", new.migration_id),
        })?;

    Ok(Scheduled { record, created })
}

/// Read one schedule.
///
/// # Errors
/// [`RepoError::Db`] on a storage failure; [`RepoError::CorruptRow`] when the
/// stored state token is outside the column's `CHECK`.
pub async fn load(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    migration_id: Uuid,
) -> Result<Option<MigrationRecord>, RepoError> {
    let row = migration::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(migration::Column::TenantId.eq(tenant_id))
                .add(migration::Column::MigrationId.eq(migration_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read migration {migration_id}: {e}")))?;

    row.map(into_record).transpose()
}

/// The **live** migrations aimed at a plan (`RETIRE_TARGET_OF_MIGRATION`).
///
/// Live means `scheduled` or `in_progress`: a completed or cancelled run moves
/// nobody, so it must not refuse a retirement. The narrowing is here rather than
/// at the caller because "what blocks a retirement" is a question about the
/// store's state and not about the retirement.
///
/// # Errors
/// [`RepoError::Db`] on a storage failure; [`RepoError::CorruptRow`] on an
/// unreadable state token.
pub async fn live_targeting(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    target_plan_id: PlanId,
) -> Result<Vec<MigrationRecord>, RepoError> {
    let rows = migration::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(migration::Column::TenantId.eq(tenant_id))
                .add(migration::Column::TargetPlanId.eq(target_plan_id.get()))
                .add(migration::Column::State.is_in([
                    MigrationState::Scheduled.as_str(),
                    MigrationState::InProgress.as_str(),
                ])),
        )
        .all(runner)
        .await
        .map_err(|e| {
            RepoError::Db(format!(
                "read live migrations targeting plan {target_plan_id}: {e}"
            ))
        })?;

    rows.into_iter().map(into_record).collect()
}

/// What [`start`] answered with (`inst-mst-start`, D-65).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Started {
    /// The schedule, now `in_progress`.
    pub record: MigrationRecord,
    /// `false` when this call replayed an earlier start (D-65's persist-and-replay).
    pub transitioned: bool,
}

/// Declare execution start, or replay the persisted exclusion set
/// (`inst-mst-start`, D-65).
///
/// `exclusion_snapshot` is written **only** on the transitioning call. A repeat
/// call returns the stored one verbatim and re-runs no D-36 re-validation; see
/// the module doc for why that ordering is the rule rather than an optimisation.
///
/// # Errors
/// [`RepoError::Db`] on a storage failure; [`RepoError::ConcurrentMutation`]
/// when another writer moved the row between the read and the write;
/// [`RepoError::CorruptRow`] on an unreadable state token.
pub async fn start(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    migration_id: Uuid,
    exclusion_snapshot: JsonValue,
    now: DateTime<Utc>,
) -> Result<Option<Started>, RepoError> {
    let Some(current) = load(runner, scope, tenant_id, migration_id).await? else {
        return Ok(None);
    };

    // D-65's replay arm, resolved before any write is attempted.
    if current.state == MigrationState::InProgress {
        return Ok(Some(Started {
            record: current,
            transitioned: false,
        }));
    }

    let updated = flip(
        runner,
        scope,
        tenant_id,
        migration_id,
        MigrationState::Scheduled,
        MigrationState::InProgress,
        vec![
            (migration::Column::StartedAt, Expr::value(now)),
            (
                migration::Column::ExclusionSnapshot,
                Expr::value(exclusion_snapshot),
            ),
        ],
    )
    .await?;

    Ok(Some(Started {
        record: updated,
        transitioned: true,
    }))
}

/// Report the scope processed and close the run (`inst-mst-complete`).
///
/// # Errors
/// [`RepoError::Db`], [`RepoError::ConcurrentMutation`] when the row is not
/// `in_progress`, [`RepoError::CorruptRow`] on an unreadable state token.
pub async fn complete(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    migration_id: Uuid,
    completion_record: JsonValue,
    now: DateTime<Utc>,
) -> Result<MigrationRecord, RepoError> {
    flip(
        runner,
        scope,
        tenant_id,
        migration_id,
        MigrationState::InProgress,
        MigrationState::Completed,
        vec![
            (migration::Column::CompletedAt, Expr::value(now)),
            (
                migration::Column::CompletionRecord,
                Expr::value(completion_record),
            ),
        ],
    )
    .await
}

/// Cancel a run from whichever live state it stands in (`inst-mst-cancel`,
/// `inst-mst-cancel-inflight`, D-34, M3).
///
/// The caller has already asked
/// [`MigrationState::ensure_cancellable`](crate::domain::migration::MigrationState::ensure_cancellable)
/// so that a `completed` run is refused in D-34's own words; this function
/// carries the state it expects to leave into the `WHERE` regardless, because
/// the interval between that check and this write is a window a concurrent
/// `complete` can land in.
///
/// `partial_record` carries the migrated / not-attempted sets when the run was
/// `in_progress` (D-34) and is `None` for a cancel of a `scheduled` one, where
/// nothing was attempted at all.
///
/// # Errors
/// [`RepoError::Db`], [`RepoError::ConcurrentMutation`] when the row left `from`
/// first, [`RepoError::CorruptRow`] on an unreadable state token.
pub async fn cancel(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    migration_id: Uuid,
    from: MigrationState,
    partial_record: Option<JsonValue>,
    now: DateTime<Utc>,
) -> Result<MigrationRecord, RepoError> {
    flip(
        runner,
        scope,
        tenant_id,
        migration_id,
        from,
        MigrationState::Cancelled,
        // The partial sets ride the same column the completion record does: they
        // are the same fact asked at a different moment, and a second column
        // would let one run answer "what was processed" two ways.
        match partial_record {
            Some(record) => vec![
                (migration::Column::CancelledAt, Expr::value(now)),
                (migration::Column::CompletionRecord, Expr::value(record)),
            ],
            None => vec![(migration::Column::CancelledAt, Expr::value(now))],
        },
    )
    .await
}

/// One compare-and-swap on the state machine, with the columns that flip
/// alongside it.
///
/// Factored rather than copied three times: the branch that must never drift is
/// "the expected current state is in the `WHERE`", and there is exactly one copy
/// of it.
async fn flip(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    migration_id: Uuid,
    from: MigrationState,
    to: MigrationState,
    extra: Vec<(migration::Column, SimpleExpr)>,
) -> Result<MigrationRecord, RepoError> {
    let mut query = migration::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(migration::Column::State, Expr::value(to.as_str()));
    for (column, value) in extra {
        query = query.col_expr(column, value);
    }
    let result = query
        .filter(
            Condition::all()
                .add(migration::Column::TenantId.eq(tenant_id))
                .add(migration::Column::MigrationId.eq(migration_id))
                // The expected current state. A concurrent writer that moved the
                // row first loses here rather than overwriting.
                .add(migration::Column::State.eq(from.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            contention_or_db(
                &e,
                &format!("migration {migration_id}"),
                &format!("move a migration {from} -> {to}"),
            )
        })?;

    if result.rows_affected == 0 {
        return Err(RepoError::ConcurrentMutation {
            aggregate: format!("migration {migration_id}"),
        });
    }

    load(runner, scope, tenant_id, migration_id)
        .await?
        .ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "migration {migration_id} moved {from} -> {to} and then could not be read back"
            ))
        })
}

/// Read a stored row into the domain's vocabulary.
fn into_record(row: migration::Model) -> Result<MigrationRecord, RepoError> {
    let state = MigrationState::parse(&row.state).ok_or_else(|| {
        RepoError::CorruptRow(format!(
            "pricing_migration.state `{}` on migration {}",
            row.state, row.migration_id
        ))
    })?;
    let Ok(source_revision) = u64::try_from(row.source_revision) else {
        return Err(RepoError::CorruptRow(format!(
            "pricing_migration.source_revision {} on migration {} is negative",
            row.source_revision, row.migration_id
        )));
    };

    Ok(MigrationRecord {
        migration_id: row.migration_id,
        source_plan_id: PlanId::new(row.source_plan_id),
        source_revision,
        target_plan_id: PlanId::new(row.target_plan_id),
        effective_at: row.effective_at,
        announced_at: row.announced_at,
        scope: row.scope,
        state,
        delta_report: row.delta_report,
        exclusion_snapshot: row.exclusion_snapshot,
        completion_record: row.completion_record,
    })
}
