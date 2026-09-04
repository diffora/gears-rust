//! The activation runner's sweep — claim, pin, finish — hosted from
//! `gear.rs`'s lifecycle loop (**P-D-113**).
//!
//! @cpt-dod:cpt-cf-bss-products-dod-activation-runner:p1
//! @cpt-dod:cpt-cf-bss-products-dod-runner-failure-posture:p1

use axum::http::StatusCode;
use chrono::{DateTime, SecondsFormat, Utc};
use toolkit_db::secure::{AccessScope, TxConfig};
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
    /// [`ProductsConfig::reference_freshness`] — the 07 predicate's cadence.
    /// The runtime does not yet carry the boot value; `activation_tick`
    /// fills this from `ProductsConfig::default().reference_freshness()`.
    pub(crate) reference_freshness: std::time::Duration,
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
    persist_finish(ctx, scope, tenant_id, row, &finish, now, actor_ref).await?;
    Ok(())
}

/// The finish, its audit row, and — on an applied retire — the head write
/// plus the flip event, in one transaction (`dod-lifecycle-events`,
/// `dod-lifecycle-audit`).
async fn persist_finish(
    ctx: &ActivationContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &scheduled_transition::Model,
    finish: &RunFinish,
    now: DateTime<Utc>,
    actor_ref: Uuid,
) -> Result<(), RepoError> {
    let sink = ctx.sink.clone();
    let scope = scope.clone();
    let finish = finish.clone();
    let row = row.clone();
    ctx.db
        .db()
        .transaction_with_retry::<(), FinishTxError, _, _>(
            TxConfig::default(),
            finish_contention,
            move |tx| {
                let sink = sink.clone();
                let scope = scope.clone();
                let finish = finish.clone();
                let row = row.clone();
                Box::pin(async move {
                    if matches!(finish, RunFinish::Applied) && row.kind == "retire" {
                        apply_retire_flip(tx, &scope, tenant_id, &row, now, &sink, actor_ref)
                            .await?;
                    }
                    let _ = repo::finish_scheduled_transition(
                        tx,
                        &scope,
                        tenant_id,
                        row.transition_id,
                        &finish,
                        now,
                    )
                    .await?;
                    let reason = match &finish {
                        RunFinish::Applied => None,
                        RunFinish::Failed { reason } | RunFinish::Deferred { reason, .. } => {
                            Some(reason.clone())
                        }
                    };
                    repo::write_eventless_act_audit(
                        tx,
                        &scope,
                        repo::AuditCommon {
                            audit_id: Uuid::now_v7(),
                            tenant_id,
                            actor_ref,
                            action: format!("activation.{}", finish.state().as_str()),
                            subject_kind: row.entity_kind.clone(),
                            reason,
                            correlation_id: None,
                            written_at: now,
                        },
                        row.entity_id,
                        None,
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
        .map_err(RepoError::from)
}

/// `transaction_with_retry` requires `From<DbError>`; [`RepoError`] has none.
enum FinishTxError {
    Repo(RepoError),
    Db(toolkit_db::DbError),
}

impl From<toolkit_db::DbError> for FinishTxError {
    fn from(error: toolkit_db::DbError) -> Self {
        Self::Db(error)
    }
}

impl From<RepoError> for FinishTxError {
    fn from(error: RepoError) -> Self {
        Self::Repo(error)
    }
}

impl From<FinishTxError> for RepoError {
    fn from(error: FinishTxError) -> Self {
        match error {
            FinishTxError::Repo(inner) => inner,
            FinishTxError::Db(inner) => Self::Db(format!("activation finish transaction: {inner}")),
        }
    }
}

fn finish_contention(error: &FinishTxError) -> Option<&sea_orm::DbErr> {
    match error {
        FinishTxError::Repo(RepoError::Driver { source, .. }) => Some(source),
        FinishTxError::Repo(_) | FinishTxError::Db(_) => None,
    }
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
    let expected = InternalRevision::new(candidate.as_ref().map_or(0, |c| c.internal_revision));
    // The pin rides the subject since P-D-125 row 52 (strand B, merged
    // 2026-09-04); this call was reconstructed at that merge.
    let subject = GateSubject::entity_publish(
        EntityRef {
            tenant_id,
            entity_kind: kind,
            entity_id: row.entity_id,
        },
        expected,
    );
    let gate = StoredApprovalGate::scheduled_flip(
        candidate.clone().into_iter().collect(),
        ApprovalId::new(row.approval_ref),
    );
    let finish = verify_activation_pin(&gate, subject, &pin);
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
                drive.now,
                ctx.reference_freshness,
            )
            .await
        }
        ("retire", EntityKind::Product) => {
            flip_product_retired(
                runner,
                scope,
                tenant_id,
                drive.row,
                drive.now,
                ctx.reference_freshness,
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
    now: DateTime<Utc>,
    freshness: std::time::Duration,
) -> Result<RunFinish, RepoError> {
    // @cpt-dod:cpt-cf-bss-products-dod-replaced-by:p1 — live pointers defer.
    let pointers = repo::find_skus_pointing_at(runner, scope, tenant_id, row.entity_id).await?;
    if !pointers.is_empty() {
        return Ok(RunFinish::Deferred {
            population: DeferralPopulation::FlipGuard,
            reason: replacement_chain_broken_reason(&pointers),
        });
    }
    if let Some(held) =
        consult_flip_guard(runner, scope, tenant_id, row.entity_id, now, freshness).await?
    {
        return Ok(held);
    }
    if let Err(error) = transition::guard(
        bss_products_sdk::models::LifecycleState::Deprecated,
        bss_products_sdk::models::LifecycleState::Retired,
    ) {
        return Ok(RunFinish::Failed {
            reason: error.code().to_owned(),
        });
    }
    Ok(RunFinish::Applied)
}

async fn flip_product_retired(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &scheduled_transition::Model,
    now: DateTime<Utc>,
    freshness: std::time::Duration,
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
    if let Some(held) =
        consult_flip_guard(runner, scope, tenant_id, row.entity_id, now, freshness).await?
    {
        return Ok(held);
    }
    if let Err(error) = transition::guard(
        bss_products_sdk::models::LifecycleState::Deprecated,
        bss_products_sdk::models::LifecycleState::Retired,
    ) {
        return Ok(RunFinish::Failed {
            reason: error.code().to_owned(),
        });
    }
    Ok(RunFinish::Applied)
}

/// @cpt-dod:cpt-cf-bss-products-dod-flip-guard:p1 — the 07 reader, not a literal.
async fn consult_flip_guard(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_id: Uuid,
    now: DateTime<Utc>,
    freshness: std::time::Duration,
) -> Result<Option<RunFinish>, RepoError> {
    let eval = crate::api::rest::reference::evaluate_reference(
        runner, scope, tenant_id, entity_id, now, freshness,
    )
    .await?;
    let predicate = predicate_from_evaluation(&eval);
    let blocking = blocking_producers(&eval);
    if let Err(mut held) = flip_guard(predicate) {
        held.blocking_producers = blocking;
        return Ok(Some(crate::domain::activation::defer_flip_guard(&held)));
    }
    Ok(None)
}

fn predicate_from_evaluation(
    eval: &crate::api::rest::reference::ReferenceEvaluation,
) -> FlipPredicate {
    use crate::api::rest::reference::ProducerVerdict;
    if eval.no_producers {
        return FlipPredicate::NoProducers;
    }
    if !eval.referenced {
        return FlipPredicate::FreshZero;
    }
    if eval
        .per_producer
        .iter()
        .any(|(_, v)| *v == ProducerVerdict::ConservativelyReferencedNeverReceived)
    {
        return FlipPredicate::NeverReceived;
    }
    if eval
        .per_producer
        .iter()
        .any(|(_, v)| *v == ProducerVerdict::ConservativelyReferencedStale)
    {
        return FlipPredicate::Stale;
    }
    FlipPredicate::FreshPositive
}

fn blocking_producers(eval: &crate::api::rest::reference::ReferenceEvaluation) -> Vec<String> {
    use crate::api::rest::reference::ProducerVerdict;
    eval.per_producer
        .iter()
        .filter(|(_, v)| *v != ProducerVerdict::FreshZero)
        .map(|(name, _)| name.clone())
        .collect()
}

/// Head write + `*RetirementEffective` on the finish transaction.
/// @cpt-dod:cpt-cf-bss-products-dod-lifecycle-events:p1
async fn apply_retire_flip(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &scheduled_transition::Model,
    now: DateTime<Utc>,
    sink: &EventSink,
    actor_ref: Uuid,
) -> Result<(), RepoError> {
    let expected = {
        // The pin already decided Applied; the head write re-pins the
        // revision it read then. Re-read so the UPDATE filter is the
        // current revision, not the approval's.
        match row.entity_kind.as_str() {
            "sku" => repo::find_sku(runner, scope, tenant_id, row.entity_id)
                .await?
                .map(|h| h.internal_revision),
            "product" => repo::find_product(runner, scope, tenant_id, row.entity_id)
                .await?
                .map(|h| h.internal_revision),
            _ => None,
        }
    };
    let Some(expected) = expected else {
        return Err(RepoError::Db("retire flip head vanished".to_owned()));
    };
    let write = match row.entity_kind.as_str() {
        "sku" => {
            repo::retire_sku_head(runner, scope, tenant_id, row.entity_id, expected, now).await?
        }
        "product" => {
            repo::retire_product_head(runner, scope, tenant_id, row.entity_id, expected, now)
                .await?
        }
        other => {
            return Err(RepoError::Db(format!(
                "retire flip has no head writer for {other}"
            )));
        }
    };
    if write == repo::HeadWrite::Unmatched {
        return Err(RepoError::Db(
            "retire flip unmatched under the finish".to_owned(),
        ));
    }
    announce_retirement_effective(runner, scope, tenant_id, row, sink, actor_ref).await
}

async fn announce_retirement_effective(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &scheduled_transition::Model,
    sink: &EventSink,
    actor_ref: Uuid,
) -> Result<(), RepoError> {
    let (entity_kind, payload, internal_revision, lifecycle_state, from_version, replaced_by) =
        match row.entity_kind.as_str() {
            "sku" => {
                let head = repo::find_sku(runner, scope, tenant_id, row.entity_id)
                    .await?
                    .ok_or_else(|| RepoError::Db("retired SKU vanished after flip".to_owned()))?;
                (
                    crate::infra::events::EntityKind::Sku.as_str(),
                    crate::infra::events::SKU_RETIREMENT_EFFECTIVE_PAYLOAD_TYPE,
                    head.internal_revision,
                    head.lifecycle_state.as_str(),
                    head.published_version,
                    head.replaced_by_sku_id,
                )
            }
            "product" => {
                let head = repo::find_product(runner, scope, tenant_id, row.entity_id)
                    .await?
                    .ok_or_else(|| {
                        RepoError::Db("retired Product vanished after flip".to_owned())
                    })?;
                (
                    crate::infra::events::EntityKind::Product.as_str(),
                    crate::infra::events::PRODUCT_RETIREMENT_EFFECTIVE_PAYLOAD_TYPE,
                    head.internal_revision,
                    head.lifecycle_state.as_str(),
                    head.published_version,
                    None,
                )
            }
            other => {
                return Err(RepoError::Db(format!(
                    "retire flip has no event for {other}"
                )));
            }
        };
    let core = crate::infra::events::EventBodyCore {
        tenant_id: row.tenant_id,
        entity_kind,
        entity_id: row.entity_id,
        internal_revision,
        lifecycle_state,
    };
    crate::infra::events::enqueue_retired(
        sink,
        runner,
        row.entity_id,
        payload,
        crate::infra::events::RetiredEventBody {
            core: &core,
            from_version,
            reason: row.retirement_reason.clone().unwrap_or_default(),
            replaced_by,
            effective_at: row.at.to_rfc3339_opts(SecondsFormat::Secs, true),
            must_migrate_by: None,
        },
        actor_ref,
    )
    .await
    .map_err(|e| RepoError::Db(format!("enqueue retirement-effective: {e}")))
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
