//! The `MigrationScheduler` of §1.7 — scheduling, cancellation, and the
//! Subscriptions execution handshake (`inst-ms-api`, `inst-ms-deltas`,
//! `inst-ms-emit`, `inst-ms-return`, `inst-mst-*`, `inst-mg-target`,
//! `inst-mg-idem`, `inst-mg-cancel`, D-36, D-49, D-65).
//!
//! The catalog **schedules**; Subscriptions **executes**. Nothing here mutates a
//! subscription and nothing re-opens a posted period (M1) — the whole of this
//! module's output is one `pricing_migration` row and one
//! `PlanMigrationScheduled` event.
//!
//! # Three of this slice's inputs have no producer in the built system
//!
//! Stated once here rather than rediscovered at each call site, and every one is
//! reported rather than smoothed over:
//!
//! 1. **The in-scope subscription set.** The catalog holds no subscriptions and
//!    the D-79 Subscriptions lane has no client — the same absence D-182 already
//!    makes normative one flow over. So [`schedule`] analyses an **empty**
//!    subject set, and [`MigrationSchedule::subjects_unresolved`] says so. That
//!    matters more than it looks: an empty delta report otherwise reads as "no
//!    subscriber has a problem" when the truth is "no subscriber could be
//!    enumerated", and those are opposite facts for an operator about to confirm.
//! 2. **The Contracts lock registry** (`inst-cl-source`). See
//!    [`crate::domain::migration_delta`] — absent, so every subscription reads
//!    locked and is excluded.
//! 3. **Entitlement grant totals** (`inst-md-entitlements`) — **the target half
//!    is built since 2026-08-08 and only the subject half is missing.** This
//!    entry read "there is no grant store in this gear at all" until then, which
//!    was true of the branch D-252 was written on and false at the merge:
//!    `pricing_plan.entitlement_grants` landed the same day in a concurrent
//!    strand (`m20260802_000053`, D-41), and [`target_shape`] was discarding a
//!    set its own `plan_repo::load_current` call already returned.
//!    [`grant_totals`] now maps it, and D-253 decides the two halves the mapping
//!    could not settle by itself — flags as `1`/`0` under a namespaced key, and
//!    the **minimum** across a phased plan's phases.
//!
//!    **The class is still silent end to end, and now for one reason instead of
//!    two**: source-side totals come from Subscriptions, which has no crate here
//!    and enumerates nothing. That is case 1 above, not a second gap.
//!
//! What is readable today is the target's **boundary coverage**, its **add-on
//! rule set** and now its **grant totals**; all three want a subject set.
//!
//! # Why scheduling opens no approval unit
//!
//! Retirement is an always-material act (D-109) and migration scheduling is not:
//! §5 types it `plan × migrate` and
//! [`crate::domain::materiality::triggers`] registers no migration trigger.
//! Scheduling therefore pins no subject and adds no fourth copy of the
//! `re_derive` assembly D-247 made normative. Were a later decision to make it
//! material, the schedule's subject is the **published** source revision — which
//! is exactly D-247's class, and `current_revision_shape` is what it must reuse.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use serde_json::{Value as JsonValue, json};
use toolkit_db::secure::{AccessScope, DBRunner, DbTx, SecureEntityExt};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::audit::{AuditAction, AuditStamp, AuditSubjectKind};
use crate::domain::error::DomainError;
use crate::domain::migration::{
    MigrationState, NoticePeriod, ensure_distinct_plans, ensure_target_publishable,
};
use crate::domain::migration_delta::{
    Boundary, ContractLockSet, DeltaReport, SubscriptionFacts, TargetShape, analyze, grant_totals,
};
use crate::domain::scope_key::PlanId;
use crate::infra::storage::entity::plan_addon_rule;
use crate::infra::storage::repo::audit_repo::NewAuditEntry;
use crate::infra::storage::repo::migration_repo::{MigrationRecord, NewMigration};
use crate::infra::storage::repo::{
    NewOutboxEvent, PlanMigrationScheduledPayload, audit_repo, migration_repo, outbox_repo,
    plan_repo, plan_shape_repo, policy_repo, price_repo,
};
use crate::infra::storage::{RepoError, repo_failure};

/// One scheduled migration, as the surfaces render it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationSchedule {
    /// The stored row.
    pub record: MigrationRecord,
    /// `false` when this call replayed an earlier create (M2, `inst-ms-api`).
    pub created: bool,
    /// The deltas as computed at schedule time.
    pub deltas: DeltaReport,
    /// **No subscription could be enumerated** (see the module doc). `true` in
    /// the built system, always, and reported so an empty delta report is not
    /// read as an all-clear.
    pub subjects_unresolved: bool,
}

/// The `MigrationScheduler` of §1.7.
///
/// `Clone` for [`crate::infra::retirement::RetirementService`]'s reason: both
/// fields are handles rather than state.
#[derive(Clone)]
pub struct MigrationService {
    db: DBProvider<DbError>,
    policies: policy_repo::PolicyObjectRepo,
}

impl MigrationService {
    /// Build the scheduler over one provider and the deployment's limits.
    ///
    /// Owns its [`policy_repo::PolicyObjectRepo`] rather than sharing one, for
    /// [`crate::infra::publish::PublishService`]'s reason: the repo is a reader
    /// bound to the ratified deployment defaults, so a second instance is a
    /// second reader of the same rows and never a second source of truth.
    #[must_use]
    pub fn new(db: DBProvider<DbError>, limits: &crate::config::LimitsConfig) -> Self {
        let policies = policy_repo::PolicyObjectRepo::new(db.clone(), limits);
        Self { db, policies }
    }

    /// Schedule a migration (`inst-ms-api`, `inst-ms-deltas`, `inst-ms-emit`,
    /// `inst-ms-return`).
    ///
    /// # Errors
    /// [`schedule_in`]'s, exactly.
    pub async fn schedule(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        request: ScheduleRequest,
        stamp: AuditStamp,
    ) -> Result<MigrationSchedule, DomainError> {
        let scope = scope.clone();
        let policies = self.policies.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<MigrationSchedule, DomainError, _>(move |txn| {
                Box::pin(async move {
                    Box::pin(schedule_in(
                        txn, &policies, &scope, tenant_id, request, stamp,
                    ))
                    .await
                })
            })
            .await;
        outcome.map_err(|err| {
            err.into_domain(|infra| {
                DomainError::Internal(format!(
                    "bss-pricing: migration schedule transaction: {infra}"
                ))
            })
        })
    }

    /// Read one schedule (§5's `GET`).
    ///
    /// # Errors
    /// [`DomainError::Internal`] on a storage failure.
    pub async fn load(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        migration_id: Uuid,
    ) -> Result<Option<MigrationRecord>, DomainError> {
        let conn = self.db.conn().map_err(|e| {
            DomainError::Internal(format!("bss-pricing: migration read connection: {e}"))
        })?;
        migration_repo::load(&conn, scope, tenant_id, migration_id)
            .await
            .map_err(|e| repo_failure(&e))
    }

    /// Cancel a `scheduled` or `in_progress` run (`inst-mst-cancel`,
    /// `inst-mst-cancel-inflight`, `inst-mg-cancel`, D-34, M3).
    ///
    /// # Errors
    /// [`DomainError::NotFound`] when no such schedule exists;
    /// [`DomainError::MigrationCompleted`] when the run finished first (D-34's
    /// one named refusal); [`DomainError::LifecycleForbidden`] when it was
    /// already cancelled; [`DomainError::Internal`] on a storage failure.
    pub async fn cancel(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        migration_id: Uuid,
        stamp: AuditStamp,
    ) -> Result<MigrationRecord, DomainError> {
        let scope = scope.clone();
        let (_, outcome) = self
            .db
            .db()
            .in_transaction::<MigrationRecord, DomainError, _>(move |txn| {
                Box::pin(
                    async move { cancel_in(txn, &scope, tenant_id, migration_id, stamp).await },
                )
            })
            .await;
        outcome.map_err(|err| {
            err.into_domain(|infra| {
                DomainError::Internal(format!(
                    "bss-pricing: migration cancel transaction: {infra}"
                ))
            })
        })
    }
}

/// What a caller must supply to schedule a migration.
#[derive(Clone, Debug)]
pub struct ScheduleRequest {
    /// **Client-supplied** (`inst-ms-api`): the idempotency key of M2.
    pub migration_id: Uuid,
    /// The retiring side.
    pub source_plan_id: PlanId,
    /// The published target.
    pub target_plan_id: PlanId,
    /// When the migration takes effect.
    pub effective_at: DateTime<Utc>,
    /// `all`, or a subscription filter.
    pub scope_json: JsonValue,
}

/// One migration scheduled, in the caller's transaction.
///
/// # The order, and why each step is where it is
///
/// 1. **Idempotency first.** A replay of a `migration_id` that already exists
///    returns the stored schedule *before* anything is validated or recomputed
///    — `inst-ms-api`'s "returns the original schedule, never a second one".
///    Re-validating first would let a retry fail because the world moved after
///    the original succeeded, which is precisely what an idempotency key exists
///    to prevent.
/// 2. **The cheap total refusals** — distinct plans, then the target's lifecycle
///    state (`inst-mg-target`). Both are permanent for this world state.
/// 3. **The notice period** (D-49), measured from *this* commit as the
///    announcement instant.
/// 4. **The deltas** (`inst-ms-deltas`), and the refusal if any block: an
///    unresolved blocking delta **never persists a schedule** (§7), so this must
///    precede the insert.
/// 5. **The row, the event, the record** — in that order and one transaction, so
///    a schedule Subscriptions never hears about cannot exist.
///
/// # Errors
/// [`DomainError::MigrationTargetInvalid`] for a non-published target or a
/// self-migration; [`DomainError::MigrationNoticeTooShort`] inside the tenant's
/// notice period; [`DomainError::MigrationBlocked`] on unresolved blocking
/// deltas; [`DomainError::NotFound`] when the source plan has no current
/// revision; [`DomainError::Internal`] on a storage failure.
pub async fn schedule_in(
    txn: &DbTx<'_>,
    policies: &policy_repo::PolicyObjectRepo,
    scope: &AccessScope,
    tenant_id: Uuid,
    request: ScheduleRequest,
    stamp: AuditStamp,
) -> Result<MigrationSchedule, DomainError> {
    let now = stamp.recorded_at;

    // 1. The replay arm, before any validation. See the doc above.
    if let Some(existing) = migration_repo::load(txn, scope, tenant_id, request.migration_id)
        .await
        .map_err(|e| repo_failure(&e))?
    {
        // Bound before the struct literal because `record` moves `existing`, and
        // the field order clippy wants would otherwise read it after the move.
        let deltas = stored_delta_report(&existing);
        return Ok(MigrationSchedule {
            record: existing,
            created: false,
            deltas,
            subjects_unresolved: true,
        });
    }

    // 2.
    ensure_distinct_plans(request.source_plan_id.get(), request.target_plan_id.get())?;
    let target = plan_repo::load_current(txn, scope, tenant_id, request.target_plan_id)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| {
            DomainError::MigrationTargetInvalid(format!(
                "migration target plan {} has no current revision; a migration may only land \
                 subscribers on a published plan",
                request.target_plan_id
            ))
        })?;
    ensure_target_publishable(request.target_plan_id.get(), target.lifecycle_state)?;

    let source = plan_repo::load_current(txn, scope, tenant_id, request.source_plan_id)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "current plan revision".to_owned(),
            id: request.source_plan_id.to_string(),
        })?;

    // 3. D-49. The announcement instant is this commit: that is the moment a
    // subscriber could first have learned of the change.
    let policy = policies
        .authoring_policy_on(txn, scope, tenant_id)
        .await
        .map_err(|e| repo_failure(&e))?;
    let notice: NoticePeriod = policy.migration_notice_period();
    notice.ensure_honoured(now, request.effective_at)?;

    // 4. The deltas. The subject set is empty and unresolvable in this system;
    // see the module doc for why that is reported rather than hidden.
    let shape = target_shape(txn, scope, tenant_id, request.target_plan_id).await?;
    let subjects: Vec<SubscriptionFacts> = Vec::new();
    let locks = ContractLockSet::fail_closed();
    let deltas = analyze(&subjects, &shape, &locks);
    deltas.ensure_schedulable()?;

    // 5.
    let scheduled = migration_repo::insert_or_load(
        txn,
        scope,
        NewMigration {
            migration_id: request.migration_id,
            tenant_id,
            source_plan_id: request.source_plan_id,
            source_revision: source.revision,
            target_plan_id: request.target_plan_id,
            effective_at: request.effective_at,
            announced_at: now,
            scope: request.scope_json,
            delta_report: delta_report_json(&deltas),
            created_by: stamp.actor_principal_id,
            created_at: now,
        },
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    // A replay that lost the race to a concurrent create: the row exists, so the
    // event and the record are already there too and must not be duplicated.
    if !scheduled.created {
        let deltas = stored_delta_report(&scheduled.record);
        return Ok(MigrationSchedule {
            record: scheduled.record,
            created: false,
            deltas,
            subjects_unresolved: true,
        });
    }

    outbox_repo::enqueue(
        txn,
        scope,
        NewOutboxEvent::plan_migration_scheduled(
            tenant_id,
            &PlanMigrationScheduledPayload {
                migration_id: request.migration_id,
                source_plan_id: request.source_plan_id,
                source_revision: source.revision,
                target_plan_id: request.target_plan_id,
                effective_at: request.effective_at,
                scope: scheduled.record.scope.clone(),
                excluded_subscription_refs: deltas.excluded().into_iter().collect(),
                exclusions_unresolved: deltas.locks_unresolved,
                // D-39, CONFIRMED 2026-08-07. The rule rides the contract.
                entry_phase_is_first_non_trial: true,
                correlation_id: stamp.correlation_id,
            },
            now,
        ),
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    audit_repo::append(
        txn,
        scope,
        NewAuditEntry {
            tenant_id,
            chain_id: audit_repo::plan_chain(request.source_plan_id),
            recorded_at: now,
            actor_principal_id: stamp.actor_principal_id,
            action: AuditAction::Migrate,
            subject_kind: AuditSubjectKind::PlanRevision,
            subject_ref: audit_repo::plan_revision_ref(request.source_plan_id, source.revision),
            before_state: None,
            after_state: Some(json!({
                "migrationId": request.migration_id,
                "targetPlanId": request.target_plan_id.get(),
                "effectiveAt": request.effective_at,
                "state": MigrationState::Scheduled.as_str(),
            })),
            approval_ref: None,
            correlation_id: stamp.correlation_id,
        },
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    Ok(MigrationSchedule {
        record: scheduled.record,
        created: true,
        deltas,
        subjects_unresolved: true,
    })
}

/// One migration cancelled, in the caller's transaction.
async fn cancel_in(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    tenant_id: Uuid,
    migration_id: Uuid,
    stamp: AuditStamp,
) -> Result<MigrationRecord, DomainError> {
    let current = migration_repo::load(txn, scope, tenant_id, migration_id)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "migration".to_owned(),
            id: migration_id.to_string(),
        })?;

    // D-34's refusal, asked of the machine so `completed` answers in its own
    // words and `cancelled` answers in the machine's.
    current.state.ensure_cancellable()?;

    // The partial sets exist only for a run that had started (D-34); a
    // `scheduled` cancel attempted nothing.
    let partial = (current.state == MigrationState::InProgress).then(|| {
        json!({
            "stoppedAt": stamp.recorded_at,
            // The catalog never held a subscription's state, so "already
            // migrated" is not a set it can enumerate - which is exactly why
            // D-34 says already-migrated subscriptions are unaffected: nothing
            // here has to compensate them.
            "migrated": "held by Subscriptions",
            "notAttempted": "held by Subscriptions",
        })
    });

    let cancelled = migration_repo::cancel(
        txn,
        scope,
        tenant_id,
        migration_id,
        current.state,
        partial,
        stamp.recorded_at,
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    audit_repo::append(
        txn,
        scope,
        NewAuditEntry {
            tenant_id,
            chain_id: audit_repo::plan_chain(current.source_plan_id),
            recorded_at: stamp.recorded_at,
            actor_principal_id: stamp.actor_principal_id,
            action: AuditAction::Migrate,
            subject_kind: AuditSubjectKind::PlanRevision,
            subject_ref: audit_repo::plan_revision_ref(
                current.source_plan_id,
                current.source_revision,
            ),
            before_state: Some(json!({
                "migrationId": migration_id,
                "state": current.state.as_str(),
            })),
            after_state: Some(json!({
                "migrationId": migration_id,
                "state": MigrationState::Cancelled.as_str(),
            })),
            approval_ref: None,
            correlation_id: stamp.correlation_id,
        },
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    Ok(cancelled)
}

/// Read the target's published shape (`inst-md-published`, `inst-md-boundary`).
///
/// **Published state only.** A draft revision is nobody's truth, and a delta
/// report computed against one would describe a migration that cannot happen:
/// `plan_repo::load_current` resolves through `uq_pricing_plan_current`, so what
/// this reads is by construction the revision consumers resolve.
///
/// The **frequency** axis of K3's boundary comes from the plan revision rather
/// than the price row — `pricing_price` carries currency and region on its
/// canonical key and the billing frequency lives on `pricing_plan` — so a
/// boundary is assembled from both. That is why checking "currency and region"
/// alone would have been the easy bug: the two facts live in different tables and
/// only one of them is on the key.
///
/// # Errors
/// [`DomainError::Internal`] on a storage failure.
async fn target_shape(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    target_plan_id: PlanId,
) -> Result<TargetShape, DomainError> {
    let current = plan_repo::load_current(runner, scope, tenant_id, target_plan_id)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "current plan revision".to_owned(),
            id: target_plan_id.to_string(),
        })?;
    let frequency = current
        .frequency
        .map_or_else(|| "unspecified".to_owned(), |f| f.to_string());

    let mut covered: Vec<Boundary> =
        price_repo::load_scope_keys_for_plan(runner, scope, tenant_id, target_plan_id)
            .await
            .map_err(|e| repo_failure(&e))?
            .into_iter()
            .map(|(_price_id, key)| Boundary {
                currency: key.currency().to_string(),
                region: key.region().to_string(),
                frequency: frequency.clone(),
            })
            .collect();
    covered.sort();
    covered.dedup();

    let revision = i64::try_from(current.revision).map_err(|_| {
        DomainError::Internal(format!(
            "plan {target_plan_id} stands at revision {}, which no column can address",
            current.revision
        ))
    })?;
    let rules = plan_addon_rule::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(plan_addon_rule::Column::TenantId.eq(tenant_id))
                .add(plan_addon_rule::Column::PlanId.eq(target_plan_id.get()))
                .add(plan_addon_rule::Column::PlanRevision.eq(revision)),
        )
        .all(runner)
        .await
        .map_err(|e| {
            repo_failure(&RepoError::Db(format!(
                "read the target plan's add-on rules: {e}"
            )))
        })?;

    let offered: BTreeSet<Uuid> = rules.iter().map(|r| r.addon_sku_id).collect();
    let required: BTreeSet<Uuid> = rules
        .iter()
        .filter(|r| r.required)
        .map(|r| r.addon_sku_id)
        .collect();

    // The target's grants, at last read rather than reported empty (D-253).
    // `load_current` above already returned them; the phase set is the second
    // read this needs, because a phased plan's grants differ per phase and
    // `grant_totals` takes the **minimum** across them -- see its doc for why
    // that is the only reading that cannot under-report a loss.
    let schedule =
        plan_shape_repo::load_phase_set(runner, scope, tenant_id, target_plan_id, current.revision)
            .await
            .map_err(|e| repo_failure(&e))?;

    Ok(TargetShape {
        covered,
        offered_addon_sku_ids: offered.into_iter().collect(),
        required_addon_sku_ids: required.into_iter().collect(),
        grants: grant_totals(&current.entitlement_grants, &schedule),
    })
}

/// Render a delta report for the `jsonb` column.
fn delta_report_json(report: &DeltaReport) -> JsonValue {
    json!({
        "locksUnresolved": report.locks_unresolved,
        "excluded": report.excluded().into_iter().collect::<Vec<_>>(),
        "deltas": report
            .deltas
            .iter()
            .map(|d| json!({
                "subscriptionRef": d.subscription_ref,
                "kind": d.kind.as_str(),
                "blocking": d.kind.is_blocking(),
            }))
            .collect::<Vec<_>>(),
        // See `infra::migration`'s module doc: an empty delta set here means
        // "nobody could be enumerated", not "nobody has a problem".
        "subjectsUnresolved": true,
    })
}

/// Read a stored report back well enough for the surfaces to render it.
///
/// Deliberately **not** a full round trip: the stored `jsonb` is the schedule-time
/// evidence and the surfaces render it verbatim from
/// [`MigrationRecord::delta_report`]. What this reconstructs is only the two flags
/// a replay's caller needs, so a replay does not silently report a *recomputed*
/// report as the stored one.
fn stored_delta_report(record: &MigrationRecord) -> DeltaReport {
    DeltaReport {
        deltas: Vec::new(),
        locks_unresolved: record
            .delta_report
            .get("locksUnresolved")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true),
    }
}
