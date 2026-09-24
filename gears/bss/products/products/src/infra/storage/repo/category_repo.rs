//! Flat category persistence; retirement and its reference check are one statement.
use super::{HeadWrite, driver_failure, map_unique};
use crate::domain::category::{CategoryPatch, NewCategory};
use crate::infra::storage::{
    RepoError,
    entity::{category, sku},
};
use bss_products_sdk::models::Category;
use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Set};
use time::OffsetDateTime;
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

fn category_of(m: category::Model) -> Category {
    Category {
        id: m.id,
        tenant_id: m.tenant_id,
        code: m.code,
        name: m.name,
        is_default: m.is_default,
        sort_order: m.sort_order,
        status: m.status,
        version: m.version,
    }
}
fn key(tenant: Uuid, id: Uuid) -> Condition {
    Condition::all()
        .add(category::Column::TenantId.eq(tenant))
        .add(category::Column::Id.eq(id))
}
/// Insert an active category.
/// # Errors
/// Returns category-code conflicts or scoped storage failures.
pub async fn insert_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    new: NewCategory,
    now: OffsetDateTime,
) -> Result<Category, RepoError> {
    let model = category::ActiveModel {
        id: Set(Uuid::new_v4()),
        tenant_id: Set(tenant_id),
        code: Set(new.code),
        name: Set(new.name),
        is_default: Set(new.is_default),
        sort_order: Set(new.sort_order),
        status: Set("active".into()),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    };
    category::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure("category scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map(category_of)
        .map_err(|e| map_unique("insert category".into(), e))
}
/// Read a category within the tenant and access scope.
/// # Errors
/// Returns scoped storage failures.
pub async fn find_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<Option<Category>, RepoError> {
    category::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(key(tenant_id, id))
        .one(runner)
        .await
        .map(|m| m.map(category_of))
        .map_err(|e| driver_failure("find category".into(), e))
}
/// List categories by display order then stable code.
/// # Errors
/// Returns scoped storage failures.
pub async fn list_categories(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<Category>, RepoError> {
    category::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(category::Column::TenantId.eq(tenant_id)))
        .order_by(category::Column::SortOrder, Order::Asc)
        .order_by(category::Column::Code, Order::Asc)
        .all(runner)
        .await
        .map(|m| m.into_iter().map(category_of).collect())
        .map_err(|e| driver_failure("list categories".into(), e))
}
/// Patch a category only at the revision the caller saw.
/// # Errors
/// Returns scoped storage failures.
pub async fn update_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    expected_version: i64,
    patch: CategoryPatch,
    now: OffsetDateTime,
) -> Result<HeadWrite<Category>, RepoError> {
    let mut q = category::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            category::Column::Version,
            Expr::col(category::Column::Version).add(1_i64),
        )
        .col_expr(category::Column::UpdatedAt, Expr::value(now));
    if let Some(v) = patch.name {
        q = q.col_expr(category::Column::Name, Expr::value(v));
    }
    if let Some(v) = patch.is_default {
        q = q.col_expr(category::Column::IsDefault, Expr::value(v));
    }
    if let Some(v) = patch.sort_order {
        q = q.col_expr(category::Column::SortOrder, Expr::value(v));
    }
    let r = q
        .filter(key(tenant_id, id).add(category::Column::Version.eq(expected_version)))
        .exec(runner)
        .await
        .map_err(|e| driver_failure("update category".into(), e))?;
    category_written(runner, scope, tenant_id, id, r.rows_affected).await
}
async fn category_written(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    affected: u64,
) -> Result<HeadWrite<Category>, RepoError> {
    if affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    find_category(runner, scope, tenant, id)
        .await?
        .map(HeadWrite::Written)
        .ok_or_else(|| RepoError::CorruptRow("written category disappeared".into()))
}
/// Retire only an active, unused category; callers use a serializable transaction.
/// # Errors
/// Returns scoped storage failures. Missing categories return `None`.
pub async fn retire_category_if_unused(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    now: OffsetDateTime,
) -> Result<Option<HeadWrite<Category>>, RepoError> {
    let used = Query::select()
        .expr(Expr::val(1))
        .from(sku::Entity)
        .and_where(sku::Column::TenantId.eq(tenant_id))
        .and_where(sku::Column::CategoryId.eq(id))
        .to_owned();
    let r = category::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(category::Column::Status, Expr::value("retired"))
        .col_expr(
            category::Column::Version,
            Expr::col(category::Column::Version).add(1_i64),
        )
        .col_expr(category::Column::UpdatedAt, Expr::value(now))
        .filter(
            key(tenant_id, id)
                .add(category::Column::Status.eq("active"))
                .add(Expr::exists(used).not()),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("retire category".into(), e))?;
    if r.rows_affected == 0 {
        return Ok(find_category(runner, scope, tenant_id, id)
            .await?
            .map(|_| HeadWrite::Unmatched));
    }
    category_written(runner, scope, tenant_id, id, r.rows_affected)
        .await
        .map(Some)
}
/// Validate the category in the caller's serializable authoring transaction.
/// # Errors
/// Returns `CATEGORY_RETIRED` for an inactive category, or a missing-category refusal.
pub(super) async fn require_active_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<(), RepoError> {
    match find_category(runner, scope, tenant, id).await? {
        Some(c) if c.status == "active" => Ok(()),
        Some(_) => Err(RepoError::Db("CATEGORY_RETIRED".into())),
        None => Err(RepoError::Db("CATEGORY_NOT_FOUND".into())),
    }
}
