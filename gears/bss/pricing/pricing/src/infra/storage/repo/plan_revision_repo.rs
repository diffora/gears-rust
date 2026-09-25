//! Scoped plan revision persistence with conditional versions and pending ownership (D-394).
use super::{driver_failure, map_unique, matched};
use crate::domain::plan::RevisionState;
use crate::infra::storage::{RepoError, entity::plan_revision as e};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Set};
use toolkit_db::secure::{
    AccessScope, DBRunner, ScopeError, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;
fn key(tenant: Uuid, id: Uuid) -> Condition {
    Condition::all()
        .add(e::Column::TenantId.eq(tenant))
        .add(e::Column::Id.eq(id))
}
fn unlocked_draft(tenant: Uuid, id: Uuid, version: i64) -> Condition {
    key(tenant, id)
        .add(e::Column::Version.eq(version))
        .add(e::Column::State.eq(RevisionState::Draft.as_str()))
        .add(e::Column::PendingUnitId.is_null())
}
/// A unique conflict of a revision write. Postgres names the index; `SQLite` names only the
/// columns of a partial index, so its single-column `plan_id` form is whichever of the two partial
/// indexes the written state joins: the published one, or the draft-or-pending one.
fn map_revision_unique(context: &str, error: ScopeError, state: &str) -> RepoError {
    if error.is_unique_violation() {
        let message = error.to_string();
        if let Some(code) = super::unique_code(&message) {
            return RepoError::Conflict { code };
        }
        if message.contains("pricing_plan_revision.plan_id") {
            let code = if state == RevisionState::Published.as_str() {
                "REVISION_PUBLISHED_EXISTS"
            } else {
                "REVISION_DRAFT_EXISTS"
            };
            return RepoError::Conflict { code };
        }
    }
    driver_failure(context.to_owned(), error)
}
async fn book_in_tenant(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    book: Uuid,
) -> Result<(), RepoError> {
    if super::book_repo::find(runner, scope, tenant, book)
        .await?
        .is_none()
    {
        return Err(RepoError::Conflict {
            code: "BOOK_NOT_FOUND",
        });
    }
    Ok(())
}
/// Insert a revision of a tenant's plan on a tenant's book.
/// # Errors
/// `PLAN_NOT_FOUND`, `BOOK_NOT_FOUND`, `REVISION_NO_TAKEN`, `REVISION_DRAFT_EXISTS` or
/// `REVISION_PUBLISHED_EXISTS`; database failures keep their type.
pub async fn insert(
    runner: &impl DBRunner,
    scope: &AccessScope,
    m: e::Model,
) -> Result<e::Model, RepoError> {
    if super::plan_repo::find(runner, scope, m.tenant_id, m.plan_id)
        .await?
        .is_none()
    {
        return Err(RepoError::Conflict {
            code: "PLAN_NOT_FOUND",
        });
    }
    book_in_tenant(runner, scope, m.tenant_id, m.book_id).await?;
    let state = m.state.clone();
    let active = e::ActiveModel {
        id: Set(m.id),
        tenant_id: Set(m.tenant_id),
        plan_id: Set(m.plan_id),
        rev_no: Set(m.rev_no),
        book_id: Set(m.book_id),
        state: Set(m.state),
        available_from: Set(m.available_from),
        pending_unit_id: Set(m.pending_unit_id),
        approved_by_unit_id: Set(m.approved_by_unit_id),
        published_at: Set(m.published_at),
        version: Set(m.version),
        created_by: Set(m.created_by),
        created_at: Set(m.created_at),
        updated_at: Set(m.updated_at),
    };
    e::Entity::insert(active.clone())
        .secure()
        .scope_with_model(scope, &active)
        .map_err(|e| driver_failure("insert plan revision scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| map_revision_unique("insert plan revision", e, &state))
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
        .map_err(|e| driver_failure("find plan revision".into(), e))
}
/// A plan's revisions by revision number.
/// # Errors
/// Returns typed database failures.
pub async fn for_plan(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    plan_id: Uuid,
) -> Result<Vec<e::Model>, RepoError> {
    e::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(e::Column::TenantId.eq(tenant))
                .add(e::Column::PlanId.eq(plan_id)),
        )
        .order_by(e::Column::RevNo, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list plan revisions".into(), e))
}
/// Change a draft's content (book, availability) at the version the caller read, only while it is
/// an unlocked draft.
/// # Errors
/// `BOOK_NOT_FOUND` for a book outside the tenant; `STALE_REVISION` for a lost version, a locked
/// or a non-draft revision.
pub async fn update_draft(
    runner: &impl DBRunner,
    scope: &AccessScope,
    m: e::Model,
) -> Result<(), RepoError> {
    book_in_tenant(runner, scope, m.tenant_id, m.book_id).await?;
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::BookId, Expr::value(m.book_id))
        .col_expr(e::Column::AvailableFrom, Expr::value(m.available_from))
        .col_expr(e::Column::UpdatedAt, Expr::value(m.updated_at))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(unlocked_draft(m.tenant_id, m.id, m.version))
        .exec(runner)
        .await
        .map_err(|e| map_unique("update plan revision".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
}
/// Acquire pending ownership only on an unlocked draft at the observed version.
/// # Errors
/// `UNIT_NOT_FOUND` or typed database failures. A lost race returns false.
pub async fn try_lock(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    unit: Uuid,
    version: i64,
) -> Result<bool, RepoError> {
    super::unit_exists(runner, scope, tenant, unit, "plan revision unit").await?;
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::PendingUnitId, Expr::value(Some(unit)))
        .col_expr(
            e::Column::State,
            Expr::value(RevisionState::Pending.as_str()),
        )
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(unlocked_draft(tenant, id, version))
        .exec(runner)
        .await
        .map_err(|e| driver_failure("lock plan revision conditionally".into(), e))?;
    Ok(result.rows_affected == 1)
}
/// A rejected or withdrawn unit returns its revision to an editable draft.
/// # Errors
/// `STALE_REVISION` when the unit does not own the pending revision.
pub async fn unlock(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    unit: Uuid,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(e::Column::PendingUnitId, Expr::value(None::<Uuid>))
        .col_expr(e::Column::State, Expr::value(RevisionState::Draft.as_str()))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(
            key(tenant, id)
                .add(e::Column::PendingUnitId.eq(unit))
                .add(e::Column::State.eq(RevisionState::Pending.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("unlock plan revision".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
}
/// Publish the revision its unit holds; the lock turns into `approved_by_unit_id`.
/// # Errors
/// `REVISION_NOT_PENDING` when the unit does not hold it; `REVISION_PUBLISHED_EXISTS` while the
/// plan still has a published revision (supersede it first).
pub async fn publish(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    unit: Uuid,
    now: time::OffsetDateTime,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            e::Column::State,
            Expr::value(RevisionState::Published.as_str()),
        )
        .col_expr(e::Column::PendingUnitId, Expr::value(None::<Uuid>))
        .col_expr(e::Column::ApprovedByUnitId, Expr::value(Some(unit)))
        .col_expr(e::Column::PublishedAt, Expr::value(Some(now)))
        .col_expr(e::Column::UpdatedAt, Expr::value(now))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(
            key(tenant, id)
                .add(e::Column::PendingUnitId.eq(unit))
                .add(e::Column::State.eq(RevisionState::Pending.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            map_revision_unique(
                "publish plan revision",
                e,
                RevisionState::Published.as_str(),
            )
        })?;
    matched(result.rows_affected, "REVISION_NOT_PENDING")
}
/// Supersede a published revision at the version the caller read.
/// # Errors
/// A concurrent change or a revision that is not published is `STALE_REVISION`.
pub async fn supersede(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
    now: time::OffsetDateTime,
) -> Result<(), RepoError> {
    let result = e::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            e::Column::State,
            Expr::value(RevisionState::Superseded.as_str()),
        )
        .col_expr(e::Column::UpdatedAt, Expr::value(now))
        .col_expr(e::Column::Version, Expr::col(e::Column::Version).add(1_i64))
        .filter(
            key(tenant, id)
                .add(e::Column::Version.eq(version))
                .add(e::Column::State.eq(RevisionState::Published.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("supersede plan revision".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
}
/// Delete an unlocked draft that has no items left, at its observed version; the caller deletes
/// the items first, with their delete ops (D-414).
/// # Errors
/// A lost version, a locked or non-draft revision, or remaining items are `STALE_REVISION`.
pub async fn delete_draft(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    version: i64,
) -> Result<(), RepoError> {
    use crate::infra::storage::entity::plan_item;
    use toolkit_db::secure::SecureDeleteExt;
    let items = sea_orm::sea_query::Query::select()
        .expr(Expr::val(1))
        .from(plan_item::Entity)
        .and_where(plan_item::Column::TenantId.eq(tenant))
        .and_where(plan_item::Column::RevisionId.eq(id))
        .to_owned();
    let result = e::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(unlocked_draft(tenant, id, version).add(Expr::exists(items).not()))
        .exec(runner)
        .await
        .map_err(|e| driver_failure("delete draft plan revision".into(), e))?;
    matched(result.rows_affected, "STALE_REVISION")
}
