//! SKU heads, conditional locks and local reference fences.
#![allow(
    clippy::too_many_arguments,
    reason = "Repository commands keep the scoped key and compare-and-swap operands explicit"
)]
use super::{HeadWrite, category_repo::require_active_category, driver_failure, map_unique};
use crate::domain::sku::NewSku;
use crate::infra::storage::{
    RepoError,
    entity::{sku, sku_reference},
};
use bss_products_sdk::models::{BillingTiming, Lifecycle, Sku, SkuContent, SkuType};
use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Set};
use time::OffsetDateTime;
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

fn key(tenant: Uuid, id: Uuid) -> Condition {
    Condition::all()
        .add(sku::Column::TenantId.eq(tenant))
        .add(sku::Column::Id.eq(id))
}
fn billing_token(b: BillingTiming) -> &'static str {
    match b {
        BillingTiming::Advance => "advance",
        BillingTiming::Arrears => "arrears",
    }
}
fn sku_of(m: sku::Model) -> Result<Sku, RepoError> {
    Ok(Sku {
        id: m.id,
        tenant_id: m.tenant_id,
        code: m.code,
        name: m.name,
        r#type: SkuType::parse(&m.r#type)
            .ok_or_else(|| RepoError::CorruptRow(format!("SKU type {}", m.r#type)))?,
        category_id: m.category_id,
        description: m.description,
        sellable: m.sellable,
        lifecycle: Lifecycle::parse(&m.lifecycle)
            .ok_or_else(|| RepoError::CorruptRow(format!("SKU lifecycle {}", m.lifecycle)))?,
        revision: m.revision,
        published_version: m.published_version,
        gl_code: m.gl_code,
        tax_category: m.tax_category,
        invoice_line_template: m.invoice_line_template,
        billing_timing: m
            .billing_timing
            .map(|v| match v.as_str() {
                "advance" => Ok(BillingTiming::Advance),
                "arrears" => Ok(BillingTiming::Arrears),
                _ => Err(RepoError::CorruptRow(format!("billing timing {v}"))),
            })
            .transpose()?,
        usage_type_ref: m.usage_type_ref,
        unit: m.unit,
        type_change_pending: m.type_change_pending,
        pending_unit_id: m.pending_unit_id,
        approved_by_unit_id: m.approved_by_unit_id,
        created_by: m.created_by,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}
/// Insert a draft under an active category in the caller's serializable transaction.
/// # Errors
/// Returns unique-code/name, category, or scoped storage errors.
pub async fn insert_sku(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    new: NewSku,
    created_by: Uuid,
    now: OffsetDateTime,
) -> Result<Sku, RepoError> {
    require_active_category(runner, scope, tenant_id, new.category_id).await?;
    let model = sku::ActiveModel {
        id: Set(Uuid::new_v4()),
        tenant_id: Set(tenant_id),
        code: Set(new.code),
        name: Set(new.name),
        r#type: Set(new.r#type.as_str().into()),
        category_id: Set(new.category_id),
        description: Set(new.description),
        sellable: Set(new.sellable),
        lifecycle: Set("draft".into()),
        fence_prior_lifecycle: Set(None),
        fenced_at: Set(None),
        fence_op_id: Set(None),
        revision: Set(1),
        published_version: Set(0),
        gl_code: Set(new.gl_code),
        tax_category: Set(new.tax_category),
        invoice_line_template: Set(new.invoice_line_template),
        billing_timing: Set(new.billing_timing.map(|v| billing_token(v).to_owned())),
        usage_type_ref: Set(new.usage_type_ref),
        unit: Set(new.unit),
        type_change_pending: Set(false),
        pending_unit_id: Set(None),
        approved_by_unit_id: Set(None),
        created_by: Set(created_by),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let row = sku::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure("SKU scope".into(), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| map_unique("insert SKU".into(), e))?;
    sku_of(row)
}
/// Find the tenant's visible SKU.
/// # Errors
/// Returns scoped storage or corrupt-row errors.
pub async fn find_sku(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<Option<Sku>, RepoError> {
    sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(key(tenant_id, id))
        .one(runner)
        .await
        .map_err(|e| driver_failure("find SKU".into(), e))?
        .map(sku_of)
        .transpose()
}
/// Filters and an exclusive code cursor; one extra row signals another page.
#[derive(Debug, Clone)]
pub struct SkuQuery {
    /// Additional validated catalog predicate, composed inside the tenant scope.
    pub catalog_filter: Option<Condition>,
    pub text: Option<String>,
    pub r#type: Option<SkuType>,
    pub category_id: Option<Uuid>,
    pub lifecycle: Option<Lifecycle>,
    pub limit: u64,
    pub after_code: Option<String>,
}
/// List matching SKUs in stable code order.
/// # Errors
/// Returns scoped storage or corrupt-row errors.
pub async fn list_skus(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    q: &SkuQuery,
) -> Result<Vec<Sku>, RepoError> {
    let mut c = Condition::all().add(sku::Column::TenantId.eq(tenant_id));
    if let Some(filter) = &q.catalog_filter {
        c = c.add(filter.clone());
    }
    if let Some(v) = &q.text {
        c = c.add(
            Condition::any()
                .add(sku::Column::Code.contains(v))
                .add(sku::Column::Name.contains(v)),
        );
    }
    if let Some(v) = q.r#type {
        c = c.add(sku::Column::Type.eq(v.as_str()));
    }
    if let Some(v) = q.category_id {
        c = c.add(sku::Column::CategoryId.eq(v));
    }
    if let Some(v) = q.lifecycle {
        c = c.add(sku::Column::Lifecycle.eq(v.as_str()));
    }
    if let Some(v) = &q.after_code {
        c = c.add(sku::Column::Code.gt(v));
    }
    sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(c)
        .order_by(sku::Column::Code, Order::Asc)
        .limit(q.limit.saturating_add(1))
        .all(runner)
        .await
        .map_err(|e| driver_failure("list SKUs".into(), e))?
        .into_iter()
        .map(sku_of)
        .collect()
}
fn content_update(
    scope: &AccessScope,
    c: &SkuContent,
    now: OffsetDateTime,
) -> toolkit_db::secure::SecureUpdateMany<sku::Entity, toolkit_db::secure::Scoped> {
    sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(sku::Column::Code, Expr::value(c.code.clone()))
        .col_expr(sku::Column::Name, Expr::value(c.name.clone()))
        .col_expr(sku::Column::Type, Expr::value(c.r#type.as_str()))
        .col_expr(sku::Column::CategoryId, Expr::value(c.category_id))
        .col_expr(sku::Column::Description, Expr::value(c.description.clone()))
        .col_expr(sku::Column::Sellable, Expr::value(c.sellable))
        .col_expr(sku::Column::GlCode, Expr::value(c.gl_code.clone()))
        .col_expr(
            sku::Column::TaxCategory,
            Expr::value(c.tax_category.clone()),
        )
        .col_expr(
            sku::Column::InvoiceLineTemplate,
            Expr::value(c.invoice_line_template.clone()),
        )
        .col_expr(
            sku::Column::BillingTiming,
            Expr::value(c.billing_timing.map(billing_token)),
        )
        .col_expr(
            sku::Column::UsageTypeRef,
            Expr::value(c.usage_type_ref.clone()),
        )
        .col_expr(sku::Column::Unit, Expr::value(c.unit.clone()))
        .col_expr(
            sku::Column::Revision,
            Expr::col(sku::Column::Revision).add(1_i64),
        )
        .col_expr(sku::Column::UpdatedAt, Expr::value(now))
}
async fn written(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    affected: u64,
) -> Result<HeadWrite<Sku>, RepoError> {
    if affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    find_sku(runner, scope, tenant, id)
        .await?
        .map(HeadWrite::Written)
        .ok_or_else(|| RepoError::CorruptRow("written SKU disappeared".into()))
}
/// Write content only to an unlocked draft at the observed revision.
/// # Errors
/// Returns category, unique-key or scoped storage errors.
pub async fn update_sku_draft(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    expected_revision: i64,
    content: &SkuContent,
    now: OffsetDateTime,
) -> Result<HeadWrite<Sku>, RepoError> {
    require_active_category(runner, scope, tenant_id, content.category_id).await?;
    let r = content_update(scope, content, now)
        .filter(
            key(tenant_id, id)
                .add(sku::Column::Lifecycle.eq("draft"))
                .add(sku::Column::Revision.eq(expected_revision))
                .add(sku::Column::PendingUnitId.is_null()),
        )
        .exec(runner)
        .await
        .map_err(|e| map_unique("update draft".into(), e))?;
    written(runner, scope, tenant_id, id, r.rows_affected).await
}
/// Apply approved business content, incrementing revision and published version.
/// # Errors
/// Returns category, unique-key or scoped storage errors; a missing head is corrupt.
pub async fn write_sku_content(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    content: &SkuContent,
    now: OffsetDateTime,
) -> Result<Sku, RepoError> {
    require_active_category(runner, scope, tenant_id, content.category_id).await?;
    let r = content_update(scope, content, now)
        .col_expr(
            sku::Column::PublishedVersion,
            Expr::col(sku::Column::PublishedVersion).add(1_i64),
        )
        .filter(key(tenant_id, id))
        .exec(runner)
        .await
        .map_err(|e| map_unique("apply SKU content".into(), e))?;
    match written(runner, scope, tenant_id, id, r.rows_affected).await? {
        HeadWrite::Written(s) => Ok(s),
        HeadWrite::Unmatched => Err(RepoError::CorruptRow("approval SKU missing".into())),
    }
}
/// Transition an approval-owned head only from the supplied states.
/// # Errors
/// Returns scoped storage failures.
pub async fn set_lifecycle(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    from: &[Lifecycle],
    to: Lifecycle,
    now: OffsetDateTime,
) -> Result<HeadWrite<Sku>, RepoError> {
    let r = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(sku::Column::Lifecycle, Expr::value(to.as_str()))
        .col_expr(sku::Column::UpdatedAt, Expr::value(now))
        .col_expr(
            sku::Column::Revision,
            Expr::col(sku::Column::Revision).add(1_i64),
        )
        .filter(
            key(tenant_id, id).add(sku::Column::Lifecycle.is_in(from.iter().map(|v| v.as_str()))),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("set SKU lifecycle".into(), e))?;
    written(runner, scope, tenant_id, id, r.rows_affected).await
}
/// The two operations that exclude every live local reference.
#[derive(Debug, Clone, Copy)]
pub enum Fence {
    Retire,
    TypeChange,
}
/// Fence and check live references in one write; the caller uses serializable isolation.
/// # Errors
/// Returns scoped storage failures.
pub async fn fence_sku(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    kind: Fence,
    op_id: Uuid,
    now: OffsetDateTime,
) -> Result<HeadWrite<Sku>, RepoError> {
    let live = Query::select()
        .expr(Expr::val(1))
        .from(sku_reference::Entity)
        .and_where(sku_reference::Column::TenantId.eq(tenant_id))
        .and_where(sku_reference::Column::SkuId.eq(id))
        .and_where(sku_reference::Column::State.ne("released"))
        .to_owned();
    let mut q = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(sku::Column::FencedAt, Expr::value(now))
        .col_expr(sku::Column::FenceOpId, Expr::value(op_id));
    q = match kind {
        Fence::Retire => q
            .col_expr(
                sku::Column::FencePriorLifecycle,
                Expr::col(sku::Column::Lifecycle),
            )
            .col_expr(sku::Column::Lifecycle, Expr::value("retiring")),
        Fence::TypeChange => q.col_expr(sku::Column::TypeChangePending, Expr::value(true)),
    };
    let r = q
        .filter(
            key(tenant_id, id)
                .add(sku::Column::Lifecycle.is_in(["published", "deprecated"]))
                .add(sku::Column::PendingUnitId.is_null())
                .add(sku::Column::FencedAt.is_null())
                .add(sku::Column::TypeChangePending.eq(false))
                .add(Expr::exists(live).not()),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("fence SKU".into(), e))?;
    written(runner, scope, tenant_id, id, r.rows_affected).await
}
fn clear_fence(
    scope: &AccessScope,
    retired: bool,
) -> toolkit_db::secure::SecureUpdateMany<sku::Entity, toolkit_db::secure::Scoped> {
    let lifecycle = Expr::case(
        sku::Column::FencePriorLifecycle.is_not_null(),
        if retired {
            Expr::value("retired")
        } else {
            Expr::col(sku::Column::FencePriorLifecycle)
        },
    )
    .finally(Expr::col(sku::Column::Lifecycle));
    sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(sku::Column::Lifecycle, lifecycle.into())
        .col_expr(sku::Column::TypeChangePending, Expr::value(false))
        .col_expr(sku::Column::FencedAt, Expr::value(None::<OffsetDateTime>))
        .col_expr(sku::Column::FenceOpId, Expr::value(None::<Uuid>))
        .col_expr(
            sku::Column::FencePriorLifecycle,
            Expr::value(None::<String>),
        )
}
/// Operator/TTL release cannot touch a fence held by a pending unit.
/// # Errors
/// Returns scoped storage failures.
pub async fn unfence_sku(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    op_id: Option<Uuid>,
) -> Result<HeadWrite<Sku>, RepoError> {
    let mut c = key(tenant_id, id).add(sku::Column::PendingUnitId.is_null());
    if let Some(op) = op_id {
        c = c.add(sku::Column::FenceOpId.eq(op));
    }
    let r = clear_fence(scope, false)
        .filter(c)
        .exec(runner)
        .await
        .map_err(|e| driver_failure("unfence SKU".into(), e))?;
    written(runner, scope, tenant_id, id, r.rows_affected).await
}
/// Release the subject's lock and matching fence atomically.
/// # Errors
/// Returns scoped storage failures.
pub async fn unlock_and_unfence(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    unit_id: Uuid,
    op_id: Uuid,
    approved_by: Option<Uuid>,
    retired: bool,
) -> Result<HeadWrite<Sku>, RepoError> {
    let mut q =
        clear_fence(scope, retired).col_expr(sku::Column::PendingUnitId, Expr::value(None::<Uuid>));
    if let Some(approved_by) = approved_by {
        q = q.col_expr(sku::Column::ApprovedByUnitId, Expr::value(approved_by));
    }
    let r = q
        .filter(
            key(tenant_id, id)
                .add(sku::Column::PendingUnitId.eq(unit_id))
                .add(sku::Column::FenceOpId.eq(op_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("unlock and unfence SKU".into(), e))?;
    written(runner, scope, tenant_id, id, r.rows_affected).await
}
/// Acquire a pending-unit lock only on the observed, unlocked revision.
/// # Errors
/// Returns scoped storage failures.
pub async fn try_lock_sku(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    unit_id: Uuid,
    expected_revision: i64,
) -> Result<bool, RepoError> {
    let r = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(sku::Column::PendingUnitId, Expr::value(unit_id))
        .filter(
            key(tenant_id, id)
                .add(sku::Column::PendingUnitId.is_null())
                .add(sku::Column::Revision.eq(expected_revision)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("lock SKU".into(), e))?;
    Ok(r.rows_affected == 1)
}
/// Release a unit lock, preserving prior approval attribution unless replaced.
/// # Errors
/// Returns scoped storage failures.
pub async fn unlock_sku(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
    approved_by: Option<Uuid>,
) -> Result<(), RepoError> {
    let mut q = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(sku::Column::PendingUnitId, Expr::value(None::<Uuid>));
    if let Some(v) = approved_by {
        q = q.col_expr(sku::Column::ApprovedByUnitId, Expr::value(v));
    }
    q.filter(key(tenant_id, id))
        .exec(runner)
        .await
        .map_err(|e| driver_failure("unlock SKU".into(), e))?;
    Ok(())
}
/// Count all heads that keep a category in use, including retired heads.
/// # Errors
/// Returns scoped storage failures.
pub async fn count_skus_in_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
) -> Result<u64, RepoError> {
    sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::CategoryId.eq(category_id)),
        )
        .count(runner)
        .await
        .map_err(|e| driver_failure("count category SKUs".into(), e))
}
#[cfg(test)]
#[path = "sku_repo_tests.rs"]
mod sku_repo_tests;

/// Read private fence ownership without putting it in business snapshots.
/// # Errors
/// Returns scoped storage failures.
pub async fn find_sku_fence(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<Option<sku::Model>, RepoError> {
    sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(key(tenant_id, id))
        .one(runner)
        .await
        .map_err(|e| driver_failure("find SKU fence".into(), e))
}

/// Recover tenant-scoped orphan fences before applying list filters.
/// Pending units cannot be released, even when their fences are old.
/// # Errors
/// Returns scoped storage failures.
pub async fn expire_orphan_fences(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    cutoff: OffsetDateTime,
) -> Result<(), RepoError> {
    clear_fence(scope, false)
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant))
                .add(sku::Column::PendingUnitId.is_null())
                .add(sku::Column::FencedAt.lte(cutoff)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure("expire orphan SKU fences".into(), e))?;
    Ok(())
}
