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
use toolkit_db::odata::sea_orm_filter::{
    FieldToColumn, LimitCfg, ODataFieldMapping, filter_node_to_condition, paginate_odata,
};
use toolkit_db::secure::{
    AccessScope, DBRunner, ScopeError, SecureDeleteExt, SecureEntityExt, SecureInsertExt,
    SecureUpdateExt,
};
use toolkit_odata::filter::convert_expr_to_filter_node;
use toolkit_odata::{ODataQuery, Page, SortDir};
use toolkit_odata_macros::ODataFilterable;
use uuid::Uuid;

use crate::domain::read_model::{
    StalenessStamp, StampAdvanceRefusal, StampApply, VisibilityFilter, advance_stamp,
};
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

/// The visibility contract's `WHERE` fragment (`dod-visibility`,
/// `inst-rb-query`): an `IN` over the states the surface serves, rendered
/// here rather than in the domain so `domain::read_model` names no ORM type
/// (P-D-163). The list is the domain's — [`VisibilityFilter::served_states`]
/// — and this function only spells it as SQL.
///
/// An `IN` over the served states rather than a `NOT IN` over the withheld
/// ones: the negative form serves any state added later, and
/// `lifecycle_state`'s roster is a five-value `CHECK` that a migration can
/// widen.
#[must_use]
pub fn visibility_condition(filter: VisibilityFilter) -> Condition {
    let served: Vec<String> = filter
        .served_states()
        .into_iter()
        .map(|s| s.as_str().to_owned())
        .collect();
    Condition::all().add(read_entity::Column::LifecycleState.is_in(served))
}

/// The query-build scope predicate for one axis (**P-D-39**).
///
/// **Empty means unrestricted**, so the predicate matches a row whose set is
/// empty **or** contains the caller's claim. Written as containment alone it
/// hides every unrestricted row — which is the whole catalogue of a tenant
/// that has set no scopes.
///
/// # It is set membership, and `contains` was a leak
///
/// The column is a **comma-joined token set**, not a scalar:
/// [`crate::domain::containment::SCOPE_VALUE_SEPARATOR`] is `,` and
/// `domain::containment` owns the rule. `ColumnTrait::contains` renders an
/// unanchored `LIKE '%claim%'` with nothing escaped, and the first version of
/// this function used it. Measured consequences, each a cross-scope read:
/// claim `eu` matched a row stored `eur`, `eu-west` or `aus,eu-central`;
/// claim `us` matched `aus`; and a claim containing `%` matched **every**
/// restricted row in the table. `SQLite`'s `LIKE` is ASCII-case-insensitive
/// and Postgres's is not, so the two engines disagreed as well.
///
/// The predicate below matches a **token by position** — the whole value, or
/// the set's first, last or a middle member — so `eu` cannot match `eur` or
/// `aus,eu-central`. A claim carrying the separator or either wildcard is
/// refused the containment arm entirely rather than escaped: it cannot be a
/// member of a well-formed set, and there is then no operand a caller
/// supplies that can widen the predicate.
///
/// **One residual, recorded rather than closed**: `SQLite`'s `LIKE` is
/// ASCII-case-insensitive and Postgres's is not, so on `SQLite` a claim `eu`
/// also admits a stored token `EU`. Scope values are resolved before
/// persistence (`domain::containment`) and nothing in the crate writes a
/// mixed-case token; closing it needs custom SQL, and the Postgres tier is
/// the authority for the served behaviour.
///
/// # It carries no tenant predicate, and must not be used alone
///
/// This is one axis of a `WHERE` clause. The tenant comes from the secure
/// scope (`.secure().scope_with(scope)`), and a query built with this filter
/// and no scope serves every tenant's unrestricted rows — which P-D-39 makes
/// the majority of rows. The probes compose it through the secure path for
/// that reason: a bare `Entity::find()` example is the one a door would copy.
#[must_use]
pub fn scope_condition(column: read_entity::Column, claim: &str) -> Condition {
    let sep = crate::domain::containment::SCOPE_VALUE_SEPARATOR;
    let unrestricted = Condition::any().add(column.eq(""));

    // **Fail closed on anything that is not a single token.** A scope value
    // is a resolved region or brand identifier; `domain::containment` chose a
    // comma precisely because neither kind is expected to contain one. A
    // claim carrying the separator, or either `LIKE` wildcard, cannot be a
    // member of a well-formed set — and admitting it as a pattern is the
    // leak: `%` alone matched every restricted row. Dropping the containment
    // arm leaves only the unrestricted rows, which is the safe direction.
    if claim.is_empty() || claim.contains([sep, '%', '_', '\\']) {
        return unrestricted;
    }

    // Token membership by position, in plain `sea_orm`: the whole value, the
    // first member, the last, or a middle one. No custom SQL and no `ESCAPE`
    // clause, because the guard above means the claim carries no wildcard to
    // escape — the two engines therefore agree on the pattern.
    unrestricted
        .add(column.eq(claim))
        .add(column.like(format!("{claim}{sep}%")))
        .add(column.like(format!("%{sep}{claim}")))
        .add(column.like(format!("%{sep}{claim}{sep}%")))
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

/// The browse door's **filterable vocabulary** (`inst-rb-query`, P-D-165).
///
/// A declaration, never constructed: `#[derive(ODataFilterable)]` reads the
/// fields and emits [`BrowseRowQueryFilterField`] (re-exported below as
/// [`BrowseFilterField`]), which is what `$filter` and `$orderby` are
/// validated against and what the `OpenAPI` `$filter` documentation is
/// generated from. Declaring the vocabulary as a struct rather than writing
/// the enum by hand is what keeps the wire contract, the spec and the
/// storage mapping from drifting apart — the `account-management` gear
/// declares its three the same way.
///
/// # What is deliberately absent, and why
///
/// * `tenant_id` — the caller does not choose it. `AccessScope` does, on the
///   `SecureSelect` this vocabulary is applied on top of.
/// * `entity_kind` — **an authorization operand, not a filter.** The door
///   gates on `product x read` when a Product may be in the answer and on
///   `sku x read` when a SKU may be, and it decides which by reading the
///   caller's requested kind: naming one kind narrows the grants required to
///   that kind's. A `$filter` cannot carry that decision safely, because
///   `entity_kind eq 'sku' or entity_kind eq 'product'` restricts nothing
///   while *reading* as a request for one kind — so a caller holding only
///   the SKU grant could reach Product rows. It stays the door's `kind`
///   operand, the way `account-management` keeps path-scoped `parent_id` off
///   its filter columns.
/// * `region_scope` / `brand_scope` — these are **not scalars**. The column
///   holds a comma-joined token set where *empty means unrestricted*
///   (P-D-39), and the predicate that reads it matches a token **by
///   position** precisely because an unanchored `LIKE '%claim%'` was a
///   measured cross-scope leak (claim `eu` matching a row stored `eur` or
///   `aus,eu-central`; a claim carrying `%` matching every restricted row).
///   Exposing them as `$filter` fields would hand that leak back to the
///   caller under a new spelling, so they stay the door's own operands
///   (`brand`, `region`) over [`scope_condition`]. `account-management`
///   keeps `parent_id` off its filter columns for the same class of reason.
/// * `display_attributes` — a canonical JSON rendering, not a queryable
///   value.
/// * `projected_at` / `generation` — the projection's own bookkeeping. The
///   serving generation is the checkpoint's and a caller may not pick
///   another one.
/// * `deprecation_provenance` / `replaced_by_sku_id` — no declared use.
///   Adding a field here later is additive; removing one is a wire break, so
///   the surface starts at what is asked for.
#[derive(ODataFilterable)]
#[allow(
    dead_code,
    reason = "a declaration read by the derive macro: only the generated \
              `BrowseRowQueryFilterField` is ever named in code, exactly as \
              `account-management`'s `IdpUserQuery` / `TenantInfoQuery` are"
)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the fields are a wire vocabulary, not state: each one names a \
              column and its comparison kind, and three of the projection's \
              columns happen to be boolean. Folding them into an enum would \
              change the filter surface a caller writes (`deprecated eq \
              false`) into something the storage cannot compare"
)]
pub struct BrowseRowQuery {
    /// The row's entity. Also the walk's unique tiebreaker.
    #[odata(filter(kind = "Uuid"))]
    pub entity_id: Uuid,
    /// `productCode` or `skuCode`; a Product may carry none.
    #[odata(filter(kind = "String"))]
    pub entity_code: String,
    /// The display name. `startswith(name, '...')` is the prefix search the
    /// door used to spell `?q=`, and the platform's lowering **escapes** the
    /// LIKE metacharacters that the hand-rolled prefix silently deleted.
    #[odata(filter(kind = "String"))]
    pub name: String,
    /// Narrowing only. The visibility predicate is `AND`ed underneath and
    /// decides what is servable at all (C2), so a caller cannot reach a
    /// state this surface does not serve by naming it here.
    #[odata(filter(kind = "String"))]
    pub lifecycle_state: String,
    /// `inst-ps-shape`'s three flags.
    #[odata(filter(kind = "Bool"))]
    pub deprecated: bool,
    #[odata(filter(kind = "Bool"))]
    pub composition_pending: bool,
    /// Only a SKU carries it.
    #[odata(filter(kind = "Bool"))]
    pub sellable: bool,
    #[odata(filter(kind = "String"))]
    pub sku_type: String,
    #[odata(filter(kind = "String"))]
    pub plan_tier_label: String,
    #[odata(filter(kind = "String"))]
    pub metering_unit: String,
    /// Every assigned category's full path, primary and secondary alike.
    /// `contains(category_paths, '...')` is what `?category=` spelled.
    #[odata(filter(kind = "String"))]
    pub category_paths: String,
    #[odata(filter(kind = "I64"))]
    pub published_version: i64,
}

/// The browse vocabulary under the name the rest of the gear uses, following
/// `account-management`'s `TenantInfoQueryFilterField as TenantInfoFilterField`.
pub use BrowseRowQueryFilterField as BrowseFilterField;

/// The browse query's operands (`inst-rb-query`): the predicates the **door**
/// owns rather than the caller's `$filter`.
///
/// Everything expressible as a column comparison moved to
/// [`BrowseRowQuery`]'s vocabulary when the door adopted the platform's
/// query contract (P-D-165). What is left is what cannot be a filter field:
/// the visibility surface (a policy over which lifecycle states are
/// servable), the entity kind (an authorization operand), the two scope
/// claims (set membership, not equality) and the serving generation (the
/// projection's, not the caller's) — see [`BrowseRowQuery`] for each reason.
#[derive(Debug, Clone, Default)]
pub struct BrowseQuery {
    pub visibility: Option<Condition>,
    /// `product`, `sku`, or both when absent — the kind the caller asked for
    /// and was authorized for.
    pub entity_kind: Option<String>,
    pub brand_claim: Option<String>,
    pub region_claim: Option<String>,
    pub generation: i64,
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
    if let Some(brand) = &query.brand_claim {
        condition = condition.add(scope_condition(read_entity::Column::BrandScope, brand));
    }
    if let Some(region) = &query.region_claim {
        condition = condition.add(scope_condition(read_entity::Column::RegionScope, region));
    }
    condition
}

/// The browse vocabulary's storage mapping: which column each filter field
/// reads, which of them a keyset walk may order by, and how a row's value is
/// encoded into a continuation token.
pub struct BrowseODataMapper;

impl FieldToColumn<BrowseFilterField> for BrowseODataMapper {
    type Column = read_entity::Column;

    fn map_field(field: BrowseFilterField) -> read_entity::Column {
        match field {
            BrowseFilterField::EntityId => read_entity::Column::EntityId,
            BrowseFilterField::EntityCode => read_entity::Column::EntityCode,
            BrowseFilterField::Name => read_entity::Column::Name,
            BrowseFilterField::LifecycleState => read_entity::Column::LifecycleState,
            BrowseFilterField::Deprecated => read_entity::Column::Deprecated,
            BrowseFilterField::CompositionPending => read_entity::Column::CompositionPending,
            BrowseFilterField::Sellable => read_entity::Column::Sellable,
            BrowseFilterField::SkuType => read_entity::Column::SkuType,
            BrowseFilterField::PlanTierLabel => read_entity::Column::PlanTierLabel,
            BrowseFilterField::MeteringUnit => read_entity::Column::MeteringUnit,
            BrowseFilterField::CategoryPaths => read_entity::Column::CategoryPaths,
            BrowseFilterField::PublishedVersion => read_entity::Column::PublishedVersion,
        }
    }

    /// A keyset walk may order only by a column that is **NOT NULL**.
    ///
    /// This is not fastidiousness about nulls: `SQLite` sorts NULLs first
    /// and Postgres sorts them last, so an order over a nullable column is a
    /// *different* order on the two engines the gear ships on. The cursor
    /// predicate `(a, b) > (a0, b0)` derived on one engine would then skip
    /// or repeat rows on the other, and the walk's whole guarantee is that
    /// it does neither. The six nullable columns stay filterable and are
    /// refused as order keys.
    fn is_orderable(field: BrowseFilterField) -> bool {
        !matches!(
            field,
            BrowseFilterField::EntityCode
                | BrowseFilterField::Sellable
                | BrowseFilterField::SkuType
                | BrowseFilterField::PlanTierLabel
                | BrowseFilterField::MeteringUnit
                | BrowseFilterField::CategoryPaths
        )
    }
}

impl ODataFieldMapping<BrowseFilterField> for BrowseODataMapper {
    type Entity = read_entity::Entity;

    /// Only the orderable fields can ever be asked for: `extract_cursor_value`
    /// is reached through `extract_cursor_values`, which walks the effective
    /// order, and `paginate_odata` has already refused a non-orderable key by
    /// then. The nullable arms are still written out rather than left to a
    /// catch-all so that adding a column to the vocabulary is a compile error
    /// here instead of a silently wrong token.
    fn extract_cursor_value(
        model: &read_entity::Model,
        field: BrowseFilterField,
    ) -> sea_orm::Value {
        match field {
            BrowseFilterField::EntityId => sea_orm::Value::from(model.entity_id),
            BrowseFilterField::EntityCode => sea_orm::Value::from(model.entity_code.clone()),
            BrowseFilterField::Name => sea_orm::Value::from(model.name.clone()),
            BrowseFilterField::LifecycleState => {
                sea_orm::Value::from(model.lifecycle_state.clone())
            }
            BrowseFilterField::Deprecated => sea_orm::Value::from(model.deprecated),
            BrowseFilterField::CompositionPending => {
                sea_orm::Value::from(model.composition_pending)
            }
            BrowseFilterField::Sellable => sea_orm::Value::from(model.sellable),
            BrowseFilterField::SkuType => sea_orm::Value::from(model.sku_type.clone()),
            BrowseFilterField::PlanTierLabel => sea_orm::Value::from(model.plan_tier_label.clone()),
            BrowseFilterField::MeteringUnit => sea_orm::Value::from(model.metering_unit.clone()),
            BrowseFilterField::CategoryPaths => sea_orm::Value::from(model.category_paths.clone()),
            BrowseFilterField::PublishedVersion => sea_orm::Value::from(model.published_version),
        }
    }
}

/// The order a browse answers in when the caller names none: `name ASC`,
/// which is what the door served before it could be asked for anything else.
pub const BROWSE_DEFAULT_ORDER: (&str, SortDir) = ("name", SortDir::Asc);

/// The walk's unique tiebreaker.
///
/// `paginate_odata` appends it to the effective order so the order is total
/// and the keyset predicate cannot straddle two rows that compare equal. It
/// must be a column the row is *uniquely* identified by within the walked
/// set, and `entity_id` is: the set is one tenant's one generation, and the
/// id is a v4 UUID minted per entity. (`account-management`'s note about a
/// non-unique tiebreaker is about two siblings sharing a `created_at`
/// microsecond — a collision with probability near one on a batch insert.
/// The collision this one would need is a UUID collision.)
pub const BROWSE_TIEBREAKER: (&str, SortDir) = ("entity_id", SortDir::Asc);

/// The caller's `$filter` lowered onto the read entity's columns.
///
/// `paginate_odata` does this internally for the page; the facet pass needs
/// the same predicate over the same set, so the lowering is exposed rather
/// than approximated a second time. Two answers to "which rows match" is
/// exactly how a facet count comes to disagree with the rows beside it.
///
/// # Errors
///
/// [`toolkit_odata::Error::InvalidFilter`] when the expression names a field
/// this door does not expose, compares it with an operator its kind does not
/// admit, or does not parse.
pub fn browse_filter_condition(odata: &ODataQuery) -> Result<Condition, toolkit_odata::Error> {
    let Some(ast) = odata.filter.as_deref() else {
        return Ok(Condition::all());
    };
    let node = convert_expr_to_filter_node::<BrowseFilterField>(ast)
        .map_err(|e| toolkit_odata::Error::InvalidFilter(e.to_string()))?;
    filter_node_to_condition::<BrowseFilterField, BrowseODataMapper>(&node)
        .map_err(toolkit_odata::Error::InvalidFilter)
}

/// The rows the facet counts are computed over: the filtered set, bounded,
/// and **not** the page.
///
/// Facets answer "what else is in this result", so computing them over the
/// page would make them a restatement of what the caller already has. They
/// are therefore taken over their own window of the matching set, in the
/// same order the page walks so the window is deterministic.
///
/// One row past `window` is fetched deliberately: whether it came back is
/// how the answer knows to say the counts are partial rather than leaving
/// the caller to assume they are not. (Before this door was paginated the
/// window and the page ceiling were the same 500 and nothing said which of
/// the two a number meant — so past 500 matches the counts were simply
/// wrong, silently.)
///
/// # Errors
///
/// [`toolkit_odata::Error`] on an unservable `$filter`, or a driver failure.
pub async fn browse_facet_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    query: &BrowseQuery,
    odata: &ODataQuery,
    window: u64,
) -> Result<(Vec<read_entity::Model>, bool), toolkit_odata::Error> {
    let rows = read_entity::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(browse_condition(tenant_id, query))
        .filter(browse_filter_condition(odata)?)
        .order_by(read_entity::Column::Name, sea_orm::Order::Asc)
        .order_by(read_entity::Column::EntityId, sea_orm::Order::Asc)
        .limit(window.saturating_add(1))
        .all(runner)
        .await
        .map_err(|e| toolkit_odata::Error::Db(e.to_string()))?;
    let complete = rows.len() as u64 <= window;
    let mut rows = rows;
    rows.truncate(usize::try_from(window).unwrap_or(usize::MAX));
    Ok((rows, complete))
}

/// Browse: one page of the serving rows the query admits, as a keyset walk.
///
/// The door's own predicates (visibility, the two scope claims, the serving
/// generation) are applied to the scoped select first; the caller's
/// `$filter`, `$orderby`, `$top` and continuation token are applied on top by
/// `paginate_odata`, which also returns the `next_cursor` the answer carries.
///
/// `limits` is the door's, not this module's: a page size is a property of
/// what the wire will carry, so the API layer owns the number and this layer
/// does not reach up for it.
///
/// # Errors
///
/// [`toolkit_odata::Error`] — the caller's four ways of writing an unservable
/// query, or a driver failure. `api::rest::odata::odata_error_to_canonical`
/// is what tells those apart.
pub async fn browse_read_entities_page(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    query: &BrowseQuery,
    odata: &ODataQuery,
    limits: LimitCfg,
) -> Result<Page<read_entity::Model>, toolkit_odata::Error> {
    let base = read_entity::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(browse_condition(tenant_id, query));

    let effective = super::effective_odata(odata, BROWSE_DEFAULT_ORDER);
    paginate_odata::<BrowseFilterField, BrowseODataMapper, _, _, _, _>(
        base,
        runner,
        &effective,
        BROWSE_TIEBREAKER,
        limits,
        |model| model,
    )
    .await
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
/// The deferred-intent dashboard's **filterable vocabulary** (P-D-165).
///
/// A declaration read by the derive macro; every column of this projection
/// is `NOT NULL`, so all five are orderable.
#[derive(ODataFilterable)]
#[allow(
    dead_code,
    reason = "a declaration read by the derive macro: only the generated \
              `DeferredIntentQueryFilterField` is ever named in code"
)]
pub struct DeferredIntentQuery {
    /// The parent whose retirement is deferred. Unique within a tenant, so
    /// it is the walk's tiebreaker.
    #[odata(filter(kind = "Uuid"))]
    pub product_id: Uuid,
    /// The cascade that deferred it.
    #[odata(filter(kind = "Uuid"))]
    pub cascade_ref: Uuid,
    /// How many children are still holding it. `children_count gt 0` is the
    /// worklist an operator actually wants.
    #[odata(filter(kind = "I64"))]
    pub children_count: i64,
    /// When the intent was recorded. The default order.
    #[odata(filter(kind = "DateTimeUtc"))]
    pub created_at: chrono::DateTime<Utc>,
    /// How long it has been held, at the projector's last apply.
    #[odata(filter(kind = "I64"))]
    pub age_secs: i64,
}

/// The vocabulary under the name the rest of the gear uses.
pub use DeferredIntentQueryFilterField as DeferredIntentFilterField;

/// The vocabulary's storage mapping.
pub struct DeferredIntentODataMapper;

impl FieldToColumn<DeferredIntentFilterField> for DeferredIntentODataMapper {
    type Column = read_deferred_intent::Column;

    fn map_field(field: DeferredIntentFilterField) -> read_deferred_intent::Column {
        use DeferredIntentFilterField as F;
        match field {
            F::ProductId => read_deferred_intent::Column::ProductId,
            F::CascadeRef => read_deferred_intent::Column::CascadeRef,
            F::ChildrenCount => read_deferred_intent::Column::ChildrenCount,
            F::CreatedAt => read_deferred_intent::Column::CreatedAt,
            F::AgeSecs => read_deferred_intent::Column::AgeSecs,
        }
    }
}

impl ODataFieldMapping<DeferredIntentFilterField> for DeferredIntentODataMapper {
    type Entity = read_deferred_intent::Entity;

    fn extract_cursor_value(
        model: &read_deferred_intent::Model,
        field: DeferredIntentFilterField,
    ) -> sea_orm::Value {
        use DeferredIntentFilterField as F;
        match field {
            F::ProductId => sea_orm::Value::from(model.product_id),
            F::CascadeRef => sea_orm::Value::from(model.cascade_ref),
            F::ChildrenCount => sea_orm::Value::from(model.children_count),
            F::CreatedAt => sea_orm::Value::from(model.created_at),
            F::AgeSecs => sea_orm::Value::from(model.age_secs),
        }
    }
}

/// The order the deferred-intent dashboard answers in when the caller names
/// none: oldest intent first, which is the order an operator works it.
pub const DEFERRED_INTENT_DEFAULT_ORDER: (&str, SortDir) = ("created_at", SortDir::Asc);

/// The walk's unique tiebreaker.
pub const DEFERRED_INTENT_TIEBREAKER: (&str, SortDir) = ("product_id", SortDir::Asc);

/// One page of the deferred-intent dashboard (P-D-165).
///
/// # Errors
///
/// [`toolkit_odata::Error`] on an unservable query, or a driver failure.
pub async fn read_deferred_intents(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    odata: &ODataQuery,
    limits: LimitCfg,
) -> Result<Page<read_deferred_intent::Model>, toolkit_odata::Error> {
    let base = read_deferred_intent::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_deferred_intent::Column::TenantId.eq(tenant_id)));
    let effective = super::effective_odata(odata, DEFERRED_INTENT_DEFAULT_ORDER);
    paginate_odata::<DeferredIntentFilterField, DeferredIntentODataMapper, _, _, _, _>(
        base,
        runner,
        &effective,
        DEFERRED_INTENT_TIEBREAKER,
        limits,
        |model| model,
    )
    .await
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
/// The freeze-status dashboard's **filterable vocabulary** (P-D-165).
///
/// Every column of this projection is `NOT NULL`, so all eight are
/// orderable. `freeze_state eq 'open'` is what the door's own description
/// calls the worklist.
#[derive(ODataFilterable)]
#[allow(
    dead_code,
    reason = "a declaration read by the derive macro: only the generated \
              `FreezeStatusQueryFilterField` is ever named in code"
)]
pub struct FreezeStatusQuery {
    /// The version. Unique within a tenant, so it is both the default order
    /// and the walk's tiebreaker.
    #[odata(filter(kind = "I64"))]
    pub catalog_version_id: i64,
    #[odata(filter(kind = "String"))]
    pub freeze_state: String,
    /// Participants that have neither acked nor released.
    #[odata(filter(kind = "I64"))]
    pub pending: i64,
    #[odata(filter(kind = "I64"))]
    pub acked: i64,
    #[odata(filter(kind = "I64"))]
    pub released: i64,
    /// Participants pinned `not_frozen` by a force-completion.
    #[odata(filter(kind = "I64"))]
    pub forced: i64,
    #[odata(filter(kind = "DateTimeUtc"))]
    pub published_at: chrono::DateTime<Utc>,
    #[odata(filter(kind = "DateTimeUtc"))]
    pub polled_at: chrono::DateTime<Utc>,
}

/// The vocabulary under the name the rest of the gear uses.
pub use FreezeStatusQueryFilterField as FreezeStatusFilterField;

/// The vocabulary's storage mapping.
pub struct FreezeStatusODataMapper;

impl FieldToColumn<FreezeStatusFilterField> for FreezeStatusODataMapper {
    type Column = read_freeze_status::Column;

    fn map_field(field: FreezeStatusFilterField) -> read_freeze_status::Column {
        use FreezeStatusFilterField as F;
        match field {
            F::CatalogVersionId => read_freeze_status::Column::CatalogVersionId,
            F::FreezeState => read_freeze_status::Column::FreezeState,
            F::Pending => read_freeze_status::Column::Pending,
            F::Acked => read_freeze_status::Column::Acked,
            F::Released => read_freeze_status::Column::Released,
            F::Forced => read_freeze_status::Column::Forced,
            F::PublishedAt => read_freeze_status::Column::PublishedAt,
            F::PolledAt => read_freeze_status::Column::PolledAt,
        }
    }
}

impl ODataFieldMapping<FreezeStatusFilterField> for FreezeStatusODataMapper {
    type Entity = read_freeze_status::Entity;

    fn extract_cursor_value(
        model: &read_freeze_status::Model,
        field: FreezeStatusFilterField,
    ) -> sea_orm::Value {
        use FreezeStatusFilterField as F;
        match field {
            F::CatalogVersionId => sea_orm::Value::from(model.catalog_version_id),
            F::FreezeState => sea_orm::Value::from(model.freeze_state.clone()),
            F::Pending => sea_orm::Value::from(model.pending),
            F::Acked => sea_orm::Value::from(model.acked),
            F::Released => sea_orm::Value::from(model.released),
            F::Forced => sea_orm::Value::from(model.forced),
            F::PublishedAt => sea_orm::Value::from(model.published_at),
            F::PolledAt => sea_orm::Value::from(model.polled_at),
        }
    }
}

/// The order the freeze dashboard answers in when the caller names none:
/// newest version first, because a freeze that is still open is a recent
/// one.
pub const FREEZE_STATUS_DEFAULT_ORDER: (&str, SortDir) = ("catalog_version_id", SortDir::Desc);

/// The walk's unique tiebreaker — the same column, which is unique within a
/// tenant, so the effective order is exactly the default.
pub const FREEZE_STATUS_TIEBREAKER: (&str, SortDir) = ("catalog_version_id", SortDir::Desc);

/// One page of the freeze-status dashboard (P-D-165).
///
/// # Errors
///
/// [`toolkit_odata::Error`] on an unservable query, or a driver failure.
pub async fn read_freeze_statuses(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    odata: &ODataQuery,
    limits: LimitCfg,
) -> Result<Page<read_freeze_status::Model>, toolkit_odata::Error> {
    let base = read_freeze_status::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(read_freeze_status::Column::TenantId.eq(tenant_id)));
    let effective = super::effective_odata(odata, FREEZE_STATUS_DEFAULT_ORDER);
    paginate_odata::<FreezeStatusFilterField, FreezeStatusODataMapper, _, _, _, _>(
        base,
        runner,
        &effective,
        FREEZE_STATUS_TIEBREAKER,
        limits,
        |model| model,
    )
    .await
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
