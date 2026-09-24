//! Live reservations, confirmation and retained release history.
#![allow(
    clippy::too_many_arguments,
    reason = "Reference commands retain their scoped key and release attribution"
)]
use super::{HeadWrite, driver_failure, map_unique};
use crate::domain::references::{RefKind, ReferenceSummary};
use crate::infra::storage::{RepoError, entity::sku_reference};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Set};
use time::OffsetDateTime;
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;
/// Stored reservation, including its release history and actor attribution.
pub type SkuReference = sku_reference::Model;
/// Confirmation distinguishes replay, tombstone and absent reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmOutcome {
    Confirmed,
    AlreadyConfirmed,
    Released,
    Missing,
}
fn key(tenant: Uuid, id: Uuid) -> Condition {
    Condition::all()
        .add(sku_reference::Column::TenantId.eq(tenant))
        .add(sku_reference::Column::Id.eq(id))
}
/// Look up a live logical reference before an idempotent reserve.
/// # Errors
/// Returns scoped storage failures.
pub async fn find_live_reference(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    owner: &str,
    kind: RefKind,
    ref_id: Uuid,
) -> Result<Option<SkuReference>, RepoError> {
    sku_reference::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(sku_reference::Column::TenantId.eq(tenant_id))
                .add(sku_reference::Column::OwnerGear.eq(owner))
                .add(sku_reference::Column::RefKind.eq(kind.as_str()))
                .add(sku_reference::Column::RefId.eq(ref_id))
                .add(sku_reference::Column::State.ne("released")),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure("find live reference".into(), e))
}
/// Insert a fresh attempt after the door's eligibility check in the same serializable transaction.
/// # Errors
/// Returns `REFERENCE_EXISTS` or scoped storage failures.
pub async fn reserve_reference(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    owner: &str,
    kind: RefKind,
    ref_id: Uuid,
    actor: Uuid,
    now: OffsetDateTime,
) -> Result<SkuReference, RepoError> {
    let model = sku_reference::ActiveModel {
        id: Set(Uuid::new_v4()),
        tenant_id: Set(tenant_id),
        sku_id: Set(sku_id),
        owner_gear: Set(owner.into()),
        ref_kind: Set(kind.as_str().into()),
        ref_id: Set(ref_id),
        state: Set("reserved".into()),
        reserved_by: Set(actor),
        reserved_at: Set(now),
        confirmed_at: Set(None),
        released_at: Set(None),
        released_by: Set(None),
        release_reason: Set(None),
        forced: Set(false),
    };
    sku_reference::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure("reference scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| map_unique("reserve reference".into(), e))
}
/// Confirm only a reserved attempt; a released attempt can never be reactivated.
/// # Errors
/// Returns scoped storage or corrupt-state failures.
pub async fn confirm_reference(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    now: OffsetDateTime,
) -> Result<ConfirmOutcome, RepoError> {
    let r = sku_reference::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(sku_reference::Column::State, Expr::value("confirmed"))
        .col_expr(sku_reference::Column::ConfirmedAt, Expr::value(now))
        .filter(key(tenant_id, id).add(sku_reference::Column::State.eq("reserved")))
        .exec(runner)
        .await
        .map_err(|e| driver_failure("confirm reference".into(), e))?;
    if r.rows_affected == 1 {
        return Ok(ConfirmOutcome::Confirmed);
    }
    match find_reference(runner, scope, tenant_id, id).await? {
        None => Ok(ConfirmOutcome::Missing),
        Some(row) => match row.state.as_str() {
            "confirmed" => Ok(ConfirmOutcome::AlreadyConfirmed),
            "released" => Ok(ConfirmOutcome::Released),
            other => Err(RepoError::CorruptRow(format!(
                "unmatched reference state {other}"
            ))),
        },
    }
}
/// Read an attempt, including a released tombstone, for owner authorization/replay.
/// # Errors
/// Returns scoped storage failures.
pub async fn find_reference(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<Option<SkuReference>, RepoError> {
    sku_reference::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(key(tenant_id, id))
        .one(runner)
        .await
        .map_err(|e| driver_failure("find reference".into(), e))
}
/// Release once; retries leave the original attribution unchanged.
/// # Errors
/// Returns scoped storage failures.
pub async fn release_reference(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    actor: Uuid,
    reason: Option<&str>,
    forced: bool,
    now: OffsetDateTime,
) -> Result<HeadWrite<SkuReference>, RepoError> {
    let r = sku_reference::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(sku_reference::Column::State, Expr::value("released"))
        .col_expr(sku_reference::Column::ReleasedAt, Expr::value(now))
        .col_expr(sku_reference::Column::ReleasedBy, Expr::value(actor))
        .col_expr(sku_reference::Column::ReleaseReason, Expr::value(reason))
        .col_expr(sku_reference::Column::Forced, Expr::value(forced))
        .filter(key(tenant_id, id).add(sku_reference::Column::State.ne("released")))
        .exec(runner)
        .await
        .map_err(|e| driver_failure("release reference".into(), e))?;
    if r.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    find_reference(runner, scope, tenant_id, id)
        .await?
        .map(HeadWrite::Written)
        .ok_or_else(|| RepoError::CorruptRow("released reference disappeared".into()))
}
/// List every live attempt, including reservations that have not been confirmed.
/// # Errors
/// Returns scoped storage failures.
pub async fn live_references(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
) -> Result<Vec<SkuReference>, RepoError> {
    sku_reference::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(sku_reference::Column::TenantId.eq(tenant_id))
                .add(sku_reference::Column::SkuId.eq(sku_id))
                .add(sku_reference::Column::State.ne("released")),
        )
        .order_by(sku_reference::Column::Id, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("live references".into(), e))
}
/// Summarize live prices/plans and the reserved subset.
/// # Errors
/// Returns scoped storage failures or an unknown stored reference kind.
pub async fn reference_summary(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
) -> Result<ReferenceSummary, RepoError> {
    let mut summary = ReferenceSummary::default();
    for r in live_references(runner, scope, tenant_id, sku_id).await? {
        match r.ref_kind.as_str() {
            "price" => summary.prices = summary.prices.saturating_add(1),
            "plan_item" | "sold_as" => summary.plans = summary.plans.saturating_add(1),
            _ => {
                return Err(RepoError::CorruptRow(format!(
                    "reference kind {}",
                    r.ref_kind
                )));
            }
        }
        if r.state == "reserved" {
            summary.reserved = summary.reserved.saturating_add(1);
        }
    }
    Ok(summary)
}
