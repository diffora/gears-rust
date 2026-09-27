//! Scoped price book entry persistence with conditional versions.
use super::{driver_failure, map_unique, matched};
use crate::infra::storage::{RepoError, entity::price_book_entry as e};
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
        && !super::dimension_repo::declare_for_entry(runner, scope, m.tenant_id, dimension).await?
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
        model: Set(m.model),
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
        .map_err(|e| driver_failure("insert price book entry scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| map_unique("insert price book entry".into(), e))
}
/// The entry's model in the pure model (D-427): every price of the entry is decoded and judged
/// with it. Unknown vocabulary is a corrupt row.
/// # Errors
/// `CorruptRow` for a stored model the domain does not know.
pub fn model_of(m: &e::Model) -> Result<crate::domain::price_book_entry::Model, RepoError> {
    m.model
        .parse()
        .map_err(|_| RepoError::CorruptRow(format!("entry {} model", m.id)))
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
        .map_err(|e| driver_failure("find price book entry".into(), e))
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
        .map_err(|e| driver_failure("list price book entries".into(), e))
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
        && !super::dimension_repo::declare_for_entry(runner, scope, m.tenant_id, dimension).await?
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
        .map_err(|e| map_unique("update price book entry".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
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
        .map_err(|e| driver_failure("list price book entries of a book".into(), e))
}
/// The tenant's entries of the SKUs, in every book and every reference state, in ONE statement
/// (D-428).
/// # Errors
/// Returns typed database failures.
pub async fn for_skus(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    skus: &[Uuid],
) -> Result<Vec<e::Model>, RepoError> {
    if skus.is_empty() {
        return Ok(Vec::new());
    }
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::TenantId.eq(tenant))
                .add(e::Column::SkuId.is_in(skus.iter().copied())),
        )
        .order_by(e::Column::Id, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list price book entries of SKUs".into(), e))
}
/// One SKU id of a set read.
#[derive(Debug, sea_orm::FromQueryResult)]
struct SkuIdRow {
    sku_id: Uuid,
}
/// The distinct SKU ids of the tenant's entries under `scope`, narrowed by `condition`, sorted,
/// in ONE statement.
async fn distinct_skus(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    condition: Condition,
    context: &str,
) -> Result<Vec<Uuid>, RepoError> {
    use sea_orm::{QueryOrder, QuerySelect};
    Ok(e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::TenantId.eq(tenant))
                .add(condition),
        )
        .project_all(runner, |q| {
            q.select_only()
                .column(e::Column::SkuId)
                .distinct()
                .order_by(e::Column::SkuId, Order::Asc)
                .into_model::<SkuIdRow>()
        })
        .await
        .map_err(|e| driver_failure(context.into(), e))?
        .into_iter()
        .map(|r| r.sku_id)
        .collect())
}
/// The SKUs with an entry in any book of the tenant, in any reference state, under `scope`: the
/// SKUs whose usage counts an entry (D-428), in ONE statement (P-D-212).
/// # Errors
/// Returns typed database failures.
pub async fn priced_skus(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
) -> Result<Vec<Uuid>, RepoError> {
    distinct_skus(runner, scope, tenant, Condition::all(), "list priced SKUs").await
}
/// The SKUs whose entries (under `scope`) a plan item of a draft, pending or published revision
/// names: the SKUs whose usage counts a plan (D-428), in ONE statement (P-D-212). The items and
/// revisions are read tenant-scoped, as the usage count reads them.
/// # Errors
/// Returns typed database failures.
pub async fn in_plan_skus(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
) -> Result<Vec<Uuid>, RepoError> {
    use crate::domain::plan::RevisionState;
    use crate::infra::storage::entity::{plan_item as item, plan_revision as revision};
    let live_item = sea_orm::sea_query::Query::select()
        .expr(Expr::val(1))
        .from(item::Entity)
        .inner_join(
            revision::Entity,
            Expr::col((revision::Entity, revision::Column::Id))
                .equals((item::Entity, item::Column::RevisionId)),
        )
        .and_where(Expr::col((item::Entity, item::Column::TenantId)).eq(tenant))
        .and_where(
            Expr::col((item::Entity, item::Column::PriceBookEntryId))
                .equals((e::Entity, e::Column::Id)),
        )
        .and_where(Expr::col((revision::Entity, revision::Column::TenantId)).eq(tenant))
        .and_where(
            Expr::col((revision::Entity, revision::Column::State))
                .ne(RevisionState::Superseded.as_str()),
        )
        .to_owned();
    distinct_skus(
        runner,
        scope,
        tenant,
        Condition::all().add(Expr::exists(live_item)),
        "list in-plan SKUs",
    )
    .await
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
    state: crate::domain::price_book_entry::ReferenceState,
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
        .map_err(|e| driver_failure("update price book entry reference".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
}
/// Delete an entry after the caller has removed its drafts, with the release op in the same transaction.
/// # Errors
/// Refuses a stale version or any remaining price; preserves database failures.
pub async fn delete_empty(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
) -> Result<(), RepoError> {
    use crate::infra::storage::entity::price;
    use toolkit_db::secure::SecureDeleteExt;
    let children = sea_orm::sea_query::Query::select()
        .expr(Expr::val(1))
        .from(price::Entity)
        .and_where(price::Column::TenantId.eq(tenant))
        .and_where(price::Column::PriceBookEntryId.eq(id))
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
        .map_err(|e| driver_failure("delete empty price book entry".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
}

/// Bounded identity-ordered scan for the trusted reconciliation worker: confirmed entries,
/// whose receipts it checks, and lost entries, which it re-reserves once their SKU admits a
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
        .map_err(|e| driver_failure("confirmed price book entry batch".into(), e))
}
#[cfg(test)]
#[path = "price_book_entry_repo_tests.rs"]
mod tests;
