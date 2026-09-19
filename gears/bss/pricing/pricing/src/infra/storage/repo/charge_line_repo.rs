//! Charge-line and line-version persistence: logical identity, revision-owned
//! shared structure, and shared tier geometry.
//!
//! Monetary versions live on [`super::price_repo`]. Market identity lives on
//! [`super::market_price_repo`].

use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, ExprTrait};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use crate::domain::lifecycle::LifecycleState;
use crate::domain::price_record::PriceContent;
use crate::domain::price_row::{BandTop, model_kind_wire};
use crate::domain::scope_key::{ChargeLineScopeKey, PlanId};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{charge_line, charge_line_version, charge_tier};
use crate::infra::storage::repo::price_repo::{allowance_json, stored_bound, stored_count};

/// Namespace for deterministic charge-line ids (stable across retries).
const LINE_NS: Uuid = Uuid::from_u128(0x7c_11_e9_a0_9c_0f_4b_21_a1_e0_c4_12_f0_01_00_01);
const VERSION_NS: Uuid = Uuid::from_u128(0x7c_11_e9_a0_9c_0f_4b_21_a1_e0_c4_12_f0_01_00_02);

/// The graph a price write binds to.
#[derive(Clone, Debug)]
pub struct ChargeGraph {
    /// Stable logical line.
    pub charge_line_id: Uuid,
    /// Structure version this money references.
    pub line_version_id: Uuid,
    /// Containing plan (copied onto `pricing_price` under a compound FK).
    pub plan_id: Uuid,
    /// Plan revision that owns the draft structure.
    pub plan_revision: i64,
}

/// Find or insert the logical line, its draft version, and shared geometry.
///
/// Shared content is taken from `content` only when the version is created or
/// is still a draft. A published version is an immutable reference.
pub async fn ensure_draft_graph(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    key: &ChargeLineScopeKey,
    content: &PriceContent,
    created_by: Uuid,
    created_at_utc: time::OffsetDateTime,
) -> Result<ChargeGraph, RepoError> {
    let plan_id = key.plan_id().get();
    let plan_revision = open_plan_revision(runner, scope, tenant_id, plan_id)
        .await?
        .unwrap_or(0);
    let charge_line_id = find_or_insert_line(runner, scope, tenant_id, key).await?;
    let line_version_id = find_or_insert_version(
        runner,
        scope,
        tenant_id,
        charge_line_id,
        plan_revision,
        content,
        created_by,
        created_at_utc,
    )
    .await?;
    // **The submitted shared content lands on a draft version, even when the
    // version was already there.** `find_or_insert_version` returns an existing
    // version untouched, which is right for identity and wrong for content: a
    // caller authoring a second market, superseding a row, or committing a bulk
    // batch submits a whole `PriceContent`, and dropping its shared half means
    // the column CHECKs never see it. Measured three ways before this line
    // existed - a `PATCH` could not clear `invoice_line_template`, a cutover's
    // successor silently kept the predecessor's proration contract, and a bulk
    // row carrying `billing_timing = 'whenever'` committed instead of being
    // refused by `chk_pricing_charge_line_version_billing_timing`.
    //
    // A **published** version is not touched: `update_draft_structure` checks the
    // lifecycle itself and returns without writing, which is the freeze the
    // append-only guard would otherwise turn into a 500.
    update_draft_structure(runner, scope, tenant_id, line_version_id, content).await?;
    // **No plan-revision bump here.** Ensuring the graph is part of authoring a
    // *price row*, and a price row's concurrency token is its own `row_version`,
    // not the plan's — that is the contract at every door today, and the design
    // keeps it: "existing partial-draft authoring, optimistic concurrency, tenant
    // authorization and idempotency contracts remain in force" (§7). Bumping the
    // plan here would make every caller holding a plan ETag across a price create
    // get a `409`, which is a public break this change was not asked to make; it
    // also broke sixty fixtures that carry a captured tag across exactly that
    // sequence. The door that authors the **line itself** takes the plan tag as
    // its precondition and bumps it, because that door's subject really is the
    // plan's content.
    Ok(ChargeGraph {
        charge_line_id,
        line_version_id,
        plan_id,
        plan_revision,
    })
}

/// Load a version that must already exist (exact-reference door).
pub async fn require_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    line_version_id: Uuid,
) -> Result<charge_line_version::Model, RepoError> {
    charge_line_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_line_version::Column::TenantId.eq(tenant_id))
                .add(charge_line_version::Column::LineVersionId.eq(line_version_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_charge_line_version: {e}")))?
        .ok_or_else(|| RepoError::NotFound {
            subject: "charge line version".to_owned(),
            id: line_version_id.to_string(),
        })
}

/// Load the logical line.
pub async fn require_line(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    charge_line_id: Uuid,
) -> Result<charge_line::Model, RepoError> {
    charge_line::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_line::Column::TenantId.eq(tenant_id))
                .add(charge_line::Column::ChargeLineId.eq(charge_line_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_charge_line: {e}")))?
        .ok_or_else(|| RepoError::NotFound {
            subject: "charge line".to_owned(),
            id: charge_line_id.to_string(),
        })
}

/// Load the logical line of a canonical charge-line key, if it exists.
pub async fn find_by_scope(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    key: &ChargeLineScopeKey,
) -> Result<Option<charge_line::Model>, RepoError> {
    charge_line::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(line_scope_filter(tenant_id, key))
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_charge_line by scope: {e}")))
}

/// Write shared structure onto a still-draft version and replace its geometry.
pub async fn update_draft_structure(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    line_version_id: Uuid,
    content: &PriceContent,
) -> Result<(), RepoError> {
    let version = require_version(runner, scope, tenant_id, line_version_id).await?;
    if version.lifecycle_state != LifecycleState::Draft.as_str() {
        return Ok(());
    }
    // The ladder comes off before the kind moves; see [`clear_draft_geometry`].
    clear_draft_geometry(runner, scope, tenant_id, line_version_id).await?;
    let model = version_model(
        tenant_id,
        line_version_id,
        version.charge_line_id,
        version.plan_revision,
        content,
        version.created_by,
        version.created_at_utc,
    )?;
    let assignments = version_content_assignments(model);
    let mut update = charge_line_version::Entity::update_many()
        .secure()
        .scope_with(scope);
    for (column, value) in assignments {
        update = update.col_expr(column, Expr::value(value));
    }
    update
        .col_expr(
            charge_line_version::Column::RowVersion,
            Expr::col(charge_line_version::Column::RowVersion).add(1_i64),
        )
        .filter(
            Condition::all()
                .add(charge_line_version::Column::TenantId.eq(tenant_id))
                .add(charge_line_version::Column::LineVersionId.eq(line_version_id))
                .add(
                    charge_line_version::Column::LifecycleState.eq(LifecycleState::Draft.as_str()),
                ),
        )
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("update pricing_charge_line_version: {e}")))?;
    replace_draft_geometry(
        runner,
        scope,
        tenant_id,
        line_version_id,
        &content.row.bands,
    )
    .await
}

fn version_content_assignments(
    model: charge_line_version::ActiveModel,
) -> Vec<(charge_line_version::Column, sea_orm::Value)> {
    let charge_line_version::ActiveModel {
        tenant_id: _,
        line_version_id: _,
        charge_line_id: _,
        plan_revision: _,
        lifecycle_state: _,
        created_by: _,
        created_at_utc: _,
        row_version: _,
        resolved_invoice_line_template: _,
        resolved_gl_code: _,
        invoice_line_template,
        gl_code_ref,
        model_kind,
        package_size,
        quantity_source,
        manual_quantity,
        meter,
        billing_granularity,
        tier_aggregation_window,
        tier_qualification_window,
        aggregation_function,
        aggregation_granularity,
        max_hold_granules,
        included_allowance,
        reservation_flavor,
        min_qty_purchase,
        min_qty_usage,
        min_qty_usage_fallback,
        discount_ref,
        billing_timing,
        billing_anchor_policy,
        anchor_day,
        proration_basis,
        credit_on_downgrade,
    } = model;
    [
        (
            charge_line_version::Column::InvoiceLineTemplate,
            invoice_line_template.into_value(),
        ),
        (
            charge_line_version::Column::GlCodeRef,
            gl_code_ref.into_value(),
        ),
        (
            charge_line_version::Column::ModelKind,
            model_kind.into_value(),
        ),
        (
            charge_line_version::Column::PackageSize,
            package_size.into_value(),
        ),
        (
            charge_line_version::Column::QuantitySource,
            quantity_source.into_value(),
        ),
        (
            charge_line_version::Column::ManualQuantity,
            manual_quantity.into_value(),
        ),
        (charge_line_version::Column::Meter, meter.into_value()),
        (
            charge_line_version::Column::BillingGranularity,
            billing_granularity.into_value(),
        ),
        (
            charge_line_version::Column::TierAggregationWindow,
            tier_aggregation_window.into_value(),
        ),
        (
            charge_line_version::Column::TierQualificationWindow,
            tier_qualification_window.into_value(),
        ),
        (
            charge_line_version::Column::AggregationFunction,
            aggregation_function.into_value(),
        ),
        (
            charge_line_version::Column::AggregationGranularity,
            aggregation_granularity.into_value(),
        ),
        (
            charge_line_version::Column::MaxHoldGranules,
            max_hold_granules.into_value(),
        ),
        (
            charge_line_version::Column::IncludedAllowance,
            included_allowance.into_value(),
        ),
        (
            charge_line_version::Column::ReservationFlavor,
            reservation_flavor.into_value(),
        ),
        (
            charge_line_version::Column::MinQtyPurchase,
            min_qty_purchase.into_value(),
        ),
        (
            charge_line_version::Column::MinQtyUsage,
            min_qty_usage.into_value(),
        ),
        (
            charge_line_version::Column::MinQtyUsageFallback,
            min_qty_usage_fallback.into_value(),
        ),
        (
            charge_line_version::Column::DiscountRef,
            discount_ref.into_value(),
        ),
        (
            charge_line_version::Column::BillingTiming,
            billing_timing.into_value(),
        ),
        (
            charge_line_version::Column::BillingAnchorPolicy,
            billing_anchor_policy.into_value(),
        ),
        (
            charge_line_version::Column::AnchorDay,
            anchor_day.into_value(),
        ),
        (
            charge_line_version::Column::ProrationBasis,
            proration_basis.into_value(),
        ),
        (
            charge_line_version::Column::CreditOnDowngrade,
            credit_on_downgrade.into_value(),
        ),
    ]
    .into_iter()
    .filter_map(|(column, value)| value.map(|value| (column, value)))
    .collect()
}

/// Freeze resolved descriptors on versions that this publish is taking live.
pub async fn freeze_published_versions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    versions: &[(Uuid, String, String)],
) -> Result<(), RepoError> {
    use std::collections::BTreeMap;
    let mut by_resolution: BTreeMap<(String, String), Vec<Uuid>> = BTreeMap::new();
    for (line_version_id, template, gl) in versions {
        by_resolution
            .entry((template.clone(), gl.clone()))
            .or_default()
            .push(*line_version_id);
    }
    for ((template, gl), ids) in by_resolution {
        let mut group = Condition::any();
        for id in &ids {
            group = group.add(charge_line_version::Column::LineVersionId.eq(*id));
        }
        charge_line_version::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(
                charge_line_version::Column::LifecycleState,
                Expr::value(LifecycleState::Published.as_str()),
            )
            .col_expr(
                charge_line_version::Column::ResolvedInvoiceLineTemplate,
                Expr::value(template),
            )
            .col_expr(charge_line_version::Column::ResolvedGlCode, Expr::value(gl))
            // **Only a draft version is flipped.** A line's markets publish
            // independently — that is the whole point of the split — so the second
            // market to publish reaches a version the first already froze. Its
            // content is immutable by then and the append-only guard says so, which
            // would turn an ordinary second publish into a 500. The predicate makes
            // the flip idempotent instead: the version is already published, with
            // the resolutions the first publish froze onto it.
            .filter(
                Condition::all()
                    .add(charge_line_version::Column::TenantId.eq(tenant_id))
                    .add(
                        charge_line_version::Column::LifecycleState
                            .eq(LifecycleState::Draft.as_str()),
                    )
                    .add(group),
            )
            .exec(runner)
            .await
            .map_err(|e| RepoError::Db(format!("publish pricing_charge_line_version: {e}")))?;
    }
    Ok(())
}

/// Shared geometry of one version, ordered by ordinal.
pub async fn load_geometry(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    line_version_id: Uuid,
) -> Result<Vec<charge_tier::Model>, RepoError> {
    charge_tier::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_tier::Column::TenantId.eq(tenant_id))
                .add(charge_tier::Column::LineVersionId.eq(line_version_id)),
        )
        .order_by(charge_tier::Column::BandOrdinal, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_charge_tier: {e}")))
}

async fn open_plan_revision(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: Uuid,
) -> Result<Option<i64>, RepoError> {
    Ok(
        super::plan_repo::load_open_draft(runner, scope, tenant_id, PlanId::new(plan_id))
            .await?
            .map(|draft| i64::try_from(draft.revision).unwrap_or(i64::MAX)),
    )
}

async fn find_or_insert_line(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    key: &ChargeLineScopeKey,
) -> Result<Uuid, RepoError> {
    if let Some(existing) = charge_line::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(line_scope_filter(tenant_id, key))
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_charge_line by scope: {e}")))?
    {
        return Ok(existing.charge_line_id);
    }
    let charge_line_id = line_id(tenant_id, key);
    let row = charge_line::ActiveModel {
        tenant_id: Set(tenant_id),
        charge_line_id: Set(charge_line_id),
        plan_id: Set(key.plan_id().get()),
        phase: Set(key.phase().get()),
        price_overlay: Set(key.price_overlay().as_str().to_owned()),
        price_eligibility: Set(key.price_eligibility().as_str().to_owned()),
        charge_kind: Set(key.charge_kind().as_str().to_owned()),
        cohort: Set(key.cohort().to_string()),
        sku_id: Set(key.sku_id().as_uuid()),
        dimension_key: Set(key.dimension_key().as_str().to_owned()),
    };
    let insert = charge_line::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| RepoError::Db(format!("pricing_charge_line scope: {e}")))?;
    match insert.exec(runner).await {
        Ok(_) => Ok(charge_line_id),
        Err(err) => {
            // A concurrent creator won the logical-scope unique. Read theirs.
            if let Some(existing) = charge_line::Entity::find()
                .secure()
                .scope_with(scope)
                .filter(line_scope_filter(tenant_id, key))
                .one(runner)
                .await
                .map_err(|e| RepoError::Db(format!("re-read pricing_charge_line: {e}")))?
            {
                Ok(existing.charge_line_id)
            } else {
                Err(RepoError::Db(format!("insert pricing_charge_line: {err}")))
            }
        }
    }
}

fn line_scope_filter(tenant_id: Uuid, key: &ChargeLineScopeKey) -> Condition {
    Condition::all()
        .add(charge_line::Column::TenantId.eq(tenant_id))
        .add(charge_line::Column::PlanId.eq(key.plan_id().get()))
        .add(charge_line::Column::SkuId.eq(key.sku_id().as_uuid()))
        .add(charge_line::Column::PriceOverlay.eq(key.price_overlay().as_str()))
        .add(charge_line::Column::Phase.eq(key.phase().get()))
        .add(charge_line::Column::PriceEligibility.eq(key.price_eligibility().as_str()))
        .add(charge_line::Column::ChargeKind.eq(key.charge_kind().as_str()))
        .add(charge_line::Column::Cohort.eq(key.cohort().to_string()))
        .add(charge_line::Column::DimensionKey.eq(key.dimension_key().as_str()))
}

fn line_id(tenant_id: Uuid, key: &ChargeLineScopeKey) -> Uuid {
    let mut bytes = Vec::new();
    bytes.extend(tenant_id.as_bytes());
    bytes.extend(key.plan_id().get().as_bytes());
    bytes.extend(key.sku_id().as_uuid().as_bytes());
    bytes.extend(key.phase().get().as_bytes());
    bytes.extend(key.price_overlay().as_str().as_bytes());
    bytes.extend(key.price_eligibility().as_str().as_bytes());
    bytes.extend(key.charge_kind().as_str().as_bytes());
    bytes.extend(key.cohort().to_string().as_bytes());
    bytes.extend(key.dimension_key().as_str().as_bytes());
    Uuid::new_v5(&LINE_NS, &bytes)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the line and revision that identify the version, the content it is \
              rendered from, and the authoring stamp"
)]
async fn find_or_insert_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    charge_line_id: Uuid,
    plan_revision: i64,
    content: &PriceContent,
    created_by: Uuid,
    created_at_utc: time::OffsetDateTime,
) -> Result<Uuid, RepoError> {
    if let Some(existing) = charge_line_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_line_version::Column::TenantId.eq(tenant_id))
                .add(charge_line_version::Column::ChargeLineId.eq(charge_line_id))
                .add(charge_line_version::Column::PlanRevision.eq(plan_revision)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_charge_line_version: {e}")))?
    {
        return Ok(existing.line_version_id);
    }
    let line_version_id = {
        let mut bytes = Vec::new();
        bytes.extend(tenant_id.as_bytes());
        bytes.extend(charge_line_id.as_bytes());
        bytes.extend(plan_revision.to_be_bytes());
        Uuid::new_v5(&VERSION_NS, &bytes)
    };
    let row = version_model(
        tenant_id,
        line_version_id,
        charge_line_id,
        plan_revision,
        content,
        created_by,
        created_at_utc,
    )?;
    let insert = charge_line_version::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| RepoError::Db(format!("pricing_charge_line_version scope: {e}")))?;
    match insert.exec(runner).await {
        Ok(_) => Ok(line_version_id),
        Err(err) => {
            if let Some(existing) = charge_line_version::Entity::find()
                .secure()
                .scope_with(scope)
                .filter(
                    Condition::all()
                        .add(charge_line_version::Column::TenantId.eq(tenant_id))
                        .add(charge_line_version::Column::ChargeLineId.eq(charge_line_id))
                        .add(charge_line_version::Column::PlanRevision.eq(plan_revision)),
                )
                .one(runner)
                .await
                .map_err(|e| RepoError::Db(format!("re-read pricing_charge_line_version: {e}")))?
            {
                Ok(existing.line_version_id)
            } else {
                Err(RepoError::Db(format!(
                    "insert pricing_charge_line_version: {err}"
                )))
            }
        }
    }
}

fn version_model(
    tenant_id: Uuid,
    line_version_id: Uuid,
    charge_line_id: Uuid,
    plan_revision: i64,
    content: &PriceContent,
    created_by: Uuid,
    created_at_utc: time::OffsetDateTime,
) -> Result<charge_line_version::ActiveModel, RepoError> {
    let row = &content.row;
    let (meter, _) = crate::domain::price_record::canonical_usage_line(row);
    Ok(charge_line_version::ActiveModel {
        tenant_id: Set(tenant_id),
        line_version_id: Set(line_version_id),
        charge_line_id: Set(charge_line_id),
        plan_revision: Set(plan_revision),
        lifecycle_state: Set(LifecycleState::Draft.as_str().to_owned()),
        invoice_line_template: Set(row.invoice_line_template.clone()),
        gl_code_ref: Set(row.gl_code_ref.clone()),
        resolved_invoice_line_template: Set(None),
        resolved_gl_code: Set(None),
        model_kind: Set(row.model_kind.map(model_kind_wire).map(str::to_owned)),
        package_size: Set(stored_count("package_size", row.package_size)?),
        quantity_source: Set(row.quantity_source.map(|s| s.as_str().to_owned())),
        manual_quantity: Set(stored_count("manual_quantity", row.manual_quantity)?),
        meter: Set(meter),
        billing_granularity: Set(row.billing_granularity.map(|g| g.as_str().to_owned())),
        tier_aggregation_window: Set(row.tier_aggregation_window.map(|w| w.as_str().to_owned())),
        tier_qualification_window: Set(row
            .tier_qualification_window
            .map(|w| w.as_str().to_owned())),
        aggregation_function: Set(row.aggregation_function.map(|f| f.as_str().to_owned())),
        aggregation_granularity: Set(row.aggregation_granularity.map(|g| g.as_str().to_owned())),
        max_hold_granules: Set(stored_count("max_hold_granules", row.max_hold_granules)?),
        included_allowance: Set(row.included_allowance.map(allowance_json)),
        reservation_flavor: Set(row.reservation_flavor.map(|f| f.as_str().to_owned())),
        min_qty_purchase: Set(stored_count("min_qty_purchase", row.min_qty_purchase)?),
        min_qty_usage: Set(stored_count("min_qty_usage", row.min_qty_usage)?),
        min_qty_usage_fallback: Set(row.min_qty_usage_fallback.map(|f| f.as_str().to_owned())),
        discount_ref: Set(row.discount_ref.clone()),
        billing_timing: Set(content.billing_timing.clone()),
        billing_anchor_policy: Set(content
            .proration_contract
            .map(|c| c.billing_anchor_policy.as_str().to_owned())),
        anchor_day: Set(content
            .proration_contract
            .and_then(|c| c.billing_anchor_policy.anchor_day())
            .map(|d| i32::from(d.get()))),
        proration_basis: Set(content
            .proration_contract
            .map(|c| c.proration_basis.as_str().to_owned())),
        credit_on_downgrade: Set(content.proration_contract.map(|c| c.credit_on_downgrade)),
        created_by: Set(created_by),
        created_at_utc: Set(created_at_utc),
        row_version: Set(0),
    })
}

/// Drop a draft version's geometry, leaving the version itself alone.
///
/// **Separate from writing the new geometry, and the gap between them is where
/// the kind moves.** `trg_pricing_charge_tier_parent_kind` refuses a version
/// that still carries bands leaving `graduated`/`volume`, and
/// `trg_pricing_charge_tier_kind_insert` refuses a band on a version that is not
/// one of those — so a `graduated → flat` edit has to clear first and a
/// `flat → graduated` edit has to write last. One `replace` could satisfy only
/// one of the two, and the tiered-to-flat direction is the one a draft edit
/// actually takes.
async fn clear_draft_geometry(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    line_version_id: Uuid,
) -> Result<(), RepoError> {
    let Some(version) = charge_line_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_line_version::Column::TenantId.eq(tenant_id))
                .add(charge_line_version::Column::LineVersionId.eq(line_version_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read line version for geometry: {e}")))?
    else {
        return Ok(());
    };
    if version.lifecycle_state != LifecycleState::Draft.as_str() {
        return Ok(());
    }
    charge_tier::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_tier::Column::TenantId.eq(tenant_id))
                .add(charge_tier::Column::LineVersionId.eq(line_version_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("delete pricing_charge_tier: {e}")))?;
    Ok(())
}

async fn replace_draft_geometry(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    line_version_id: Uuid,
    bands: &[crate::domain::price_row::TierBand],
) -> Result<(), RepoError> {
    let Some(version) = charge_line_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_line_version::Column::TenantId.eq(tenant_id))
                .add(charge_line_version::Column::LineVersionId.eq(line_version_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read line version for geometry: {e}")))?
    else {
        return Ok(());
    };
    if version.lifecycle_state != LifecycleState::Draft.as_str() {
        return Ok(());
    }
    charge_tier::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_tier::Column::TenantId.eq(tenant_id))
                .add(charge_tier::Column::LineVersionId.eq(line_version_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("delete pricing_charge_tier: {e}")))?;
    for (ordinal, band) in crate::domain::price_row::bands_in_ordinal_order(bands) {
        let ordinal = i32::try_from(ordinal).map_err(|_| RepoError::ValueOutOfRange {
            field: "band_ordinal".to_owned(),
            value: ordinal.to_string(),
        })?;
        let from_qty = stored_bound("band from_qty", band.from_qty)?;
        let to_qty = match band.to_qty {
            BandTop::Open => None,
            BandTop::Closed(top) => Some(stored_bound("band to_qty", top)?),
        };
        let row = charge_tier::ActiveModel {
            tenant_id: Set(tenant_id),
            line_version_id: Set(line_version_id),
            band_ordinal: Set(ordinal),
            from_qty: Set(from_qty),
            to_qty: Set(to_qty),
        };
        charge_tier::Entity::insert(row.clone())
            .secure()
            .scope_with_model(scope, &row)
            .map_err(|e| RepoError::Db(format!("pricing_charge_tier scope: {e}")))?
            .exec(runner)
            .await
            .map_err(|e| RepoError::Db(format!("insert pricing_charge_tier: {e}")))?;
    }
    Ok(())
}
