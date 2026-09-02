//! Persistence for the two lifecycle stores — `products_scheduled_transition`
//! and `products_deferred_retirement` (`design/04-lifecycle.md` §4).
//!
//! Phase D1 closes the store Definitions of Done: insert one row, read it
//! back. The runner's claim CAS, supersede and resolve writers arrive with
//! the `DoD`s that need them; giving those a typed surface now would be scope
//! taken from later groups.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-scheduled-transition-store:p1
//! @cpt-dod:cpt-cf-bss-products-dod-deferred-retirement-store:p1

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;

use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{deferred_retirement, scheduled_transition};

use super::driver_failure;

/// Columns a new scheduled-transition row carries at insert.
#[derive(Debug, Clone)]
pub struct NewScheduledTransition {
    /// Surrogate key.
    pub transition_id: Uuid,
    /// Tenant partition.
    pub tenant_id: Uuid,
    /// `product` or `sku`.
    pub entity_kind: String,
    /// Subject entity id.
    pub entity_id: Uuid,
    /// `publish` or `retire`.
    pub kind: String,
    /// UTC activation instant.
    pub at: DateTime<Utc>,
    /// Pinned approval snapshot.
    pub approval_ref: Uuid,
    /// Operator retirement text; `None` on a publish intent.
    pub retirement_reason: Option<String>,
    /// Insert clock.
    pub now: DateTime<Utc>,
}

/// Insert one `pending` scheduled transition with `attempt = 0`.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, including a partial-UNIQUE
/// collision with an already-live intent for the same entity and kind.
pub async fn insert_scheduled_transition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    row: &NewScheduledTransition,
) -> Result<(), RepoError> {
    let model = scheduled_transition::ActiveModel {
        transition_id: Set(row.transition_id),
        tenant_id: Set(row.tenant_id),
        entity_kind: Set(row.entity_kind.clone()),
        entity_id: Set(row.entity_id),
        kind: Set(row.kind.clone()),
        at: Set(row.at),
        approval_ref: Set(row.approval_ref),
        state: Set("pending".to_owned()),
        claimed_at: Set(None),
        attempt: Set(0),
        retirement_reason: Set(row.retirement_reason.clone()),
        outcome_reason: Set(None),
        created_at: Set(row.now),
        updated_at: Set(row.now),
    };
    scheduled_transition::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| {
            driver_failure(
                format!("scope scheduled transition {}", row.transition_id),
                e,
            )
        })?
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("insert scheduled transition {}", row.transition_id),
                e,
            )
        })?;
    Ok(())
}

/// Load one scheduled transition by id, or `None` if it does not resolve
/// under the caller's scope.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_scheduled_transition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    transition_id: Uuid,
) -> Result<Option<scheduled_transition::Model>, RepoError> {
    scheduled_transition::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(scheduled_transition::Column::TenantId.eq(tenant_id))
                .add(scheduled_transition::Column::TransitionId.eq(transition_id)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("find scheduled transition {transition_id}"), e))
}

/// Columns a new deferred-retirement row carries at insert.
#[derive(Debug, Clone)]
pub struct NewDeferredRetirement {
    /// Tenant partition.
    pub tenant_id: Uuid,
    /// Parent Product.
    pub product_id: Uuid,
    /// Parent's `ScheduledTransition` id.
    pub cascade_ref: Uuid,
    /// Leave-and-list snapshot JSON.
    pub children_snapshot: String,
    /// Actor who recorded the deferral.
    pub created_by: Uuid,
    /// Insert clock.
    pub now: DateTime<Utc>,
}

/// Insert one unresolved deferred-retirement snapshot.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, including a partial-UNIQUE
/// collision with an already-live deferral for the same Product, or an FK
/// miss on `cascade_ref`.
pub async fn insert_deferred_retirement(
    runner: &impl DBRunner,
    scope: &AccessScope,
    row: &NewDeferredRetirement,
) -> Result<(), RepoError> {
    let model = deferred_retirement::ActiveModel {
        tenant_id: Set(row.tenant_id),
        product_id: Set(row.product_id),
        cascade_ref: Set(row.cascade_ref),
        children_snapshot: Set(row.children_snapshot.clone()),
        created_by: Set(row.created_by),
        resolved_at: Set(None),
        resolution: Set(None),
        created_at: Set(row.now),
    };
    deferred_retirement::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| {
            driver_failure(
                format!(
                    "scope deferred retirement {}/{}",
                    row.product_id, row.cascade_ref
                ),
                e,
            )
        })?
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!(
                    "insert deferred retirement {}/{}",
                    row.product_id, row.cascade_ref
                ),
                e,
            )
        })?;
    Ok(())
}

/// Load one deferred-retirement row by its composite key, or `None`.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_deferred_retirement(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
    cascade_ref: Uuid,
) -> Result<Option<deferred_retirement::Model>, RepoError> {
    deferred_retirement::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(deferred_retirement::Column::TenantId.eq(tenant_id))
                .add(deferred_retirement::Column::ProductId.eq(product_id))
                .add(deferred_retirement::Column::CascadeRef.eq(cascade_ref)),
        )
        .one(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("find deferred retirement {product_id}/{cascade_ref}"),
                e,
            )
        })
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
