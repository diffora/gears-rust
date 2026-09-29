//! Scoped plan persistence with conditional versions (D-394).
use super::{driver_failure, map_unique, matched};
use crate::infra::storage::{RepoError, entity::plan as e};
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
/// Insert a tenant-scoped plan in the caller's transaction.
/// # Errors
/// `PLAN_CODE_TAKEN`; database failures keep their type.
pub async fn insert(
    runner: &impl DBRunner,
    scope: &AccessScope,
    m: e::Model,
) -> Result<e::Model, RepoError> {
    let active = e::ActiveModel {
        id: Set(m.id),
        tenant_id: Set(m.tenant_id),
        code: Set(m.code),
        name: Set(m.name),
        published_rev: Set(m.published_rev),
        version: Set(m.version),
        created_by: Set(m.created_by),
        created_at: Set(m.created_at),
        updated_at: Set(m.updated_at),
    };
    e::Entity::insert(active.clone())
        .secure()
        .scope_with_model(scope, &active)
        .map_err(|e| driver_failure("insert plan scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| map_unique("insert plan".into(), e))
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
        .map_err(|e| driver_failure("find plan".into(), e))
}
/// List the tenant's plans by code.
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
        .order_by(e::Column::Code, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list plans".into(), e))
}
/// The tenant's plans, by code, that have a draft, pending or published revision whose items
/// name an entry of `sku` — the plans the SKU's usage counts (D-428, D-434) — in ONE statement
/// whatever their number. An included item without an entry names no entry and does not count.
/// The revisions, items and entries are read tenant-scoped.
/// # Errors
/// Returns typed database failures.
pub async fn naming_sku(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    sku: Uuid,
) -> Result<Vec<e::Model>, RepoError> {
    use crate::domain::plan::RevisionState;
    use crate::infra::storage::entity::{
        plan_item as item, plan_revision as revision, price_book_entry as entry,
    };
    let naming = sea_orm::sea_query::Query::select()
        .expr(Expr::val(1))
        .from(revision::Entity)
        .inner_join(
            item::Entity,
            Expr::col((item::Entity, item::Column::RevisionId))
                .equals((revision::Entity, revision::Column::Id)),
        )
        .inner_join(
            entry::Entity,
            Expr::col((entry::Entity, entry::Column::Id))
                .equals((item::Entity, item::Column::PriceBookEntryId)),
        )
        .and_where(
            Expr::col((revision::Entity, revision::Column::PlanId))
                .equals((e::Entity, e::Column::Id)),
        )
        .and_where(Expr::col((revision::Entity, revision::Column::TenantId)).eq(tenant))
        .and_where(
            Expr::col((revision::Entity, revision::Column::State))
                .ne(RevisionState::Superseded.as_str()),
        )
        .and_where(Expr::col((item::Entity, item::Column::TenantId)).eq(tenant))
        .and_where(Expr::col((entry::Entity, entry::Column::TenantId)).eq(tenant))
        .and_where(Expr::col((entry::Entity, entry::Column::SkuId)).eq(sku))
        .to_owned();
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::TenantId.eq(tenant))
                .add(Expr::exists(naming)),
        )
        .order_by(e::Column::Code, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list the plans naming a SKU".into(), e))
}
/// Rename at the version the caller read.
/// # Errors
/// A concurrent change is `STALE_REVISION`; database failures keep their type.
pub async fn rename(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
    name: String,
    now: time::OffsetDateTime,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::Name, Expr::value(name))
        .col_expr(e::Column::UpdatedAt, Expr::value(now))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(key(tenant, id).add(e::Column::Version.eq(version)))
        .exec(runner)
        .await
        .map_err(|e| driver_failure("rename plan".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
}
/// Write the revision number a revision's apply publishes, at the version the caller read.
/// # Errors
/// A concurrent change is `STALE_REVISION`; database failures keep their type.
pub async fn set_published(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
    published_rev: i32,
    now: time::OffsetDateTime,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::PublishedRev, Expr::value(Some(published_rev)))
        .col_expr(e::Column::UpdatedAt, Expr::value(now))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(key(tenant, id).add(e::Column::Version.eq(version)))
        .exec(runner)
        .await
        .map_err(|e| driver_failure("publish plan projection".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
}
/// Advance the revision number a due switch publishes (D-448), in the caller's transaction.
/// `published_rev` is a projection of the revisions, not an edit of the plan: neither the plan's
/// `version` nor its `updated_at` moves, so an If-Match read before the switch stays good and a
/// read that derives the switch (D-447) shows the same plan row as one after it (plan rev 2 L6).
/// # Errors
/// `PLAN_NOT_FOUND` for a plan the tenant does not hold; database failures keep their type.
pub async fn advance_published(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    published_rev: i32,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::PublishedRev, Expr::value(Some(published_rev)))
        .filter(key(tenant, id))
        .exec(runner)
        .await
        .map_err(|e| driver_failure("advance the plan's published projection".into(), e))?;
    matched(result.rows_affected, "PLAN_NOT_FOUND")
}
/// Delete a plan that was never published and has no revision left, at the version the caller
/// read, in the caller's transaction (D-417): its code is free again.
/// # Errors
/// A published plan, a remaining revision or a lost version is `STALE_REVISION`; database
/// failures keep their type.
pub async fn delete_unpublished(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
) -> Result<(), RepoError> {
    use crate::infra::storage::entity::plan_revision;
    use toolkit_db::secure::SecureDeleteExt;
    let revisions = sea_orm::sea_query::Query::select()
        .expr(Expr::val(1))
        .from(plan_revision::Entity)
        .and_where(plan_revision::Column::TenantId.eq(tenant))
        .and_where(plan_revision::Column::PlanId.eq(id))
        .to_owned();
    let result = e::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            key(tenant, id)
                .add(e::Column::Version.eq(version))
                .add(e::Column::PublishedRev.is_null())
                .add(Expr::exists(revisions).not()),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("delete unpublished plan".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
}
