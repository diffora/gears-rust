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
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QuerySelect};
use toolkit_db::secure::{
    AccessScope, DBRunner, ScopeError, SecureDeleteExt, SecureEntityExt, SecureInsertExt,
    SecureUpdateExt,
};
use uuid::Uuid;

use crate::domain::read_model::{StalenessStamp, StampAdvanceRefusal, StampApply, advance_stamp};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    read_checkpoint, read_deferred_intent, read_delivery_state, read_entity, read_freeze_status,
    read_inbox, read_poison, read_stamp,
};

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
        generation: Set(0),
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

// ---------------------------------------------------------------------------
// The projection plane (P-D-150): inbox, checkpoint, poison, the serving rows,
// the browse query, the dashboards.
// ---------------------------------------------------------------------------

/// One inbox row as the projector reads it.
#[derive(Debug, Clone)]
pub struct InboxRow {
    pub inbox_id: i64,
    pub tenant_id: Uuid,
    pub aggregate_id: Uuid,
    pub payload_type: String,
    pub payload: String,
    pub actor_ref: Uuid,
    pub created_at: DateTime<Utc>,
}

/// Write one consumed event to the inbox **inside the caller's transaction**
/// — the same one that wrote the outbox row, so `created_at` is the commit
/// instant (P-D-124).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
#[allow(clippy::too_many_arguments)] // the row's columns, all of them the event's
pub async fn record_read_inbox(
    runner: &impl DBRunner,
    tenant_id: Uuid,
    partition: u32,
    aggregate_id: Uuid,
    payload_type: &str,
    payload: &str,
    actor_ref: Uuid,
    created_at: DateTime<Utc>,
) -> Result<(), RepoError> {
    let scope = AccessScope::for_tenant(tenant_id);
    let model = read_inbox::ActiveModel {
        inbox_id: sea_orm::ActiveValue::NotSet,
        tenant_id: Set(tenant_id),
        partition: Set(i32::try_from(partition).unwrap_or(i32::MAX)),
        aggregate_id: Set(aggregate_id),
        payload_type: Set(payload_type.to_owned()),
        payload: Set(payload.to_owned()),
        actor_ref: Set(actor_ref),
        created_at: Set(created_at),
    };
    read_inbox::Entity::insert(model.clone())
        .secure()
        .scope_with_model(&scope, &model)
        .map_err(|e| driver_failure(format!("read inbox {payload_type} scope"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("read inbox {payload_type}"), e))?;
    Ok(())
}

/// The tenants holding inbox rows — the projector's discovery read.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn tenants_with_inbox(
    runner: &impl DBRunner,
    scope: &AccessScope,
) -> Result<Vec<Uuid>, RepoError> {
    #[derive(Debug, sea_orm::FromQueryResult)]
    struct TenantRow {
        tenant_id: Uuid,
    }
    let rows: Vec<TenantRow> = read_inbox::Entity::find()
        .secure()
        .scope_with(scope)
        .project_all(runner, |q| {
            q.select_only()
                .column(read_inbox::Column::TenantId)
                .distinct()
                .into_model::<TenantRow>()
        })
        .await
        .map_err(|e| driver_failure("discover inbox tenants".to_owned(), e))?;
    let mut tenants: Vec<Uuid> = rows.into_iter().map(|row| row.tenant_id).collect();
    tenants.sort();
    Ok(tenants)
}

/// The tenant's inbox rows above `after`, oldest first, at most `limit`.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn inbox_after(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    after: i64,
    limit: u64,
) -> Result<Vec<InboxRow>, RepoError> {
    let rows = read_inbox::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(read_inbox::Column::TenantId.eq(tenant_id))
                .add(read_inbox::Column::InboxId.gt(after)),
        )
        .order_by(read_inbox::Column::InboxId, sea_orm::Order::Asc)
        .limit(limit)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read inbox of {tenant_id}"), e))?;
    Ok(rows
        .into_iter()
        .map(|row| InboxRow {
            inbox_id: row.inbox_id,
            tenant_id: row.tenant_id,
            aggregate_id: row.aggregate_id,
            payload_type: row.payload_type,
            payload: row.payload,
            actor_ref: row.actor_ref,
            created_at: row.created_at,
        })
        .collect())
}

/// The inbox's oldest and newest ids for a tenant, `None` when empty.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn inbox_bounds(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Option<(i64, i64)>, RepoError> {
    let first = read_inbox::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_inbox::Column::TenantId.eq(tenant_id)))
        .order_by(read_inbox::Column::InboxId, sea_orm::Order::Asc)
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("inbox head of {tenant_id}"), e))?;
    let last = read_inbox::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_inbox::Column::TenantId.eq(tenant_id)))
        .order_by(read_inbox::Column::InboxId, sea_orm::Order::Desc)
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("inbox tail of {tenant_id}"), e))?;
    Ok(first.zip(last).map(|(a, b)| (a.inbox_id, b.inbox_id)))
}

/// Count the tenant's inbox rows above `after`, and the oldest such row's
/// `created_at` — the delivery dashboard's operands.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn inbox_pending(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    after: i64,
) -> Result<(u64, Option<DateTime<Utc>>), RepoError> {
    let rows = read_inbox::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(read_inbox::Column::TenantId.eq(tenant_id))
                .add(read_inbox::Column::InboxId.gt(after)),
        )
        .order_by(read_inbox::Column::InboxId, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("inbox pending of {tenant_id}"), e))?;
    let oldest = rows.first().map(|row| row.created_at);
    Ok((rows.len() as u64, oldest))
}

/// Sweep the tenant's inbox rows at or below `up_to` that are older than
/// `before` — consumed and past the replay window.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn sweep_inbox(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    up_to: i64,
    before: DateTime<Utc>,
) -> Result<u64, RepoError> {
    let result = read_inbox::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(read_inbox::Column::TenantId.eq(tenant_id))
                .add(read_inbox::Column::InboxId.lte(up_to))
                .add(read_inbox::Column::CreatedAt.lt(before)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("sweep inbox of {tenant_id}"), e))?;
    Ok(result.rows_affected)
}

/// The tenant's checkpoint: `(inbox_id, serving generation)`, or `None`
/// before the first pass.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn load_read_checkpoint(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Option<(i64, i64)>, RepoError> {
    let row = read_checkpoint::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_checkpoint::Column::TenantId.eq(tenant_id)))
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read checkpoint of {tenant_id}"), e))?;
    Ok(row.map(|row| (row.inbox_id, row.generation)))
}

/// Write the tenant's checkpoint (upsert).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn write_read_checkpoint(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    inbox_id: i64,
    generation: i64,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    let updated = read_checkpoint::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(read_checkpoint::Column::InboxId, Expr::value(inbox_id))
        .col_expr(read_checkpoint::Column::Generation, Expr::value(generation))
        .col_expr(read_checkpoint::Column::UpdatedAt, Expr::value(now))
        .filter(Condition::all().add(read_checkpoint::Column::TenantId.eq(tenant_id)))
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("write checkpoint of {tenant_id}"), e))?;
    if updated.rows_affected > 0 {
        return Ok(());
    }
    let model = read_checkpoint::ActiveModel {
        tenant_id: Set(tenant_id),
        inbox_id: Set(inbox_id),
        generation: Set(generation),
        updated_at: Set(now),
    };
    read_checkpoint::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("insert checkpoint of {tenant_id} scope"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("insert checkpoint of {tenant_id}"), e))?;
    Ok(())
}

/// Park (or re-park, bumping `attempts`) a poison inbox row.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn park_poison(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    inbox_id: i64,
    payload_type: &str,
    error: &str,
    now: DateTime<Utc>,
) -> Result<i32, RepoError> {
    let existing = read_poison::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_poison::Column::InboxId.eq(inbox_id)))
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read poison {inbox_id}"), e))?;
    if let Some(row) = existing {
        let attempts = row.attempts + 1;
        read_poison::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(read_poison::Column::Attempts, Expr::value(attempts))
            .col_expr(
                read_poison::Column::LastError,
                Expr::value(error.to_owned()),
            )
            .filter(Condition::all().add(read_poison::Column::InboxId.eq(inbox_id)))
            .exec(runner)
            .await
            .map_err(|e| driver_failure(format!("re-park poison {inbox_id}"), e))?;
        return Ok(attempts);
    }
    let model = read_poison::ActiveModel {
        inbox_id: Set(inbox_id),
        tenant_id: Set(tenant_id),
        payload_type: Set(payload_type.to_owned()),
        attempts: Set(1),
        last_error: Set(error.to_owned()),
        parked_at: Set(now),
        released_at: Set(None),
    };
    read_poison::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("park poison {inbox_id} scope"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("park poison {inbox_id}"), e))?;
    Ok(1)
}

/// The tenant's parked rows that are not yet released.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn parked_poison(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<read_poison::Model>, RepoError> {
    read_poison::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(read_poison::Column::TenantId.eq(tenant_id))
                .add(read_poison::Column::ReleasedAt.is_null()),
        )
        .order_by(read_poison::Column::InboxId, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read parked poison of {tenant_id}"), e))
}

/// Release a parked row (it projected after all, or the operator dismissed
/// it).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn release_poison(
    runner: &impl DBRunner,
    scope: &AccessScope,
    inbox_id: i64,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    read_poison::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(read_poison::Column::ReleasedAt, Expr::value(Some(now)))
        .filter(Condition::all().add(read_poison::Column::InboxId.eq(inbox_id)))
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("release poison {inbox_id}"), e))?;
    Ok(())
}

/// One serving row as the projector writes it — every column.
#[derive(Debug, Clone)]
pub struct ReadEntityRow {
    pub tenant_id: Uuid,
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub entity_code: Option<String>,
    pub name: String,
    pub lifecycle_state: String,
    pub deprecated: bool,
    pub composition_pending: bool,
    pub sellable: Option<bool>,
    pub deprecation_provenance: Option<String>,
    pub replaced_by_sku_id: Option<Uuid>,
    pub region_scope: String,
    pub brand_scope: String,
    pub sku_type: Option<String>,
    pub plan_tier_label: Option<String>,
    pub metering_unit: Option<String>,
    pub display_attributes: Option<String>,
    pub category_paths: Option<String>,
    pub published_version: i64,
    pub projected_at: DateTime<Utc>,
    pub generation: i64,
}

/// Upsert one serving row (the projector's write; idempotent per event).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn upsert_read_entity(
    runner: &impl DBRunner,
    scope: &AccessScope,
    row: ReadEntityRow,
) -> Result<(), RepoError> {
    let model = read_entity::ActiveModel {
        tenant_id: Set(row.tenant_id),
        entity_kind: Set(row.entity_kind.clone()),
        entity_id: Set(row.entity_id),
        entity_code: Set(row.entity_code),
        name: Set(row.name),
        lifecycle_state: Set(row.lifecycle_state),
        deprecated: Set(row.deprecated),
        composition_pending: Set(row.composition_pending),
        sellable: Set(row.sellable),
        deprecation_provenance: Set(row.deprecation_provenance),
        replaced_by_sku_id: Set(row.replaced_by_sku_id),
        region_scope: Set(row.region_scope),
        brand_scope: Set(row.brand_scope),
        sku_type: Set(row.sku_type),
        plan_tier_label: Set(row.plan_tier_label),
        metering_unit: Set(row.metering_unit),
        display_attributes: Set(row.display_attributes),
        category_paths: Set(row.category_paths),
        published_version: Set(row.published_version),
        projected_at: Set(row.projected_at),
        generation: Set(row.generation),
    };
    let on_conflict = OnConflict::columns([
        read_entity::Column::TenantId,
        read_entity::Column::EntityKind,
        read_entity::Column::EntityId,
    ])
    .update_columns([
        read_entity::Column::EntityCode,
        read_entity::Column::Name,
        read_entity::Column::LifecycleState,
        read_entity::Column::Deprecated,
        read_entity::Column::CompositionPending,
        read_entity::Column::Sellable,
        read_entity::Column::DeprecationProvenance,
        read_entity::Column::ReplacedBySkuId,
        read_entity::Column::RegionScope,
        read_entity::Column::BrandScope,
        read_entity::Column::SkuType,
        read_entity::Column::PlanTierLabel,
        read_entity::Column::MeteringUnit,
        read_entity::Column::DisplayAttributes,
        read_entity::Column::CategoryPaths,
        read_entity::Column::PublishedVersion,
        read_entity::Column::ProjectedAt,
        read_entity::Column::Generation,
    ])
    .to_owned();
    match read_entity::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| {
            driver_failure(
                format!(
                    "upsert read entity {}/{} scope",
                    row.entity_kind, row.entity_id
                ),
                e,
            )
        })?
        .on_conflict_raw(on_conflict)
        .exec(runner)
        .await
    {
        Ok(_) | Err(ScopeError::Db(sea_orm::DbErr::RecordNotInserted)) => Ok(()),
        Err(e) => Err(driver_failure(
            format!("upsert read entity {}/{}", row.entity_kind, row.entity_id),
            e,
        )),
    }
}

/// The head-read fields (`lifecycle_state`, `deprecation_provenance`,
/// `replaced_by_sku_id`, the flags) on one serving row — the `04` flips'
/// projection, which moves no frozen content.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
#[allow(clippy::too_many_arguments)] // the carve-out's columns, all of them
pub async fn set_read_entity_head_fields(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
    lifecycle_state: &str,
    deprecated: bool,
    deprecation_provenance: Option<&str>,
    replaced_by_sku_id: Option<Uuid>,
    projected_at: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let sellable = if entity_kind == "sku" {
        Some(lifecycle_state == "published")
    } else {
        None
    };
    let result = read_entity::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            read_entity::Column::LifecycleState,
            Expr::value(lifecycle_state.to_owned()),
        )
        .col_expr(read_entity::Column::Deprecated, Expr::value(deprecated))
        .col_expr(
            read_entity::Column::DeprecationProvenance,
            Expr::value(deprecation_provenance.map(str::to_owned)),
        )
        .col_expr(
            read_entity::Column::ReplacedBySkuId,
            Expr::value(replaced_by_sku_id),
        )
        .col_expr(read_entity::Column::Sellable, Expr::value(sellable))
        .col_expr(read_entity::Column::ProjectedAt, Expr::value(projected_at))
        .filter(
            Condition::all()
                .add(read_entity::Column::TenantId.eq(tenant_id))
                .add(read_entity::Column::EntityKind.eq(entity_kind))
                .add(read_entity::Column::EntityId.eq(entity_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("head fields of read entity {entity_id}"), e))?;
    Ok(result.rows_affected > 0)
}

/// One serving row by id, whatever its state (the read doors apply the
/// visibility filter themselves).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_read_entity(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
) -> Result<Option<read_entity::Model>, RepoError> {
    read_entity::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(read_entity::Column::TenantId.eq(tenant_id))
                .add(read_entity::Column::EntityKind.eq(entity_kind))
                .add(read_entity::Column::EntityId.eq(entity_id)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read entity {entity_id}"), e))
}

/// The browse query's operands (`inst-rb-query`): the visibility and scope
/// predicates are built into the statement — a shed row is never fetched.
#[derive(Debug, Clone, Default)]
pub struct BrowseQuery {
    pub visibility: Option<Condition>,
    pub entity_kind: Option<String>,
    pub category_path: Option<String>,
    pub sku_type: Option<String>,
    pub plan_tier_label: Option<String>,
    pub sellable: Option<bool>,
    pub metering_unit: Option<String>,
    pub brand_claim: Option<String>,
    pub region_claim: Option<String>,
    pub name_prefix: Option<String>,
    pub generation: i64,
    pub limit: u64,
}

fn browse_condition(tenant_id: Uuid, query: &BrowseQuery) -> Condition {
    let mut condition = Condition::all()
        .add(read_entity::Column::TenantId.eq(tenant_id))
        .add(read_entity::Column::Generation.eq(query.generation));
    if let Some(visibility) = query.visibility.clone() {
        condition = condition.add(visibility);
    }
    if let Some(kind) = &query.entity_kind {
        condition = condition.add(read_entity::Column::EntityKind.eq(kind.as_str()));
    }
    if let Some(path) = &query.category_path {
        // Every assigned category, primary and secondary alike
        // (`inst-rb-facets`): the paths column carries them all.
        condition = condition.add(read_entity::Column::CategoryPaths.like(format!("%{path}%")));
    }
    if let Some(sku_type) = &query.sku_type {
        condition = condition.add(read_entity::Column::SkuType.eq(sku_type.as_str()));
    }
    if let Some(label) = &query.plan_tier_label {
        condition = condition.add(read_entity::Column::PlanTierLabel.eq(label.as_str()));
    }
    if let Some(sellable) = query.sellable {
        condition = condition.add(read_entity::Column::Sellable.eq(sellable));
    }
    if let Some(unit) = &query.metering_unit {
        condition = condition.add(read_entity::Column::MeteringUnit.eq(unit.as_str()));
    }
    if let Some(prefix) = &query.name_prefix {
        let escaped = prefix.replace(['%', '_', '\\'], "");
        condition = condition.add(read_entity::Column::Name.like(format!("{escaped}%")));
    }
    if let Some(brand) = &query.brand_claim {
        condition = condition.add(crate::domain::read_model::scope_condition(
            read_entity::Column::BrandScope,
            brand,
        ));
    }
    if let Some(region) = &query.region_claim {
        condition = condition.add(crate::domain::read_model::scope_condition(
            read_entity::Column::RegionScope,
            region,
        ));
    }
    condition
}

/// Browse: the serving rows the query admits, ordered by name then id.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn browse_read_entities(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    query: &BrowseQuery,
) -> Result<Vec<read_entity::Model>, RepoError> {
    read_entity::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(browse_condition(tenant_id, query))
        .order_by(read_entity::Column::Name, sea_orm::Order::Asc)
        .order_by(read_entity::Column::EntityId, sea_orm::Order::Asc)
        .limit(query.limit.max(1))
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("browse read entities of {tenant_id}"), e))
}

/// The serving rows under a generation (the swap's operands).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn delete_read_generation(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    generation: i64,
) -> Result<u64, RepoError> {
    let result = read_entity::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(read_entity::Column::TenantId.eq(tenant_id))
                .add(read_entity::Column::Generation.eq(generation)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("drop read generation {generation}"), e))?;
    Ok(result.rows_affected)
}

/// Upsert one deferred-intent dashboard row.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn upsert_read_deferred_intent(
    runner: &impl DBRunner,
    scope: &AccessScope,
    row: read_deferred_intent::Model,
) -> Result<(), RepoError> {
    let model: read_deferred_intent::ActiveModel = row.clone().into();
    let on_conflict = OnConflict::columns([
        read_deferred_intent::Column::TenantId,
        read_deferred_intent::Column::ProductId,
    ])
    .update_columns([
        read_deferred_intent::Column::CascadeRef,
        read_deferred_intent::Column::ChildrenCount,
        read_deferred_intent::Column::CreatedAt,
        read_deferred_intent::Column::AgeSecs,
        read_deferred_intent::Column::PolledAt,
    ])
    .to_owned();
    match read_deferred_intent::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure("deferred intent dashboard scope".to_owned(), e))?
        .on_conflict_raw(on_conflict)
        .exec(runner)
        .await
    {
        Ok(_) | Err(ScopeError::Db(sea_orm::DbErr::RecordNotInserted)) => Ok(()),
        Err(e) => Err(driver_failure("deferred intent dashboard".to_owned(), e)),
    }
}

/// Drop the deferred-intent rows the poll no longer sees (resolved intents).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn prune_read_deferred_intents(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    keep: &[Uuid],
) -> Result<u64, RepoError> {
    let mut condition = Condition::all().add(read_deferred_intent::Column::TenantId.eq(tenant_id));
    if !keep.is_empty() {
        condition = condition.add(read_deferred_intent::Column::ProductId.is_not_in(keep.to_vec()));
    }
    let result = read_deferred_intent::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(condition)
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("prune deferred intents of {tenant_id}"), e))?;
    Ok(result.rows_affected)
}

/// The deferred-intent dashboard rows of a tenant.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn read_deferred_intents(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<read_deferred_intent::Model>, RepoError> {
    read_deferred_intent::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_deferred_intent::Column::TenantId.eq(tenant_id)))
        .order_by(read_deferred_intent::Column::CreatedAt, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("deferred intent dashboard of {tenant_id}"), e))
}

/// Upsert one freeze-status dashboard row.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn upsert_read_freeze_status(
    runner: &impl DBRunner,
    scope: &AccessScope,
    row: read_freeze_status::Model,
) -> Result<(), RepoError> {
    let model: read_freeze_status::ActiveModel = row.clone().into();
    let on_conflict = OnConflict::columns([
        read_freeze_status::Column::TenantId,
        read_freeze_status::Column::CatalogVersionId,
    ])
    .update_columns([
        read_freeze_status::Column::FreezeState,
        read_freeze_status::Column::Pending,
        read_freeze_status::Column::Acked,
        read_freeze_status::Column::Released,
        read_freeze_status::Column::Forced,
        read_freeze_status::Column::PublishedAt,
        read_freeze_status::Column::PolledAt,
    ])
    .to_owned();
    match read_freeze_status::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure("freeze status dashboard scope".to_owned(), e))?
        .on_conflict_raw(on_conflict)
        .exec(runner)
        .await
    {
        Ok(_) | Err(ScopeError::Db(sea_orm::DbErr::RecordNotInserted)) => Ok(()),
        Err(e) => Err(driver_failure("freeze status dashboard".to_owned(), e)),
    }
}

/// The freeze-status dashboard rows of a tenant, newest version first.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn read_freeze_statuses(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<read_freeze_status::Model>, RepoError> {
    read_freeze_status::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_freeze_status::Column::TenantId.eq(tenant_id)))
        .order_by(
            read_freeze_status::Column::CatalogVersionId,
            sea_orm::Order::Desc,
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("freeze status dashboard of {tenant_id}"), e))
}

/// Upsert the tenant's delivery-state dashboard row.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn upsert_read_delivery_state(
    runner: &impl DBRunner,
    scope: &AccessScope,
    row: read_delivery_state::Model,
) -> Result<(), RepoError> {
    let model: read_delivery_state::ActiveModel = row.clone().into();
    let on_conflict = OnConflict::column(read_delivery_state::Column::TenantId)
        .update_columns([
            read_delivery_state::Column::InboxPending,
            read_delivery_state::Column::Parked,
            read_delivery_state::Column::OldestPendingAgeSecs,
            read_delivery_state::Column::PolledAt,
        ])
        .to_owned();
    match read_delivery_state::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure("delivery state dashboard scope".to_owned(), e))?
        .on_conflict_raw(on_conflict)
        .exec(runner)
        .await
    {
        Ok(_) | Err(ScopeError::Db(sea_orm::DbErr::RecordNotInserted)) => Ok(()),
        Err(e) => Err(driver_failure("delivery state dashboard".to_owned(), e)),
    }
}

/// The tenant's delivery-state row.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn read_delivery_state(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Option<read_delivery_state::Model>, RepoError> {
    read_delivery_state::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_delivery_state::Column::TenantId.eq(tenant_id)))
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("delivery state dashboard of {tenant_id}"), e))
}

/// The serving rows of a tenant under `generation` (the refresh loops' and
/// the swap's operand).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn read_entities_of(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    generation: i64,
) -> Result<Vec<read_entity::Model>, RepoError> {
    read_entity::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(read_entity::Column::TenantId.eq(tenant_id))
                .add(read_entity::Column::Generation.eq(generation)),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read entities of {tenant_id}"), e))
}
