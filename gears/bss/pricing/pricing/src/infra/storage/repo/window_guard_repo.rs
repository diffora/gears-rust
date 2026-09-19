//! The per-plan window-guard lock row (D-374).
//!
//! One row per `(tenant_id, plan_id)`, not per revision. [`acquire`] is a scoped
//! `UPDATE serial = serial + 1` that holds the write lock until the caller's
//! transaction ends. Callers own that transaction; acquiring and then dropping
//! the handle releases the lock.

use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, Condition, DbErr, EntityTrait, ExprTrait};
use toolkit_db::secure::{AccessScope, DBRunner, ScopeError, SecureInsertExt, SecureUpdateExt};
use uuid::Uuid;

use crate::infra::storage::RepoError;
use crate::infra::storage::entity::window_guard;

/// Advance `serial` by one, holding the row lock until `runner`'s transaction
/// ends.
///
/// # Errors
/// [`RepoError::NotFound`] when no guard row exists for this tenant and plan
/// (including a foreign tenant's plan, which the scope cannot see);
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn acquire(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: Uuid,
) -> Result<(), RepoError> {
    let result = window_guard::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            window_guard::Column::Serial,
            Expr::col(window_guard::Column::Serial).add(1_i64),
        )
        .filter(
            Condition::all()
                .add(window_guard::Column::TenantId.eq(tenant_id))
                .add(window_guard::Column::PlanId.eq(plan_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("acquire pricing_window_guard: {e}")))?;
    if result.rows_affected == 0 {
        return Err(RepoError::NotFound {
            subject: "window guard".to_owned(),
            id: plan_id.to_string(),
        });
    }
    Ok(())
}

/// Acquire every named plan guard in sorted `(tenant_id, plan_id)` order.
///
/// Callers that lock more than one plan must go through this so two transactions
/// cannot deadlock by taking the same pair in opposite orders.
///
/// # Errors
/// [`acquire`]'s, for the first plan that has no guard row or fails to lock.
pub async fn acquire_sorted(
    runner: &impl DBRunner,
    scope: &AccessScope,
    plans: impl IntoIterator<Item = (Uuid, Uuid)>,
) -> Result<(), RepoError> {
    let mut plans: Vec<(Uuid, Uuid)> = plans.into_iter().collect();
    plans.sort_unstable();
    plans.dedup();
    for (tenant_id, plan_id) in plans {
        acquire(runner, scope, tenant_id, plan_id).await?;
    }
    Ok(())
}

/// Insert the lock row for a newly created plan. A successor revision of the
/// same `plan_id` is a no-op on the primary key.
pub(crate) async fn ensure(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: Uuid,
) -> Result<(), RepoError> {
    let row = window_guard::ActiveModel {
        tenant_id: Set(tenant_id),
        plan_id: Set(plan_id),
        serial: Set(0),
    };
    let on_conflict =
        OnConflict::columns([window_guard::Column::TenantId, window_guard::Column::PlanId])
            .do_nothing()
            .to_owned();
    match window_guard::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| RepoError::Db(format!("pricing_window_guard scope: {e}")))?
        .on_conflict_raw(on_conflict)
        .exec(runner)
        .await
    {
        Ok(_) | Err(ScopeError::Db(DbErr::RecordNotInserted)) => Ok(()),
        Err(e) => Err(RepoError::Db(format!("insert pricing_window_guard: {e}"))),
    }
}
