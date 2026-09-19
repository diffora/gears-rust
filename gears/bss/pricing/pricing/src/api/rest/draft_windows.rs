//! Draft-window HTTP orchestration (D-374).
//!
//! Live scheduling stays in [`super`] and still runs through
//! [`crate::infra::window::WindowService`]. Every draft write shares one
//! guarded envelope: namespaced digest and revision match are the caller's;
//! claim, [`crate::infra::draft_window::apply_command`] (which takes the per-plan
//! serial lock), and replay live here. Draft commands never enter `WindowService`.
//!
//! Recovery routes are still registered by [`super::router`] so the
//! census-visible `pub fn router` is the mount that carries them. This file
//! declares no `OperationBuilder` and no `router`.

use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::http::StatusCode;
use axum::http::header::{ETAG, LOCATION};
use axum::response::{IntoResponse, Response};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_db::secure::{AccessScope, DbTx};
use uuid::Uuid;

use crate::api::rest::preconditions;
use crate::api::rest::state::GovernanceState;
use crate::domain::audit::AuditStamp;
use crate::domain::concurrency::RowVersion;
use crate::domain::draft_window::{
    DraftStart, DraftWindowAction, DraftWindowEntry, DraftWindowOwner,
};
use crate::domain::error::DomainError;
use crate::domain::scope_key::PlanId;
use crate::infra::draft_window::{self, DraftWindowCommand};
use crate::infra::idempotent::{self, Guarded, GuardedRequest};
use crate::infra::storage::repo::{draft_window_repo, window_baseline_repo};
use crate::infra::storage::repo_failure;
use time::OffsetDateTime;

use super::{
    AdjustWindowRequest, DraftOperationRemovedView, DraftStartView, DraftWindowView, PRICE_WINDOW,
    PRICE_WINDOWS, PRICE_WINDOWS_LIST, RefreshBaselineView, ScheduleWindowRequest,
    WindowDeleteQuery, draft_owner, draft_window_json, namespaced_digest, replayed,
    require_matching_revision, start_view,
};

pub(super) const SCHEDULE_DRAFT_WINDOW_OPERATION: &str = "bss_pricing.schedule_draft_window";
pub(super) const ADJUST_DRAFT_WINDOW_OPERATION: &str = "bss_pricing.adjust_draft_window";
pub(super) const CANCEL_DRAFT_WINDOW_OPERATION: &str = "bss_pricing.cancel_draft_window";
pub(super) const REMOVE_DRAFT_WINDOW_OPERATION: &str = "bss_pricing.remove_draft_window_operation";
pub(super) const REFRESH_DRAFT_WINDOW_BASELINE_OPERATION: &str =
    "bss_pricing.refresh_draft_window_baseline";

/// Inputs every draft write shares once headers and the digest are known.
pub(super) struct DraftCommandRun {
    pub state: Arc<GovernanceState>,
    pub scope: AccessScope,
    pub tenant: Uuid,
    pub plan_id: PlanId,
    pub plan_revision: u64,
    pub expected_plan_version: u64,
    pub stamp: AuditStamp,
    pub key: String,
    pub digest: Vec<u8>,
    pub operation: &'static str,
    pub status: StatusCode,
    pub now: OffsetDateTime,
}

/// The command-specific work that runs inside the shared envelope.
pub(super) enum DraftWork {
    Schedule {
        price_id: Uuid,
        start: DraftStart,
        effective_to: Option<OffsetDateTime>,
        reason_code: String,
        start_wire: DraftStartView,
    },
    Adjust {
        window_id: Uuid,
        effective_to: Option<OffsetDateTime>,
    },
    Cancel {
        window_id: Uuid,
    },
    Undo {
        operation_id: Uuid,
    },
    Refresh,
}

struct DraftHttpOutcome {
    status: StatusCode,
    body: serde_json::Value,
    plan_revision: u64,
    row_version: u64,
    location: Option<String>,
}

struct WireIdentity {
    price_id: Uuid,
    start: DraftStartView,
    reason_code: String,
}

/// Claim, apply, record — one transaction, every draft write.
/// The per-plan window guard is acquired inside [`apply_command`], not here.
pub(super) async fn run_draft_command(
    run: DraftCommandRun,
    work: DraftWork,
) -> Result<Response, CanonicalError> {
    let mutation_scope = run.scope.clone();
    let tenant = run.tenant;
    let plan_id = run.plan_id;
    let plan_revision = run.plan_revision;
    let expected = run.expected_plan_version;
    let stamp = run.stamp;
    let operation = run.operation;
    let status = run.status;
    let owner = draft_owner(tenant, plan_id, plan_revision);
    let guarded = idempotent::guarded(
        &run.state.db,
        &run.state.idempotency,
        &run.scope,
        GuardedRequest {
            operation: run.operation,
            client_key: run.key,
            request_hash: run.digest,
            tenant_id: tenant,
            status: run.status.as_u16().into(),
            now: run.now,
        },
        move |txn| {
            Box::pin(async move {
                apply_work(txn, &mutation_scope, &owner, expected, stamp, status, work).await
            })
        },
        |outcome| Ok(outcome.body.clone()),
    )
    .await?;
    match guarded {
        Guarded::Performed(outcome) => Ok(answer_draft(outcome)),
        Guarded::Replayed { status, body } => Ok(replayed(operation, status, &body)?),
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply_work(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    expected: u64,
    stamp: AuditStamp,
    status: StatusCode,
    work: DraftWork,
) -> Result<DraftHttpOutcome, DomainError> {
    match work {
        DraftWork::Schedule {
            price_id,
            start,
            effective_to,
            reason_code,
            start_wire,
        } => {
            schedule(
                txn,
                scope,
                owner,
                expected,
                stamp,
                status,
                price_id,
                start,
                effective_to,
                reason_code,
                start_wire,
            )
            .await
        }
        DraftWork::Adjust {
            window_id,
            effective_to,
        } => {
            adjust(
                txn,
                scope,
                owner,
                expected,
                stamp,
                status,
                window_id,
                effective_to,
            )
            .await
        }
        DraftWork::Cancel { window_id } => {
            cancel(txn, scope, owner, expected, stamp, status, window_id).await
        }
        DraftWork::Undo { operation_id } => {
            undo(txn, scope, owner, expected, stamp, status, operation_id).await
        }
        DraftWork::Refresh => refresh(txn, scope, owner, expected, stamp, status).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn schedule(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    expected: u64,
    stamp: AuditStamp,
    status: StatusCode,
    price_id: Uuid,
    start: DraftStart,
    effective_to: Option<OffsetDateTime>,
    reason_code: String,
    start_wire: DraftStartView,
) -> Result<DraftHttpOutcome, DomainError> {
    let window_id = Uuid::now_v7();
    let entry = DraftWindowEntry {
        operation_id: window_id,
        action: DraftWindowAction::Create {
            window_id,
            price_id,
            start,
            effective_to,
        },
        reason_code: reason_code.clone(),
    };
    let row_version = draft_window::apply_command(
        txn,
        scope,
        owner,
        expected,
        DraftWindowCommand::Put(entry),
        stamp,
    )
    .await?;
    let view = DraftWindowView {
        window_id,
        operation_id: window_id,
        plan_id: owner.plan_id,
        price_id,
        plan_revision: owner.plan_revision,
        state: "draft".to_owned(),
        start: start_wire,
        effective_to,
        reason_code,
    };
    Ok(DraftHttpOutcome {
        status,
        body: draft_window_json(&view)?,
        plan_revision: owner.plan_revision,
        row_version,
        location: Some(draft_location(
            PlanId::new(owner.plan_id),
            owner.plan_revision,
        )),
    })
}

#[allow(clippy::too_many_arguments)]
async fn adjust(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    expected: u64,
    stamp: AuditStamp,
    status: StatusCode,
    window_id: Uuid,
    effective_to: Option<OffsetDateTime>,
) -> Result<DraftHttpOutcome, DomainError> {
    let existing = load_entry(txn, scope, owner.tenant_id, window_id).await?;
    let (operation_id, action) = match existing.as_ref() {
        Some(entry) if matches!(entry.action, DraftWindowAction::Create { .. }) => {
            let DraftWindowAction::Create {
                window_id: created_id,
                price_id,
                start,
                ..
            } = entry.action
            else {
                unreachable!("matched Create");
            };
            (
                entry.operation_id,
                DraftWindowAction::Create {
                    window_id: created_id,
                    price_id,
                    start,
                    effective_to,
                },
            )
        }
        Some(entry) => (
            entry.operation_id,
            DraftWindowAction::AdjustEnd {
                window_id,
                effective_to,
            },
        ),
        None => (
            Uuid::now_v7(),
            DraftWindowAction::AdjustEnd {
                window_id,
                effective_to,
            },
        ),
    };
    let reason_code = existing
        .as_ref()
        .map_or_else(|| "adjust".to_owned(), |entry| entry.reason_code.clone());
    let row_version = draft_window::apply_command(
        txn,
        scope,
        owner,
        expected,
        DraftWindowCommand::Put(DraftWindowEntry {
            operation_id,
            action: action.clone(),
            reason_code,
        }),
        stamp,
    )
    .await?;
    let identity = wire_identity(txn, scope, owner, window_id, existing.as_ref(), &action).await?;
    let view = DraftWindowView {
        window_id,
        operation_id,
        plan_id: owner.plan_id,
        price_id: identity.price_id,
        plan_revision: owner.plan_revision,
        state: "draft".to_owned(),
        start: identity.start,
        effective_to,
        reason_code: identity.reason_code,
    };
    ok_outcome(status, &view, owner.plan_revision, row_version)
}

async fn cancel(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    expected: u64,
    stamp: AuditStamp,
    status: StatusCode,
    window_id: Uuid,
) -> Result<DraftHttpOutcome, DomainError> {
    let existing = load_entry(txn, scope, owner.tenant_id, window_id).await?;
    let (command, action_for_wire) = match existing.as_ref() {
        Some(entry) if matches!(entry.action, DraftWindowAction::Create { .. }) => (
            DraftWindowCommand::Remove {
                operation_id: entry.operation_id,
            },
            entry.action.clone(),
        ),
        Some(entry) => (
            DraftWindowCommand::Put(DraftWindowEntry {
                operation_id: entry.operation_id,
                action: DraftWindowAction::Cancel { window_id },
                reason_code: entry.reason_code.clone(),
            }),
            DraftWindowAction::Cancel { window_id },
        ),
        None => (
            DraftWindowCommand::Put(DraftWindowEntry {
                operation_id: Uuid::now_v7(),
                action: DraftWindowAction::Cancel { window_id },
                reason_code: "cancel".to_owned(),
            }),
            DraftWindowAction::Cancel { window_id },
        ),
    };
    let operation_id = match &command {
        DraftWindowCommand::Remove { operation_id } => *operation_id,
        DraftWindowCommand::Put(entry) => entry.operation_id,
        DraftWindowCommand::RefreshBaseline => window_id,
    };
    let row_version =
        draft_window::apply_command(txn, scope, owner, expected, command, stamp).await?;
    let identity = wire_identity(
        txn,
        scope,
        owner,
        window_id,
        existing.as_ref(),
        &action_for_wire,
    )
    .await?;
    let view = DraftWindowView {
        window_id,
        operation_id,
        plan_id: owner.plan_id,
        price_id: identity.price_id,
        plan_revision: owner.plan_revision,
        state: "draft".to_owned(),
        start: identity.start,
        effective_to: None,
        reason_code: identity.reason_code,
    };
    ok_outcome(status, &view, owner.plan_revision, row_version)
}

async fn undo(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    expected: u64,
    stamp: AuditStamp,
    status: StatusCode,
    operation_id: Uuid,
) -> Result<DraftHttpOutcome, DomainError> {
    let row_version = draft_window::apply_command(
        txn,
        scope,
        owner,
        expected,
        DraftWindowCommand::Remove { operation_id },
        stamp,
    )
    .await?;
    let view = DraftOperationRemovedView { operation_id };
    Ok(DraftHttpOutcome {
        status,
        body: serde_json::to_value(view)
            .map_err(|e| DomainError::Internal(format!("cannot render a draft undo: {e}")))?,
        plan_revision: owner.plan_revision,
        row_version,
        location: None,
    })
}

async fn refresh(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    expected: u64,
    stamp: AuditStamp,
    status: StatusCode,
) -> Result<DraftHttpOutcome, DomainError> {
    let before = draft_window_repo::list(txn, scope, owner)
        .await
        .map_err(|e| repo_failure(&e))?;
    let row_version = draft_window::apply_command(
        txn,
        scope,
        owner,
        expected,
        DraftWindowCommand::RefreshBaseline,
        stamp,
    )
    .await?;
    let after = draft_window_repo::list(txn, scope, owner)
        .await
        .map_err(|e| repo_failure(&e))?;
    let kept: HashSet<Uuid> = after.iter().map(|row| row.operation_id).collect();
    let discarded_operation_ids: Vec<Uuid> = before
        .into_iter()
        .filter(|row| !kept.contains(&row.operation_id))
        .map(|row| row.operation_id)
        .collect();
    let view = RefreshBaselineView {
        plan_id: owner.plan_id,
        plan_revision: owner.plan_revision,
        discarded_operation_ids,
    };
    Ok(DraftHttpOutcome {
        status,
        body: serde_json::to_value(view)
            .map_err(|e| DomainError::Internal(format!("cannot render a baseline refresh: {e}")))?,
        plan_revision: owner.plan_revision,
        row_version,
        location: None,
    })
}

/// Shared header-to-command glue for the mixed POST/PATCH/DELETE doors.
#[allow(clippy::too_many_arguments)]
pub(super) async fn schedule_draft_window(
    state: Arc<GovernanceState>,
    ctx: toolkit_security::SecurityContext,
    scope: AccessScope,
    correlation: Uuid,
    tenant: Uuid,
    plan_id: PlanId,
    price_id: Uuid,
    key: String,
    request: ScheduleWindowRequest,
    tag: preconditions::RevisionTag,
    plan_revision: u64,
) -> Result<Response, CanonicalError> {
    require_matching_revision(&tag, plan_revision)?;
    let start = super::draft_schedule_start(&request)?;
    let digest = namespaced_digest(
        "draft",
        Some(plan_revision),
        plan_id.get(),
        "POST",
        PRICE_WINDOWS,
        &request,
    )?;
    let now = OffsetDateTime::now_utc();
    let stamp = crate::api::rest::auth_context::audit_stamp(&ctx, now, correlation);
    run_draft_command(
        DraftCommandRun {
            state,
            scope,
            tenant,
            plan_id,
            plan_revision,
            expected_plan_version: tag.version.get(),
            stamp,
            key,
            digest,
            operation: SCHEDULE_DRAFT_WINDOW_OPERATION,
            status: StatusCode::CREATED,
            now,
        },
        DraftWork::Schedule {
            price_id,
            start,
            effective_to: request.effective_to,
            reason_code: request.reason_code,
            start_wire: start_view(start),
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn adjust_draft_window(
    state: Arc<GovernanceState>,
    ctx: toolkit_security::SecurityContext,
    scope: AccessScope,
    correlation: Uuid,
    tenant: Uuid,
    plan_id: PlanId,
    window_id: Uuid,
    key: String,
    request: AdjustWindowRequest,
    tag: preconditions::RevisionTag,
    plan_revision: u64,
) -> Result<Response, CanonicalError> {
    require_matching_revision(&tag, plan_revision)?;
    let digest = namespaced_digest(
        "draft",
        Some(plan_revision),
        plan_id.get(),
        "PATCH",
        PRICE_WINDOW,
        &request,
    )?;
    let now = OffsetDateTime::now_utc();
    let stamp = crate::api::rest::auth_context::audit_stamp(&ctx, now, correlation);
    run_draft_command(
        DraftCommandRun {
            state,
            scope,
            tenant,
            plan_id,
            plan_revision,
            expected_plan_version: tag.version.get(),
            stamp,
            key,
            digest,
            operation: ADJUST_DRAFT_WINDOW_OPERATION,
            status: StatusCode::OK,
            now,
        },
        DraftWork::Adjust {
            window_id,
            effective_to: request.effective_to,
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn cancel_draft_window(
    state: Arc<GovernanceState>,
    ctx: toolkit_security::SecurityContext,
    scope: AccessScope,
    correlation: Uuid,
    tenant: Uuid,
    plan_id: PlanId,
    window_id: Uuid,
    key: String,
    query: WindowDeleteQuery,
    tag: preconditions::RevisionTag,
    plan_revision: u64,
) -> Result<Response, CanonicalError> {
    require_matching_revision(&tag, plan_revision)?;
    let digest = namespaced_digest(
        "draft",
        Some(plan_revision),
        plan_id.get(),
        "DELETE",
        PRICE_WINDOW,
        &serde_json::json!({
            "window_id": window_id,
            "plan_id": query.plan_id,
            "plan_revision": plan_revision,
        }),
    )?;
    let now = OffsetDateTime::now_utc();
    let stamp = crate::api::rest::auth_context::audit_stamp(&ctx, now, correlation);
    run_draft_command(
        DraftCommandRun {
            state,
            scope,
            tenant,
            plan_id,
            plan_revision,
            expected_plan_version: tag.version.get(),
            stamp,
            key,
            digest,
            operation: CANCEL_DRAFT_WINDOW_OPERATION,
            status: StatusCode::OK,
            now,
        },
        DraftWork::Cancel { window_id },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn undo_draft_operation(
    state: Arc<GovernanceState>,
    ctx: toolkit_security::SecurityContext,
    scope: AccessScope,
    correlation: Uuid,
    tenant: Uuid,
    plan_id: PlanId,
    operation_id: Uuid,
    key: String,
    tag: preconditions::RevisionTag,
    plan_revision: u64,
) -> Result<Response, CanonicalError> {
    require_matching_revision(&tag, plan_revision)?;
    let digest = namespaced_digest(
        "draft",
        Some(plan_revision),
        plan_id.get(),
        "DELETE",
        super::DRAFT_WINDOW_OPERATION,
        &serde_json::json!({ "operation_id": operation_id, "plan_revision": plan_revision }),
    )?;
    let now = OffsetDateTime::now_utc();
    let stamp = crate::api::rest::auth_context::audit_stamp(&ctx, now, correlation);
    run_draft_command(
        DraftCommandRun {
            state,
            scope,
            tenant,
            plan_id,
            plan_revision,
            expected_plan_version: tag.version.get(),
            stamp,
            key,
            digest,
            operation: REMOVE_DRAFT_WINDOW_OPERATION,
            status: StatusCode::OK,
            now,
        },
        DraftWork::Undo { operation_id },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn refresh_draft_baseline(
    state: Arc<GovernanceState>,
    ctx: toolkit_security::SecurityContext,
    scope: AccessScope,
    correlation: Uuid,
    tenant: Uuid,
    plan_id: PlanId,
    key: String,
    tag: preconditions::RevisionTag,
    request: super::RefreshBaselineRequest,
) -> Result<Response, CanonicalError> {
    require_matching_revision(&tag, request.plan_revision)?;
    let digest = namespaced_digest(
        "draft",
        Some(request.plan_revision),
        plan_id.get(),
        "POST",
        super::DRAFT_WINDOW_BASELINE_REFRESH,
        &request,
    )?;
    let now = OffsetDateTime::now_utc();
    let stamp = crate::api::rest::auth_context::audit_stamp(&ctx, now, correlation);
    run_draft_command(
        DraftCommandRun {
            state,
            scope,
            tenant,
            plan_id,
            plan_revision: request.plan_revision,
            expected_plan_version: tag.version.get(),
            stamp,
            key,
            digest,
            operation: REFRESH_DRAFT_WINDOW_BASELINE_OPERATION,
            status: StatusCode::OK,
            now,
        },
        DraftWork::Refresh,
    )
    .await
}

async fn load_entry(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    tenant: Uuid,
    window_id: Uuid,
) -> Result<Option<DraftWindowEntry>, DomainError> {
    Ok(
        draft_window_repo::find_addressing(txn, scope, tenant, window_id)
            .await
            .map_err(|e| repo_failure(&e))?
            .map(|(_, entry)| entry),
    )
}

async fn wire_identity(
    txn: &DbTx<'_>,
    scope: &AccessScope,
    owner: &DraftWindowOwner,
    window_id: Uuid,
    existing: Option<&DraftWindowEntry>,
    action: &DraftWindowAction,
) -> Result<WireIdentity, DomainError> {
    let fallback_reason = match action {
        DraftWindowAction::Cancel { .. } => "cancel",
        _ => "adjust",
    };
    if let DraftWindowAction::Create {
        price_id, start, ..
    } = action
    {
        return Ok(WireIdentity {
            price_id: *price_id,
            start: start_view(*start),
            reason_code: existing.map_or_else(
                || fallback_reason.to_owned(),
                |entry| entry.reason_code.clone(),
            ),
        });
    }
    if let Some(entry) = existing
        && let DraftWindowAction::Create {
            price_id, start, ..
        } = &entry.action
    {
        return Ok(WireIdentity {
            price_id: *price_id,
            start: start_view(*start),
            reason_code: entry.reason_code.clone(),
        });
    }
    let baseline = window_baseline_repo::list(txn, scope, owner)
        .await
        .map_err(|e| repo_failure(&e))?;
    let row = baseline
        .iter()
        .find(|row| row.window_id == window_id)
        .ok_or_else(|| {
            DomainError::Internal(format!(
                "draft window {window_id} has no captured baseline identity to render"
            ))
        })?;
    Ok(WireIdentity {
        price_id: row.price_id,
        start: DraftStartView::At {
            at: row.effective_from,
        },
        reason_code: existing.map_or_else(
            || fallback_reason.to_owned(),
            |entry| entry.reason_code.clone(),
        ),
    })
}

fn draft_location(plan_id: PlanId, plan_revision: u64) -> String {
    format!(
        "{PRICE_WINDOWS_LIST}?view=working&plan_id={}&plan_revision={plan_revision}",
        plan_id.get()
    )
}

fn answer_draft(outcome: DraftHttpOutcome) -> Response {
    let etag =
        preconditions::revision_etag(outcome.plan_revision, RowVersion::new(outcome.row_version));
    match outcome.location {
        Some(location) => (
            outcome.status,
            [(ETAG, etag), (LOCATION, location)],
            Json(outcome.body),
        )
            .into_response(),
        None => (outcome.status, [(ETAG, etag)], Json(outcome.body)).into_response(),
    }
}

fn ok_outcome(
    status: StatusCode,
    view: &DraftWindowView,
    plan_revision: u64,
    row_version: u64,
) -> Result<DraftHttpOutcome, DomainError> {
    Ok(DraftHttpOutcome {
        status,
        body: draft_window_json(view)?,
        plan_revision,
        row_version,
        location: None,
    })
}
