//! Governed submissions claim replay first, then fence and submit atomically.
use super::{
    ApiState, TxError, category_tx_config, contention_db_err,
    dto::{EmptyRequest, SkuChangeRequest, SkuDto, SubmitReceipt},
    governance as g, idempotency_key, json_body, replay_response, require_authenticated,
    tx_to_canonical,
};
use crate::{
    authz::actions,
    domain::{
        approvals::{Subject, change::SkuChange, publish::SkuPublish, retire::SkuRetire},
        idempotency::payload_digest,
        sku::{SkuPatch, apply_patch},
    },
    infra::{
        idempotency::{ClaimVerdict, IdempotencyClaimInput, claim_idempotency},
        storage::repo,
    },
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{
    Extension, Json, Router,
    extract::{Path, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use bss_approval::{Engine, SubmitRequest};
use bss_products_sdk::models::SkuContent;
use serde_json::Value;
use std::sync::Arc;
use time::OffsetDateTime;
use toolkit::api::{
    OpenApiRegistry, canonical_prelude::CanonicalError, operation_builder::OperationBuilder,
};
use toolkit_db::{DbTx, secure::AccessScope};
use toolkit_security::SecurityContext;
use uuid::Uuid;

#[derive(Clone, Copy)]
enum SubmitKind {
    Publish,
    Change,
    Retire,
}
impl SubmitKind {
    fn suffix(self) -> &'static str {
        match self {
            Self::Publish => "submit",
            Self::Change => "changes",
            Self::Retire => "retire",
        }
    }
}

/// Register submission and orphan-fence recovery operations.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = OperationBuilder::post("/bss-products/v1/skus/{id}/submit")
        .operation_id("bss_products.submit_sku")
        .summary("submit_sku")
        .tag("SKU governance")
        .authenticated()
        .no_license_required()
        .path_param("id", "SKU id")
        .param(idempotency_param())
        .handler(submit)
        .json_response_with_schema::<SubmitReceipt>(openapi, StatusCode::OK, "Receipt")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-products/v1/skus/{id}/changes")
        .operation_id("bss_products.change_sku")
        .summary("change_sku")
        .tag("SKU governance")
        .authenticated()
        .no_license_required()
        .path_param("id", "SKU id")
        .json_request::<SkuChangeRequest>(openapi, "Request")
        .param(idempotency_param())
        .handler(changes)
        .json_response_with_schema::<SubmitReceipt>(openapi, StatusCode::OK, "Receipt")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-products/v1/skus/{id}/retire")
        .operation_id("bss_products.retire_sku")
        .summary("retire_sku")
        .tag("SKU governance")
        .authenticated()
        .no_license_required()
        .path_param("id", "SKU id")
        .param(idempotency_param())
        .handler(retire)
        .json_response_with_schema::<SubmitReceipt>(openapi, StatusCode::OK, "Receipt")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-products/v1/skus/{id}/unfence")
        .operation_id("bss_products.unfence_sku")
        .summary("unfence_sku")
        .tag("SKU governance")
        .authenticated()
        .no_license_required()
        .path_param("id", "SKU id")
        .handler(unfence)
        .json_response_with_schema::<SkuDto>(openapi, StatusCode::OK, "Receipt")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    router.layer(Extension(state))
}
async fn submit(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Result<Option<Json<Value>>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(&enforcer, &ctx, actions::SUBMIT, false).await?;
    run(
        state,
        scope,
        ctx,
        id,
        headers,
        body.map(|body| body.unwrap_or_else(|| Json(serde_json::json!({})))),
        SubmitKind::Publish,
    )
    .await
}
async fn changes(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(&enforcer, &ctx, actions::SUBMIT, false).await?;
    run(state, scope, ctx, id, headers, body, SubmitKind::Change).await
}
async fn retire(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Result<Option<Json<Value>>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(&enforcer, &ctx, actions::SUBMIT, false).await?;
    run(
        state,
        scope,
        ctx,
        id,
        headers,
        body.map(|body| body.unwrap_or_else(|| Json(serde_json::json!({})))),
        SubmitKind::Retire,
    )
    .await
}
/// Resume only the same kind of orphan fence, after rechecking live reservations.
/// @cpt-cf-bss-products-fr-sku-retire-fenced
async fn fence(
    tx: &DbTx<'_>,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
    kind: repo::Fence,
    now: OffsetDateTime,
) -> Result<Uuid, TxError> {
    let op = Uuid::now_v7();
    if matches!(
        repo::fence_sku(tx, scope, tenant, id, kind, op, now)
            .await
            .map_err(TxError::Repo)?,
        repo::HeadWrite::Written(_)
    ) {
        return Ok(op);
    }
    let s = repo::find_sku_fence(tx, scope, tenant, id)
        .await
        .map_err(TxError::Repo)?
        .ok_or(TxError::Refused(
            crate::domain::error::DomainError::NotFound { what: "sku", id },
        ))?;
    let live = repo::live_references(tx, scope, tenant, id)
        .await
        .map_err(TxError::Repo)?;
    if !live.is_empty() {
        let rows = live
            .into_iter()
            .map(super::dto::ReferenceReceipt::from)
            .collect::<Vec<_>>();
        let rows = serde_json::to_value(rows)
            .map_err(|e| TxError::Repo(crate::infra::storage::RepoError::Db(e.to_string())))?;
        return Err(TxError::FencedReferences {
            code: if matches!(kind, repo::Fence::Retire) {
                "SKU_REFERENCED"
            } else {
                "SKU_TYPE_FROZEN"
            },
            rows,
        });
    }
    if s.pending_unit_id.is_some() {
        return Err(g::conflict(
            "ROW_LOCKED_PENDING",
            "SKU belongs to a pending unit",
        ));
    }
    if let Some(op) = s.fence_op_id
        && match kind {
            repo::Fence::Retire => s.lifecycle == "retiring",
            repo::Fence::TypeChange => {
                s.type_change_pending && matches!(s.lifecycle.as_str(), "published" | "deprecated")
            }
        }
    {
        return Ok(op);
    }
    Err(g::conflict(
        "ILLEGAL_TRANSITION",
        "SKU cannot acquire this fence",
    ))
}
#[allow(
    clippy::too_many_arguments,
    reason = "One HTTP command's dependencies remain explicit"
)]
/// @cpt-cf-bss-products-fr-approval-units
async fn run(
    state: Arc<ApiState>,
    scope: AccessScope,
    ctx: SecurityContext,
    id: Uuid,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
    kind: SubmitKind,
) -> Result<Response, CanonicalError> {
    let payload = json_body(body)?;
    let now = OffsetDateTime::now_utc();
    let tenant = ctx.subject_tenant_id();
    let (patch, date) = if matches!(kind, SubmitKind::Change) {
        let parsed: SkuChangeRequest = serde_json::from_value(payload.clone())
            .map_err(|e| CanonicalError::from(g::validation("body", e.to_string())))?;
        (
            SkuPatch::try_from(parsed.patch).map_err(|r| {
                CanonicalError::from(crate::domain::error::DomainError::Validation(r))
            })?,
            Some(parsed.effective_from.unwrap_or(now.date())),
        )
    } else {
        let _: EmptyRequest = serde_json::from_value(payload.clone())
            .map_err(|e| CanonicalError::from(g::validation("body", e.to_string())))?;
        (SkuPatch::default(), None)
    };
    let claim = idempotency_key(&headers)?.map(|key| {
        IdempotencyClaimInput::new(
            format!("/bss-products/v1/skus/{id}/{}", kind.suffix()),
            key,
            payload_digest(&payload),
            now,
            state.idempotency_retention_hours,
        )
    });
    if let Some(input) = claim.clone() {
        let scope = scope.clone();
        let verdict = state
            .db
            .db()
            .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
                let scope = scope.clone();
                let input = input.clone();
                Box::pin(async move {
                    claim_idempotency(tx, &scope, tenant, &input)
                        .await
                        .map_err(TxError::Repo)
                })
            })
            .await
            .map_err(tx_to_canonical)?;
        match verdict {
            ClaimVerdict::Replay { status, body } => return Ok(replay_response(status, body)),
            ClaimVerdict::Refused(e) => return Err(e.into()),
            ClaimVerdict::Proceed => {}
        }
    }
    let result = execute(
        state.clone(),
        scope.clone(),
        ctx,
        id,
        kind,
        patch,
        date,
        now,
        claim.clone(),
    )
    .await;
    if (result.is_err() || result.as_ref().is_ok_and(|r| !r.status().is_success()))
        && let Some(input) = claim
    {
        // The small claim transaction already committed. A failed operation releases its claim.
        state
            .db
            .db()
            .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
                let scope = scope.clone();
                let input = input.clone();
                Box::pin(async move {
                    repo::release_idempotency_claim(
                        tx,
                        &scope,
                        tenant,
                        &input.endpoint,
                        &input.client_key,
                    )
                    .await
                    .map_err(TxError::Repo)
                })
            })
            .await
            .map_err(tx_to_canonical)?;
    }
    result
}
#[allow(
    clippy::too_many_arguments,
    reason = "Submission captures all values once before transaction retries"
)]
async fn execute(
    state: Arc<ApiState>,
    scope: AccessScope,
    ctx: SecurityContext,
    id: Uuid,
    kind: SubmitKind,
    patch: SkuPatch,
    date: Option<time::Date>,
    now: OffsetDateTime,
    claim: Option<IdempotencyClaimInput>,
) -> Result<Response, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let current = g::find(
        &state.db.conn().map_err(|e| tx_to_canonical(e.into()))?,
        &scope,
        tenant,
        id,
    )
    .await
    .map_err(tx_to_canonical)?;
    let proposed = apply_patch(&SkuContent::from(&current), &patch);
    let usage = if matches!(kind, SubmitKind::Retire) {
        None
    } else {
        g::resolve(&state, &ctx, &proposed).await?
    };
    let db = state.db.db();
    let receipt = db
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let state = state.clone();
            let scope = scope.clone();
            let ctx = ctx.clone();
            let patch = patch.clone();
            let usage = usage.clone();
            let proposed = proposed.clone();
            let claim = claim.clone();
            Box::pin(async move {
                g::expire(tx, &scope, tenant, id, state.fence_ttl_minutes, now).await?;
                let current = g::find(tx, &scope, tenant, id).await?;
                if !matches!(kind, SubmitKind::Retire)
                    && apply_patch(&SkuContent::from(&current), &patch).usage_type_ref
                        != proposed.usage_type_ref
                {
                    return Err(g::conflict(
                        "STALE_REVISION",
                        "meter changed during resolution; retry",
                    ));
                }
                if matches!(kind, SubmitKind::Change)
                    && patch.r#type.is_none()
                    && current.type_change_pending
                {
                    return Err(g::conflict(
                        "SKU_FENCED",
                        "resume the type change or unfence it first",
                    ));
                }
                let base = SkuPublish {
                    scope: scope.clone(),
                    tenant_id: tenant,
                    sink: state.sink.clone(),
                    actor: ctx.subject_id(),
                    now,
                    usage_type: usage,
                };
                let subject = match kind {
                    SubmitKind::Publish => Subject::Publish(base),
                    SubmitKind::Retire => Subject::Retire(SkuRetire {
                        base,
                        fence_op_id: fence(tx, &scope, tenant, id, repo::Fence::Retire, now)
                            .await?,
                    }),
                    SubmitKind::Change => {
                        let fence_op_id = if patch.r#type.is_some() {
                            Some(fence(tx, &scope, tenant, id, repo::Fence::TypeChange, now).await?)
                        } else {
                            None
                        };
                        Subject::Change(SkuChange {
                            base,
                            patch,
                            effective_from: date.unwrap_or(now.date()),
                            fence_op_id,
                        })
                    }
                };
                let store = repo::ProductsApprovalStore {
                    scope: scope.clone(),
                    tenant_id: tenant,
                };
                let policy = repo::read_policy(tx, &scope, tenant)
                    .await
                    .map_err(TxError::Repo)?;
                let submitted = Engine::submit(
                    &store,
                    &subject,
                    tx,
                    SubmitRequest {
                        tenant_id: tenant,
                        ref_id: id,
                        item_ids: &[id],
                        actor: ctx.subject_id(),
                        policy: &policy,
                        common_effective_date: date,
                        now,
                    },
                )
                .await?;
                g::audit(
                    tx,
                    &scope,
                    &ctx,
                    "approval.submit",
                    "approval_unit",
                    submitted.unit.id,
                    None,
                    now,
                )
                .await?;
                if submitted.applied {
                    g::audit(
                        tx,
                        &scope,
                        &ctx,
                        "approval.applied",
                        "approval_unit",
                        submitted.unit.id,
                        None,
                        now,
                    )
                    .await?;
                    g::decided(&state, tx, &store, &submitted.unit, ctx.subject_id()).await?;
                }
                let receipt = SubmitReceipt {
                    applied: submitted.applied,
                    unit: submitted.unit.into(),
                    sku: g::find(tx, &scope, tenant, id).await?.into(),
                };
                if let Some(input) = claim {
                    let body = serde_json::to_value(&receipt).map_err(|e| {
                        TxError::Repo(crate::infra::storage::RepoError::Db(e.to_string()))
                    })?;
                    if matches!(
                        repo::answer_idempotency_key(
                            tx,
                            &scope,
                            tenant,
                            &input.endpoint,
                            &input.client_key,
                            200,
                            body
                        )
                        .await
                        .map_err(TxError::Repo)?,
                        repo::IdempotencyAnswer::NotHeld
                    ) {
                        return Err(g::conflict("IDEMPOTENCY_CONFLICT", "claim lost"));
                    }
                }
                Ok(receipt)
            })
        })
        .await;
    match receipt {
        Ok(receipt) => Ok(Json(receipt).into_response()),
        Err(TxError::FencedReferences { code, rows }) => {
            let error = crate::domain::error::DomainError::Conflict {
                code,
                detail: "live references block the fence".into(),
            };
            let mut problem =
                toolkit::api::canonical_prelude::Problem::from(CanonicalError::from(error));
            problem.context["references"] = rows;
            Ok(problem.into_response())
        }
        Err(e) => Err(tx_to_canonical(e)),
    }
}
async fn unfence(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(&enforcer, &ctx, actions::SUBMIT, false).await?;
    let sku = state
        .db
        .db()
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let scope = scope.clone();
            let ctx = ctx.clone();
            Box::pin(async move {
                g::find(tx, &scope, ctx.subject_tenant_id(), id).await?;
                let result = repo::unfence_sku(tx, &scope, ctx.subject_tenant_id(), id, None)
                    .await
                    .map_err(TxError::Repo)?;
                let repo::HeadWrite::Written(sku) = result else {
                    return Err(g::conflict(
                        "ROW_LOCKED_PENDING",
                        "a pending unit owns this fence",
                    ));
                };
                g::audit(
                    tx,
                    &scope,
                    &ctx,
                    "sku.unfence",
                    "sku",
                    id,
                    None,
                    OffsetDateTime::now_utc(),
                )
                .await?;
                Ok(sku)
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    Ok(Json(SkuDto::from(sku)).into_response())
}
#[cfg(test)]
#[path = "sku_governance_tests.rs"]
mod sku_governance_tests;

/// Optional replay header in the generated API contract.
fn idempotency_param() -> toolkit::api::operation_builder::ParamSpec {
    toolkit::api::operation_builder::ParamSpec {
        name: "Idempotency-Key".into(),
        location: toolkit::api::operation_builder::ParamLocation::Header,
        required: false,
        description: Some("Replay this concrete endpoint's stored receipt for 24 hours".into()),
        param_type: "string".into(),
        array: false,
    }
}
