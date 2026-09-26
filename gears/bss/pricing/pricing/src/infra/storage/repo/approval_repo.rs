//! Scoped approval repo; follows the Products implementation.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-unit-store:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-quorum-policy:p1
use super::driver_failure;
const DEFAULT_QUORUM: u32 = 1;
use crate::infra::storage::{
    RepoError,
    entity::{approval_decision, approval_policy, approval_unit, approval_unit_item},
};
use bss_approval::{ApprovalError, Decision, ItemRef, Policy, Store, Unit, UnitState, Verdict};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order, Set};
use time::OffsetDateTime;
use toolkit_db::DbTx;
use toolkit_db::secure::{
    AccessScope, DBRunner, ScopeError, SecureDeleteExt, SecureEntityExt, SecureInsertExt,
    SecureOnConflict, SecureUpdateExt,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct PricingApprovalStore {
    pub scope: AccessScope,
    pub tenant_id: Uuid,
}
fn store_err(context: &str, e: ScopeError) -> ApprovalError {
    match e {
        ScopeError::Db(e) => ApprovalError::Db(e),
        other => ApprovalError::Store(format!("{context}: {other}")),
    }
}
fn unit_key(tenant: Uuid, id: Uuid) -> Condition {
    Condition::all()
        .add(approval_unit::Column::TenantId.eq(tenant))
        .add(approval_unit::Column::Id.eq(id))
}
fn item_key(tenant: Uuid, id: Uuid) -> Condition {
    Condition::all()
        .add(approval_unit_item::Column::TenantId.eq(tenant))
        .add(approval_unit_item::Column::UnitId.eq(id))
}
fn decision_key(tenant: Uuid, id: Uuid) -> Condition {
    Condition::all()
        .add(approval_decision::Column::TenantId.eq(tenant))
        .add(approval_decision::Column::UnitId.eq(id))
}
fn unit_from_model(m: approval_unit::Model) -> Result<Unit, ApprovalError> {
    Ok(Unit {
        id: m.id,
        tenant_id: m.tenant_id,
        kind: m.kind,
        ref_type: m.ref_type,
        ref_id: m.ref_id,
        state: UnitState::parse(&m.state)
            .ok_or_else(|| ApprovalError::Store(format!("invalid unit state {}", m.state)))?,
        common_effective_date: m.common_effective_date,
        quorum_required: u32::try_from(m.quorum_required)
            .map_err(|e| ApprovalError::Store(e.to_string()))?,
        generation: m.generation,
        submitted_by: m.submitted_by,
        submitted_at: m.submitted_at,
        decided_at: m.decided_at,
        decided_note: m.decided_note,
        snapshot: m.snapshot,
        snapshot_hash: m.snapshot_hash,
        version: m.version,
    })
}
fn decision_from_model(m: approval_decision::Model) -> Result<Decision, ApprovalError> {
    Ok(Decision {
        unit_id: m.unit_id,
        actor: m.actor,
        generation: m.generation,
        verdict: Verdict::parse(&m.decision)
            .ok_or_else(|| ApprovalError::Store(format!("invalid decision {}", m.decision)))?,
        note: m.note,
        at: m.at,
        stale: m.stale,
    })
}
impl PricingApprovalStore {
    async fn require_unit(&self, runner: &impl DBRunner, id: Uuid) -> Result<(), ApprovalError> {
        if find_unit(runner, &self.scope, self.tenant_id, id)
            .await?
            .is_some()
        {
            Ok(())
        } else {
            Err(ApprovalError::Store("UNIT_NOT_FOUND".into()))
        }
    }
    async fn insert_items(
        &self,
        runner: &impl DBRunner,
        id: Uuid,
        items: &[ItemRef],
    ) -> Result<(), ApprovalError> {
        for i in items {
            let m = approval_unit_item::ActiveModel {
                unit_id: Set(id),
                tenant_id: Set(self.tenant_id),
                item_type: Set(i.item_type.clone()),
                item_id: Set(i.item_id),
                created_by: Set(i.created_by),
                before_json: Set(i.before.clone()),
                after_json: Set(i.after.clone()),
            };
            approval_unit_item::Entity::insert(m.clone())
                .secure()
                .scope_with_model(&self.scope, &m)
                .map_err(|e| store_err("item scope", e))?
                .exec(runner)
                .await
                .map_err(|e| store_err("insert item", e))?;
        }
        Ok(())
    }
}
#[async_trait::async_trait]
impl<'a> Store<DbTx<'a>> for PricingApprovalStore {
    async fn insert_unit(
        &self,
        runner: &DbTx<'a>,
        unit: &Unit,
        items: &[ItemRef],
    ) -> Result<(), ApprovalError> {
        if unit.tenant_id != self.tenant_id {
            return Err(ApprovalError::Store(
                "unit tenant differs from store tenant".into(),
            ));
        }
        let m = approval_unit::ActiveModel {
            id: Set(unit.id),
            tenant_id: Set(self.tenant_id),
            kind: Set(unit.kind.clone()),
            ref_type: Set(unit.ref_type.clone()),
            ref_id: Set(unit.ref_id),
            state: Set(unit.state.as_str().into()),
            common_effective_date: Set(unit.common_effective_date),
            quorum_required: Set(i32::try_from(unit.quorum_required)
                .map_err(|e| ApprovalError::Store(e.to_string()))?),
            generation: Set(unit.generation),
            submitted_by: Set(unit.submitted_by),
            submitted_at: Set(unit.submitted_at),
            decided_at: Set(unit.decided_at),
            decided_note: Set(unit.decided_note.clone()),
            snapshot: Set(unit.snapshot.clone()),
            snapshot_hash: Set(unit.snapshot_hash.clone()),
            version: Set(unit.version),
        };
        approval_unit::Entity::insert(m.clone())
            .secure()
            .scope_with_model(&self.scope, &m)
            .map_err(|e| store_err("unit scope", e))?
            .exec(runner)
            .await
            .map_err(|e| store_err("insert unit", e))?;
        self.insert_items(runner, unit.id, items).await
    }
    async fn unit(&self, runner: &DbTx<'a>, id: Uuid) -> Result<Option<Unit>, ApprovalError> {
        find_unit(runner, &self.scope, self.tenant_id, id).await
    }
    async fn bump_version(
        &self,
        runner: &DbTx<'a>,
        id: Uuid,
        expected: i64,
    ) -> Result<bool, ApprovalError> {
        let r = approval_unit::Entity::update_many()
            .secure()
            .scope_with(&self.scope)
            .col_expr(
                approval_unit::Column::Version,
                Expr::col(approval_unit::Column::Version).add(1_i64),
            )
            .filter(unit_key(self.tenant_id, id).add(approval_unit::Column::Version.eq(expected)))
            .exec(runner)
            .await
            .map_err(|e| store_err("bump version", e))?;
        Ok(r.rows_affected == 1)
    }
    async fn items(&self, runner: &DbTx<'a>, id: Uuid) -> Result<Vec<ItemRef>, ApprovalError> {
        Ok(approval_unit_item::Entity::find()
            .secure()
            .scope_with(&self.scope)
            .filter(item_key(self.tenant_id, id))
            .order_by(approval_unit_item::Column::ItemType, Order::Asc)
            .order_by(approval_unit_item::Column::ItemId, Order::Asc)
            .all(runner)
            .await
            .map_err(|e| store_err("read items", e))?
            .into_iter()
            .map(|m| ItemRef {
                item_type: m.item_type,
                item_id: m.item_id,
                created_by: m.created_by,
                before: m.before_json,
                after: m.after_json,
            })
            .collect())
    }
    async fn decisions(&self, runner: &DbTx<'a>, id: Uuid) -> Result<Vec<Decision>, ApprovalError> {
        decision_rows(runner, &self.scope, self.tenant_id, id)
            .await
            .map_err(|e| store_err("decisions", e))?
            .into_iter()
            .map(decision_from_model)
            .collect()
    }
    async fn insert_decision(&self, runner: &DbTx<'a>, d: &Decision) -> Result<(), ApprovalError> {
        self.require_unit(runner, d.unit_id).await?;
        let m = approval_decision::ActiveModel {
            unit_id: Set(d.unit_id),
            tenant_id: Set(self.tenant_id),
            actor: Set(d.actor),
            generation: Set(d.generation),
            decision: Set(d.verdict.as_str().into()),
            note: Set(d.note.clone()),
            at: Set(d.at),
            stale: Set(d.stale),
        };
        approval_decision::Entity::insert(m.clone())
            .secure()
            .scope_with_model(&self.scope, &m)
            .map_err(|e| store_err("decision scope", e))?
            .exec(runner)
            .await
            .map_err(|e| {
                if e.is_unique_violation() {
                    ApprovalError::Store("DUPLICATE decision for actor and generation".into())
                } else {
                    store_err("insert decision", e)
                }
            })?;
        Ok(())
    }
    async fn refresh(
        &self,
        runner: &DbTx<'a>,
        id: Uuid,
        items: &[ItemRef],
        snapshot: &serde_json::Value,
        snapshot_hash: &str,
        generation: i32,
    ) -> Result<(), ApprovalError> {
        self.require_unit(runner, id).await?;
        approval_unit_item::Entity::delete_many()
            .secure()
            .scope_with(&self.scope)
            .filter(item_key(self.tenant_id, id))
            .exec(runner)
            .await
            .map_err(|e| store_err("delete old items", e))?;
        self.insert_items(runner, id, items).await?;
        approval_unit::Entity::update_many()
            .secure()
            .scope_with(&self.scope)
            .col_expr(
                approval_unit::Column::Snapshot,
                Expr::value(snapshot.clone()),
            )
            .col_expr(
                approval_unit::Column::SnapshotHash,
                Expr::value(snapshot_hash),
            )
            .col_expr(approval_unit::Column::Generation, Expr::value(generation))
            .filter(unit_key(self.tenant_id, id))
            .exec(runner)
            .await
            .map_err(|e| store_err("refresh unit", e))?;
        approval_decision::Entity::update_many()
            .secure()
            .scope_with(&self.scope)
            .col_expr(approval_decision::Column::Stale, Expr::value(true))
            .filter(
                decision_key(self.tenant_id, id)
                    .add(approval_decision::Column::Generation.lt(generation)),
            )
            .exec(runner)
            .await
            .map_err(|e| store_err("stale votes", e))?;
        Ok(())
    }
    async fn set_state(
        &self,
        runner: &DbTx<'a>,
        id: Uuid,
        state: UnitState,
        decided_at: Option<OffsetDateTime>,
        note: Option<&str>,
    ) -> Result<(), ApprovalError> {
        approval_unit::Entity::update_many()
            .secure()
            .scope_with(&self.scope)
            .col_expr(approval_unit::Column::State, Expr::value(state.as_str()))
            .col_expr(approval_unit::Column::DecidedAt, Expr::value(decided_at))
            .col_expr(approval_unit::Column::DecidedNote, Expr::value(note))
            .filter(unit_key(self.tenant_id, id))
            .exec(runner)
            .await
            .map_err(|e| store_err("decide unit", e))?;
        Ok(())
    }
}
/// Read policy.
/// # Errors
/// Returns scoped storage failures, preserving database errors for retry.
pub async fn read_policy(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Policy, RepoError> {
    let mut p = Policy {
        default_quorum: DEFAULT_QUORUM,
        overrides: std::collections::BTreeMap::new(),
    };
    for row in approval_policy::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(approval_policy::Column::TenantId.eq(tenant_id)))
        .all(runner)
        .await
        .map_err(|e| driver_failure("read policy".into(), e))?
    {
        let quorum = u32::try_from(row.quorum).map_err(|e| RepoError::CorruptRow(e.to_string()))?;
        if row.kind == "*" {
            p.default_quorum = quorum;
        } else {
            p.overrides.insert(row.kind, quorum);
        }
    }
    Ok(p)
}
/// Write policy.
/// # Errors
/// Returns scoped storage failures, preserving database errors for retry.
pub async fn write_policy(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    kind: &str,
    quorum: u32,
) -> Result<(), RepoError> {
    let m = approval_policy::ActiveModel {
        tenant_id: Set(tenant_id),
        kind: Set(kind.into()),
        quorum: Set(i32::try_from(quorum).map_err(|e| RepoError::Db(e.to_string()))?),
    };
    let conflict = SecureOnConflict::<approval_policy::Entity>::columns([
        approval_policy::Column::TenantId,
        approval_policy::Column::Kind,
    ])
    .update_columns([approval_policy::Column::Quorum])
    .map_err(|e| driver_failure("policy conflict".into(), e))?;
    approval_policy::Entity::insert(m.clone())
        .secure()
        .scope_with_model(scope, &m)
        .map_err(|e| driver_failure("policy scope".into(), e))?
        .on_conflict(conflict)
        .exec(runner)
        .await
        .map_err(|e| driver_failure("write policy".into(), e))?;
    Ok(())
}
/// List units.
/// # Errors
/// Returns scoped storage failures, preserving database errors for retry.
pub async fn list_units(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    state: Option<UnitState>,
    kind: Option<&str>,
    ref_id: Option<Uuid>,
) -> Result<Vec<Unit>, RepoError> {
    let mut c = Condition::all().add(approval_unit::Column::TenantId.eq(tenant_id));
    if let Some(s) = state {
        c = c.add(approval_unit::Column::State.eq(s.as_str()));
    }
    if let Some(k) = kind {
        c = c.add(approval_unit::Column::Kind.eq(k));
    }
    if let Some(id) = ref_id {
        c = c.add(approval_unit::Column::RefId.eq(id));
    }
    approval_unit::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(c)
        .order_by(approval_unit::Column::SubmittedAt, Order::Asc)
        .order_by(approval_unit::Column::Id, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure("list units".into(), e))?
        .into_iter()
        .map(|m| unit_from_model(m).map_err(|e| RepoError::CorruptRow(e.to_string())))
        .collect()
}
async fn decision_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Vec<approval_decision::Model>, ScopeError> {
    approval_decision::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(decision_key(tenant, id))
        .order_by(approval_decision::Column::Generation, Order::Asc)
        .order_by(approval_decision::Column::At, Order::Asc)
        .order_by(approval_decision::Column::Actor, Order::Asc)
        .all(runner)
        .await
}
/// Decisions of.
/// # Errors
/// Returns scoped storage failures, preserving database errors for retry.
pub async fn decisions_of(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    unit_id: Uuid,
) -> Result<Vec<Decision>, RepoError> {
    decision_rows(runner, scope, tenant_id, unit_id)
        .await
        .map_err(|e| driver_failure("read decisions".into(), e))?
        .into_iter()
        .map(|m| decision_from_model(m).map_err(|e| RepoError::CorruptRow(e.to_string())))
        .collect()
}

/// Find unit.
/// # Errors
/// Returns scoped storage failures, preserving database errors for retry.
pub async fn find_unit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<Option<Unit>, ApprovalError> {
    approval_unit::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(unit_key(tenant, id))
        .one(runner)
        .await
        .map_err(|e| store_err("read unit", e))?
        .map(unit_from_model)
        .transpose()
}

#[cfg(test)]
#[path = "approval_repo_tests.rs"]
mod tests;
