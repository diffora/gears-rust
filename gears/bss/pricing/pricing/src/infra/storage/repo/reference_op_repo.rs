//! Durable reference work; compare-and-swap never locks a row or drops failed work.
use super::{driver_failure, matched};
use crate::{
    domain::{
        price_book_entry::OpState,
        reference_op::{OpKind, RefKind},
    },
    infra::storage::{RepoError, entity::reference_op as e},
};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Set};
use time::OffsetDateTime;
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;
/// Insert before reserve, or with deletion, in the caller's transaction.
/// # Errors
/// Returns scoped storage failures with their original database type.
pub async fn insert(
    runner: &impl DBRunner,
    scope: &AccessScope,
    m: e::Model,
) -> Result<e::Model, RepoError> {
    let active = e::ActiveModel {
        op_id: Set(m.op_id),
        tenant_id: Set(m.tenant_id),
        kind: Set(m.kind),
        ref_kind: Set(m.ref_kind),
        ref_id: Set(m.ref_id),
        sku_id: Set(m.sku_id),
        reservation_id: Set(m.reservation_id),
        idempotency_key: Set(m.idempotency_key),
        state: Set(m.state),
        outcome: Set(m.outcome),
        attempts: Set(m.attempts),
        next_attempt_at: Set(m.next_attempt_at),
        last_error: Set(m.last_error),
        created_by: Set(m.created_by),
        created_at: Set(m.created_at),
        updated_at: Set(m.updated_at),
    };
    e::Entity::insert(active.clone())
        .secure()
        .scope_with_model(scope, &active)
        .map_err(|e| driver_failure("op scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| driver_failure("insert op".into(), e))
}
/// Read one tenant's operation.
/// # Errors
/// Returns scoped database failures.
pub async fn find(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Option<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::TenantId.eq(tenant))
                .add(e::Column::OpId.eq(id)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure("find op".into(), e))
}
/// Entire mutable work receipt written with each conditional state transition.
#[derive(Debug, Clone)]
pub struct TransitionFields {
    pub reservation_id: Option<Uuid>,
    pub outcome: Option<String>,
    pub attempts: i32,
    pub next_attempt_at: OffsetDateTime,
    pub last_error: Option<String>,
    pub updated_at: OffsetDateTime,
}
/// Atomically transition only the observed state. Zero matches is a typed conflict.
/// # Errors
/// Returns `REFERENCE_OP_CONTENDED` or typed scoped database failures.
pub async fn transition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    op_id: Uuid,
    from_state: OpState,
    to_state: OpState,
    fields: &TransitionFields,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::State, Expr::value(to_state.as_str()))
        .col_expr(e::Column::ReservationId, Expr::value(fields.reservation_id))
        .col_expr(e::Column::Outcome, Expr::value(fields.outcome.clone()))
        .col_expr(e::Column::Attempts, Expr::value(fields.attempts))
        .col_expr(
            e::Column::NextAttemptAt,
            Expr::value(fields.next_attempt_at),
        )
        .col_expr(e::Column::LastError, Expr::value(fields.last_error.clone()))
        .col_expr(e::Column::UpdatedAt, Expr::value(fields.updated_at))
        .filter(
            Condition::all()
                .add(e::Column::OpId.eq(op_id))
                .add(e::Column::State.eq(from_state.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("transition reference op".into(), e))?;
    matched(result.rows_affected, "REFERENCE_OP_CONTENDED")
}
/// Find bounded, due, unfinished work; failed work remains eligible indefinitely.
/// # Errors
/// Returns typed scoped database failures.
pub async fn due(
    runner: &impl DBRunner,
    scope: &AccessScope,
    now: OffsetDateTime,
    limit: u64,
) -> Result<Vec<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::State.ne(OpState::Done.as_str()))
                .add(e::Column::NextAttemptAt.lte(now)),
        )
        .order_by(e::Column::NextAttemptAt, Order::Asc)
        .order_by(e::Column::OpId, Order::Asc)
        .limit(limit)
        .all(runner)
        .await
        .map_err(|e| driver_failure("due reference ops".into(), e))
}

/// Operator pagination uses a stable exclusive op-id cursor and an optional state.
/// # Errors
/// Returns typed scoped storage failures.
pub async fn page(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    state: Option<OpState>,
    cursor: Option<Uuid>,
    limit: u64,
) -> Result<Vec<e::Model>, RepoError> {
    let mut filter = Condition::all().add(e::Column::TenantId.eq(tenant));
    if let Some(state) = state {
        filter = filter.add(e::Column::State.eq(state.as_str()));
    }
    if let Some(cursor) = cursor {
        filter = filter.add(e::Column::OpId.gt(cursor));
    }
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(filter)
        .order_by(e::Column::OpId, Order::Asc)
        .limit(limit)
        .all(runner)
        .await
        .map_err(|e| driver_failure("reference op page".into(), e))
}

/// Whether a reference already has unfinished work of `kind`: one re-reservation per
/// reference. The guard is keyed by the reference's kind as well as its id.
/// # Errors
/// Returns typed scoped storage failures.
pub async fn open_for_ref(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    ref_kind: RefKind,
    ref_id: Uuid,
    kind: OpKind,
) -> Result<bool, RepoError> {
    Ok(e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::TenantId.eq(tenant))
                .add(e::Column::RefKind.eq(ref_kind.as_str()))
                .add(e::Column::RefId.eq(ref_id))
                .add(e::Column::Kind.eq(kind.as_str()))
                .add(e::Column::State.ne(OpState::Done.as_str())),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure("open reference op".into(), e))?
        .is_some())
}
