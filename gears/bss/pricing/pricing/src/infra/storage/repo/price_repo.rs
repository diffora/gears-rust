//! Scoped price persistence with conditional versions.
use super::{driver_failure, map_unique, matched};
use crate::infra::storage::{RepoError, entity::price as e};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Set};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;
fn key(tenant: Uuid, id: Uuid) -> Condition {
    Condition::all()
        .add(e::Column::TenantId.eq(tenant))
        .add(e::Column::Id.eq(id))
}
/// Insert a tenant-scoped row in the caller's transaction.
/// # Errors
/// Returns unique conflicts, parent ownership refusals or typed database failures.
pub async fn insert(
    runner: &impl DBRunner,
    scope: &AccessScope,
    m: e::Model,
) -> Result<e::Model, RepoError> {
    if super::book_repo::find(runner, scope, m.tenant_id, m.book_id)
        .await?
        .is_none()
    {
        return Err(RepoError::Conflict {
            code: "BOOK_NOT_FOUND",
        });
    }
    if let Some(dimension) = &m.dimension_key
        && super::dimension_repo::find(runner, scope, m.tenant_id, dimension)
            .await?
            .is_none()
    {
        return Err(RepoError::Conflict {
            code: "DIM_NOT_DECLARED",
        });
    }
    let active = e::ActiveModel {
        id: Set(m.id),
        tenant_id: Set(m.tenant_id),
        book_id: Set(m.book_id),
        sku_id: Set(m.sku_id),
        charge_kind: Set(m.charge_kind),
        period: Set(m.period),
        dimension_key: Set(m.dimension_key),
        invoice_line_override: Set(m.invoice_line_override),
        reservation_id: Set(m.reservation_id),
        reference_state: Set(m.reference_state),
        version: Set(m.version),
        created_at: Set(m.created_at),
        updated_at: Set(m.updated_at),
    };
    e::Entity::insert(active.clone())
        .secure()
        .scope_with_model(scope, &active)
        .map_err(|e| driver_failure("insert scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| map_unique("insert price".into(), e))
}
/// Read by tenant and identity within the authorized scope.
/// # Errors
/// Returns typed database failures.
pub async fn find(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Option<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(key(tenant, id))
        .one(runner)
        .await
        .map_err(|e| driver_failure("find price".into(), e))
}
/// List tenant rows in stable identity order.
/// # Errors
/// Returns typed database failures.
pub async fn list(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
) -> Result<Vec<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(e::Column::TenantId.eq(tenant)))
        .order_by(e::Column::Id, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list price".into(), e))
}
/// Change business columns only if the caller's version still owns the row.
/// # Errors
/// Zero matches is a typed version conflict; database failures preserve their type.
pub async fn update(
    runner: &impl DBRunner,
    scope: &AccessScope,
    m: e::Model,
) -> Result<(), RepoError> {
    let predicate = key(m.tenant_id, m.id).add(e::Column::Version.eq(m.version));
    if let Some(dimension) = &m.dimension_key
        && super::dimension_repo::find(runner, scope, m.tenant_id, dimension)
            .await?
            .is_none()
    {
        return Err(RepoError::Conflict {
            code: "DIM_NOT_DECLARED",
        });
    }
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::DimensionKey, Expr::value(m.dimension_key))
        .col_expr(
            e::Column::InvoiceLineOverride,
            Expr::value(m.invoice_line_override),
        )
        .col_expr(e::Column::UpdatedAt, Expr::value(m.updated_at))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(predicate)
        .exec(runner)
        .await
        .map_err(|e| map_unique("update price".into(), e))?;
    matched(result.rows_affected, "VERSION_CONFLICT")
}
/// List rows of one scoped parent in stable order.
/// # Errors
/// Returns typed database failures.
pub async fn for_book(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    parent: Uuid,
) -> Result<Vec<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::TenantId.eq(tenant))
                .add(e::Column::BookId.eq(parent)),
        )
        .order_by(e::Column::Id, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list parent rows".into(), e))
}
/// Change the reference receipt/state at the observed version.
/// # Errors
/// Returns a version conflict or a typed database failure.
#[allow(
    clippy::too_many_arguments,
    reason = "tenant identity, version and receipt are the conditional write operands"
)]
pub async fn set_reference(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
    state: crate::domain::price::ReferenceState,
    reservation_id: Uuid,
    now: time::OffsetDateTime,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::ReferenceState, Expr::value(state.as_str()))
        .col_expr(e::Column::ReservationId, Expr::value(reservation_id))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .col_expr(e::Column::UpdatedAt, Expr::value(now))
        .filter(key(tenant, id).add(e::Column::Version.eq(version)))
        .exec(runner)
        .await
        .map_err(|e| driver_failure("update price reference".into(), e))?;
    matched(result.rows_affected, "VERSION_CONFLICT")
}
/// Delete a price after the caller has removed its drafts, with the release op in the same transaction.
/// # Errors
/// Refuses a stale version or any remaining row; preserves database failures.
pub async fn delete_empty(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
) -> Result<(), RepoError> {
    use crate::infra::storage::entity::price_row;
    use toolkit_db::secure::SecureDeleteExt;
    let children = sea_orm::sea_query::Query::select()
        .expr(Expr::val(1))
        .from(price_row::Entity)
        .and_where(price_row::Column::TenantId.eq(tenant))
        .and_where(price_row::Column::PriceId.eq(id))
        .to_owned();
    let result = e::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            key(tenant, id)
                .add(e::Column::Version.eq(version))
                .add(Expr::exists(children).not()),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("delete empty price".into(), e))?;
    matched(result.rows_affected, "VERSION_CONFLICT")
}

/// Bounded identity-ordered scan for the trusted reconciliation worker: confirmed prices,
/// whose receipts it checks, and lost prices, which it re-reserves once their SKU admits a
/// reservation again.
/// # Errors
/// Returns typed scoped storage failures.
pub async fn reconcile_batch(
    runner: &impl DBRunner,
    scope: &AccessScope,
    cursor: Option<Uuid>,
    limit: u64,
) -> Result<Vec<e::Model>, RepoError> {
    let mut filter = Condition::all().add(e::Column::ReferenceState.is_in(["confirmed", "lost"]));
    if let Some(cursor) = cursor {
        filter = filter.add(e::Column::Id.gt(cursor));
    }
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(filter)
        .order_by(e::Column::Id, Order::Asc)
        .limit(limit)
        .all(runner)
        .await
        .map_err(|e| driver_failure("confirmed price batch".into(), e))
}
