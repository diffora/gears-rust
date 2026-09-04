//! The activation runner's sweep — claim, pin, finish — hosted from
//! `gear.rs`'s lifecycle loop (**P-D-113**).
//!
//! @cpt-dod:cpt-cf-bss-products-dod-activation-runner:p1
//! @cpt-dod:cpt-cf-bss-products-dod-runner-failure-posture:p1

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

use bss_products_sdk::models::EntityKind;

use crate::api::rest::skus::{self, HeadActError, HeadActInputs, MutationOutcome};
use crate::domain::activation::{
    ACTIVATION_LANE, AttemptBudget, CASCADE_LEG_LANE, ClaimDecision, ClaimLease,
    DeferralPopulation, DoorRefusal, RunFinish, ScheduledActivation, StoredRunState,
    claim_decision, classify_door_refusal, internal_lane_body, verify_activation_pin,
};
use crate::domain::approval::StoredApprovalGate;
use crate::domain::cascade::{PARENT_FLIP_HELD_REASON, parent_flip_clears};
use crate::domain::concurrency::InternalRevision;
use crate::domain::deprecation::no_orphan_at_flip;
use crate::domain::governance::{ApprovalId, EntityRef, GateMode, GateSubject};
use crate::domain::idempotency;
use crate::domain::retention::RetentionHold;
use crate::domain::retirement::{FlipPredicate, flip_guard, replacement_chain_broken_reason};
use crate::domain::transition;
use crate::infra::broker::EventSink;
use crate::infra::idempotency::{ClaimVerdict, IdempotencyClaimInput, record_idempotency_answer};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::scheduled_transition;
use crate::infra::storage::repo::{self};

/// What one sweep needs, built in `gear.rs` from the same boot state the
/// doors use — the worker is infra and does not read `api::rest::ApiState`.
pub(crate) struct ActivationContext {
    /// Provider the discovery, claims and finishes run on.
    pub(crate) db: toolkit_db::DBProvider<toolkit_db::DbError>,
    /// [`ProductsConfig::activation_claim_lease_secs`].
    pub(crate) lease: ClaimLease,
    /// [`ProductsConfig::activation_attempt_budget`].
    pub(crate) budget: AttemptBudget,
    /// [`ProductsConfig::retirement_held_alert_hours`] — read here so the
    /// tick never inlines 72; the alert itself lands with §2.7.
    pub(crate) retirement_held_alert_hours: u32,
    /// The same sink the Foundation doors enqueue through.
    pub(crate) sink: EventSink,
    /// [`ProductsConfig::idempotency_retention_hours`], for the `internal:`
    /// lane claim.
    pub(crate) idempotency_retention_hours: u32,
}

/// One tick: discover due rows, claim or reclaim, verify the pin, finish.
/// A failed sweep is returned to the caller to `warn!`; later tenants
/// continue.
///
/// # Errors
///
/// [`RepoError`] when every tenant's pass failed.
pub(crate) async fn sweep(
    ctx: &ActivationContext,
    actor_ref: Uuid,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), RepoError> {
    let tenants = {
        let conn = ctx
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("activation sweep connection: {e}")))?;
        repo::tenants_with_due_transitions(&conn, &AccessScope::allow_all(), now, ctx.lease).await?
    };
    tracing::debug!(
        tenants = tenants.len(),
        alert_hours = ctx.retirement_held_alert_hours,
        "bss-products: activation sweep"
    );
    let total = tenants.len();
    let mut failed = 0_usize;
    let mut last_err: Option<RepoError> = None;
    for tenant in tenants {
        if cancel.is_cancelled() {
            return Ok(());
        }
        if let Err(e) = sweep_tenant(ctx, tenant, actor_ref, now, cancel).await {
            failed += 1;
            last_err = Some(note_tenant_failure(tenant, e));
        }
    }
    match last_err {
        Some(e) if failed == total && total > 0 => Err(e),
        _ => Ok(()),
    }
}

fn note_tenant_failure(tenant: Uuid, error: RepoError) -> RepoError {
    tracing::error!(
        %tenant,
        error = %error,
        "bss-products: activation pass failed; later tenants continue"
    );
    error
}

async fn sweep_tenant(
    ctx: &ActivationContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    now: DateTime<Utc>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), RepoError> {
    let conn = ctx
        .db
        .conn()
        .map_err(|e| RepoError::Db(format!("activation tenant connection: {e}")))?;
    let scope = AccessScope::for_tenant(tenant_id);
    let rows = repo::list_due_transitions(&conn, &scope, tenant_id, now, ctx.lease).await?;
    for row in rows {
        if cancel.is_cancelled() {
            return Ok(());
        }
        run_one(&conn, &scope, tenant_id, &row, now, ctx, actor_ref).await?;
    }
    emit_retirement_held_alerts(&conn, &scope, tenant_id, now, ctx, actor_ref).await?;
    Ok(())
}

/// `retirement_held` — a deferral older than
/// [`ActivationContext::retirement_held_alert_hours`] (**P-D-133**).
/// The gear's alert channel is `tracing::warn!` with a stable event name;
/// the same fact is an audit row (`dod-lifecycle-audit`).
async fn emit_retirement_held_alerts(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    now: DateTime<Utc>,
    ctx: &ActivationContext,
    actor_ref: Uuid,
) -> Result<(), RepoError> {
    let cutoff = now - chrono::Duration::hours(i64::from(ctx.retirement_held_alert_hours));
    let held = repo::list_held_deferrals(runner, scope, tenant_id, cutoff).await?;
    for row in held {
        let reason = row.outcome_reason.clone().unwrap_or_default();
        tracing::warn!(
            event = "retirement_held",
            tenant_id = %tenant_id,
            transition_id = %row.transition_id,
            entity_id = %row.entity_id,
            outcome_reason = %reason,
            hours = ctx.retirement_held_alert_hours,
            "bss-products: retirement held past configured hours"
        );
        repo::write_eventless_act_audit(
            runner,
            scope,
            repo::AuditCommon {
                audit_id: Uuid::now_v7(),
                tenant_id,
                actor_ref,
                action: "retirement_held".to_owned(),
                subject_kind: row.entity_kind.clone(),
                reason: Some(reason),
                correlation_id: None,
                written_at: now,
            },
            row.entity_id,
            None,
        )
        .await?;
    }
    Ok(())
}

async fn run_one(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &scheduled_transition::Model,
    now: DateTime<Utc>,
    ctx: &ActivationContext,
    actor_ref: Uuid,
) -> Result<(), RepoError> {
    let state = stored_state(&row.state);
    match claim_decision(state, row.at, row.claimed_at, now, ctx.lease) {
        ClaimDecision::Skip => return Ok(()),
        ClaimDecision::ReclaimLease => {
            let _ = repo::reclaim_expired_lease(
                runner,
                scope,
                tenant_id,
                row.transition_id,
                now,
                ctx.lease,
            )
            .await?;
            return Ok(());
        }
        ClaimDecision::Claim => {
            if !repo::claim_due_transition(runner, scope, tenant_id, row.transition_id, now).await?
            {
                return Ok(());
            }
        }
    }

    let finish = pin_finish(runner, scope, tenant_id, row, now, ctx, actor_ref).await?;
    let _ = repo::finish_scheduled_transition(
        runner,
        scope,
        tenant_id,
        row.transition_id,
        &finish,
        now,
    )
    .await?;
    Ok(())
}

async fn pin_finish(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &scheduled_transition::Model,
    now: DateTime<Utc>,
    ctx: &ActivationContext,
    actor_ref: Uuid,
) -> Result<RunFinish, RepoError> {
    let claimed = repo::find_scheduled_transition(runner, scope, tenant_id, row.transition_id)
        .await?
        .ok_or_else(|| RepoError::Db("claimed row vanished".to_owned()))?;
    let candidate =
        repo::gate_candidate_by_id(runner, scope, tenant_id, ApprovalId::new(row.approval_ref))
            .await?;
    let pin = ScheduledActivation {
        row_approval_ref: row.approval_ref,
        record_id: candidate
            .as_ref()
            .map_or(row.approval_ref, |c| c.approval_id.get()),
        record_consumed: candidate
            .as_ref()
            .is_some_and(|c| c.state == crate::domain::approval::ApprovalState::Consumed),
    };
    let Some(kind) = parse_kind(&row.entity_kind) else {
        return Ok(RunFinish::Failed {
            reason: format!("unknown entity_kind {}", row.entity_kind),
        });
    };
    let subject = GateSubject::entity_publish(EntityRef {
        tenant_id,
        entity_kind: kind,
        entity_id: row.entity_id,
    });
    let expected = InternalRevision::new(candidate.as_ref().map_or(0, |c| c.internal_revision));
    let gate = StoredApprovalGate::scheduled_flip(
        candidate.clone().into_iter().collect(),
        ApprovalId::new(row.approval_ref),
    );
    let finish = verify_activation_pin(&gate, subject, expected, &pin);
    let finish = spend_transient_budget(finish, claimed.attempt, ctx.budget);
    if !matches!(finish, RunFinish::Applied) {
        return Ok(finish);
    }

    let lane = activation_lane(candidate.as_ref(), row.entity_id);
    let claim = IdempotencyClaimInput::new(
        lane,
        row.transition_id.to_string(),
        idempotency::payload_digest(&serde_json::json!({})),
        now,
        ctx.idempotency_retention_hours,
    );
    match crate::infra::idempotency::claim_idempotency(runner, scope, tenant_id, &claim).await? {
        ClaimVerdict::Replay { .. } => return Ok(RunFinish::Applied),
        ClaimVerdict::Refused(error) => {
            return Ok(RunFinish::Failed {
                reason: error.code().to_owned(),
            });
        }
        ClaimVerdict::Proceed => {}
    }

    let finish = drive_door(
        runner,
        scope,
        tenant_id,
        ctx,
        DoorDrive {
            row,
            kind,
            expected,
            now,
            actor_ref,
            gate: &gate,
            attempt: claimed.attempt,
        },
    )
    .await?;
    record_idempotency_answer(
        runner,
        scope,
        tenant_id,
        &claim,
        StatusCode::OK,
        &internal_lane_body(row.transition_id, &finish),
    )
    .await?;
    Ok(finish)
}

struct DoorDrive<'a> {
    row: &'a scheduled_transition::Model,
    kind: EntityKind,
    expected: InternalRevision,
    now: DateTime<Utc>,
    actor_ref: Uuid,
    gate: &'a StoredApprovalGate,
    attempt: i32,
}

async fn drive_door(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    ctx: &ActivationContext,
    drive: DoorDrive<'_>,
) -> Result<RunFinish, RepoError> {
    match (drive.row.kind.as_str(), drive.kind) {
        ("publish", EntityKind::Sku) => {
            let inputs = HeadActInputs {
                scope: scope.clone(),
                tenant_id,
                sku_id: drive.row.entity_id,
                actor_ref: drive.actor_ref,
                expected: drive.expected.get(),
                now: drive.now,
                claim: None,
            };
            let outcome = skus::run_publish(
                runner,
                &inputs,
                drive.gate,
                GateMode::PreAuthorized(ApprovalId::new(drive.row.approval_ref)),
                &ctx.sink,
            )
            .await;
            Ok(map_sku_door(outcome, drive.attempt, ctx.budget))
        }
        ("retire", EntityKind::Sku) => {
            flip_sku_retired(
                runner,
                scope,
                tenant_id,
                drive.row,
                drive.expected,
                drive.now,
            )
            .await
        }
        ("retire", EntityKind::Product) => {
            flip_product_retired(
                runner,
                scope,
                tenant_id,
                drive.row,
                drive.expected,
                drive.now,
            )
            .await
        }
        (kind, _) => Ok(RunFinish::Failed {
            reason: format!("no Foundation door wired for scheduled {kind}"),
        }),
    }
}

async fn flip_sku_retired(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &scheduled_transition::Model,
    expected: InternalRevision,
    now: DateTime<Utc>,
) -> Result<RunFinish, RepoError> {
    if let Err(held) = flip_guard(FlipPredicate::FreshZero) {
        return Ok(crate::domain::activation::defer_flip_guard(&held));
    }
    // @cpt-dod:cpt-cf-bss-products-dod-replaced-by:p1 — live pointers defer.
    let pointers = repo::find_skus_pointing_at(runner, scope, tenant_id, row.entity_id).await?;
    if !pointers.is_empty() {
        return Ok(RunFinish::Deferred {
            population: DeferralPopulation::FlipGuard,
            reason: replacement_chain_broken_reason(&pointers),
        });
    }
    if let Err(error) = transition::guard(
        bss_products_sdk::models::LifecycleState::Deprecated,
        bss_products_sdk::models::LifecycleState::Retired,
    ) {
        return Ok(RunFinish::Failed {
            reason: error.code().to_owned(),
        });
    }
    match repo::retire_sku_head(runner, scope, tenant_id, row.entity_id, expected.get(), now)
        .await?
    {
        repo::HeadWrite::Applied => Ok(RunFinish::Applied),
        repo::HeadWrite::Unmatched => Ok(RunFinish::Failed {
            reason: "retire flip unmatched".to_owned(),
        }),
    }
}

async fn flip_product_retired(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &scheduled_transition::Model,
    expected: InternalRevision,
    now: DateTime<Utc>,
) -> Result<RunFinish, RepoError> {
    let children = repo::find_skus_of_product(runner, scope, tenant_id, row.entity_id).await?;
    let states: Vec<_> = children.iter().map(|c| c.lifecycle_state).collect();
    // @cpt-dod:cpt-cf-bss-products-dod-no-orphan:p1 — re-check at flip.
    if !no_orphan_at_flip(&states) {
        return Ok(RunFinish::Deferred {
            population: DeferralPopulation::FlipGuard,
            reason: RetentionHold::REASON.to_owned(),
        });
    }
    // @cpt-dod:cpt-cf-bss-products-dod-cascade-parent-path:p1 — all children terminal.
    if !parent_flip_clears(&states) {
        return Ok(RunFinish::Deferred {
            population: DeferralPopulation::FlipGuard,
            reason: PARENT_FLIP_HELD_REASON.to_owned(),
        });
    }
    if let Err(held) = flip_guard(FlipPredicate::FreshZero) {
        return Ok(crate::domain::activation::defer_flip_guard(&held));
    }
    if let Err(error) = transition::guard(
        bss_products_sdk::models::LifecycleState::Deprecated,
        bss_products_sdk::models::LifecycleState::Retired,
    ) {
        return Ok(RunFinish::Failed {
            reason: error.code().to_owned(),
        });
    }
    match repo::retire_product_head(runner, scope, tenant_id, row.entity_id, expected.get(), now)
        .await?
    {
        repo::HeadWrite::Applied => Ok(RunFinish::Applied),
        repo::HeadWrite::Unmatched => Ok(RunFinish::Failed {
            reason: "retire flip unmatched".to_owned(),
        }),
    }
}

fn map_sku_door(
    outcome: Result<MutationOutcome, HeadActError>,
    attempt: i32,
    budget: AttemptBudget,
) -> RunFinish {
    match outcome {
        Ok(MutationOutcome::Applied { .. } | MutationOutcome::Replay { .. }) => RunFinish::Applied,
        Err(HeadActError::Refused(error)) => match classify_door_refusal(
            DoorRefusal {
                code: error.code(),
                transient: false,
            },
            attempt,
            budget,
        ) {
            Ok(finish) => finish,
            Err(refusal) => RunFinish::Failed {
                reason: refusal.code.to_owned(),
            },
        },
        Err(HeadActError::Vanished) => RunFinish::Failed {
            reason: "head vanished".to_owned(),
        },
        Err(HeadActError::Db(error)) => RunFinish::Failed {
            reason: format!("door storage failed: {error}"),
        },
    }
}

/// A cascade leg's record names the parent; the row names the child
/// (**P-D-105**). That mismatch is the only signal the store carries.
fn activation_lane(
    candidate: Option<&crate::domain::approval::CandidateApproval>,
    entity_id: Uuid,
) -> &'static str {
    let Some(candidate) = candidate else {
        return ACTIVATION_LANE;
    };
    if candidate
        .subject
        .reference
        .ends_with(&entity_id.to_string())
    {
        ACTIVATION_LANE
    } else {
        CASCADE_LEG_LANE
    }
}

fn spend_transient_budget(finish: RunFinish, attempt: i32, budget: AttemptBudget) -> RunFinish {
    match finish {
        RunFinish::Deferred {
            population: DeferralPopulation::TransientDependency,
            reason,
        } if attempt >= budget.max => RunFinish::Failed {
            reason: format!("transient budget exhausted after {attempt} attempts: {reason}"),
        },
        other => other,
    }
}

fn stored_state(raw: &str) -> StoredRunState {
    match raw {
        "pending" => StoredRunState::Pending,
        "running" => StoredRunState::Running,
        "deferred" => StoredRunState::Deferred,
        _ => StoredRunState::Terminal,
    }
}

fn parse_kind(raw: &str) -> Option<EntityKind> {
    match raw {
        "product" => Some(EntityKind::Product),
        "sku" => Some(EntityKind::Sku),
        _ => None,
    }
}

#[cfg(test)]
#[path = "activation_runner_tests.rs"]
mod activation_runner_tests;
