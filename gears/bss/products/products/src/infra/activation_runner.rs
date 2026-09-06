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
    /// `gear.rs` fills this from the **boot** configuration through
    /// `ProductsRuntime::reference_freshness` (P-D-137; it read
    /// `ProductsConfig::default()` until then).
    pub(crate) reference_freshness: std::time::Duration,
    /// The same resolver the REST publish door asks (P-D-141), so a usage
    /// SKU's scheduled publish resolves its `usageTypeRef` too (P-D-157):
    /// `Unavailable` joins the `deferred` set, `Unresolved` fails the run.
    pub(crate) usage_type_resolver:
        std::sync::Arc<dyn crate::infra::usage_types::UsageTypeResolver>,
}

/// The context the scheduled lane resolves a `usageTypeRef` under: the
/// gear's own system principal in the row's tenant, the shape the broker
/// producer lane already uses. It carries no caller token, so a collector
/// client that only forwards a bearer answers `Unavailable` here — which
/// **defers** the row (P-D-131's fail-closed reading) rather than publishing
/// a ref nobody resolved.
fn system_security_context(tenant_id: Uuid) -> toolkit_security::SecurityContext {
    #[allow(clippy::expect_used)]
    toolkit_security::SecurityContext::builder()
        .subject_id(crate::gear::system_actor_ref())
        .subject_type("bss-products.system")
        .subject_tenant_id(tenant_id)
        .build()
        .expect("both required builder fields are set unconditionally above")
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
    apply_held_composition_clears(&conn, &scope, tenant_id, now, ctx, actor_ref).await?;
    Ok(())
}

/// The `(sku_id, signal_ref)` a held `composition_clear` signal names, or
/// `None` for a record that is some other signal — or one this lane cannot
/// read. **An unreadable one is named, never skipped in silence**: the clear
/// it carried will not re-evaluate, and a bare `continue` was the one place
/// a held clear could be lost without a line.
fn held_composition_clear(
    tenant_id: Uuid,
    record: &crate::infra::storage::entity::approval::Model,
) -> Option<(Uuid, Uuid)> {
    let snapshot = match serde_json::from_str::<serde_json::Value>(&record.content_snapshot) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(
                event = "composition_clear_signal_unreadable",
                %tenant_id,
                approval_id = %record.approval_id,
                %error,
                "bss-products: a held system signal's snapshot does not parse; the clear it \
                 carried is not re-evaluated"
            );
            return None;
        }
    };
    if snapshot["act"].as_str() != Some("composition_clear") {
        return None;
    }
    let id_at = |key: &str| snapshot[key].as_str().and_then(|s| s.parse::<Uuid>().ok());
    if let (Some(sku_id), Some(signal_ref)) = (id_at("sku_id"), id_at("signal_ref")) {
        Some((sku_id, signal_ref))
    } else {
        tracing::warn!(
            event = "composition_clear_signal_unreadable",
            %tenant_id,
            approval_id = %record.approval_id,
            "bss-products: a held composition_clear signal names no sku_id/signal_ref pair; \
             the clear it carried is not re-evaluated"
        );
        None
    }
}

/// The held composition clears, re-evaluated once their head goes clean
/// (`dod-composition-clear`: *"the clear re-evaluates when the head next
/// goes clean … without the signal being re-sent"*; P-D-148). A held signal is
/// its still-open `system_signal` record; the clear that applies it consumes
/// it. A binding is not re-resolved on this lane — `03` §7 row 22's gap.
async fn apply_held_composition_clears(
    conn: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    now: DateTime<Utc>,
    ctx: &ActivationContext,
    actor_ref: Uuid,
) -> Result<(), RepoError> {
    let open = repo::open_system_signals(conn, scope, tenant_id).await?;
    for record in open {
        let Some((sku_id, signal_ref)) = held_composition_clear(tenant_id, &record) else {
            continue;
        };
        match skus::try_apply_composition_clear(
            &ctx.db, &ctx.sink, tenant_id, sku_id, signal_ref, actor_ref, now,
        )
        .await
        {
            Ok(skus::ClearOutcome::Cleared { published_version }) => tracing::info!(
                event = "composition_clear_applied",
                %tenant_id,
                %sku_id,
                %signal_ref,
                published_version,
                "bss-products: a held composition clear ran on the next clean head"
            ),
            Ok(_) => {}
            Err(error) => tracing::warn!(
                %tenant_id,
                %sku_id,
                %signal_ref,
                error = %composition_clear_error(&error),
                "bss-products: a held composition clear could not run this pass"
            ),
        }
    }
    Ok(())
}

fn composition_clear_error(error: &skus::HeadActError) -> String {
    match error {
        skus::HeadActError::Refused(refusal) => refusal.code().to_owned(),
        skus::HeadActError::Vanished => "head vanished".to_owned(),
        skus::HeadActError::Db(db) => format!("door storage failed: {db}"),
    }
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
    if matches!(finish, RunFinish::Deferred { .. }) {
        // A held run consumed nothing: the claim goes with it, so the next
        // sweep claims the same `(lane, transition)` afresh instead of
        // replaying a "deferred" answer as applied (P-D-157 — the probe that
        // drove a usage SKU through the lane found exactly that replay).
        repo::release_idempotency_claim(
            runner,
            scope,
            tenant_id,
            &claim.endpoint,
            &claim.client_key,
        )
        .await?;
        return Ok(finish);
    }
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
            // The pre-transaction resolve, as the REST door does it (P-D-157):
            // a usage SKU's ref is judged before the publish runs, under the
            // gear's system principal; `Unavailable` defers the row through
            // `publish_refusal_is_transient`, `Unresolved` fails it.
            let binding = match resolve_usage_type_for_scheduled_publish(
                runner,
                scope,
                tenant_id,
                ctx,
                drive.row.entity_id,
            )
            .await
            {
                Ok(binding) => binding,
                Err(refused) => {
                    return Ok(map_sku_door(
                        Err(skus::HeadActError::Refused(refused)),
                        drive.attempt,
                        ctx.budget,
                    ));
                }
            };
            let outcome = skus::run_publish(
                runner,
                &inputs,
                drive.gate,
                GateMode::PreAuthorized(ApprovalId::new(drive.row.approval_ref)),
                &ctx.sink,
                skus::PublishOperands {
                    binding: binding.as_ref(),
                    ..skus::PublishOperands::default()
                },
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
            flip_product_retired(runner, scope, tenant_id, drive.row).await
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

/// The Product flip. It does **not** consult the 07 predicate
/// (**P-D-137**): [`crate::api::rest::reference::evaluate_reference`] is
/// SKU-keyed and a Product has no watermark. The guard is the children's
/// states (P-D-115); the children are retired by then.
async fn flip_product_retired(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    row: &scheduled_transition::Model,
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

/// Resolve a scheduled publish's `usageTypeRef` the way the REST door does
/// before its transaction (`resolve_usage_type_before_publish`): a SKU with no
/// ref, or no head, resolves to nothing and never calls the collector.
///
/// # Errors
///
/// The door's own refusal — `USAGE_TYPE_UNRESOLVED` or `USAGE_TYPE_UNAVAILABLE`
/// — for [`map_sku_door`] to classify.
async fn resolve_usage_type_for_scheduled_publish(
    runner: &(impl toolkit_db::secure::DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    ctx: &ActivationContext,
    sku_id: Uuid,
) -> Result<Option<crate::domain::recognized::UsageTypeBinding>, crate::domain::error::DomainError>
{
    let Ok(Some(head)) = repo::find_sku(runner, scope, tenant_id, sku_id).await else {
        return Ok(None);
    };
    let Some(usage_type_ref) = head.usage_type_ref.as_deref() else {
        return Ok(None);
    };
    let answer = ctx
        .usage_type_resolver
        .resolve(&system_security_context(tenant_id), usage_type_ref)
        .await;
    crate::domain::recognized::judge_usage_type(answer, usage_type_ref).map(Some)
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
                transient: publish_refusal_is_transient(error.code()),
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

/// The one publish refusal the scheduled lane **defers** rather than fails
/// (`dod-classification-errors`, `design/03` §4: *"on the scheduled lane
/// joins the runner's `deferred` set"*). `USAGE_TYPE_UNAVAILABLE` is the
/// collector not answering (**P-D-131**) — a transient dependency under the
/// attempt budget, `DeferralPopulation::TransientDependency`. Every other
/// door code is a decision about the SKU and fails the run.
///
/// **Reachable since P-D-157.** P-D-146 measured that this lane entered
/// `run_publish` without resolving `usageTypeRef`, the resolve living in the
/// REST door with the caller's `SecurityContext`. The runner now carries the
/// resolver (`ActivationContext::usage_type_resolver`) and resolves under the
/// gear's system principal (`resolve_usage_type_for_scheduled_publish`), so a
/// collector that does not answer lands the row here, in the `deferred` set.
///
/// @cpt-dod:cpt-cf-bss-products-dod-classification-errors:p1
pub(crate) fn publish_refusal_is_transient(code: &str) -> bool {
    code == "USAGE_TYPE_UNAVAILABLE"
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
