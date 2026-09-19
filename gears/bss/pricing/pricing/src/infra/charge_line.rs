//! Line-first authoring: the charge line, its shared structure, and the market
//! prices filed under it.
//!
//! The HTTP door owns idempotency and response rendering around these functions;
//! each of them runs inside the caller's transaction.
//!
//! # Which tag guards which act
//!
//! **A line's structure is the plan's content**, so the three acts on it --
//! create, replace, delete -- take the plan revision's tag as their precondition
//! and move it, exactly as a draft-window write does
//! ([`crate::infra::draft_window::apply_command`]). Two authors restructuring one
//! plan collide on that tag rather than last-writer-wins.
//!
//! **A market price is a row of its own**, and keeps the contract it has always
//! had: no precondition on create, its own entity tag on edit and delete. Filing a
//! price under a line does not move the plan's tag, for the reason
//! [`ensure_draft_graph`](charge_line_repo::ensure_draft_graph) records -- it would
//! refuse every caller holding a plan tag across a price create.

use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{
    AccessScope, DBRunner, DbTx, SecureDeleteExt, SecureEntityExt, SecureInsertExt,
};
use uuid::Uuid;

use crate::domain::audit::{AuditAction, AuditStamp};
use crate::domain::concurrency::RowVersion;
use crate::domain::error::DomainError;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::price_record::PriceContent;
use crate::domain::scope_key::{ChargeLineScopeKey, PlanId};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    charge_line, charge_line_version, charge_tier, market_price, price, price_tier_band,
};
use crate::infra::storage::repo::plan_repo::{
    load_revision, record_revision_mutation, refuse, swap_guard,
};
use crate::infra::storage::repo::plan_shape_repo::plan_revision_bump;
use crate::infra::storage::repo::price_repo::{self, LineRecord};
use crate::infra::storage::repo::{charge_line_repo, window_guard_repo};
use crate::infra::storage::repo_failure;

/// `DELETE … WHERE filter`, scoped. A macro rather than a generic function because
/// the secured delete is bounded on a crate-private entity trait.
macro_rules! delete_where {
    ($entity:ty, $filter:expr, $scope:expr, $runner:expr, $table:literal) => {
        <$entity>::delete_many()
            .secure()
            .scope_with($scope)
            .filter($filter)
            .exec($runner)
            .await
            .map(|_| ())
            .map_err(|e| DomainError::Internal(format!(concat!("delete ", $table, ": {}"), e)))
    };
}

/// The plan revision a structural write asserts: which revision, at which version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanTag {
    /// The plan being authored.
    pub plan_id: PlanId,
    /// The revision the caller read.
    pub revision: u64,
    /// The version that revision stood at.
    pub version: RowVersion,
}

/// What a structural write hands back: the line as it now stands, and the plan
/// revision's new row version for the response's tag.
#[derive(Clone, Debug)]
pub struct LineWrite {
    /// The line version after the write.
    pub line: LineRecord,
    /// The plan revision's row version after the bump.
    pub plan_row_version: u64,
}

/// Draft a new logical line with its shared structure, and no market price yet.
///
/// # Errors
/// [`DomainError::StaleVersion`] (or the revision's own refusal) when the tag is
/// not the open draft's current one; [`DomainError::DuplicateScopeKey`] when the
/// plan already holds a line on these eight axes -- a fresh id does not buy a
/// second one; storage failures through [`repo_failure`].
pub async fn create_line(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    tenant_id: Uuid,
    tag: PlanTag,
    key: &ChargeLineScopeKey,
    content: &PriceContent,
    stamp: AuditStamp,
) -> Result<LineWrite, DomainError> {
    let guard = open_cas(txn, scope, tenant_id, tag).await?;
    if charge_line_repo::find_by_scope(txn, scope, tenant_id, key)
        .await
        .map_err(|e| repo_failure(&e))?
        .is_some()
    {
        return Err(DomainError::DuplicateScopeKey(key.to_string()));
    }
    let graph = charge_line_repo::ensure_draft_graph(
        txn,
        scope,
        tenant_id,
        key,
        content,
        stamp.actor_principal_id,
        stamp.recorded_at,
    )
    .await
    .map_err(|e| repo_failure(&e))?;
    let plan_row_version = close_cas(txn, scope, tenant_id, tag, guard, stamp).await?;
    let line = require_line_record(txn, scope, tenant_id, graph.line_version_id).await?;
    Ok(LineWrite {
        line,
        plan_row_version,
    })
}

/// Replace a draft version's whole shared structure, tier geometry included.
///
/// **The markets' rates survive a geometry edit where they still have a band to
/// be the rate of.** A rate names its band by ordinal through a compound key into
/// the geometry, so the ladder cannot be replaced under it; the rates of every
/// market of this version are lifted off, the structure and geometry are
/// rewritten, and each rate whose ordinal still exists goes back. A market left
/// with fewer rates than bands is an incomplete draft -- which a draft may be, and
/// which the publish pre-check reports by name -- rather than a refusal here that
/// would make a ladder uneditable once priced.
///
/// # Errors
/// As [`create_line`] for the tag; [`DomainError::NotFound`] when the version is
/// not one of this plan's; [`DomainError::LifecycleForbidden`] when it has left
/// `draft`.
pub async fn replace_structure(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    tenant_id: Uuid,
    tag: PlanTag,
    line_version_id: Uuid,
    content: &PriceContent,
    stamp: AuditStamp,
) -> Result<LineWrite, DomainError> {
    let guard = open_cas(txn, scope, tenant_id, tag).await?;
    let current = require_line_of_plan(txn, scope, tenant_id, tag.plan_id, line_version_id).await?;
    require_draft(&current)?;

    let lifted = lift_rates(txn, scope, tenant_id, line_version_id).await?;
    charge_line_repo::update_draft_structure(txn, scope, tenant_id, line_version_id, content)
        .await
        .map_err(|e| repo_failure(&e))?;
    let bands = i32::try_from(content.row.bands.len()).unwrap_or(i32::MAX);
    for rate in lifted {
        if rate.band_ordinal < bands {
            let row = price_tier_band::ActiveModel::from(rate);
            price_tier_band::Entity::insert(row.clone())
                .secure()
                .scope_with_model(scope, &row)
                .map_err(|e| DomainError::Internal(format!("pricing_price_tier_band scope: {e}")))?
                .exec(txn)
                .await
                .map_err(|e| DomainError::Internal(format!("restore tier rate: {e}")))?;
        }
    }

    let plan_row_version = close_cas(txn, scope, tenant_id, tag, guard, stamp).await?;
    let line = require_line_record(txn, scope, tenant_id, line_version_id).await?;
    Ok(LineWrite {
        line,
        plan_row_version,
    })
}

/// Delete a draft line version nothing is priced against.
///
/// The logical line and its markets go with it when this was the line's only
/// version -- a line no revision describes is not a line of the plan.
///
/// # Errors
/// As [`replace_structure`]; [`DomainError::ChargeLineInUse`] when a monetary
/// version still references the structure.
pub async fn delete_version(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    tenant_id: Uuid,
    tag: PlanTag,
    line_version_id: Uuid,
    stamp: AuditStamp,
) -> Result<u64, DomainError> {
    let guard = open_cas(txn, scope, tenant_id, tag).await?;
    let current = require_line_of_plan(txn, scope, tenant_id, tag.plan_id, line_version_id).await?;
    require_draft(&current)?;

    let priced = price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(price::Column::TenantId.eq(tenant_id))
                .add(price::Column::LineVersionId.eq(line_version_id)),
        )
        .one(txn)
        .await
        .map_err(|e| DomainError::Internal(format!("read prices of a line version: {e}")))?;
    if let Some(row) = priced {
        return Err(DomainError::ChargeLineInUse(format!(
            "line version {line_version_id} is still priced (price row {}); delete its market \
             prices first",
            row.price_id
        )));
    }

    delete_where!(
        charge_tier::Entity,
        Condition::all()
            .add(charge_tier::Column::TenantId.eq(tenant_id))
            .add(charge_tier::Column::LineVersionId.eq(line_version_id)),
        scope,
        txn,
        "pricing_charge_tier"
    )?;
    delete_where!(
        charge_line_version::Entity,
        Condition::all()
            .add(charge_line_version::Column::TenantId.eq(tenant_id))
            .add(charge_line_version::Column::LineVersionId.eq(line_version_id)),
        scope,
        txn,
        "pricing_charge_line_version"
    )?;

    let line_id = current.charge_line_id;
    let survivor = charge_line_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_line_version::Column::TenantId.eq(tenant_id))
                .add(charge_line_version::Column::ChargeLineId.eq(line_id)),
        )
        .one(txn)
        .await
        .map_err(|e| DomainError::Internal(format!("read surviving line versions: {e}")))?;
    if survivor.is_none() {
        delete_where!(
            market_price::Entity,
            Condition::all()
                .add(market_price::Column::TenantId.eq(tenant_id))
                .add(market_price::Column::ChargeLineId.eq(line_id)),
            scope,
            txn,
            "pricing_market_price"
        )?;
        delete_where!(
            charge_line::Entity,
            Condition::all()
                .add(charge_line::Column::TenantId.eq(tenant_id))
                .add(charge_line::Column::ChargeLineId.eq(line_id)),
            scope,
            txn,
            "pricing_charge_line"
        )?;
    }
    close_cas(txn, scope, tenant_id, tag, guard, stamp).await
}

/// The named version, **confirmed to be one of `plan_id`'s**.
///
/// A version under another plan's URL answers exactly like an absent one: the
/// caller named a resource that does not exist at that address, and saying it
/// exists elsewhere would leak which plans hold which lines.
///
/// # Errors
/// [`DomainError::NotFound`]; storage failures through [`repo_failure`].
pub async fn require_line_of_plan(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    plan_id: PlanId,
    line_version_id: Uuid,
) -> Result<LineRecord, DomainError> {
    match price_repo::load_line_record(runner, scope, tenant_id, line_version_id)
        .await
        .map_err(|e| repo_failure(&e))?
    {
        Some(record) if record.scope_key.plan_id() == plan_id => Ok(record),
        _ => Err(not_found(line_version_id)),
    }
}

async fn require_line_record(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    line_version_id: Uuid,
) -> Result<LineRecord, DomainError> {
    price_repo::load_line_record(runner, scope, tenant_id, line_version_id)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| not_found(line_version_id))
}

fn not_found(line_version_id: Uuid) -> DomainError {
    DomainError::NotFound {
        subject: "charge line version".to_owned(),
        id: line_version_id.to_string(),
    }
}

fn require_draft(line: &LineRecord) -> Result<(), DomainError> {
    if line.lifecycle_state == LifecycleState::Draft {
        return Ok(());
    }
    Err(repo_failure(&RepoError::NotDraft {
        subject: "charge line version".to_owned(),
        id: line.line_version_id.to_string(),
        state: line.lifecycle_state.as_str().to_owned(),
    }))
}

/// Take the plan's serial lock and check the tag names the open draft as it stands.
async fn open_cas(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    tag: PlanTag,
) -> Result<Condition, DomainError> {
    window_guard_repo::acquire(runner, scope, tenant_id, tag.plan_id.get())
        .await
        .map_err(|e| repo_failure(&e))?;
    let refusal = || {
        refuse(
            runner,
            scope,
            tenant_id,
            tag.plan_id,
            tag.revision,
            tag.version,
        )
    };
    let Some(guard) = swap_guard(tenant_id, tag.plan_id, tag.revision, tag.version) else {
        return Err(repo_failure(&refusal().await));
    };
    let current = load_revision(runner, scope, tenant_id, tag.plan_id, tag.revision)
        .await
        .map_err(|e| repo_failure(&e))?;
    match current {
        Some(revision)
            if revision.row_version == tag.version
                && revision.lifecycle_state.is_content_mutable() =>
        {
            Ok(guard)
        }
        _ => Err(repo_failure(&refusal().await)),
    }
}

/// Move the plan revision's tag and record the mutation it stands for.
async fn close_cas(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    tag: PlanTag,
    guard: Condition,
    stamp: AuditStamp,
) -> Result<u64, DomainError> {
    let moved = plan_revision_bump(runner, scope, guard)
        .await
        .map_err(|e| repo_failure(&e))?;
    if moved == 0 {
        return Err(repo_failure(
            &refuse(
                runner,
                scope,
                tenant_id,
                tag.plan_id,
                tag.revision,
                tag.version,
            )
            .await,
        ));
    }
    let updated = load_revision(runner, scope, tenant_id, tag.plan_id, tag.revision)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "plan revision".to_owned(),
            id: format!("{}/{}", tag.plan_id, tag.revision),
        })?;
    record_revision_mutation(
        runner,
        scope,
        tenant_id,
        &updated,
        AuditAction::Update,
        tag.version,
        stamp,
    )
    .await
    .map_err(|e| repo_failure(&e))?;
    Ok(updated.row_version.get())
}

/// Read every market's tier rates of one version, then take them off.
async fn lift_rates(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    line_version_id: Uuid,
) -> Result<Vec<price_tier_band::Model>, DomainError> {
    let filter = || {
        Condition::all()
            .add(price_tier_band::Column::TenantId.eq(tenant_id))
            .add(price_tier_band::Column::LineVersionId.eq(line_version_id))
    };
    let rates = price_tier_band::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(filter())
        .all(runner)
        .await
        .map_err(|e| DomainError::Internal(format!("read tier rates of a line version: {e}")))?;
    delete_where!(
        price_tier_band::Entity,
        filter(),
        scope,
        runner,
        "pricing_price_tier_band"
    )?;
    Ok(rates)
}
