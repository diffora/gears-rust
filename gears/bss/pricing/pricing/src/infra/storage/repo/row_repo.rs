//! Scoped row persistence with conditional versions.
use super::{driver_failure, map_unique, matched};
use crate::infra::storage::{RepoError, entity::price_row as e};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Set};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;
fn key(tenant: Uuid, id: Uuid) -> Condition {
    Condition::all()
        .add(e::Column::TenantId.eq(tenant))
        .add(e::Column::Id.eq(id))
}
/// Insert a tenant-scoped row in the caller's transaction.
/// # Errors
/// Returns unique conflicts, parent ownership refusals or typed database failures.
pub async fn insert(
    runner: &impl DBRunner,
    scope: &AccessScope,
    m: e::Model,
) -> Result<e::Model, RepoError> {
    let price = super::price_repo::find(runner, scope, m.tenant_id, m.price_id)
        .await?
        .ok_or(RepoError::Conflict {
            code: "PRICE_NOT_FOUND",
        })?;
    if price.reference_state == "lost" {
        return Err(RepoError::Conflict {
            code: "PRICE_REFERENCE_LOST",
        });
    }
    let active = e::ActiveModel {
        id: Set(m.id),
        tenant_id: Set(m.tenant_id),
        price_id: Set(m.price_id),
        version_no: Set(m.version_no),
        dim_value: Set(m.dim_value),
        model: Set(m.model),
        price_json: Set(m.price_json),
        min_fee: Set(m.min_fee),
        eligibility: Set(m.eligibility),
        effective_from: Set(m.effective_from),
        effective_to: Set(m.effective_to),
        keep_for_bound: Set(m.keep_for_bound),
        closed_explicitly: Set(m.closed_explicitly),
        temporary_until: Set(m.temporary_until),
        paired_row_id: Set(m.paired_row_id),
        return_of_row_id: Set(m.return_of_row_id),
        state: Set(m.state),
        pending_unit_id: Set(m.pending_unit_id),
        approved_by_unit_id: Set(m.approved_by_unit_id),
        note: Set(m.note),
        created_by: Set(m.created_by),
        approved_at: Set(m.approved_at),
        version: Set(m.version),
        created_at: Set(m.created_at),
        updated_at: Set(m.updated_at),
    };
    e::Entity::insert(active.clone())
        .secure()
        .scope_with_model(scope, &active)
        .map_err(|e| driver_failure("insert scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| map_unique("insert price_row".into(), e))
}
/// Read by tenant and identity within the authorized scope.
/// # Errors
/// Returns typed database failures.
pub async fn find(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Option<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(key(tenant, id))
        .one(runner)
        .await
        .map_err(|e| driver_failure("find price_row".into(), e))
}
/// List tenant rows in stable identity order.
/// # Errors
/// Returns typed database failures.
pub async fn list(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
) -> Result<Vec<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(e::Column::TenantId.eq(tenant)))
        .order_by(e::Column::Id, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list price_row".into(), e))
}
/// Change business columns only if the caller's version still owns the row.
/// # Errors
/// Zero matches is a typed version conflict; database failures preserve their type.
pub async fn update_draft(
    runner: &impl DBRunner,
    scope: &AccessScope,
    m: e::Model,
) -> Result<(), RepoError> {
    let predicate = key(m.tenant_id, m.id).add(e::Column::Version.eq(m.version));
    let predicate = predicate
        .add(e::Column::State.eq("draft"))
        .add(e::Column::PendingUnitId.is_null());
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::DimValue, Expr::value(m.dim_value))
        .col_expr(e::Column::Model, Expr::value(m.model))
        .col_expr(e::Column::PriceJson, Expr::value(m.price_json))
        .col_expr(e::Column::MinFee, Expr::value(m.min_fee))
        .col_expr(e::Column::Eligibility, Expr::value(m.eligibility))
        .col_expr(e::Column::EffectiveFrom, Expr::value(m.effective_from))
        .col_expr(e::Column::EffectiveTo, Expr::value(m.effective_to))
        .col_expr(
            e::Column::ClosedExplicitly,
            Expr::value(m.closed_explicitly),
        )
        .col_expr(e::Column::TemporaryUntil, Expr::value(m.temporary_until))
        .col_expr(e::Column::PairedRowId, Expr::value(m.paired_row_id))
        .col_expr(e::Column::ReturnOfRowId, Expr::value(m.return_of_row_id))
        .col_expr(e::Column::Note, Expr::value(m.note))
        .col_expr(e::Column::UpdatedAt, Expr::value(m.updated_at))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(predicate)
        .exec(runner)
        .await
        .map_err(|e| map_unique("update price_row".into(), e))?;
    matched(result.rows_affected, "VERSION_CONFLICT")
}
/// List rows of one scoped parent in stable order.
/// # Errors
/// Returns typed database failures.
pub async fn for_price(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    parent: Uuid,
) -> Result<Vec<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::TenantId.eq(tenant))
                .add(e::Column::PriceId.eq(parent)),
        )
        .order_by(e::Column::VersionNo, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list parent rows".into(), e))
}
/// Remove a draft only at its current version and outside an approval unit.
/// # Errors
/// Refuses stale versions, non-drafts and pending ownership.
pub async fn delete_draft(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
) -> Result<(), RepoError> {
    use toolkit_db::secure::SecureDeleteExt;
    let result = e::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            key(tenant, id)
                .add(e::Column::Version.eq(version))
                .add(e::Column::State.eq("draft"))
                .add(e::Column::PendingUnitId.is_null()),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("delete draft".into(), e))?;
    matched(result.rows_affected, "VERSION_CONFLICT")
}
/// Decode a stored row into the pure model; unknown vocabulary is a corrupt row.
/// # Errors
/// Returns `CorruptRow` for a stored enum or price shape the model does not know.
pub fn to_domain(m: &e::Model) -> Result<crate::domain::row::Row, RepoError> {
    use crate::domain::{money, price::Model, row};
    let corrupt = |what: &str| RepoError::CorruptRow(format!("price_row {} {what}", m.id));
    let model: Model = m.model.parse().map_err(|_| corrupt("model"))?;
    Ok(row::Row {
        id: m.id,
        price_id: m.price_id,
        version_no: m.version_no,
        dim_value: m.dim_value.clone(),
        model,
        price: Some(money::decode(model, m.price_json.clone()).map_err(|_| corrupt("price_json"))?),
        min_fee: m.min_fee,
        eligibility: m.eligibility.parse().map_err(|_| corrupt("eligibility"))?,
        effective_from: m.effective_from,
        effective_to: m.effective_to,
        temporary_until: m.temporary_until,
        paired_row_id: m.paired_row_id,
        return_of_row_id: m.return_of_row_id,
        closed_explicitly: m.closed_explicitly,
        state: m.state.parse().map_err(|_| corrupt("state"))?,
    })
}
/// Point a freshly inserted draft at its pair partner, without a version step.
/// The partner must exist first: the pair reference is a foreign key.
/// # Errors
/// Refuses anything but an unlinked, unlocked draft; preserves database failures.
pub async fn link_pair(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    partner: Uuid,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::PairedRowId, Expr::value(Some(partner)))
        .filter(
            key(tenant, id)
                .add(e::Column::State.eq("draft"))
                .add(e::Column::PendingUnitId.is_null())
                .add(e::Column::PairedRowId.is_null()),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("link pair".into(), e))?;
    matched(result.rows_affected, "VERSION_CONFLICT")
}
/// Delete unlocked drafts, each at its observed version, in ONE statement so a
/// pair's mutual references never dangle between two deletes.
/// # Errors
/// Refuses when any row moved or is no longer an unlocked draft.
pub async fn delete_drafts(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    rows: &[(Uuid, i64)],
) -> Result<(), RepoError> {
    use toolkit_db::secure::SecureDeleteExt;
    if rows.is_empty() {
        return Ok(());
    }
    let mut any = Condition::any();
    for (id, version) in rows {
        any = any.add(
            Condition::all()
                .add(e::Column::Id.eq(*id))
                .add(e::Column::Version.eq(*version)),
        );
    }
    let result = e::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::TenantId.eq(tenant))
                .add(any)
                .add(e::Column::State.eq("draft"))
                .add(e::Column::PendingUnitId.is_null()),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("delete drafts".into(), e))?;
    if usize::try_from(result.rows_affected).ok() == Some(rows.len()) {
        Ok(())
    } else {
        Err(RepoError::Conflict {
            code: "VERSION_CONFLICT",
        })
    }
}
/// Run price-row apply work under serializable isolation, retrying driver contention.
/// The approval subject and doors use this boundary when they arrive in Task 2c.7.
/// # Errors
/// Returns the final business refusal or original database failure after bounded retries.
pub async fn transaction<T: Send + 'static>(
    db: &toolkit_db::Db,
    work: impl for<'a> FnMut(
        &'a toolkit_db::DbTx<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, RepoError>> + Send + 'a>,
    > + Send,
) -> Result<T, RepoError> {
    db.transaction_with_retry(
        toolkit_db::secure::TxConfig::serializable(),
        |error| match error {
            RepoError::Driver { source, .. } => Some(source),
            _ => None,
        },
        work,
    )
    .await
}
/// Acquire pending ownership only on an unlocked draft at the observed version.
/// # Errors
/// Returns missing-unit or typed database failures. A lost race returns false.
pub async fn try_lock(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    unit: Uuid,
    version: i64,
) -> Result<bool, RepoError> {
    let parent = super::approval_repo::find_unit(runner, scope, tenant, unit)
        .await
        .map_err(|error| {
            error.db_err().map_or_else(
                || RepoError::Db(error.to_string()),
                |source| RepoError::Driver {
                    context: "row unit".into(),
                    source: source.clone(),
                },
            )
        })?;
    if parent.is_none() {
        return Err(RepoError::Conflict {
            code: "UNIT_NOT_FOUND",
        });
    }
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::PendingUnitId, Expr::value(Some(unit)))
        .col_expr(e::Column::State, Expr::value("pending"))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(
            key(tenant, id)
                .add(e::Column::Version.eq(version))
                .add(e::Column::State.eq("draft"))
                .add(e::Column::PendingUnitId.is_null()),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("lock row conditionally".into(), e))?;
    Ok(result.rows_affected == 1)
}
/// What closing a unit leaves on each of its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unlock {
    /// Apply already approved the row; the lock turns into `approved_by_unit_id`.
    Approved,
    /// Withdrawn: the row is an editable draft again.
    Draft,
    /// Rejected: the row keeps its review history and stays rejected.
    Rejected,
}
/// Release only the owning unit's row; approved rows retain the unit identity.
/// # Errors
/// Returns a conditional conflict or typed database failure.
pub async fn unlock(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    unit: Uuid,
    outcome: Unlock,
) -> Result<(), RepoError> {
    let (state, from) = match outcome {
        Unlock::Approved => ("approved", "approved"),
        Unlock::Draft => ("draft", "pending"),
        Unlock::Rejected => ("rejected", "pending"),
    };
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::PendingUnitId, Expr::value(None::<Uuid>))
        .col_expr(
            e::Column::ApprovedByUnitId,
            Expr::value((outcome == Unlock::Approved).then_some(unit)),
        )
        .col_expr(e::Column::State, Expr::value(state))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(
            key(tenant, id)
                .add(e::Column::PendingUnitId.eq(unit))
                .add(e::Column::State.eq(from)),
        )
        .exec(runner)
        .await
        .map_err(|e| map_unique("unlock row".into(), e))?;
    matched(result.rows_affected, "VERSION_CONFLICT")
}
/// The window an applied row takes, after the unit's shift and the chain's normalisation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Approval {
    pub effective_from: time::Date,
    pub effective_to: Option<time::Date>,
    pub temporary_until: Option<time::Date>,
    pub keep_for_bound: bool,
}
/// Approve a row the unit holds; the approved-start index arbitrates a racing chain.
/// # Errors
/// `WINDOW_OVERLAP` when the chain already has an approved row on that start,
/// `ROW_NOT_PENDING` when the unit does not hold the row.
pub async fn approve(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    unit: Uuid,
    window: Approval,
    now: time::OffsetDateTime,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::State, Expr::value("approved"))
        .col_expr(e::Column::EffectiveFrom, Expr::value(window.effective_from))
        .col_expr(e::Column::EffectiveTo, Expr::value(window.effective_to))
        .col_expr(
            e::Column::TemporaryUntil,
            Expr::value(window.temporary_until),
        )
        .col_expr(e::Column::KeepForBound, Expr::value(window.keep_for_bound))
        .col_expr(e::Column::ApprovedAt, Expr::value(Some(now)))
        .col_expr(e::Column::UpdatedAt, Expr::value(now))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(
            key(tenant, id)
                .add(e::Column::PendingUnitId.eq(unit))
                .add(e::Column::State.eq("pending")),
        )
        .exec(runner)
        .await
        .map_err(|e| map_unique("approve row".into(), e))?;
    matched(result.rows_affected, "ROW_NOT_PENDING")
}
/// Re-close an approved row after its chain changed, at the version the caller read.
/// # Errors
/// A concurrent change is `VERSION_CONFLICT`; database failures keep their type.
#[allow(
    clippy::too_many_arguments,
    reason = "tenant identity, version and the two recomputed columns are the write's operands"
)]
pub async fn set_window(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
    effective_to: Option<time::Date>,
    keep_for_bound: bool,
    now: time::OffsetDateTime,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::EffectiveTo, Expr::value(effective_to))
        .col_expr(e::Column::KeepForBound, Expr::value(keep_for_bound))
        .col_expr(e::Column::UpdatedAt, Expr::value(now))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(
            key(tenant, id)
                .add(e::Column::Version.eq(version))
                .add(e::Column::State.eq("approved")),
        )
        .exec(runner)
        .await
        .map_err(|e| map_unique("re-close row".into(), e))?;
    matched(result.rows_affected, "VERSION_CONFLICT")
}
