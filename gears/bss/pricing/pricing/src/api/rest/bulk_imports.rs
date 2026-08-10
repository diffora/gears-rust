//! The bulk import's three surfaces
//! (`design/12-operator-efficiency.md` §5, §4 `inst-bi-api`, `inst-bi-return`,
//! `inst-bs-abort`; D-118, D-290, D-291, D-292).
//!
//! # The two phases are one request, and that is what `202` is for
//!
//! `POST /bulk-imports` runs Phase 1 and, if it passes, Phase 2 — and answers
//! **202 with the operation ref** rather than the outcome. `inst-bi-return` says
//! so, and the reason survives the current implementation running both inline: a
//! batch is a long act, the run is a durable object with a report, and a caller
//! given a `202` and an id is a caller that does not have to hold the connection
//! open to learn what happened. `GET /bulk-imports/{id}` is where the answer
//! lives, for the first call and for every replay alike.
//!
//! A Phase-1 failure is the exception: `BULK_VALIDATION_FAILED`, and it is a
//! **400** — Foundation §3.3 gives the platform no 422 category, so every
//! architectural 422 reaches the wire as a 400 carrying its code. The refusal
//! carries a sentence and not the per-row report: the run is left
//! `validation_failed` holding it, and the `GET` is where a batch's answer lives
//! for every other outcome too.
//!
//! # The client key is the run's, not the gate's
//!
//! O4's replay here is **not** `idempotent::guarded`'s. That gate stores a
//! response body against a key with a TTL; a bulk run stores its own report on
//! its own row, unique per `(tenant, client_key)` by the table's index, and
//! `inst-bk-idem` replays *that* — "including during/after the lock window",
//! which outlives any dedup TTL. So the replay is a read of
//! `bulk_repo::find_by_client_key`, and a second `POST` under a key that already
//! opened a run answers the run it opened.
//!
//! # Abort is an edge, not a new state
//!
//! `inst-bs-abort`: `committing → completed_with_conflicts`, uncommitted rows
//! reported `not-attempted`, the lock cleared. §4 already has that edge, which is
//! why abort needs no state of its own — and why a crashed import cannot freeze
//! interactive authoring: the operator has a door.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::HeaderMap, http::StatusCode};
use chrono::Utc;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::auth_context::{audit_stamp, require_authenticated};
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::plans::idempotency_key_param;
use crate::api::rest::preconditions;
use crate::api::rest::prices::{PriceContentView, ScopeKeyRequest, content_of, scope_key_of};
use crate::api::rest::state::AuthoringState;
use crate::domain::bulk::{BulkKind, BulkState};
use crate::domain::concurrency::RowVersion;
use crate::domain::error::DomainError;
use crate::domain::import::{ImportRow, classify};
use crate::domain::scope_key::PlanId;
use crate::infra::bulk::commit_batch;
use crate::infra::import::classify_against_store;
use crate::infra::storage::repo::{NewBulkOperation, bulk_repo};
use crate::infra::storage::repo_failure;

const TAG: &str = "BSS Pricing Bulk Import";

/// `POST` — submit a batch.
pub const BULK_IMPORTS: &str = "/bss-pricing/v1/bulk-imports";

/// `GET` — the run and its per-row report.
pub const BULK_IMPORT: &str = "/bss-pricing/v1/bulk-imports/{operationId}";

/// `POST` — abort a stalled mid-commit run (D-37).
pub const BULK_IMPORT_ABORT: &str = "/bss-pricing/v1/bulk-imports/{operationId}/abort";

/// §5's batch-level refusal: Phase 1 blocked the import.
pub const BULK_VALIDATION_FAILED: &str = "BULK_VALIDATION_FAILED";

/// One submitted row.
///
/// `plan_id` is a **field** here and a path segment on the interactive surface,
/// because a batch may span plans — `pricing_bulk_operation` carries a tenant and
/// no plan. The key and content views are the interactive route's own, so a row
/// an operator can author one at a time is the row they can author a thousand of.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct BulkImportRowRequest {
    /// The plan this row prices.
    pub plan_id: Uuid,
    /// The six axes authored as a key; `plan_id` above is the seventh, and the
    /// usage pair is derived from `content` rather than authored here.
    pub scope_key: ScopeKeyRequest,
    /// The row's whole content.
    pub content: PriceContentView,
    /// The version of the draft this row edits, absent when it claims a new key.
    pub if_match: Option<u64>,
}

/// The submitted batch.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct BulkImportRequest {
    /// The rows, in the order the report will name them.
    pub rows: Vec<BulkImportRowRequest>,
}

/// A run, and whatever report it has reached.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct BulkImportView {
    /// The run's durable name.
    pub operation_id: Uuid,
    /// One of §4's seven states.
    pub state: String,
    /// The client key it was opened under.
    pub client_key: String,
    /// The report as it stands — Phase 1's violations, or Phase 2's
    /// `{committed, conflicted}`. Shape depends on how far the run got, which is
    /// what `state` is for.
    pub report: serde_json::Value,
    /// When the run ended, absent while it is still going.
    pub completed_at: Option<String>,
}

/// What this surface needs.
pub struct ApiState {
    /// The authoring plane's state — the provider, the price repository and the
    /// scope every read goes through. No new field: Phase 2's engine takes the
    /// provider and the repository, and the run's own statements are free
    /// functions.
    pub authoring: Arc<AuthoringState>,
}

fn run_view(run: &crate::infra::storage::repo::BulkOperationRecord) -> BulkImportView {
    BulkImportView {
        operation_id: run.operation_id,
        state: run.state.as_str().to_owned(),
        client_key: run.client_key.clone(),
        report: run.report.clone(),
        completed_at: run.completed_at.map(|at| at.to_rfc3339()),
    }
}

/// Build the router for the three bulk-import surfaces.
pub fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router = Router::new();

    router = OperationBuilder::post(BULK_IMPORTS)
        .operation_id("bss_pricing.submit_bulk_import")
        .summary("Submit a bulk price import")
        .description(
            "Runs the two-phase import. Phase 1 validates the whole batch and refuses it if any \
             row is invalid, answering `400` `BULK_VALIDATION_FAILED` and committing nothing; \
             the per-row report is on the run, which the `GET` serves. Phase 2 then commits row by row under each row's own ETag: \
             a conflict fails only that row, committed rows stand, and the report lists the \
             conflicted for retry. The answer is `202` with the operation ref; the report is at \
             `GET /bss-pricing/v1/bulk-imports/{operationId}`. The import authors **drafts** \
             (D-118): a row aimed at a published row's key is refused per-row, its remedy being \
             a repricing run. Replaying the `Idempotency-Key` answers the run it opened, \
             including during and after the lock window.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .param(idempotency_key_param())
        .json_request::<BulkImportRequest>(openapi, "The rows to import.")
        .handler(submit_bulk_import)
        .json_response_with_schema::<BulkImportView>(
            openapi,
            StatusCode::ACCEPTED,
            "The run, with its state and whatever report it has reached.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::get(BULK_IMPORT)
        .operation_id("bss_pricing.read_bulk_import")
        .summary("Read a bulk import's per-row report")
        .description(
            "The run and its report, which is where the answer to a submitted batch lives. \
             Gates on `plan` x `read`: the report is plan and price data, and an operator who \
             may read prices may read what an import did to them.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("operationId", "The run to read.")
        .handler(read_bulk_import)
        .json_response_with_schema::<BulkImportView>(
            openapi,
            StatusCode::OK,
            "The run and its report.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::post(BULK_IMPORT_ABORT)
        .operation_id("bss_pricing.abort_bulk_import")
        .summary("Abort a stalled mid-commit run")
        .description(
            "Moves a `committing` run to `completed_with_conflicts`, clearing its row locks so \
             interactive authoring is not frozen by a run that stopped (D-37). Uncommitted rows \
             stay in the report as not attempted; committed rows stand, as they do on every \
             other exit from Phase 2. A run that is not `committing` is refused by the state \
             machine: this is an edge, not a new state.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("operationId", "The run to abort.")
        .param(idempotency_key_param())
        .handler(abort_bulk_import)
        .json_response_with_schema::<BulkImportView>(
            openapi,
            StatusCode::OK,
            "The aborted run, with its locks released.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    // D-178's edge. Two of the three surfaces write — the submit authors rows and
    // an audit record per row, the abort moves the run — so every record a
    // request produces has to carry one correlation, not one each.
    router
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}

/// `POST /bulk-imports`.
async fn submit_bulk_import(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let scope = write_scope(&enforcer, &ctx).await?;
    let client_key = preconditions::idempotency_key(&headers)?;
    // Parsed **after** the gate, this directory's stated discipline: a caller
    // outside the scope learns they are denied, not that their body is malformed.
    // `Json<T>` as an extractor would run before the handler and answer 415 or
    // 400 where a 403 is owed — which is what the authz census caught.
    let body: BulkImportRequest = preconditions::parse_body(&body)?;

    let conn =
        state.authoring.db.conn().map_err(|e| {
            CanonicalError::from(DomainError::Internal(format!("bulk import: {e}")))
        })?;

    // **The replay is a read of the run, not of a dedup row.** `inst-bk-idem`
    // replays the report "including during/after the lock window", which outlives
    // any gate TTL — so the run's own unique `(tenant, client_key)` is the record.
    if let Some(existing) =
        bulk_repo::find_by_client_key(&conn, &scope, tenant, BulkKind::Import, &client_key)
            .await
            .map_err(|e| CanonicalError::from(repo_failure(&e)))?
    {
        // **A replay answers what the first call answered** (D-295). A batch
        // refused in Phase 1 was answered `400`, so a retry that answered `202`
        // would tell a client the resubmit succeeded where the original failed —
        // the one conclusion idempotency exists to prevent, and precisely the
        // client that retried on a timeout and cannot otherwise tell. The report
        // is replayed either way: the run holds it and the `GET` serves it.
        //
        // The message also states what the key now costs, because this is where
        // an operator meets it: the key is **spent on the run**, so a corrected
        // batch resubmitted under it would import nothing and be told nothing.
        if existing.state == BulkState::ValidationFailed {
            return Err(CanonicalError::from(DomainError::BulkValidationFailed {
                operation_id: existing.operation_id.to_string(),
                detail: "this key opened a batch that was refused in validation; a corrected \
                         batch is a new batch and needs its own idempotency key"
                    .to_owned(),
            }));
        }
        return Ok((StatusCode::ACCEPTED, Json(run_view(&existing))).into_response());
    }

    let rows = rows_of(&body)?;
    let now = Utc::now();
    let stamp = audit_stamp(&ctx, now, correlation);
    let run = bulk_repo::open(
        &conn,
        &scope,
        NewBulkOperation {
            operation_id: Uuid::now_v7(),
            tenant_id: tenant,
            kind: BulkKind::Import,
            client_key,
            report: serde_json::json!({ "rows": [] }),
            submitted_by: ctx.subject_id(),
            submitted_at: now,
        },
    )
    .await
    .map_err(|e| CanonicalError::from(repo_failure(&e)))?;

    // Phase 1: both halves, into one report.
    let mut report = classify(&rows);
    classify_against_store(&conn, &scope, tenant, &rows, &mut report)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;

    if report.blocks_the_batch() {
        let stored = serde_json::to_value(&report).unwrap_or_else(|_| serde_json::json!([]));
        bulk_repo::advance(
            &conn,
            &scope,
            tenant,
            run.operation_id,
            BulkState::ValidationFailed,
            stored,
            now,
        )
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
        // **A `400`, not the `422` §5 writes.** Foundation §3.3 states it once
        // for the whole design set: the platform has no 422 category, and every
        // architectural 422 reaches the wire as a 400 carrying its code. The
        // per-row report is not lost — the run holds it and the `GET` serves it,
        // which is where a caller reads a batch's answer anyway.
        return Err(CanonicalError::from(DomainError::BulkValidationFailed {
            operation_id: run.operation_id.to_string(),
            detail: format!(
                "{} of {} row(s) were refused before anything was committed; the run holds the \
                 per-row report",
                report.rows().len(),
                rows.len()
            ),
        }));
    }

    commit_batch(
        &state.authoring.db,
        &state.authoring.prices,
        &scope,
        tenant,
        run.operation_id,
        &rows,
        stamp,
        now,
    )
    .await
    .map_err(CanonicalError::from)?;

    let landed = bulk_repo::read(&conn, &scope, tenant, run.operation_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?
        .ok_or_else(|| {
            CanonicalError::from(DomainError::Internal(
                "bulk import: the run vanished mid-request".to_owned(),
            ))
        })?;
    Ok((StatusCode::ACCEPTED, Json(run_view(&landed))).into_response())
}

/// `GET /bulk-imports/{operationId}`.
async fn read_bulk_import(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(operation_id): Path<Uuid>,
) -> Result<Json<BulkImportView>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant = ctx.subject_tenant_id();
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::READ,
        /* owner_tenant_id */ None,
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    let conn =
        state.authoring.db.conn().map_err(|e| {
            CanonicalError::from(DomainError::Internal(format!("bulk import: {e}")))
        })?;
    let run = bulk_repo::read(&conn, &scope, tenant, operation_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?
        .ok_or_else(|| {
            CanonicalError::from(DomainError::NotFound {
                subject: "bulk import".to_owned(),
                id: operation_id.to_string(),
            })
        })?;
    Ok(Json(run_view(&run)))
}

/// `POST /bulk-imports/{operationId}/abort`.
async fn abort_bulk_import(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(operation_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<BulkImportView>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant = ctx.subject_tenant_id();
    let scope = write_scope(&enforcer, &ctx).await?;
    // Read for its refusal, not for a value: an abort with no key is a request the
    // operator cannot safely retry.
    preconditions::idempotency_key(&headers)?;

    let conn = state
        .authoring
        .db
        .conn()
        .map_err(|e| CanonicalError::from(DomainError::Internal(format!("bulk abort: {e}"))))?;
    let run = bulk_repo::read(&conn, &scope, tenant, operation_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?
        .ok_or_else(|| {
            CanonicalError::from(DomainError::NotFound {
                subject: "bulk import".to_owned(),
                id: operation_id.to_string(),
            })
        })?;

    // **The trigger does not refuse this one, and D-293 claimed it did.** A move
    // to the state a run is already in returns early on both engines, so an abort
    // against a run already in `completed_with_conflicts` — the ordinary terminal
    // state of any partially-conflicted import — would rewrite `completed_at` and
    // stamp an abort note over a report where every row WAS attempted. §4 makes
    // abort an edge out of `committing`; a run that is over is refused here,
    // because the state machine cannot see the difference between a move and a
    // no-op.
    if run.state != BulkState::Committing {
        return Err(CanonicalError::from(DomainError::LifecycleForbidden(
            format!(
                "bulk operation {operation_id} is {}; abort acts on a run that is still \
                 committing, and a run that is over has no locks to clear and no work to stop",
                run.state.as_str()
            ),
        )));
    }

    // The report the abort adds to is the one the run reached, so what it
    // committed survives the note.
    let mut report = run.report.clone();
    if let Some(object) = report.as_object_mut() {
        object.insert(
            "aborted".to_owned(),
            serde_json::json!("uncommitted rows were not attempted"),
        );
    }
    // **The release goes first, and D-300 is why.** With the terminal move ahead of
    // it, a release that failed for any transient reason left the run terminal and
    // every lock held — and D-294's own state guard above then refuses the retry
    // that used to rescue it, because a terminal run is no longer `committing`. The
    // rows would be frozen by a finished operation with no remedy at all: nothing
    // else calls `release_locks`, the lock table has no sweeper, and D-37's lease
    // takeover is designed and unbuilt. Releasing first makes a failed abort
    // **retryable** — the run is still `committing`, so the operator simply asks
    // again — which is the property the guard was written assuming.
    bulk_repo::release_locks(&conn, &scope, tenant, operation_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    let aborted = bulk_repo::advance(
        &conn,
        &scope,
        tenant,
        operation_id,
        BulkState::CompletedWithConflicts,
        report,
        Utc::now(),
    )
    .await
    .map_err(|e| CanonicalError::from(repo_failure(&e)))?;

    Ok(Json(run_view(&aborted)))
}

/// The `plan x write` gate both mutating surfaces take.
///
/// `resource_id = None` for `create_plan`'s reason, sharpened by D-281: a batch
/// spans plans and writes rows whose ids it mints, so there is no single resource
/// to name — and an id-shaped constraint could not filter these tables anyway.
async fn write_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<toolkit_db::secure::AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(ctx.subject_tenant_id()),
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)
}

/// The submitted body as the domain's rows.
///
/// **The usage line is derived here, through the store's own derivation** (D-306).
/// `ScopeKeyRequest` carries six axes and the pair `(meter, dimensionKey)` is
/// authored on the *content* (D-196 clause (3)), so a key built from the request
/// alone is line-less — and every Phase-1 rule that groups or looks up by key was
/// then comparing keys the store would never file these rows under.
///
/// Two consequences, both live before this: `duplicate_scope_keys` grouped two
/// rows differing **only** in their meter as one key and refused them as
/// duplicates, which its own sentence says they are not — *"those are two keys
/// and both author"*; and `draft_rows`' lookup could never match a metered draft,
/// so a bulk import could not edit one at all — the row took the create path and
/// came back `DUPLICATE_SCOPE_KEY`.
///
/// It goes through `price_repo::resolve_authored_usage_line` rather than a
/// spelling of its own, because a second derivation of the same pair is the
/// hazard the interactive plane's version already records: the store has one home
/// for the columns, so a disagreement between two derivations would survive
/// exactly as far as the first metered import.
fn rows_of(body: &BulkImportRequest) -> Result<Vec<ImportRow>, CanonicalError> {
    body.rows
        .iter()
        .map(|row| {
            let content = content_of(&row.content)?;
            let key = scope_key_of(PlanId::new(row.plan_id), &row.scope_key)?;
            let scope_key = crate::infra::storage::repo::price_repo::resolve_authored_usage_line(
                &key,
                &content.row,
            )
            .map_err(|e| repo_failure(&e))?;
            Ok(ImportRow {
                scope_key,
                content,
                if_match: row.if_match.map(RowVersion::new),
            })
        })
        .collect::<Result<Vec<_>, DomainError>>()
        .map_err(CanonicalError::from)
}
