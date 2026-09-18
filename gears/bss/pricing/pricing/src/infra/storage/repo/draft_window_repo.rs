//! Reads and writes of `pricing_draft_window` behind the tenant gate (D-374).
//!
//! Free functions taking a **runner** rather than a provider, following
//! [`super::window_repo::schedule`]: callers own the transaction. These methods
//! do not open a successor revision and do not bump the plan's row version —
//! orchestration (Task 4) owns that compare-and-swap.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt, SecureInsertExt,
};
use uuid::Uuid;

use crate::domain::draft_window::{
    DraftStart, DraftWindowAction, DraftWindowEntry, DraftWindowOwner,
};
use crate::domain::scope_key::PlanId;
use crate::infra::storage::entity::{draft_window, price};
use crate::infra::storage::repo::check_authored_instant;
use crate::infra::storage::repo::plan_repo::{load_revision, not_found};
use crate::infra::storage::{RepoError, contention_or_db};

/// List the draft-window operations of one owner revision, ordered by
/// `operation_id`.
///
/// A published or abandoned revision is still readable: those rows are retained
/// for audit. SQL-level BOLA: a foreign tenant's set is empty.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure;
/// [`RepoError::CorruptRow`] when a stored token is outside its CHECK set.
pub async fn list(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
) -> Result<Vec<DraftWindowEntry>, RepoError> {
    let number = stored_revision(owner)?;
    let rows = draft_window::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(owner_filter(owner, number))
        .order_by(draft_window::Column::OperationId, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("list pricing_draft_window: {e}")))?;
    rows.into_iter().map(to_entry).collect()
}

/// Insert or replace one draft-window operation under an open draft owner.
///
/// # Errors
/// [`RepoError::NotFound`] when the owner revision is absent or outside `scope`;
/// [`RepoError::NotDraft`] when the owner is frozen;
/// [`RepoError::TimestampPrecisionExceeded`] on an authored instant finer than
/// the millisecond quantum;
/// [`RepoError::ConcurrentMutation`] on a duplicate target window;
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn put(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    entry: &DraftWindowEntry,
) -> Result<(), RepoError> {
    require_draft_owner(runner, scope, owner).await?;
    let number = stored_revision(owner)?;
    if let DraftWindowAction::Create { price_id, .. } = &entry.action {
        require_price_on_plan(runner, scope, owner.tenant_id, owner.plan_id, *price_id).await?;
    }
    let row = to_model(owner, number, entry)?;

    draft_window::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            owner_filter(owner, number)
                .add(draft_window::Column::OperationId.eq(entry.operation_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("replace pricing_draft_window: {e}")))?;

    draft_window::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| RepoError::Db(format!("pricing_draft_window scope: {e}")))?
        .exec(runner)
        .await
        .map_err(|e| contention_or_db(&e, "draft window", "insert pricing_draft_window"))?;
    Ok(())
}

/// Remove one operation from an open draft owner.
///
/// # Errors
/// [`RepoError::NotFound`] when the owner or the operation is absent or outside
/// `scope`; [`RepoError::NotDraft`] when the owner is frozen;
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn remove(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    operation_id: Uuid,
) -> Result<(), RepoError> {
    require_draft_owner(runner, scope, owner).await?;
    let number = stored_revision(owner)?;
    let result = draft_window::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(owner_filter(owner, number).add(draft_window::Column::OperationId.eq(operation_id)))
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("delete pricing_draft_window: {e}")))?;
    if result.rows_affected == 0 {
        return Err(RepoError::NotFound {
            subject: "draft window operation".to_owned(),
            id: operation_id.to_string(),
        });
    }
    Ok(())
}

/// The draft operation that addresses `window_id`, either as its operation id
/// or as the live window it mutates.
///
/// Used by `PATCH`/`DELETE /price-windows/{windowId}` to resolve a plan when the
/// id is not yet a `pricing_price_window` row.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure;
/// [`RepoError::CorruptRow`] when a stored token is outside its CHECK set.
pub async fn find_addressing(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    window_id: Uuid,
) -> Result<Option<(DraftWindowOwner, DraftWindowEntry)>, RepoError> {
    let row = draft_window::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(draft_window::Column::TenantId.eq(tenant_id))
                .add(
                    Condition::any()
                        .add(draft_window::Column::OperationId.eq(window_id))
                        .add(draft_window::Column::TargetWindowId.eq(window_id)),
                ),
        )
        .order_by(draft_window::Column::PlanRevision, Order::Desc)
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("find pricing_draft_window: {e}")))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let plan_revision = u64::try_from(row.plan_revision).map_err(|_| {
        RepoError::CorruptRow(format!(
            "pricing_draft_window {} plan_revision {} is negative",
            row.operation_id, row.plan_revision
        ))
    })?;
    let owner = DraftWindowOwner {
        tenant_id: row.tenant_id,
        plan_id: row.plan_id,
        plan_revision,
    };
    let entry = to_entry(row)?;
    Ok(Some((owner, entry)))
}

fn owner_filter(owner: &DraftWindowOwner, number: i64) -> Condition {
    Condition::all()
        .add(draft_window::Column::TenantId.eq(owner.tenant_id))
        .add(draft_window::Column::PlanId.eq(owner.plan_id))
        .add(draft_window::Column::PlanRevision.eq(number))
}

pub(super) async fn require_draft_owner(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
) -> Result<(), RepoError> {
    let plan_id = PlanId::new(owner.plan_id);
    match load_revision(runner, scope, owner.tenant_id, plan_id, owner.plan_revision).await? {
        None => Err(not_found(plan_id, owner.plan_revision)),
        Some(row) if !row.lifecycle_state.is_content_mutable() => Err(RepoError::NotDraft {
            subject: "plan revision".to_owned(),
            id: format!("{plan_id}/{}", owner.plan_revision),
            state: row.lifecycle_state.to_string(),
        }),
        Some(_) => Ok(()),
    }
}

pub(super) async fn require_price_on_plan(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: Uuid,
    price_id: Uuid,
) -> Result<(), RepoError> {
    let missing = RepoError::NotFound {
        subject: "price".to_owned(),
        id: price_id.to_string(),
    };
    let Some(row) = price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price::Column::TenantId.eq(tenant_id))
                .add(price::Column::PriceId.eq(price_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_price for draft window: {e}")))?
    else {
        return Err(missing);
    };
    if row.plan_id != plan_id {
        return Err(missing);
    }
    Ok(())
}

pub(super) fn stored_revision(owner: &DraftWindowOwner) -> Result<i64, RepoError> {
    i64::try_from(owner.plan_revision).map_err(|_| {
        RepoError::CorruptRow(format!(
            "plan {} revision {} exceeds the storable range",
            owner.plan_id, owner.plan_revision
        ))
    })
}

fn to_model(
    owner: &DraftWindowOwner,
    number: i64,
    entry: &DraftWindowEntry,
) -> Result<draft_window::ActiveModel, RepoError> {
    let (target_window_id, action, price_id, start_kind, effective_from, effective_to) =
        match &entry.action {
            DraftWindowAction::Create {
                window_id,
                price_id,
                start,
                effective_to,
            } => {
                if entry.operation_id != *window_id {
                    return Err(RepoError::Db(
                        "create draft window operation_id must equal target_window_id".to_owned(),
                    ));
                }
                check_authored_instant("effectiveTo", *effective_to)?;
                let (start_kind, effective_from) = match start {
                    DraftStart::AtPublish => ("at_publish", None),
                    DraftStart::At(at) => {
                        check_authored_instant("effectiveFrom", Some(*at))?;
                        ("at", Some(*at))
                    }
                };
                (
                    *window_id,
                    "create",
                    Some(*price_id),
                    Some(start_kind.to_owned()),
                    effective_from,
                    *effective_to,
                )
            }
            DraftWindowAction::AdjustEnd {
                window_id,
                effective_to,
            } => {
                check_authored_instant("effectiveTo", *effective_to)?;
                (*window_id, "adjust_end", None, None, None, *effective_to)
            }
            DraftWindowAction::Cancel { window_id } => {
                (*window_id, "cancel", None, None, None, None)
            }
        };
    Ok(draft_window::ActiveModel {
        tenant_id: Set(owner.tenant_id),
        plan_id: Set(owner.plan_id),
        plan_revision: Set(number),
        operation_id: Set(entry.operation_id),
        target_window_id: Set(target_window_id),
        action: Set(action.to_owned()),
        price_id: Set(price_id),
        start_kind: Set(start_kind),
        effective_from: Set(effective_from),
        effective_to: Set(effective_to),
        reason_code: Set(entry.reason_code.clone()),
    })
}

fn to_entry(row: draft_window::Model) -> Result<DraftWindowEntry, RepoError> {
    let action = match row.action.as_str() {
        "create" => {
            let price_id = row.price_id.ok_or_else(|| {
                RepoError::CorruptRow(format!(
                    "pricing_draft_window {} create is missing price_id",
                    row.operation_id
                ))
            })?;
            let start = match row.start_kind.as_deref() {
                Some("at_publish") => DraftStart::AtPublish,
                Some("at") => {
                    let at = row.effective_from.ok_or_else(|| {
                        RepoError::CorruptRow(format!(
                            "pricing_draft_window {} start_kind=at is missing effective_from",
                            row.operation_id
                        ))
                    })?;
                    DraftStart::At(at)
                }
                other => {
                    return Err(RepoError::CorruptRow(format!(
                        "pricing_draft_window {} holds start_kind {other:?}",
                        row.operation_id
                    )));
                }
            };
            DraftWindowAction::Create {
                window_id: row.target_window_id,
                price_id,
                start,
                effective_to: row.effective_to,
            }
        }
        "adjust_end" => DraftWindowAction::AdjustEnd {
            window_id: row.target_window_id,
            effective_to: row.effective_to,
        },
        "cancel" => DraftWindowAction::Cancel {
            window_id: row.target_window_id,
        },
        other => {
            return Err(RepoError::CorruptRow(format!(
                "pricing_draft_window {} holds action {other}",
                row.operation_id
            )));
        }
    };
    Ok(DraftWindowEntry {
        operation_id: row.operation_id,
        action,
        reason_code: row.reason_code,
    })
}
