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
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, ExprTrait, QuerySelect};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use bss_products_sdk::models::LifecycleState;

use crate::domain::activation::{ClaimLease, RunFinish};
use crate::domain::deprecation::Provenance;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{deferred_retirement, product, scheduled_transition, sku};

use super::{HeadWrite, TenantIdRow, driver_failure};

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

/// The open deferred-retirement row for one Product, if any.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_unresolved_deferred_retirement(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
) -> Result<Option<deferred_retirement::Model>, RepoError> {
    deferred_retirement::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(deferred_retirement::Column::TenantId.eq(tenant_id))
                .add(deferred_retirement::Column::ProductId.eq(product_id))
                .add(deferred_retirement::Column::ResolvedAt.is_null()),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("unresolved deferred retirement of {product_id}"), e))
}

/// SKU ids whose live `replaced_by_sku_id` names `target`.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_skus_pointing_at(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    target: Uuid,
) -> Result<Vec<Uuid>, RepoError> {
    let rows = sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::ReplacedBySkuId.eq(target))
                .add(sku::Column::LifecycleState.ne(LifecycleState::Discarded.as_str())),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("skus pointing at {target}"), e))?;
    Ok(rows.into_iter().map(|row| row.sku_id).collect())
}

/// Deferred transitions whose announced `at` is older than `cutoff`.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn list_held_deferrals(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    cutoff: DateTime<Utc>,
) -> Result<Vec<scheduled_transition::Model>, RepoError> {
    scheduled_transition::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(scheduled_transition::Column::TenantId.eq(tenant_id))
                .add(scheduled_transition::Column::State.eq("deferred"))
                .add(scheduled_transition::Column::At.lte(cutoff)),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("held deferrals for {tenant_id}"), e))
}

/// Live retire intents for one entity (`pending`/`running`/`deferred`).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_live_retire_intents(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_id: Uuid,
) -> Result<Vec<scheduled_transition::Model>, RepoError> {
    scheduled_transition::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(scheduled_transition::Column::TenantId.eq(tenant_id))
                .add(scheduled_transition::Column::EntityId.eq(entity_id))
                .add(scheduled_transition::Column::Kind.eq("retire"))
                .add(scheduled_transition::Column::State.is_in(["pending", "running", "deferred"])),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("live retire intents of {entity_id}"), e))
}

/// Rows a tick may act on: due `pending`/`deferred`, or `running` past the lease.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn list_due_transitions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    now: DateTime<Utc>,
    lease: ClaimLease,
) -> Result<Vec<scheduled_transition::Model>, RepoError> {
    let cutoff = now - lease.ttl;
    scheduled_transition::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(scheduled_transition::Column::TenantId.eq(tenant_id))
                .add(
                    Condition::any()
                        .add(
                            Condition::all()
                                .add(
                                    scheduled_transition::Column::State
                                        .is_in(["pending", "deferred"]),
                                )
                                .add(scheduled_transition::Column::At.lte(now)),
                        )
                        .add(
                            Condition::all()
                                .add(scheduled_transition::Column::State.eq("running"))
                                .add(scheduled_transition::Column::ClaimedAt.is_not_null())
                                .add(scheduled_transition::Column::ClaimedAt.lte(cutoff)),
                        ),
                ),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("list due scheduled transitions for {tenant_id}"), e))
}

/// Tenants holding a row [`list_due_transitions`] would return.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn tenants_with_due_transitions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    now: DateTime<Utc>,
    lease: ClaimLease,
) -> Result<Vec<Uuid>, RepoError> {
    let cutoff = now - lease.ttl;
    let rows: Vec<TenantIdRow> = scheduled_transition::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::any()
                .add(
                    Condition::all()
                        .add(scheduled_transition::Column::State.is_in(["pending", "deferred"]))
                        .add(scheduled_transition::Column::At.lte(now)),
                )
                .add(
                    Condition::all()
                        .add(scheduled_transition::Column::State.eq("running"))
                        .add(scheduled_transition::Column::ClaimedAt.is_not_null())
                        .add(scheduled_transition::Column::ClaimedAt.lte(cutoff)),
                ),
        )
        .project_all(runner, |q| {
            q.select_only()
                .column(scheduled_transition::Column::TenantId)
                .distinct()
                .into_model::<TenantIdRow>()
        })
        .await
        .map_err(|e| driver_failure("discover due scheduled transitions".to_owned(), e))?;
    let mut tenants: Vec<Uuid> = rows.into_iter().map(|row| row.tenant_id).collect();
    tenants.sort();
    Ok(tenants)
}

/// Atomic claim: `pending|deferred → running` when `at` is due.
///
/// **P-D-113 arm 3**: `attempt` increments here, on the claim statement
/// itself. First claim, re-poll after a deferral — each spends one.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn claim_due_transition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    transition_id: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let result = scheduled_transition::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(scheduled_transition::Column::State, Expr::value("running"))
        .col_expr(scheduled_transition::Column::ClaimedAt, Expr::value(now))
        .col_expr(scheduled_transition::Column::UpdatedAt, Expr::value(now))
        .col_expr(
            scheduled_transition::Column::Attempt,
            Expr::col(scheduled_transition::Column::Attempt).add(Expr::val(1_i32)),
        )
        .filter(
            Condition::all()
                .add(scheduled_transition::Column::TenantId.eq(tenant_id))
                .add(scheduled_transition::Column::TransitionId.eq(transition_id))
                .add(scheduled_transition::Column::State.is_in(["pending", "deferred"]))
                .add(scheduled_transition::Column::At.lte(now)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("claim scheduled transition {transition_id}"), e))?;
    Ok(result.rows_affected == 1)
}

/// Lease reclaim: `running → pending`, `attempt += 1`, `claimed_at` cleared.
/// The lease is a try of its own (P-D-113); the numbers come from config.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn reclaim_expired_lease(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    transition_id: Uuid,
    now: DateTime<Utc>,
    lease: ClaimLease,
) -> Result<bool, RepoError> {
    let cutoff = now - lease.ttl;
    let result = scheduled_transition::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(scheduled_transition::Column::State, Expr::value("pending"))
        .col_expr(
            scheduled_transition::Column::ClaimedAt,
            Expr::value(Option::<DateTime<Utc>>::None),
        )
        .col_expr(
            scheduled_transition::Column::Attempt,
            Expr::col(scheduled_transition::Column::Attempt).add(Expr::val(1_i32)),
        )
        .col_expr(scheduled_transition::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(scheduled_transition::Column::TenantId.eq(tenant_id))
                .add(scheduled_transition::Column::TransitionId.eq(transition_id))
                .add(scheduled_transition::Column::State.eq("running"))
                .add(scheduled_transition::Column::ClaimedAt.is_not_null())
                .add(scheduled_transition::Column::ClaimedAt.lte(cutoff)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("reclaim scheduled transition {transition_id}"), e))?;
    Ok(result.rows_affected == 1)
}

/// Finish a `running` row. The stored state comes from [`RunFinish`] so a
/// raw `"pending"` cannot un-finish the row. `attempt` is not moved here:
/// the claim already spent one, and a later re-poll spends the next.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn finish_scheduled_transition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    transition_id: Uuid,
    finish: &RunFinish,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let outcome_reason = match finish {
        RunFinish::Applied => None,
        RunFinish::Failed { reason } | RunFinish::Deferred { reason, .. } => Some(reason.clone()),
    };
    let update = scheduled_transition::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            scheduled_transition::Column::State,
            Expr::value(finish.state().as_str()),
        )
        .col_expr(
            scheduled_transition::Column::OutcomeReason,
            Expr::value(outcome_reason),
        )
        .col_expr(scheduled_transition::Column::UpdatedAt, Expr::value(now));
    let result = update
        .filter(
            Condition::all()
                .add(scheduled_transition::Column::TenantId.eq(tenant_id))
                .add(scheduled_transition::Column::TransitionId.eq(transition_id))
                .add(scheduled_transition::Column::State.eq("running")),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("finish scheduled transition {transition_id}"), e))?;
    Ok(result.rows_affected == 1)
}

/// Tenant-scoped list for the scheduled-transition read door (**P-D-134**).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn list_scheduled_transitions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    state: Option<&str>,
) -> Result<Vec<scheduled_transition::Model>, RepoError> {
    let mut filter = Condition::all().add(scheduled_transition::Column::TenantId.eq(tenant_id));
    if let Some(state) = state {
        filter = filter.add(scheduled_transition::Column::State.eq(state));
    }
    scheduled_transition::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(filter)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("list scheduled transitions for {tenant_id}"), e))
}

/// Supersede one live row by id — the governed cancel (**P-D-114**).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn supersede_scheduled_transition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    transition_id: Uuid,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let result = scheduled_transition::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            scheduled_transition::Column::State,
            Expr::value("superseded"),
        )
        .col_expr(scheduled_transition::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(scheduled_transition::Column::TenantId.eq(tenant_id))
                .add(scheduled_transition::Column::TransitionId.eq(transition_id))
                .add(scheduled_transition::Column::State.is_in(["pending", "running", "deferred"])),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(format!("supersede scheduled transition {transition_id}"), e)
        })?;
    Ok(result.rows_affected == 1)
}

/// Supersede every live intent of one entity and kind.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn supersede_live_intents(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_id: Uuid,
    kind: &str,
    now: DateTime<Utc>,
) -> Result<u64, RepoError> {
    let result = scheduled_transition::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            scheduled_transition::Column::State,
            Expr::value("superseded"),
        )
        .col_expr(scheduled_transition::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(scheduled_transition::Column::TenantId.eq(tenant_id))
                .add(scheduled_transition::Column::EntityId.eq(entity_id))
                .add(scheduled_transition::Column::Kind.eq(kind))
                .add(scheduled_transition::Column::State.is_in(["pending", "running", "deferred"])),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("supersede {kind} intents of {entity_id}"), e))?;
    Ok(result.rows_affected)
}

/// Resolve a live deferred-retirement row. Never deletes.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn resolve_deferred_retirement(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
    cascade_ref: Uuid,
    resolution: &str,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let result = deferred_retirement::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(deferred_retirement::Column::ResolvedAt, Expr::value(now))
        .col_expr(
            deferred_retirement::Column::Resolution,
            Expr::value(resolution),
        )
        .filter(
            Condition::all()
                .add(deferred_retirement::Column::TenantId.eq(tenant_id))
                .add(deferred_retirement::Column::ProductId.eq(product_id))
                .add(deferred_retirement::Column::CascadeRef.eq(cascade_ref))
                .add(deferred_retirement::Column::ResolvedAt.is_null()),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("resolve deferred retirement {product_id}/{cascade_ref}"),
                e,
            )
        })?;
    Ok(result.rows_affected == 1)
}

/// Deprecate one SKU head — `published → deprecated`, stamping
/// `deprecation_provenance` in the same statement.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. An inadmissible write is
/// [`HeadWrite::Unmatched`].
pub async fn deprecate_sku_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    expected_internal_revision: i64,
    provenance: Provenance,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let result = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            sku::Column::LifecycleState,
            Expr::value(LifecycleState::Deprecated.as_str()),
        )
        .col_expr(
            sku::Column::DeprecationProvenance,
            Expr::value(provenance.as_str()),
        )
        .col_expr(
            sku::Column::InternalRevision,
            Expr::col(sku::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(sku::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::SkuId.eq(sku_id))
                .add(sku::Column::InternalRevision.eq(expected_internal_revision))
                .add(sku::Column::LifecycleState.eq(LifecycleState::Published.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("deprecate sku {sku_id}"), e))?;
    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Un-deprecate one Product head — `deprecated → published`, clearing
/// `deprecation_provenance` in the same statement (P-D-34).
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. An inadmissible write is
/// [`HeadWrite::Unmatched`].
pub async fn undeprecate_product_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
    expected_internal_revision: i64,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let result = product::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            product::Column::LifecycleState,
            Expr::value(LifecycleState::Published.as_str()),
        )
        .col_expr(
            product::Column::DeprecationProvenance,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            product::Column::InternalRevision,
            Expr::col(product::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(product::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::ProductId.eq(product_id))
                .add(product::Column::InternalRevision.eq(expected_internal_revision))
                .add(product::Column::LifecycleState.eq(LifecycleState::Deprecated.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("undeprecate product {product_id}"), e))?;
    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Un-deprecate one SKU head — `deprecated → published`, clearing
/// provenance in the same statement. `required_provenance` pins the
/// reversal operand: `Some(Cascaded)` for a parent-driven revival,
/// `None` for an operator act (any stored provenance).
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. An inadmissible write is
/// [`HeadWrite::Unmatched`].
pub async fn undeprecate_sku_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    expected_internal_revision: i64,
    required_provenance: Option<Provenance>,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let mut filter = Condition::all()
        .add(sku::Column::TenantId.eq(tenant_id))
        .add(sku::Column::SkuId.eq(sku_id))
        .add(sku::Column::InternalRevision.eq(expected_internal_revision))
        .add(sku::Column::LifecycleState.eq(LifecycleState::Deprecated.as_str()));
    if let Some(provenance) = required_provenance {
        filter = filter.add(sku::Column::DeprecationProvenance.eq(provenance.as_str()));
    }
    let result = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            sku::Column::LifecycleState,
            Expr::value(LifecycleState::Published.as_str()),
        )
        .col_expr(
            sku::Column::DeprecationProvenance,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            sku::Column::InternalRevision,
            Expr::col(sku::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(sku::Column::UpdatedAt, Expr::value(now))
        .filter(filter)
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("undeprecate sku {sku_id}"), e))?;
    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Write `replaced_by_sku_id` on a retirement initiation that takes no
/// edge — the SKU is already `deprecated`. Provenance is not touched
/// (P-D-34).
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. An inadmissible write is
/// [`HeadWrite::Unmatched`].
pub async fn write_replaced_by(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    expected_internal_revision: i64,
    replaced_by: Option<Uuid>,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let result = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(sku::Column::ReplacedBySkuId, Expr::value(replaced_by))
        .col_expr(
            sku::Column::InternalRevision,
            Expr::col(sku::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(sku::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::SkuId.eq(sku_id))
                .add(sku::Column::InternalRevision.eq(expected_internal_revision))
                .add(sku::Column::LifecycleState.eq(LifecycleState::Deprecated.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("write replaced_by on sku {sku_id}"), e))?;
    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Clear `replaced_by_sku_id` on the governed cancel (P-D-49).
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. An inadmissible write is
/// [`HeadWrite::Unmatched`].
pub async fn clear_replaced_by(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    expected_internal_revision: i64,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    write_replaced_by(
        runner,
        scope,
        tenant_id,
        sku_id,
        expected_internal_revision,
        None,
        now,
    )
    .await
}

/// Operands for [`deprecate_sku_head_with_replacement`].
#[derive(Debug, Clone, Copy)]
pub struct SkuDeprecationWrite {
    /// Tenant partition.
    pub tenant_id: Uuid,
    /// The SKU to force `deprecated`.
    pub sku_id: Uuid,
    /// Pinned revision.
    pub expected_internal_revision: i64,
    /// Provenance stamped in the same statement.
    pub provenance: Provenance,
    /// Optional successor.
    pub replaced_by: Option<Uuid>,
    /// Write clock.
    pub now: DateTime<Utc>,
}

/// Stamp `replaced_by` in the same statement as a deprecation.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. An inadmissible write is
/// [`HeadWrite::Unmatched`].
pub async fn deprecate_sku_head_with_replacement(
    runner: &impl DBRunner,
    scope: &AccessScope,
    write: SkuDeprecationWrite,
) -> Result<HeadWrite, RepoError> {
    let SkuDeprecationWrite {
        tenant_id,
        sku_id,
        expected_internal_revision,
        provenance,
        replaced_by,
        now,
    } = write;
    let result = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            sku::Column::LifecycleState,
            Expr::value(LifecycleState::Deprecated.as_str()),
        )
        .col_expr(
            sku::Column::DeprecationProvenance,
            Expr::value(provenance.as_str()),
        )
        .col_expr(sku::Column::ReplacedBySkuId, Expr::value(replaced_by))
        .col_expr(
            sku::Column::InternalRevision,
            Expr::col(sku::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(sku::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::SkuId.eq(sku_id))
                .add(sku::Column::InternalRevision.eq(expected_internal_revision))
                .add(sku::Column::LifecycleState.eq(LifecycleState::Published.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("deprecate sku {sku_id} with replacement"), e))?;
    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Flip a SKU `deprecated → retired`. The runner is this edge's door
/// (**P-D-113**): no wire surface writes it.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. An inadmissible write is
/// [`HeadWrite::Unmatched`].
pub async fn retire_sku_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    expected_internal_revision: i64,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let result = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            sku::Column::LifecycleState,
            Expr::value(LifecycleState::Retired.as_str()),
        )
        .col_expr(
            sku::Column::InternalRevision,
            Expr::col(sku::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(sku::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::SkuId.eq(sku_id))
                .add(sku::Column::InternalRevision.eq(expected_internal_revision))
                .add(sku::Column::LifecycleState.eq(LifecycleState::Deprecated.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("retire sku {sku_id}"), e))?;
    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Flip a Product `deprecated → retired`, same terms as [`retire_sku_head`].
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. An inadmissible write is
/// [`HeadWrite::Unmatched`].
pub async fn retire_product_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
    expected_internal_revision: i64,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let result = product::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            product::Column::LifecycleState,
            Expr::value(LifecycleState::Retired.as_str()),
        )
        .col_expr(
            product::Column::InternalRevision,
            Expr::col(product::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(product::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::ProductId.eq(product_id))
                .add(product::Column::InternalRevision.eq(expected_internal_revision))
                .add(product::Column::LifecycleState.eq(LifecycleState::Deprecated.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("retire product {product_id}"), e))?;
    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;

/// Every unresolved deferred retirement of a tenant — the deferred-intent
/// dashboard's source (`inst-ps-dashboards`, P-D-150), read by the poller.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn unresolved_deferred_retirements(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<deferred_retirement::Model>, RepoError> {
    deferred_retirement::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(deferred_retirement::Column::TenantId.eq(tenant_id))
                .add(deferred_retirement::Column::ResolvedAt.is_null()),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("unresolved deferred retirements of {tenant_id}"), e))
}

/// The tenants holding an unresolved deferred retirement.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn tenants_with_unresolved_deferred_retirements(
    runner: &impl DBRunner,
    scope: &AccessScope,
) -> Result<Vec<Uuid>, RepoError> {
    let rows: Vec<TenantIdRow> = deferred_retirement::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(deferred_retirement::Column::ResolvedAt.is_null()))
        .project_all(runner, |q| {
            q.select_only()
                .column(deferred_retirement::Column::TenantId)
                .distinct()
                .into_model::<TenantIdRow>()
        })
        .await
        .map_err(|e| driver_failure("discover deferred tenants".to_owned(), e))?;
    let mut tenants: Vec<Uuid> = rows.into_iter().map(|row| row.tenant_id).collect();
    tenants.sort();
    Ok(tenants)
}
