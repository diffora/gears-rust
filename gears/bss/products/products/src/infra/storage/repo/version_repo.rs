//! Append-only published content and deterministic date resolution.
//! @cpt-dod:cpt-cf-bss-products-dod-versions-as-of:p1
use super::{driver_failure, map_unique};
use crate::infra::storage::{RepoError, entity::sku_version};
use bss_products_sdk::models::{SkuContent, SkuVersion};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Set};
use time::{Date, OffsetDateTime};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;
fn key(tenant: Uuid, sku: Uuid) -> Condition {
    Condition::all()
        .add(sku_version::Column::TenantId.eq(tenant))
        .add(sku_version::Column::SkuId.eq(sku))
}
fn version_of(m: sku_version::Model) -> Result<SkuVersion, RepoError> {
    Ok(SkuVersion {
        sku_id: m.sku_id,
        published_version: m.published_version,
        effective_from: m.effective_from,
        content: serde_json::from_value(m.content)
            .map_err(|e| RepoError::CorruptRow(format!("version content: {e}")))?,
        created_at: m.created_at,
    })
}
/// Append a version at or after the latest date in the caller's transaction.
/// # Errors
/// Returns `VERSION_ORDER`, serialization errors, or scoped storage failures.
#[allow(
    clippy::too_many_arguments,
    reason = "The version key and its content/date are explicit repository operands"
)]
pub async fn append_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    published_version: i64,
    effective_from: Date,
    content: &SkuContent,
    now: OffsetDateTime,
) -> Result<SkuVersion, RepoError> {
    let latest = sku_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(key(tenant_id, sku_id))
        .order_by(sku_version::Column::EffectiveFrom, Order::Desc)
        .limit(1)
        .one(runner)
        .await
        .map_err(|e| driver_failure("latest version".into(), e))?;
    if latest.is_some_and(|v| effective_from < v.effective_from) {
        return Err(RepoError::Db("VERSION_ORDER".into()));
    }
    let model = sku_version::ActiveModel {
        sku_id: Set(sku_id),
        tenant_id: Set(tenant_id),
        published_version: Set(published_version),
        effective_from: Set(effective_from),
        content: Set(serde_json::to_value(content)
            .map_err(|e| RepoError::Db(format!("serialize version: {e}")))?),
        created_at: Set(now),
    };
    let row = sku_version::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure("version scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| map_unique("append version".into(), e))?;
    version_of(row)
}
/// All versions in effective-date and version order.
/// # Errors
/// Returns scoped storage or corrupt-content errors.
pub async fn versions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
) -> Result<Vec<SkuVersion>, RepoError> {
    sku_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(key(tenant_id, sku_id))
        .order_by(sku_version::Column::EffectiveFrom, Order::Asc)
        .order_by(sku_version::Column::PublishedVersion, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("versions".into(), e))?
        .into_iter()
        .map(version_of)
        .collect()
}
/// Resolve the highest version on the latest date at or before `as_of`.
/// # Errors
/// Returns scoped storage or corrupt-content errors.
pub async fn version_as_of(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    as_of: Date,
) -> Result<Option<SkuVersion>, RepoError> {
    sku_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(key(tenant_id, sku_id).add(sku_version::Column::EffectiveFrom.lte(as_of)))
        .order_by(sku_version::Column::EffectiveFrom, Order::Desc)
        .order_by(sku_version::Column::PublishedVersion, Order::Desc)
        .limit(1)
        .one(runner)
        .await
        .map_err(|e| driver_failure("version as of".into(), e))?
        .map(version_of)
        .transpose()
}
