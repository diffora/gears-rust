//! Reads and writes of `pricing_window_baseline` behind the tenant gate (D-374).
//!
//! Free functions taking a **runner** rather than a provider, following
//! [`super::window_repo::schedule`]: callers own the transaction. Replacement is
//! wholesale for one owner revision; this module does not open a successor.

use std::collections::HashSet;

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt, SecureInsertExt,
};
use uuid::Uuid;

use crate::domain::draft_window::{DraftWindowOwner, WindowBaseline};
use crate::domain::scope_key::PlanId;
use crate::domain::window::WindowState;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::window_baseline;
use crate::infra::storage::repo::check_authored_instant;
use crate::infra::storage::repo::draft_window_repo::{
    require_draft_owner, require_price_on_plan, stored_revision,
};
use crate::infra::storage::repo::price_repo::{self, CANDIDATE_ROW_STATES};
use crate::infra::storage::repo::window_repo;

/// List the captured live-window references of one owner revision, ordered by
/// `window_id`.
///
/// A published or abandoned revision is still readable: those rows are retained
/// for audit.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure;
/// [`RepoError::CorruptRow`] when a stored `mutation_seq` is outside the
/// unsigned range.
pub async fn list(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
) -> Result<Vec<WindowBaseline>, RepoError> {
    let number = stored_revision(owner)?;
    let rows = window_baseline::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(baseline_owner_filter(owner, number))
        .order_by(window_baseline::Column::WindowId, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("list pricing_window_baseline: {e}")))?;
    rows.into_iter().map(|row| to_baseline(&row)).collect()
}

/// Replace the captured baseline of an open draft owner wholesale.
///
/// # Errors
/// [`RepoError::NotFound`] when the owner revision is absent or outside `scope`,
/// or a named price is not on this tenant and plan;
/// [`RepoError::NotDraft`] when the owner is frozen;
/// [`RepoError::TimestampPrecisionExceeded`] on an authored instant finer than
/// the millisecond quantum;
/// [`RepoError::CorruptRow`] when a `mutation_seq` exceeds the storable range;
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn replace(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    windows: &[WindowBaseline],
) -> Result<(), RepoError> {
    require_draft_owner(runner, scope, owner).await?;
    let number = stored_revision(owner)?;
    let mut rows = Vec::with_capacity(windows.len());
    for window in windows {
        check_authored_instant("effectiveFrom", Some(window.effective_from))?;
        check_authored_instant("effectiveTo", window.effective_to)?;
        require_price_on_plan(
            runner,
            scope,
            owner.tenant_id,
            owner.plan_id,
            window.price_id,
        )
        .await?;
        rows.push(to_model(owner, number, window)?);
    }

    window_baseline::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(baseline_owner_filter(owner, number))
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("clear pricing_window_baseline: {e}")))?;

    for row in rows {
        window_baseline::Entity::insert(row.clone())
            .secure()
            .scope_with_model(scope, &row)
            .map_err(|e| RepoError::Db(format!("pricing_window_baseline scope: {e}")))?
            .exec(runner)
            .await
            .map_err(|e| RepoError::Db(format!("insert pricing_window_baseline: {e}")))?;
    }
    Ok(())
}

/// Capture the plan's committed live windows as this draft owner's baseline.
///
/// Binding is by `price_id`: an authored scope-key change does not retarget
/// captured intervals onto another row. Membership is the publish candidate set
/// ([`CANDIDATE_ROW_STATES`]): superseded prices keep their live windows for
/// activation, but those windows are not compose operands. Ids, operator
/// versions and intervals come from the live plane under the caller's guard.
///
/// # Errors
/// The same refusals as [`replace`].
pub async fn replace_from_live(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
) -> Result<(), RepoError> {
    let captured =
        snapshot_live(runner, scope, owner.tenant_id, PlanId::new(owner.plan_id)).await?;
    replace(runner, scope, owner, &captured).await
}

/// The plan's live `pricing_price_window` rows on Published+Draft prices, as
/// baseline references, without writing.
///
/// Superseded covering stays on the live table. Frozen assemble, successor
/// capture and captured-baseline drift all use this membership so a leftover
/// predecessor window cannot fail compose as `not in the draft candidate set`.
///
/// # Errors
/// [`RepoError::Db`] / [`RepoError::CorruptRow`] from the live list or the
/// candidate-row load.
pub async fn snapshot_live(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
) -> Result<Vec<WindowBaseline>, RepoError> {
    let live = window_repo::list_for_plan(runner, scope, tenant_id, plan_id).await?;
    let candidates =
        price_repo::load_for_plan(runner, scope, tenant_id, plan_id, CANDIDATE_ROW_STATES).await?;
    let ids: HashSet<Uuid> = candidates.into_iter().map(|row| row.price_id).collect();
    Ok(live
        .into_iter()
        .filter(|row| ids.contains(&row.price_id))
        .map(|row| WindowBaseline {
            window_id: row.window_id,
            price_id: row.price_id,
            mutation_seq: row.mutation_seq,
            effective_from: row.effective_from,
            effective_to: row.effective_to,
            cancelled: row.state == WindowState::Cancelled,
        })
        .collect())
}

fn baseline_owner_filter(owner: &DraftWindowOwner, number: i64) -> Condition {
    Condition::all()
        .add(window_baseline::Column::TenantId.eq(owner.tenant_id))
        .add(window_baseline::Column::PlanId.eq(owner.plan_id))
        .add(window_baseline::Column::PlanRevision.eq(number))
}

fn to_model(
    owner: &DraftWindowOwner,
    number: i64,
    window: &WindowBaseline,
) -> Result<window_baseline::ActiveModel, RepoError> {
    let mutation_seq = i64::try_from(window.mutation_seq).map_err(|_| {
        RepoError::CorruptRow(format!(
            "window {} mutation_seq {} exceeds the storable range",
            window.window_id, window.mutation_seq
        ))
    })?;
    Ok(window_baseline::ActiveModel {
        tenant_id: Set(owner.tenant_id),
        plan_id: Set(owner.plan_id),
        plan_revision: Set(number),
        window_id: Set(window.window_id),
        price_id: Set(window.price_id),
        mutation_seq: Set(mutation_seq),
        effective_from: Set(window.effective_from),
        effective_to: Set(window.effective_to),
        cancelled: Set(window.cancelled),
    })
}

fn to_baseline(row: &window_baseline::Model) -> Result<WindowBaseline, RepoError> {
    let mutation_seq = u64::try_from(row.mutation_seq).map_err(|_| {
        RepoError::CorruptRow(format!(
            "pricing_window_baseline {} mutation_seq {} is negative",
            row.window_id, row.mutation_seq
        ))
    })?;
    Ok(WindowBaseline {
        window_id: row.window_id,
        price_id: row.price_id,
        mutation_seq,
        effective_from: row.effective_from,
        effective_to: row.effective_to,
        cancelled: row.cancelled,
    })
}
