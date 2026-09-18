//! Revision-owned draft-window authoring (D-374).
//!
//! The HTTP door owns idempotency and audit persistence around this function.
//! [`apply_command`] takes the per-plan serial lock, then the compare-and-swap
//! of the draft parent plus the composition that refuses overlap, empty
//! intervals, wrong ownership, captured-baseline drift and illegal live-history
//! edits. Coverage holes are not a save failure.

use std::collections::{BTreeMap, HashSet};

use toolkit_db::secure::{AccessScope, DBRunner};
use uuid::Uuid;

use crate::domain::audit::{AuditAction, AuditStamp};
use crate::domain::concurrency::RowVersion;
use crate::domain::draft_window::{
    DraftWindowAction, DraftWindowEntry, DraftWindowOwner, compose_windows,
};
use crate::domain::error::DomainError;
use crate::domain::scope_key::{PlanId, ScopeKey};
use crate::infra::publish::CANDIDATE_ROW_STATES;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::plan_repo::{
    load_revision, record_revision_mutation, refuse, swap_guard,
};
use crate::infra::storage::repo::plan_shape_repo::plan_revision_bump;
use crate::infra::storage::repo::{
    draft_window_repo, price_repo, window_baseline_repo, window_guard_repo,
};
use crate::infra::storage::repo_failure;

/// One authoring act against a draft window set.
pub enum DraftWindowCommand {
    /// Insert or replace one operation, then compose the whole set.
    Put(DraftWindowEntry),
    /// Drop one operation by id, then compose the remainder.
    Remove { operation_id: Uuid },
    /// Recapture live windows and drop operations the new baseline cannot carry.
    RefreshBaseline,
}

/// Apply one command under the caller's transaction and the draft parent's CAS.
///
/// The returned integer is the new plan **row version**.
///
/// # Errors
/// [`DomainError::DraftWindowContextChanged`] when the owner is no longer an
/// open draft; [`DomainError::StaleVersion`] when `expected_plan_version` is
/// not current; [`DomainError::WindowBaselineChanged`] when the captured live
/// baseline no longer matches committed windows, or an adjust or cancel names a
/// window the captured baseline no longer carries; composition refusals from
/// [`compose_windows`]; storage failures through [`repo_failure`].
pub async fn apply_command(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    expected_plan_version: u64,
    command: DraftWindowCommand,
    stamp: AuditStamp,
) -> Result<u64, DomainError> {
    // Guard owner: apply_command is the draft-window orchestration body.
    // HTTP `run_draft_command` supplies the transaction and must not acquire again.
    window_guard_repo::acquire(runner, scope, owner.tenant_id, owner.plan_id)
        .await
        .map_err(map_repo)?;
    let plan_id = PlanId::new(owner.plan_id);
    let expected = RowVersion::new(expected_plan_version);
    let Some(guard) = swap_guard(owner.tenant_id, plan_id, owner.plan_revision, expected) else {
        return Err(map_repo(
            refuse(
                runner,
                scope,
                owner.tenant_id,
                plan_id,
                owner.plan_revision,
                expected,
            )
            .await,
        ));
    };
    let current = map_repo_result(
        load_revision(runner, scope, owner.tenant_id, plan_id, owner.plan_revision).await,
    )?
    .ok_or_else(|| DomainError::NotFound {
        subject: "plan revision".to_owned(),
        id: format!("{}/{}", owner.plan_id, owner.plan_revision),
    })?;
    if current.row_version != expected || !current.lifecycle_state.is_content_mutable() {
        return Err(map_repo(
            refuse(
                runner,
                scope,
                owner.tenant_id,
                plan_id,
                owner.plan_revision,
                expected,
            )
            .await,
        ));
    }

    let keys = candidate_keys(runner, scope, owner).await?;
    let evaluated_at = stamp.recorded_at;
    if !matches!(command, DraftWindowCommand::RefreshBaseline) {
        refuse_captured_baseline_drift(runner, scope, owner).await?;
    }

    match command {
        DraftWindowCommand::Put(entry) => {
            refuse_stale_baseline(runner, scope, owner, &entry.action).await?;
            let mut entries = map_repo_result(draft_window_repo::list(runner, scope, owner).await)?;
            entries.retain(|row| row.operation_id != entry.operation_id);
            entries.push(entry.clone());
            let baseline = map_repo_result(window_baseline_repo::list(runner, scope, owner).await)?;
            compose_windows(&baseline, &entries, &keys, evaluated_at)?;
            map_repo_result(draft_window_repo::put(runner, scope, owner, &entry).await)?;
        }
        DraftWindowCommand::Remove { operation_id } => {
            map_repo_result(draft_window_repo::remove(runner, scope, owner, operation_id).await)?;
            let entries = map_repo_result(draft_window_repo::list(runner, scope, owner).await)?;
            let baseline = map_repo_result(window_baseline_repo::list(runner, scope, owner).await)?;
            compose_windows(&baseline, &entries, &keys, evaluated_at)?;
        }
        DraftWindowCommand::RefreshBaseline => {
            let captured = map_repo_result(
                window_baseline_repo::snapshot_live(runner, scope, owner.tenant_id, plan_id).await,
            )?;
            map_repo_result(window_baseline_repo::replace(runner, scope, owner, &captured).await)?;
            let entries = map_repo_result(draft_window_repo::list(runner, scope, owner).await)?;
            let mut kept = Vec::new();
            for entry in entries {
                let mut trial = kept.clone();
                trial.push(entry.clone());
                if compose_windows(&captured, &trial, &keys, evaluated_at).is_ok() {
                    kept.push(entry);
                }
            }
            let kept_ids: HashSet<Uuid> = kept.iter().map(|row| row.operation_id).collect();
            let current = map_repo_result(draft_window_repo::list(runner, scope, owner).await)?;
            for row in current {
                if !kept_ids.contains(&row.operation_id) {
                    map_repo_result(
                        draft_window_repo::remove(runner, scope, owner, row.operation_id).await,
                    )?;
                }
            }
        }
    }

    let moved = map_repo_result(plan_revision_bump(runner, scope, guard).await)?;
    if moved == 0 {
        return Err(map_repo(
            refuse(
                runner,
                scope,
                owner.tenant_id,
                plan_id,
                owner.plan_revision,
                expected,
            )
            .await,
        ));
    }
    let updated = map_repo_result(
        load_revision(runner, scope, owner.tenant_id, plan_id, owner.plan_revision).await,
    )?
    .ok_or_else(|| DomainError::NotFound {
        subject: "plan revision".to_owned(),
        id: format!("{}/{}", owner.plan_id, owner.plan_revision),
    })?;
    map_repo_result(
        record_revision_mutation(
            runner,
            scope,
            owner.tenant_id,
            &updated,
            AuditAction::Update,
            expected,
            stamp,
        )
        .await,
    )?;
    Ok(updated.row_version.get())
}

/// Compare captured baseline IDs, operator versions, interval fields and
/// membership against the candidate-filtered live window plane under the
/// caller's guard.
///
/// Membership is [`CANDIDATE_ROW_STATES`]: superseded covering stays on the live
/// table and is omitted here, matching successor capture. Clock-only
/// activation/expiry leaves captured fields unchanged, so it does not conflict.
/// A newly committed window on a candidate price, or any operator edit of a
/// captured one, is [`DomainError::WindowBaselineChanged`].
pub(crate) async fn refuse_captured_baseline_drift(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
) -> Result<(), DomainError> {
    let mut live_view = map_repo_result(
        window_baseline_repo::snapshot_live(
            runner,
            scope,
            owner.tenant_id,
            PlanId::new(owner.plan_id),
        )
        .await,
    )?;
    let mut captured_view =
        map_repo_result(window_baseline_repo::list(runner, scope, owner).await)?;
    live_view.sort_by_key(|row| row.window_id);
    captured_view.sort_by_key(|row| row.window_id);
    if live_view == captured_view {
        return Ok(());
    }
    Err(DomainError::WindowBaselineChanged(
        "the captured live baseline no longer matches committed windows (ids, operator versions, \
         intervals or membership); refresh the baseline and reapprove"
            .to_owned(),
    ))
}

async fn refuse_stale_baseline(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    action: &DraftWindowAction,
) -> Result<(), DomainError> {
    let window_id = match action {
        DraftWindowAction::AdjustEnd { window_id, .. }
        | DraftWindowAction::Cancel { window_id } => *window_id,
        DraftWindowAction::Create { .. } => return Ok(()),
    };
    let baseline = map_repo_result(window_baseline_repo::list(runner, scope, owner).await)?;
    if baseline
        .iter()
        .any(|row| row.window_id == window_id && !row.cancelled)
    {
        return Ok(());
    }
    Err(DomainError::WindowBaselineChanged(format!(
        "window {window_id} is not on the captured live baseline; refresh the baseline before \
         adjusting or cancelling it"
    )))
}

async fn candidate_keys(
    runner: &impl DBRunner,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
) -> Result<BTreeMap<Uuid, ScopeKey>, DomainError> {
    let rows = map_repo_result(
        price_repo::load_for_plan(
            runner,
            scope,
            owner.tenant_id,
            PlanId::new(owner.plan_id),
            CANDIDATE_ROW_STATES,
        )
        .await,
    )?;
    Ok(rows
        .into_iter()
        .map(|row| (row.price_id, row.scope_key))
        .collect())
}

fn map_repo_result<T>(result: Result<T, RepoError>) -> Result<T, DomainError> {
    result.map_err(map_repo)
}

fn map_repo(err: RepoError) -> DomainError {
    match err {
        RepoError::NotDraft { .. } => DomainError::DraftWindowContextChanged(err.to_string()),
        other => repo_failure(&other),
    }
}
