//! The mass-repricing run's apply lane — `inst-mr-apply`, `inst-mr-validate-scope`
//! (`design/12-operator-efficiency.md` §3; D-134).
//!
//! This module owes what `api::rest::repricing_runs`' own module doc names as
//! undone: a run that reached `awaiting_approval` or `committing` stops there
//! today. [`apply_run_in`] is the writer that takes it the rest of the way.
//!
//! # The shape, settled 2026-08-11 and not re-opened here
//!
//! Three decisions cost real time to establish and are taken as given:
//!
//! * Successor rows are written through [`commit_supersession`], **not**
//!   [`crate::infra::supersession::supersede_in`] — the latter's per-call
//!   registry request, outbox enqueue, audit append and materiality
//!   evaluation are exactly the four things a repricing run does once for the
//!   whole run (materiality, already built) or once per plan (the other
//!   three), never once per row.
//! * The row-local rule set is [`plan_supersession`], run once per row,
//!   unchanged from the interactive supersession's own compose step.
//! * One `CatalogVersion` per plan, requested once after every row of that
//!   plan has committed — never per row.
//!
//! # The order inside one plan's transaction
//!
//! 1. For every pending row of the plan: resolve the predecessor **fresh, in
//!    this transaction**, run [`plan_supersession`] (the row-local rules plus
//!    the touched key's window overlap/gap/trailing-void check) at
//!    [`ChangeoverMoment::Commit`] — the stricter, batching-delay floor
//!    `api::rest::repricing_runs`' own module doc names as belonging to "a
//!    commit that does not exist", which this now is — stage the successor
//!    draft, and write it through [`commit_supersession`]. The journal row
//!    flips `pending -> applied` and one audit record lands on the plan's
//!    chain, in the same transaction as the write it describes (D-14).
//! 2. **Once**, over the plan's row set **as this transaction just left it**:
//!    [`crate::infra::publish::assemble_from`] against the plan's current
//!    revision (never `assemble`, which demands an open draft a repricing
//!    plan does not have), [`crate::infra::publish::rule_params`], then
//!    [`run_publish_rules`]. This is the step Step 0 exists for — it depends
//!    on the transaction seeing the writes step 1 just made, uncommitted.
//! 3. On a violation: `Err`, and every write above rolls back with it. The
//!    caller marks every one of this plan's rows `failed` with the one
//!    reason, in a **separate** transaction — this module doc's own
//!    consequence, decided rather than discovered mid-build: a rolled-back
//!    transaction cannot also be the one recording that it rolled back.
//! 4. On success: one `CatalogVersion` request for the plan, `record_pending`
//!    against [`SubjectRef::Plan`], and — now that the pending ref exists —
//!    the two outbox events per row step 1 could not enqueue yet, each keyed
//!    `(run_id, price_id)` so a redrive cannot double-fire either.
//!
//! # What the aggregate pass actually guards here
//!
//! Step 2's re-run of `run_publish_rules` cannot fail *because of this run's
//! own effect*. A markup/discount/fixed adjustment moves only a row's amount,
//! and every rule `run_publish_rules` registers that reads `amount_minor`,
//! `package_price_minor` or `unit_price_minor` is row-local — already run,
//! pre-commit, inside [`plan_supersession`]'s own `price_row_rules()` call.
//! None of the genuinely aggregate-only rules (phase coverage, descriptor
//! completeness, region declaration, window coverage) is sensitive to an
//! amount's value at all. So for this lane the post-commit pass is not a
//! soundness guard over what this transaction just wrote — it cannot catch a
//! bad *repricing*, because a repricing has no way to produce one. What it
//! guards against is **concurrency**: that nothing *else* changed the plan's
//! structure between this run's row selection and this transaction's commit —
//! a phase dropped, a region retired, a stray draft authored on an untouched
//! key. That is a real property worth its cost even though this run's own
//! adjustment can never be the thing that trips it.
//!
//! # The bulk lock is not taken here, and taking it would not close the hole
//!
//! `inst-bs-commit` gives the run's bulk lock over its rows starting at entry
//! to `committing`, and the module doc `api::rest::repricing_runs` carries
//! names it beside this apply's other debts. It is not built in this task:
//! neither the brief's Interfaces list nor its numbered steps name
//! `bulk_repo::take_locks`, and building it was judged out of this task's
//! scope rather than forgotten. The consequence is real and is named rather
//! than hidden: without the lock, a concurrent interactive edit on the same
//! plan could leave a stray `draft` row on the plan when this transaction's
//! aggregate pass reads `CANDIDATE_ROW_STATES = [Published, Draft]` back, and
//! that stray row would be judged as though this act were about to publish
//! it. The failure mode this leaves open is a false aggregate refusal (or,
//! rarer, a false pass) attributable to a row this apply never touched — not
//! a corrupted write, because [`price_repo::insert_successor_draft_on`] and
//! [`price_repo::commit_supersession_rows`] each re-read their own
//! preconditions and refuse rather than misapply when the world has moved.
//!
//! **And the lock would not close it even built as D-134 describes it.**
//! `take_locks` is scoped to the rows *this run selected*; the aggregate
//! pass's candidate set is the *whole plan's* published-and-draft rows. A
//! stray draft on a key the run never targeted sits outside any lock scoped
//! to "the run's rows" — `tests/sqlite_repricing_apply.rs`'s own atomicity
//! test exploits exactly that key. D-134's soundness sentence does not hold
//! as written for that case; that is a design question for a later task, not
//! one this module can paper over by taking a lock that would not answer it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use toolkit_db::secure::{AccessScope, DBRunner, DbTx};
use toolkit_db::{DBProvider, DbError};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::domain::audit::{AuditAction, AuditStamp, AuditSubjectKind};
use crate::domain::bulk::{BulkState, JournalState};
use crate::domain::error::DomainError;
use crate::domain::events::CatalogEvent;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::overlay::Adjustment;
use crate::domain::ports::{CatalogVersionRegistryV1, registry_failure};
use crate::domain::price_record::PriceContent;
use crate::domain::publish::rules::run_publish_rules;
use crate::domain::read_model::SubjectRef;
use crate::domain::repricing::project_row;
use crate::domain::scope_key::{PlanId, ScopeKey};
use crate::domain::supersession::{ChangeoverMoment, NamedWindow, plan_supersession};
use crate::domain::window::WindowInterval;
use crate::infra::publish::{assemble_from, rule_params};
use crate::infra::storage::repo::outbox_repo::{
    NewOutboxEvent, PriceUpdatedPayload, PriceWindowTransitionPayload,
};
use crate::infra::storage::repo::repricing_journal_repo::JournalRow;
use crate::infra::storage::repo::{
    NewAuditEntry, NewPriceDraft, PendingVersionRow, PolicyObjectRepo, audit_repo, bulk_repo,
    catalog_version_ref_repo, outbox_repo, plan_repo, price_repo, repricing_journal_repo,
    window_repo,
};
use crate::infra::storage::repo_failure;
use crate::infra::supersession::{SupersessionCommit, commit_supersession, supersession_unit_ref};

/// What one call to [`apply_run_in`] found once it finished — the run's whole
/// journal tally, not only what this particular call decided.
///
/// **Not "this call's delta".** [`apply_run_in`] is the re-drive as well as
/// the first attempt (`inst-mp-journal`), and a re-drive over a journal that
/// is already fully decided does no writes at all — so a delta-shaped
/// `RunOutcome` would answer `{0, 0}` for a run that plainly applied rows,
/// which is not what "the same outcome" means for an idempotent replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOutcome {
    /// How many of the run's journal rows now read `applied`.
    pub applied: usize,
    /// How many now read `failed`.
    pub failed: usize,
}

/// Apply a mass-repricing run — `inst-mr-apply`'s per-plan commit and
/// `inst-mr-validate-scope`'s aggregate pass, over whatever the run's journal
/// still holds `pending`.
///
/// # What this call may find the run standing at, and what it does about it
///
/// * **`awaiting_approval`**: reachable only because a second principal has
///   just approved the run's batch unit — nothing else moves a repricing run
///   off that state. This call performs `inst-bs-commit`'s own edge itself,
///   `awaiting_approval -> committing`, before doing anything else: the two
///   callers this function has (`api::rest::repricing_runs`' non-material
///   path, which already left the run `committing`, and `api::rest::approvals`'
///   approved-bulk-operation arm, which has not) would otherwise each need
///   their own copy of that one-line transition.
/// * **`committing`**: proceeds directly to the per-plan work below.
/// * **A terminal state**: every row is already decided (or the run never
///   selected one). No write happens; [`RunOutcome`] answers the journal's
///   standing tally. This is what makes a second call over an already-decided
///   run idempotent rather than an error.
/// * **`validating`**: a caller error — nothing advances a run to `committing`
///   or `awaiting_approval` without evaluating materiality first, so this
///   state can only mean `apply_run_in` was invoked before that happened.
///   [`DomainError::Internal`], because there is no operator remedy for it.
///
/// # Grouping and the per-plan transaction
///
/// [`repricing_journal_repo::pending_for_run`] is the resume predicate
/// (`inst-mp-journal`): a row already `applied` or `failed` is frozen by the
/// journal's own trigger and is never re-read here. The pending set is
/// grouped by the plan its price row sits on — [`crate::domain::repricing`]'s
/// own module doc gives the reason a selector's `plan_id` axis and D-134's
/// transaction unit are the same column — and each plan's rows commit or fail
/// together in one transaction, per [`apply_plan_in`].
///
/// A plan whose transaction returns `Err` has every one of its rows marked
/// `failed` with that error's rendering as the **one shared reason**
/// (`inst-mr-validate-scope`), in a transaction of its own: the plan's
/// transaction already rolled back by the time this runs, so the failure
/// record cannot live inside it.
///
/// # Errors
/// [`DomainError::NotFound`] when `operation_id` names no run in scope;
/// [`DomainError::Internal`] when the run is not in a state this call may act
/// on, when its stored report cannot be read back into an adjustment or a
/// changeover, or on a storage failure recording a plan's failure or the
/// run's own final transition. A plan-level refusal — a row-local rule
/// violation, a window conflict, the aggregate pass, a lost registry request —
/// never surfaces here: it is absorbed into that plan's journal rows and
/// counted in the returned [`RunOutcome`].
pub async fn apply_run_in(
    db: &DBProvider<DbError>,
    policies: &PolicyObjectRepo,
    registry: &Arc<dyn CatalogVersionRegistryV1>,
    ctx: &SecurityContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    operation_id: Uuid,
    stamp: AuditStamp,
) -> Result<RunOutcome, DomainError> {
    let conn = db
        .conn()
        .map_err(|e| DomainError::Internal(format!("bss-pricing: repricing apply: conn: {e}")))?;
    let run = bulk_repo::read(&conn, scope, tenant_id, operation_id)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "repricing run".to_owned(),
            id: operation_id.to_string(),
        })?;

    if run.state.is_terminal() {
        return tally(&conn, scope, tenant_id, operation_id).await;
    }
    if run.state != BulkState::AwaitingApproval && run.state != BulkState::Committing {
        return Err(DomainError::Internal(format!(
            "bss-pricing: apply_run_in called on repricing run {operation_id} in state {}, \
             which is neither awaiting_approval nor committing nor terminal",
            run.state.as_str()
        )));
    }
    if run.state == BulkState::AwaitingApproval {
        // `inst-bs-commit`'s own state edge — `awaiting_approval -> committing`
        // — and the one this apply is the sole owner of taking. **Not** the
        // bulk lock itself: this module does not take `bulk_repo::take_locks`
        // at all (see the module doc's own named gap). See the module doc for
        // why this edge is spent here rather than by either of this
        // function's two callers.
        bulk_repo::advance(
            &conn,
            scope,
            tenant_id,
            operation_id,
            BulkState::Committing,
            run.report.clone(),
            stamp.recorded_at,
        )
        .await
        .map_err(|e| repo_failure(&e))?;
    }

    let adjustment = crate::api::rest::repricing_runs::adjustment_of_report(&run.report)?;
    let changeover = crate::api::rest::repricing_runs::changeover_of_report(&run.report)?;

    let pending = repricing_journal_repo::pending_for_run(&conn, scope, tenant_id, operation_id)
        .await
        .map_err(|e| repo_failure(&e))?;

    let mut by_plan: BTreeMap<PlanId, Vec<JournalRow>> = BTreeMap::new();
    if !pending.is_empty() {
        let ids: Vec<Uuid> = pending.iter().map(|row| row.price_id).collect();
        let plan_of: HashMap<Uuid, PlanId> =
            price_repo::load_plan_ids(&conn, scope, ids.iter().copied())
                .await
                .map_err(|e| repo_failure(&e))?
                .into_iter()
                .collect();
        for row in pending {
            match plan_of.get(&row.price_id) {
                Some(&plan_id) => by_plan.entry(plan_id).or_default().push(row),
                // Unreachable in this schema — a price row is append-only and
                // never deleted — but a journal row cannot be left `pending`
                // forever over an id that cannot be grouped, so it is decided
                // here rather than silently skipped.
                None => {
                    repricing_journal_repo::mark_failed(
                        &conn,
                        scope,
                        tenant_id,
                        operation_id,
                        row.price_id,
                        "bss-pricing: this row's price_id no longer resolves to any plan",
                    )
                    .await
                    .map_err(|e| repo_failure(&e))?;
                }
            }
        }
    }

    for (plan_id, rows) in by_plan {
        let row_ids: Vec<Uuid> = rows.iter().map(|row| row.price_id).collect();
        let scope_for_tx = scope.clone();
        let policies_for_tx = policies.clone();
        let registry_for_tx = Arc::clone(registry);
        let ctx_for_tx = ctx.clone();
        let adjustment_for_tx = adjustment.clone();
        let (_, outcome) = db
            .db()
            .in_transaction::<(), DomainError, _>(move |txn| {
                Box::pin(async move {
                    apply_plan_in(
                        txn,
                        &policies_for_tx,
                        &registry_for_tx,
                        &ctx_for_tx,
                        &scope_for_tx,
                        tenant_id,
                        operation_id,
                        plan_id,
                        &rows,
                        &adjustment_for_tx,
                        changeover,
                        stamp,
                    )
                    .await
                })
            })
            .await;

        if let Err(err) = outcome.map_err(|err| {
            err.into_domain(|infra| {
                DomainError::Internal(format!(
                    "bss-pricing: repricing apply transaction (plan {plan_id}): {infra}"
                ))
            })
        }) {
            // **A separate transaction, never the one that just rolled back**
            // (module doc, and the task-5 brief's own consequence): every row of
            // this plan reads `failed` with one shared reason, the rolled-back
            // transaction's own rendering.
            let reason = failure_reason(&err);
            let scope_for_tx = scope.clone();
            let (_, mark_outcome) = db
                .db()
                .in_transaction::<(), DomainError, _>(move |txn| {
                    let scope = scope_for_tx.clone();
                    let reason = reason.clone();
                    Box::pin(async move {
                        for price_id in row_ids {
                            repricing_journal_repo::mark_failed(
                                txn,
                                &scope,
                                tenant_id,
                                operation_id,
                                price_id,
                                &reason,
                            )
                            .await
                            .map_err(|e| repo_failure(&e))?;
                        }
                        Ok(())
                    })
                })
                .await;
            mark_outcome.map_err(|infra_err| {
                infra_err.into_domain(|infra| {
                    DomainError::Internal(format!(
                        "bss-pricing: repricing apply failure recording (plan {plan_id}): {infra}"
                    ))
                })
            })?;
        }
    }

    let outcome = tally(&conn, scope, tenant_id, operation_id).await?;
    bulk_repo::advance(
        &conn,
        scope,
        tenant_id,
        operation_id,
        if outcome.failed > 0 {
            BulkState::CompletedWithConflicts
        } else {
            BulkState::Completed
        },
        run.report.clone(),
        stamp.recorded_at,
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    Ok(outcome)
}

/// The run's journal, summed into [`RunOutcome`] — [`apply_run_in`]'s answer
/// on every path, not only the one that just wrote.
async fn tally(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    operation_id: Uuid,
) -> Result<RunOutcome, DomainError> {
    let journal = repricing_journal_repo::list_for_run(runner, scope, tenant_id, operation_id)
        .await
        .map_err(|e| repo_failure(&e))?;
    let applied = journal
        .iter()
        .filter(|row| row.state == JournalState::Applied)
        .count();
    let failed = journal
        .iter()
        .filter(|row| row.state == JournalState::Failed)
        .count();
    Ok(RunOutcome { applied, failed })
}

/// The one shared reason a plan's every pending row is marked `failed` with.
///
/// [`DomainError`]'s own `Display` is deliberately terse on
/// [`DomainError::ValidationFailed`] — "N blocking violation(s)", because a
/// route rendering the full report has the structured envelope to put it in
/// and a log line repeating it per rule would be noise. A journal's
/// `failure_reason` is that structured envelope's only substitute here — a
/// plan-transactional refusal has no response body of its own to carry the
/// report in — so this renders every violation's code and detail rather than
/// the bare count `{err}` would give an operator nothing to act on from.
fn failure_reason(err: &DomainError) -> String {
    let DomainError::ValidationFailed(report) = err else {
        return err.to_string();
    };
    let mut rendered = format!("{err}: ");
    let details: Vec<String> = report
        .violations
        .iter()
        .map(|violation| {
            format!(
                "{} ({}): {}",
                violation.code, violation.subject, violation.detail
            )
        })
        .collect();
    rendered.push_str(&details.join("; "));
    rendered
}

/// One row this plan's transaction committed, carrying what the outbox events
/// enqueued after the aggregate pass need — collected in
/// [`apply_plan_in`]'s first pass because neither event can be built until the
/// plan's one `CatalogVersion` request has answered.
struct AppliedRow {
    predecessor_price_id: Uuid,
    successor_price_id: Uuid,
    scope_key: ScopeKey,
    scheduled_window: crate::infra::storage::repo::WindowRecord,
}

/// One plan's whole commit, inside the caller's transaction — [`apply_run_in`]'s
/// per-plan unit, and the body D-134 makes normative.
///
/// See the module doc for the order and the reasons behind it. Every write
/// below lands in `txn`, so a `?` anywhere here rolls every one of them back
/// together with the caller's own transaction wrapper.
#[allow(
    clippy::too_many_arguments,
    reason = "every argument is a precondition apply_run_in already resolved once for the whole \
              run — the runner and its collaborators, the tenant and the run's own identity, the \
              plan and its rows, the adjustment and changeover the report carried, and the stamp \
              — and this function is the one place they are all needed together. Bundling them \
              would name a struct with exactly one reader"
)]
async fn apply_plan_in(
    txn: &DbTx<'_>,
    policies: &PolicyObjectRepo,
    registry: &Arc<dyn CatalogVersionRegistryV1>,
    ctx: &SecurityContext,
    scope: &AccessScope,
    tenant_id: Uuid,
    operation_id: Uuid,
    plan_id: PlanId,
    rows: &[JournalRow],
    adjustment: &Adjustment,
    changeover: DateTime<Utc>,
    stamp: AuditStamp,
) -> Result<(), DomainError> {
    let now = stamp.recorded_at;

    // Every published row of the plan, read **fresh in this transaction** —
    // the predecessor of every row this plan's part of the run selected has
    // to be found here, not carried from `apply_run_in`'s own read minutes
    // earlier and a world away.
    let mut published_by_id: HashMap<_, _> =
        price_repo::load_for_plan(txn, scope, tenant_id, plan_id, &[LifecycleState::Published])
            .await
            .map_err(|e| repo_failure(&e))?
            .into_iter()
            .map(|record| (record.price_id, record))
            .collect();

    let stored_windows = window_repo::list_for_plan(txn, scope, tenant_id, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?;

    let mut applied_rows: Vec<AppliedRow> = Vec::with_capacity(rows.len());
    for row in rows {
        let predecessor = published_by_id.remove(&row.price_id).ok_or_else(|| {
            DomainError::LifecycleForbidden(format!(
                "price {} is no longer the published occupant of its canonical scope key; a \
                 concurrent change moved it after this run selected it, and it cannot be \
                 superseded by this repricing run",
                row.price_id
            ))
        })?;
        let key = predecessor.scope_key.clone();
        let plane: Vec<NamedWindow> = stored_windows
            .iter()
            .filter(|window| window.scope_key == key)
            .map(|window| NamedWindow {
                window_id: window.window_id,
                interval: WindowInterval::new(
                    window.effective_from,
                    window.effective_to,
                    window.state,
                ),
            })
            .collect();

        // The row-local rule set already exists as one call
        // (`domain::supersession::plan_supersession`): `price_row_rules()`,
        // `supersession_rules()` (D-82/D-98) and `compose_windows`'
        // overlap/gap/trailing-void check, over the successor built from the
        // run's own adjustment.
        let projected = project_row(predecessor.clone(), adjustment);
        let successor_content = price_repo::authored_content(
            &key,
            PriceContent {
                supersedes_price_id: Some(predecessor.price_id),
                // `inst-mp-grandfathered`: the selector structurally excludes
                // the retained class, so a row this apply ever reaches never
                // carries a horizon to begin with — cleared explicitly rather
                // than trusted to already be absent, `supersede_in`'s own
                // reason for refusing one outright on this same field.
                grandfather_until: None,
                ..projected.content()
            },
        );

        // `ChangeoverMoment::Commit`: the batching-delay floor
        // `api::rest::repricing_runs`'s module doc names as belonging to "a
        // commit that does not exist" — this transaction is that commit.
        let plan = plan_supersession(
            &predecessor.row,
            &successor_content.row,
            &plane,
            changeover,
            now,
            ChangeoverMoment::Commit,
        )?;

        let (successor, _) = price_repo::insert_successor_draft_on(
            txn,
            scope,
            tenant_id,
            NewPriceDraft {
                price_id: Uuid::now_v7(),
                scope_key: key.clone(),
                content: successor_content,
                created_by: stamp.actor_principal_id,
                created_at_utc: now,
                correlation_id: stamp.correlation_id,
            },
        )
        .await
        .map_err(|e| repo_failure(&e))?;

        let shorten_target = stored_windows
            .iter()
            .find(|window| window.window_id == plan.windows.shorten.window_id)
            .ok_or_else(|| {
                // Unreachable by construction: `compose_windows` picked this id
                // off the very plane just built from `stored_windows`.
                DomainError::Internal(format!(
                    "bss-pricing: repricing apply composed shorten names window {} which is not \
                     on the plane it was composed from",
                    plan.windows.shorten.window_id
                ))
            })?;

        let written = commit_supersession(
            txn,
            scope,
            tenant_id,
            SupersessionCommit::of_plan(
                &plan,
                plan_id,
                predecessor.price_id,
                shorten_target.mutation_seq,
                (successor.price_id, successor.row_version),
                Uuid::now_v7(),
                format!("mass repricing run {operation_id}"),
            ),
            stamp,
        )
        .await
        .map_err(|e| repo_failure(&e))?;

        repricing_journal_repo::mark_applied(
            txn,
            scope,
            tenant_id,
            operation_id,
            row.price_id,
            successor.price_id,
            now,
        )
        .await
        .map_err(|e| repo_failure(&e))?;

        // The audit row §10's own cost breakdown for a repricing run's per-row
        // commit names alongside the row, the outbox and the journal: one
        // record per row, on the plan's chain, in this same transaction
        // (D-14) — `supersede_in`'s identical record, minted here instead
        // because this act is not that one.
        audit_repo::append(
            txn,
            scope,
            NewAuditEntry {
                tenant_id,
                chain_id: audit_repo::plan_chain(plan_id),
                recorded_at: now,
                actor_principal_id: stamp.actor_principal_id,
                action: AuditAction::Publish,
                subject_kind: AuditSubjectKind::PriceUnit,
                subject_ref: supersession_unit_ref(plan_id, &key, changeover),
                before_state: Some(serde_json::json!({
                    "predecessorPriceId": predecessor.price_id,
                    "predecessorState": LifecycleState::Published.as_str(),
                    "successorPriceId": successor.price_id,
                    "successorState": LifecycleState::Draft.as_str(),
                    "scopeKey": key.to_string(),
                })),
                after_state: Some(serde_json::json!({
                    "predecessorPriceId": predecessor.price_id,
                    "predecessorState": LifecycleState::Superseded.as_str(),
                    "successorPriceId": successor.price_id,
                    "successorState": LifecycleState::Published.as_str(),
                    "scopeKey": key.to_string(),
                })),
                // `None`: this row's own approval, if the run was material, is
                // the run's — `AuditSubjectKind::BulkOperation`'s record, not a
                // per-row `price_unit` unit this run never opens.
                approval_ref: None,
                correlation_id: stamp.correlation_id,
            },
        )
        .await
        .map_err(|e| repo_failure(&e))?;

        applied_rows.push(AppliedRow {
            predecessor_price_id: predecessor.price_id,
            successor_price_id: successor.price_id,
            scope_key: key,
            scheduled_window: written.scheduled,
        });
    }

    // **Once**, over the plan's row set as this transaction just left it —
    // Step 0's premise is what makes this a real post-commit read rather than
    // a re-statement of what was just asked to be true.
    let current = plan_repo::load_current(txn, scope, tenant_id, plan_id)
        .await
        .map_err(|e| repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "current plan revision".to_owned(),
            id: plan_id.to_string(),
        })?;
    let revision_no = current.revision;
    let revision_lifecycle = current.lifecycle_state;
    let shape = assemble_from(txn, scope, tenant_id, plan_id, current, now).await?;
    let params = rule_params(policies, txn, scope, tenant_id, &shape).await?;
    let report = run_publish_rules(&shape, &params);
    if !report.is_publishable() {
        return Err(DomainError::ValidationFailed(report));
    }

    // One `CatalogVersion` per **plan**, not per row (`PublishService::commit`'s
    // own keying, `SubjectRef::Plan`) — D-47's batching then coalesces across
    // plans and runs, which is `inst-mr-coalesce`'s ask.
    let pending = registry
        .request_version(ctx, &repricing_request_id(tenant_id, operation_id, plan_id))
        .await
        .map_err(|e| registry_failure(&e))?;
    catalog_version_ref_repo::record_pending(
        txn,
        scope,
        PendingVersionRow::for_subject(
            tenant_id,
            pending.pending_ref.clone(),
            &SubjectRef::Plan(plan_id.get()),
            Some(revision_no),
            Some(revision_lifecycle),
            now,
        ),
    )
    .await
    .map_err(|e| repo_failure(&e))?;

    // The two events every applied row owes, now that the pending ref they
    // both carry exists. `(run_id, price_id)` dedup keys, per the task-5
    // brief: keyed on the journal's own identity rather than on the freshly
    // minted successor/window ids, so a dedup key survives a hypothetical
    // retry that mints different ones for the same journal row.
    for applied in &applied_rows {
        outbox_repo::enqueue(
            txn,
            scope,
            NewOutboxEvent {
                dedup_key: format!(
                    "repricing/{operation_id}/{}/price",
                    applied.predecessor_price_id
                ),
                ..NewOutboxEvent::price_updated(
                    tenant_id,
                    &PriceUpdatedPayload {
                        plan_id,
                        price_id: applied.successor_price_id,
                        scope_key: applied.scope_key.to_string(),
                        supersedes_price_id: applied.predecessor_price_id,
                        changeover,
                        pending_version_ref: pending.pending_ref.clone(),
                        correlation_id: stamp.correlation_id,
                    },
                    now,
                )
            },
        )
        .await
        .map_err(|e| repo_failure(&e))?;

        outbox_repo::enqueue(
            txn,
            scope,
            NewOutboxEvent {
                dedup_key: format!(
                    "repricing/{operation_id}/{}/window",
                    applied.predecessor_price_id
                ),
                ..NewOutboxEvent::price_window_mutation(
                    tenant_id,
                    CatalogEvent::PriceWindowScheduled,
                    &PriceWindowTransitionPayload {
                        window_id: applied.scheduled_window.window_id,
                        plan_id,
                        price_id: applied.successor_price_id,
                        effective_from: applied.scheduled_window.effective_from,
                        effective_to: applied.scheduled_window.effective_to,
                        correlation_id: stamp.correlation_id,
                    },
                    now,
                    &format!("repricing/{operation_id}"),
                )
            },
        )
        .await
        .map_err(|e| repo_failure(&e))?;
    }

    Ok(())
}

/// The registry's idempotency handle for one plan's repricing-run commit.
///
/// Deterministic in `(tenant, run, plan)`, `unit_request_id`'s and
/// `publish_request_id`'s own reason: a commit refused after the request —
/// storage fault, a lost race — is retried under the **same** handle rather
/// than orphaning one, and two plans of one run (or the same plan across two
/// runs) never share a handle.
fn repricing_request_id(tenant_id: Uuid, operation_id: Uuid, plan_id: PlanId) -> String {
    format!("repricing-run/{tenant_id}/{operation_id}/{plan_id}")
}

#[cfg(test)]
mod step0_probe {
    //! **Step 0 of the task-5 brief, kept as a documented premise test rather
    //! than deleted.** The whole shape below — run the plan-level aggregate
    //! pass *inside* the same transaction that just wrote the plan's
    //! successor rows, over the state [`crate::infra::publish::assemble_from`]
    //! reads back — rests on one assumption: that a transaction sees its own
    //! uncommitted writes when that function reads rows back. That is
    //! ordinary READ COMMITTED behaviour and every reader of
    //! `infra/publish.rs` already reasons as though it holds, but nothing in
    //! this crate's own harness had exercised it in as many words. This test
    //! is that exercise: insert a price row and read it back through
    //! `assemble_from`, both inside one `DbTx`, neither committed.
    //!
    //! It passed (`cargo test -p bss-pricing --lib
    //! infra::repricing::step0_probe -- --nocapture`), so the design proceeds
    //! as written. Kept rather than thrown away: it is cheap insurance against
    //! a future change to the transaction runner or the isolation level
    //! silently invalidating the one premise the rest of this module is built
    //! on.

    use chrono::{TimeZone, Utc};
    use sea_orm_migration::MigratorTrait;
    use toolkit_db::migration_runner::run_migrations_for_testing;
    use toolkit_db::secure::AccessScope;
    use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
    use uuid::Uuid;

    use crate::domain::error::DomainError;
    use crate::domain::money::{CurrencyCode, MinorAmount};
    use crate::domain::price_record::PriceContent;
    use crate::domain::price_row::{ModelKind, PriceRow};
    use crate::domain::scope_key::{
        ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
    };
    use crate::infra::publish::assemble_from;
    use crate::infra::storage::migrations::Migrator;
    use crate::infra::storage::repo::{
        NewPlanDraft, NewPriceDraft, PlanRepo, plan_repo, price_repo,
    };
    use crate::infra::storage::repo_failure;

    #[tokio::test]
    async fn a_transaction_sees_its_own_uncommitted_price_row_through_assemble_from() {
        let db = connect_db("sqlite::memory:", ConnectOpts::default())
            .await
            .expect("connect in-memory sqlite");
        run_migrations_for_testing(&db, Migrator::migrations())
            .await
            .expect("run migrator");
        let provider = DBProvider::<DbError>::new(db);

        let tenant_id = Uuid::from_u128(0x7e);
        let scope = AccessScope::for_tenant(tenant_id);
        let plan_id = PlanId::new(Uuid::from_u128(0x9a));
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();

        // The plan and its first draft revision, committed ahead of the probe:
        // what is under test is the price row's visibility, not the plan's.
        // `PlanRepo::create_draft` opens its own transaction for the pair of
        // rows it writes (the revision and its audit record) — `create_draft_on`
        // is the runner-generic body every caller must supply a transaction to.
        PlanRepo::new(provider.clone())
            .create_draft(
                &scope,
                NewPlanDraft {
                    plan_id,
                    tenant_id,
                    created_by: Uuid::from_u128(0x1),
                    created_at_utc: now,
                    sku_id: None,
                    plan_tier: None,
                    billing_cycle: None,
                    frequency: None,
                    plan_tier_override: false,
                    purchase_min_qty: None,
                    purchase_max_qty: None,
                    invoice_grouping_key: None,
                    available_from: None,
                    available_to: None,
                    cloned_from: None,
                    correlation_id: Uuid::from_u128(0x2),
                },
            )
            .await
            .expect("create the plan's first draft");

        let price_id = Uuid::from_u128(0xb_00);
        let scope_for_tx = scope.clone();
        let (_, outcome) = provider
            .db()
            .in_transaction::<bool, DomainError, _>(move |txn| {
                let scope = scope_for_tx.clone();
                Box::pin(async move {
                    let key = ScopeKey::new(
                        plan_id,
                        CurrencyCode::new("EUR").expect("three letters"),
                        Region::new("eu").expect("non-blank"),
                        PhaseId::new(Uuid::from_u128(0xc0)),
                        PriceEligibility::AllSubscriptions,
                        ChargeKind::Recurring,
                        Cohort::None,
                    )
                    .expect("the class pairs with cohort none");
                    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
                    row.amount_minor = Some(MinorAmount::new(1_000).expect("non-negative"));

                    // The write under test: **not yet committed** when the read below
                    // runs, because both are inside this one `txn`.
                    price_repo::create_draft_on(
                        txn,
                        &scope,
                        tenant_id,
                        NewPriceDraft {
                            price_id,
                            scope_key: key,
                            content: PriceContent {
                                row,
                                tax_inclusive: false,
                                tax_category_ref: None,
                                billing_timing: None,
                                proration_contract: None,
                                rounding_policy_ref: None,
                                grandfather_until: None,
                                supersedes_price_id: None,
                            },
                            created_by: Uuid::from_u128(0x1),
                            created_at_utc: now,
                            correlation_id: Uuid::from_u128(0x2),
                        },
                    )
                    .await
                    .map_err(|e| repo_failure(&e))?;

                    // `load_open_draft`, not `load_current`: `load_current` answers
                    // only `published | retired` (`is_current_revision`), and this
                    // probe's plan was never published — it only needs a revision
                    // to assemble against, and the property under test is about the
                    // price row's visibility, not about which revision state reads
                    // it back.
                    let revision = plan_repo::load_open_draft(txn, &scope, tenant_id, plan_id)
                        .await
                        .map_err(|e| repo_failure(&e))?
                        .expect("the plan carries the draft revision just created");

                    // The read under test, in the same transaction as the write
                    // above and before either commits.
                    let shape =
                        assemble_from(txn, &scope, tenant_id, plan_id, revision, now).await?;

                    Ok(shape.rows.iter().any(|record| record.price_id == price_id))
                })
            })
            .await;

        let saw_own_write = outcome.expect("the transaction itself does not fail");
        assert!(
            saw_own_write,
            "a transaction must see its own uncommitted price-row insert through \
             assemble_from, or D-134's design — the aggregate pass over post-commit state, \
             inside the plan's own transaction — is unbuildable as written"
        );
    }
}
