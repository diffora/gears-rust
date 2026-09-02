//! The browse projection's stamp and serving-row persistence
//! (`design/08-read-models.md` `inst-rp-stamp`, P-D-07, P-D-70).
//!
//! The projector (`dod-projector`) is **not** this module: that consumer
//! lives in `infra/events` / `infra/broker` and is a different `DoD`. What
//! ships here is the host those apply steps call — load the per-tenant
//! stamp, run [`advance_stamp`](crate::domain::read_model::advance_stamp),
//! persist the result — so a projector can drive the floor without this
//! module inventing one.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-staleness-stamp:p1

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use crate::domain::read_model::{StalenessStamp, StampAdvanceRefusal, StampApply, advance_stamp};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{read_entity, read_stamp};

use super::driver_failure;

/// Load the per-tenant stamp row, or `None` before the first apply.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn load_read_stamp(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Option<StalenessStamp>, RepoError> {
    let row = read_stamp::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_stamp::Column::TenantId.eq(tenant_id)))
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("load read stamp of {tenant_id}"), e))?;
    Ok(row.map(|row| StalenessStamp {
        as_of_catalog_version: row.catalog_version_id,
        projected_at: row.projected_at,
    }))
}

/// Persist one stamp row, inserting on the first apply and overwriting after.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn write_read_stamp(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    stamp: StalenessStamp,
) -> Result<(), RepoError> {
    let updated = read_stamp::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            read_stamp::Column::CatalogVersionId,
            Expr::value(stamp.as_of_catalog_version),
        )
        .col_expr(
            read_stamp::Column::ProjectedAt,
            Expr::value(stamp.projected_at),
        )
        .filter(Condition::all().add(read_stamp::Column::TenantId.eq(tenant_id)))
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("update read stamp of {tenant_id}"), e))?;
    if updated.rows_affected > 0 {
        return Ok(());
    }

    let model = read_stamp::ActiveModel {
        tenant_id: Set(tenant_id),
        catalog_version_id: Set(stamp.as_of_catalog_version),
        projected_at: Set(stamp.projected_at),
    };
    read_stamp::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("insert read stamp scope of {tenant_id}"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("insert read stamp of {tenant_id}"), e))?;
    Ok(())
}

/// Advance and persist the stamp in one step — the projector's stamp host.
///
/// Loads the current row, runs [`advance_stamp`], writes the result. The
/// domain refusal surfaces as [`RepoError::Db`] with the refusal's name, so
/// a caller that stamped before projecting entities fails loudly rather than
/// silently claiming a version whose content is missing.
///
/// # Errors
///
/// [`RepoError`] on a domain refusal or a storage / scope failure.
pub async fn apply_read_stamp(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    apply: StampApply,
) -> Result<StalenessStamp, RepoError> {
    let current = load_read_stamp(runner, scope, tenant_id).await?;
    let next = advance_stamp(current, apply).map_err(|refusal| {
        let detail = match refusal {
            StampAdvanceRefusal::EntitiesNotYetProjected => {
                "entities not yet projected in this step"
            }
            StampAdvanceRefusal::ProjectedAtDidNotAdvance => "projected_at did not advance",
        };
        RepoError::Db(format!("read stamp of {tenant_id}: {detail}"))
    })?;
    write_read_stamp(runner, scope, tenant_id, next).await?;
    Ok(next)
}

/// One browse projection row as the stamp-floor probe writes and removes it.
///
/// Deliberately minimal: the floor probe needs a row that can disappear
/// without a catalog-version bump, not the full projector shape.
#[derive(Clone, Debug)]
pub struct NewReadEntity {
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The entity's id.
    pub entity_id: Uuid,
    /// Operator-facing name.
    pub name: String,
    /// Lifecycle token the projector recorded.
    pub lifecycle_state: String,
    /// Published version carried on the serving row.
    pub published_version: i64,
    /// This row's own last apply.
    pub projected_at: DateTime<Utc>,
}

/// Insert one serving row. The table admits overwrite on rebuild; this
/// insert is the first write a floor probe needs.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn insert_read_entity(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewReadEntity,
) -> Result<(), RepoError> {
    let model = read_entity::ActiveModel {
        tenant_id: Set(new.tenant_id),
        entity_kind: Set(new.entity_kind.clone()),
        entity_id: Set(new.entity_id),
        entity_code: Set(None),
        name: Set(new.name),
        lifecycle_state: Set(new.lifecycle_state),
        deprecated: Set(false),
        composition_pending: Set(false),
        sellable: Set(None),
        deprecation_provenance: Set(None),
        replaced_by_sku_id: Set(None),
        region_scope: Set(String::new()),
        brand_scope: Set(String::new()),
        sku_type: Set(None),
        plan_tier_label: Set(None),
        metering_unit: Set(None),
        display_attributes: Set(None),
        category_paths: Set(None),
        published_version: Set(new.published_version),
        projected_at: Set(new.projected_at),
    };
    read_entity::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| {
            driver_failure(
                format!(
                    "insert read entity {}/{} scope",
                    new.entity_kind, new.entity_id
                ),
                e,
            )
        })?
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("insert read entity {}/{}", new.entity_kind, new.entity_id),
                e,
            )
        })?;
    Ok(())
}

/// Remove one serving row — the retirement flip's projection effect.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn delete_read_entity(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
) -> Result<u64, RepoError> {
    let result = read_entity::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(read_entity::Column::TenantId.eq(tenant_id))
                .add(read_entity::Column::EntityKind.eq(entity_kind))
                .add(read_entity::Column::EntityId.eq(entity_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("delete read entity {entity_kind}/{entity_id}"), e))?;
    Ok(result.rows_affected)
}

/// Count serving rows for one tenant — the floor probe's content coordinate.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn count_read_entities(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<usize, RepoError> {
    let rows = read_entity::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_entity::Column::TenantId.eq(tenant_id)))
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("count read entities of {tenant_id}"), e))?;
    Ok(rows.len())
}

#[cfg(test)]
#[path = "read_models_tests.rs"]
mod read_models_tests;
