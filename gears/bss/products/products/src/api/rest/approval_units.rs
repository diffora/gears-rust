//! @cpt-dod:cpt-cf-bss-products-dod-unit-contended:p1
//! @cpt-dod:cpt-cf-bss-products-dod-stale-refresh-generation:p1
//! @cpt-dod:cpt-cf-bss-products-dod-sod-excludes-authors:p1
//! Approval queue and generation-bound decisions on the caller's transaction.
use super::{
    ApiState, TxError, category_tx_config,
    closed_sets::ProductsVoteOutcome,
    contention_db_err,
    dto::{UnitDto, UnitList, VoteReceipt, VoteRequest},
    governance as g, json_body, replay, require_authenticated, tx_to_canonical,
    unit_tx_to_canonical,
};
use crate::{
    authz::{actions, resource_types},
    domain::{
        approvals::{
            KIND_SKU_CHANGE, KIND_SKU_PUBLISH, KIND_SKU_RETIRE, SkuProposal, Subject,
            change::SkuChange, publish::SkuPublish, retire::SkuRetire,
        },
        error::DomainError,
        recognized::UsageTypeAnswer,
        sku::SkuPatch,
    },
    infra::{
        events::{self, TxOutbox},
        storage::repo,
    },
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{
    Extension, Json, Router,
    extract::{
        Path, Query,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use bss_approval::{
    ApprovalError, ApprovalSubject, ApproveOutcome, Engine, RejectOutcome, Store, Unit, UnitState,
};
use bss_products_sdk::models::SkuContent;
use std::sync::Arc;
use time::OffsetDateTime;
use toolkit::api::{
    OpenApiRegistry, canonical_prelude::CanonicalError, operation_builder::OperationBuilder,
};
use toolkit_db::{DbTx, secure::AccessScope};
use toolkit_security::SecurityContext;
use uuid::Uuid;

#[toolkit_macros::api_dto(request)]
struct ListQuery {
    state: Option<String>,
    kind: Option<String>,
    ref_id: Option<Uuid>,
    /// Page size (P-D-224): 200 by default, clamped at 500.
    limit: Option<u64>,
    /// The opaque continuation of a page's `page_info.next_cursor`.
    cursor: Option<String>,
}
#[derive(Clone, Copy)]
enum Vote {
    Approve,
    Reject,
    Withdraw,
}
/// Register the queue, card and three decision operations.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = OperationBuilder::get("/bss-products/v1/approval-units")
        .operation_id("bss_products.list_approval_units")
        .summary("list_approval_units")
        .description(
            "One page of the tenant's approval units in submission order (P-D-224), filtered by \
             state, kind and SKU, each with its stored snapshot and the decisions of every \
             generation. `limit` (default 200, clamped at 500) and `cursor` from `page_info` \
             page it. Refusals: 400 for an unknown state or a query that does not parse; 400 \
             FILTER_MISMATCH for a cursor replayed with another state, kind or SKU; 400 for a \
             cursor that does not read.",
        )
        .tag("Approval units")
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "state",
            false,
            "Unit state: pending, approved, rejected or withdrawn",
            "string",
        )
        .query_param_typed(
            "kind",
            false,
            "Approval kind: sku_publish, sku_change or sku_retire",
            "string",
        )
        .query_param_typed("ref_id", false, "SKU id (a UUID)", "string")
        .query_param_typed(
            "limit",
            false,
            "Page size (default 200, clamped at 500)",
            "integer",
        )
        .query_param_typed("cursor", false, "Continuation from page_info", "string")
        .handler(list)
        .json_response_with_schema::<UnitList>(openapi, StatusCode::OK, "Result")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::get("/bss-products/v1/approval-units/{id}")
        .operation_id("bss_products.get_approval_unit")
        .summary("get_approval_unit")
        .tag("Approval units")
        .authenticated()
        .no_license_required()
        .path_param("id", "Unit id")
        .handler(get)
        .json_response_with_schema::<UnitDto>(openapi, StatusCode::OK, "Result")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-products/v1/approval-units/{id}/approve")
        .operation_id("bss_products.approve_unit")
        .summary("approve_unit")
        .description(
            "Approves the unit at the generation its reviewer saw. The note is at most 2000 \
             characters (the approval engine's cap). Refusals include 400 NOTE_TOO_LONG on a \
             longer note, 400 GENERATION_MISMATCH, 400 UNIT_STALE after a refresh, 403 \
             SOD_VIOLATION and 409 DUPLICATE_VOTE.",
        )
        .tag("Approval units")
        .authenticated()
        .no_license_required()
        .path_param("id", "Unit id")
        .json_request::<VoteRequest>(openapi, "Generation reviewed and optional note")
        .param(replay::param())
        .handler(approve)
        .json_response_with_schema::<VoteReceipt>(openapi, StatusCode::OK, "Result")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-products/v1/approval-units/{id}/reject")
        .operation_id("bss_products.reject_unit")
        .summary("reject_unit")
        .description(
            "Rejects the unit at the generation its reviewer saw; a reject needs a note. The \
             note is at most 2000 characters (the approval engine's cap). Refusals include 400 \
             NOTE_TOO_LONG on a longer note and 400 NOTE_REQUIRED without one, 400 \
             GENERATION_MISMATCH, 400 UNIT_STALE after a refresh, 403 SOD_VIOLATION and 409 \
             DUPLICATE_VOTE.",
        )
        .tag("Approval units")
        .authenticated()
        .no_license_required()
        .path_param("id", "Unit id")
        .json_request::<VoteRequest>(openapi, "Generation reviewed and optional note")
        .param(replay::param())
        .handler(reject)
        .json_response_with_schema::<VoteReceipt>(openapi, StatusCode::OK, "Result")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-products/v1/approval-units/{id}/withdraw")
        .operation_id("bss_products.withdraw_unit")
        .summary("withdraw_unit")
        .tag("Approval units")
        .authenticated()
        .no_license_required()
        .path_param("id", "Unit id")
        .param(replay::param())
        .handler(withdraw)
        .json_response_with_schema::<VoteReceipt>(openapi, StatusCode::OK, "Result")
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
async fn approve(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(
        &enforcer,
        &ctx,
        &resource_types::APPROVAL_UNIT,
        actions::APPROVE,
    )
    .await?;
    let body = json_body(body)?;
    vote(state, scope, ctx, id, Vote::Approve, Some(body), headers).await
}
async fn reject(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(
        &enforcer,
        &ctx,
        &resource_types::APPROVAL_UNIT,
        actions::APPROVE,
    )
    .await?;
    let body = json_body(body)?;
    vote(state, scope, ctx, id, Vote::Reject, Some(body), headers).await
}
async fn withdraw(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(
        &enforcer,
        &ctx,
        &resource_types::APPROVAL_UNIT,
        actions::SUBMIT,
    )
    .await?;
    vote(state, scope, ctx, id, Vote::Withdraw, None, headers).await
}
async fn list(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(
        &enforcer,
        &ctx,
        &resource_types::APPROVAL_UNIT,
        actions::READ,
    )
    .await?;
    let Query(q) =
        query.map_err(|e| CanonicalError::from(g::validation("query", e.to_string())))?;
    let filter = q
        .state
        .as_deref()
        .map(|s| {
            UnitState::parse(s)
                .ok_or_else(|| CanonicalError::from(g::validation("state", "unknown unit state")))
        })
        .transpose()?;
    let filter = repo::UnitListFilter {
        state: filter,
        kind: q.kind,
        ref_id: q.ref_id,
    };
    let page = unit_page(&filter, q.limit, q.cursor.as_deref())?;
    let tenant = ctx.subject_tenant_id();
    let list = state
        .db
        .db()
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let (scope, filter, page) = (scope.clone(), filter.clone(), page.clone());
            Box::pin(async move {
                // One page, and all its units' decisions in one read (P-D-224): the same
                // statements whatever the page's size.
                let page = repo::page_units(tx, &scope, tenant, &filter, &page)
                    .await
                    .map_err(|e| match e {
                        repo::UnitListError::Query(e) => TxError::OData(e),
                        repo::UnitListError::Repo(e) => TxError::Repo(e),
                    })?;
                let ids: Vec<Uuid> = page.items.iter().map(|u| u.id).collect();
                let mut decisions = repo::decisions_of_units(tx, &scope, tenant, &ids)
                    .await
                    .map_err(TxError::Repo)?;
                let items = page
                    .items
                    .into_iter()
                    .map(|unit| {
                        let id = unit.id;
                        let mut dto = UnitDto::from(unit);
                        dto.decisions = decisions
                            .remove(&id)
                            .unwrap_or_default()
                            .into_iter()
                            .map(Into::into)
                            .collect();
                        dto
                    })
                    .collect();
                Ok(UnitList {
                    items,
                    page_info: page.page_info,
                })
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    Ok(Json(list).into_response())
}
/// The unit list's page (P-D-224): `limit`, and `cursor` from a page's `page_info`, which carries a
/// hash of the narrowing (`state`, `kind` and `ref_id`), so a cursor replayed under another is 400
/// `FILTER_MISMATCH`, as pricing's list's is (D-458).
fn unit_page(
    filter: &repo::UnitListFilter,
    limit: Option<u64>,
    cursor: Option<&str>,
) -> Result<toolkit_odata::ODataQuery, CanonicalError> {
    let narrowing = serde_json::json!({
        "state": filter.state.map(UnitState::as_str),
        "kind": filter.kind,
        "ref_id": filter.ref_id,
    });
    let digest = crate::domain::canonical::content_digest(
        &crate::domain::canonical::canonical_rendering(&narrowing),
    );
    let hash = digest
        .iter()
        .take(8)
        .fold(String::with_capacity(16), |mut hex, b| {
            const DIGITS: &[u8; 16] = b"0123456789abcdef";
            hex.push(char::from(DIGITS[usize::from(b >> 4)]));
            hex.push(char::from(DIGITS[usize::from(b & 0x0f)]));
            hex
        });
    let mut query = toolkit_odata::ODataQuery::new().with_filter_hash(hash.clone());
    if let Some(limit) = limit {
        query = query.with_limit(limit);
    }
    if let Some(token) = cursor {
        let cursor = toolkit_odata::CursorV1::decode(token).map_err(CanonicalError::from)?;
        if cursor.f.as_deref() != Some(hash.as_str()) {
            return Err(toolkit_odata::Error::FilterMismatch.into());
        }
        query = query.with_cursor(cursor);
    }
    Ok(query)
}
/// Every embedded unit reports the decisions actually stored for all generations.
async fn with_decisions(
    tx: &DbTx<'_>,
    store: &repo::ProductsApprovalStore,
    unit: Unit,
) -> Result<UnitDto, TxError> {
    let decisions = store
        .decisions(tx, unit.id)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    let mut dto = UnitDto::from(unit);
    dto.decisions = decisions;
    Ok(dto)
}

async fn get(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(
        &enforcer,
        &ctx,
        &resource_types::APPROVAL_UNIT,
        actions::READ,
    )
    .await?;
    let ttl = state.fence_ttl_minutes;
    let card = state
        .db
        .db()
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let scope = scope.clone();
            let ctx = ctx.clone();
            Box::pin(async move {
                let store = repo::ProductsApprovalStore {
                    scope: scope.clone(),
                    tenant_id: ctx.subject_tenant_id(),
                };
                let unit = load(tx, &store, id).await?;
                let scope = AccessScope::for_tenant(unit.tenant_id);
                g::expire(
                    tx,
                    &scope,
                    ctx.subject_tenant_id(),
                    unit.ref_id,
                    ttl,
                    OffsetDateTime::now_utc(),
                )
                .await?;
                // A never-published draft may be deleted after its unit was rejected or withdrawn
                // (P-D-206): the unit stays readable, with no live SKU to recompute against.
                let live = repo::find_sku(tx, &scope, ctx.subject_tenant_id(), unit.ref_id)
                    .await
                    .map_err(TxError::Repo)?;
                let decisions = store
                    .decisions(tx, id)
                    .await?
                    .into_iter()
                    .map(Into::into)
                    .collect();
                let mut dto = UnitDto::from(unit);
                dto.decisions = decisions;
                dto.impact_live = live
                    .map(|live| {
                        serde_json::to_value(super::dto::SkuDto::from(live))
                            .map_err(|e| TxError::from(ApprovalError::Store(e.to_string())))
                    })
                    .transpose()?;
                Ok(dto)
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    Ok(Json(card).into_response())
}
async fn load(
    tx: &DbTx<'_>,
    store: &repo::ProductsApprovalStore,
    id: Uuid,
) -> Result<Unit, TxError> {
    store
        .unit(tx, id)
        .await?
        .ok_or(TxError::Refused(DomainError::NotFound {
            what: "approval_unit",
            id,
        }))
}
fn proposal(value: &serde_json::Value) -> Result<SkuProposal, TxError> {
    serde_json::from_value(value.clone())
        .map_err(|e| TxError::from(ApprovalError::Store(e.to_string())))
}
/// Recover only fields changed by the original proposal, leaving untouched live fields visible to refresh.
fn patch_between(before: &SkuProposal, after: &SkuProposal) -> SkuPatch {
    let a = &before.content;
    let b = &after.content;
    let mut p = SkuPatch {
        lifecycle: after.lifecycle,
        ..SkuPatch::default()
    };
    macro_rules! field {
        ($f:ident) => {
            if a.$f != b.$f {
                p.$f = Some(b.$f.clone());
            }
        };
    }
    field!(name);
    field!(category_id);
    field!(description);
    field!(sellable);
    field!(gl_code);
    field!(tax_category);
    field!(invoice_line_template);
    field!(billing_timing);
    field!(usage_type_ref);
    field!(unit);
    field!(r#type);
    p
}
async fn subject(
    outbox: &TxOutbox,
    tx: &DbTx<'_>,
    store: &repo::ProductsApprovalStore,
    ctx: &SecurityContext,
    unit: &Unit,
    usage: Option<UsageTypeAnswer>,
) -> Result<Subject, TxError> {
    let sku_scope = AccessScope::for_tenant(store.tenant_id);
    let base = SkuPublish {
        scope: sku_scope.clone(),
        tenant_id: store.tenant_id,
        outbox: outbox.clone(),
        actor: ctx.subject_id(),
        now: OffsetDateTime::now_utc(),
        usage_type: usage,
    };
    let fence = repo::find_sku_fence(tx, &sku_scope, store.tenant_id, unit.ref_id)
        .await
        .map_err(TxError::Repo)?
        .ok_or(TxError::Refused(DomainError::NotFound {
            what: "sku",
            id: unit.ref_id,
        }))?;
    match unit.kind.as_str() {
        KIND_SKU_PUBLISH => Ok(Subject::Publish(base)),
        KIND_SKU_RETIRE => Ok(Subject::Retire(SkuRetire {
            base,
            fence_op_id: fence
                .fence_op_id
                .ok_or_else(|| g::conflict("SKU_FENCED", "retire fence missing"))?,
        })),
        KIND_SKU_CHANGE => {
            let items = store.items(tx, unit.id).await?;
            let item = items
                .first()
                .ok_or_else(|| TxError::from(ApprovalError::Empty))?;
            let after = proposal(&item.after)?;
            let before = proposal(item.before.as_ref().ok_or_else(|| {
                TxError::from(ApprovalError::Store("change before missing".into()))
            })?)?;
            let mut patch = patch_between(&before, &after);
            if fence.type_change_pending {
                patch.r#type = Some(after.content.r#type);
            }
            Ok(Subject::Change(SkuChange {
                base,
                patch,
                effective_from: unit.common_effective_date.ok_or_else(|| {
                    TxError::from(ApprovalError::Store("change date missing".into()))
                })?,
                fence_op_id: if fence.type_change_pending {
                    fence.fence_op_id
                } else {
                    None
                },
            }))
        }
        _ => Err(TxError::from(ApprovalError::Store(
            "unknown approval kind".into(),
        ))),
    }
}
async fn proposed(subject: &Subject, tx: &DbTx<'_>, unit: &Unit) -> Result<SkuContent, TxError> {
    let items = subject.collect(tx, &[unit.ref_id]).await?;
    let first = items
        .first()
        .ok_or_else(|| TxError::from(ApprovalError::Empty))?;
    if unit.kind == KIND_SKU_CHANGE {
        Ok(proposal(&first.after)?.content)
    } else {
        serde_json::from_value(first.after.clone())
            .map_err(|e| TxError::from(ApprovalError::Store(e.to_string())))
    }
}
/// @cpt-cf-bss-products-fr-concurrency-idempotency
async fn vote(
    state: Arc<ApiState>,
    scope: AccessScope,
    ctx: SecurityContext,
    id: Uuid,
    action: Vote,
    body: Option<serde_json::Value>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let suffix = match action {
        Vote::Approve => "approve",
        Vote::Reject => "reject",
        Vote::Withdraw => "withdraw",
    };
    let claim = replay::input(
        &state,
        &headers,
        format!("/bss-products/v1/approval-units/{id}/{suffix}"),
        &body.clone().unwrap_or_else(|| serde_json::json!({})),
    )?;
    let body: Option<VoteRequest> = body
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| CanonicalError::from(g::validation("body", e.to_string())))?;
    // Check resource authorization even for a receipt replay, without requiring Pending.
    let conn = state.db.conn().map_err(|e| tx_to_canonical(e.into()))?;
    if repo::find_unit(&conn, &scope, ctx.subject_tenant_id(), id)
        .await
        .map_err(|e| tx_to_canonical(e.into()))?
        .is_none()
    {
        return Err(DomainError::NotFound {
            what: "approval_unit",
            id,
        }
        .into());
    }
    if let Some(response) = replay::lookup(&conn, ctx.subject_tenant_id(), claim.as_ref())
        .await
        .map_err(tx_to_canonical)?
    {
        return Ok(response);
    }
    let mut resolved_ref = None;
    let mut usage = None;
    if matches!(action, Vote::Approve) {
        let content = review_content(&state, &scope, &ctx, id).await?;
        if let Some(content) = content {
            resolved_ref = content.usage_type_ref.clone();
            usage = g::resolve(&state, &ctx, &content).await?;
        }
    }
    let seen = body.as_ref().map(|b| b.generation);
    let note = body.and_then(|b| b.note);
    let (db, sink, config) = (
        state.db.db(),
        state.sink.clone(),
        category_tx_config(&state),
    );
    let result = events::transaction(&db, &sink, config, contention_db_err, move |tx, outbox| {
        let scope = scope.clone();
        let ctx = ctx.clone();
        let usage = usage.clone();
        let resolved_ref = resolved_ref.clone();
        let note = note.clone();
        let claim = claim.clone();
        Box::pin(async move {
            let store = repo::ProductsApprovalStore {
                scope: scope.clone(),
                tenant_id: ctx.subject_tenant_id(),
            };
            let mut unit = load(tx, &store, id).await?;
            if let Some(response) =
                replay::begin(tx, ctx.subject_tenant_id(), claim.as_ref()).await?
            {
                return Ok(response);
            }
            if unit.state != UnitState::Pending {
                return Err(ApprovalError::AlreadyDecided.into());
            }
            let sub = subject(&outbox, tx, &store, &ctx, &unit, usage).await?;
            // P-D-213: the SKU's lifecycle before the decision, and after it below.
            let found = g::lifecycle(tx, ctx.subject_tenant_id(), unit.ref_id).await?;
            let now = OffsetDateTime::now_utc();
            let outcome = match action {
                Vote::Approve => {
                    if unit.kind != KIND_SKU_RETIRE
                        && proposed(&sub, tx, &unit).await?.usage_type_ref != resolved_ref
                    {
                        return Err(g::conflict(
                            "STALE_REVISION",
                            "meter changed during resolution; retry",
                        ));
                    }
                    Engine::approve(
                        &store,
                        &sub,
                        tx,
                        id,
                        ctx.subject_id(),
                        seen.ok_or_else(|| {
                            TxError::Refused(g::validation("generation", "required"))
                        })?,
                        note.as_deref(),
                        now,
                    )
                    .await?
                }
                Vote::Reject => {
                    let note = note
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or(ApprovalError::NoteRequired)?;
                    let seen = seen
                        .ok_or_else(|| TxError::Refused(g::validation("generation", "required")))?;
                    // A stale unit is refreshed without applying or resolving the catalog.
                    match Engine::reject(&store, &sub, tx, id, ctx.subject_id(), seen, note, now)
                        .await?
                    {
                        RejectOutcome::Refreshed { generation } => {
                            ApproveOutcome::Refreshed { generation }
                        }
                        RejectOutcome::Rejected => ApproveOutcome::Applied,
                    }
                }
                Vote::Withdraw => {
                    Engine::withdraw(&store, &sub, tx, id, ctx.subject_id(), now).await?;
                    ApproveOutcome::Applied
                }
            };
            let (label, have, need) = match outcome {
                ApproveOutcome::Refreshed { generation } => {
                    decision_audit(tx, &scope, &ctx, "refreshed", &unit, found, None, now).await?;
                    let mut problem = toolkit::api::canonical_prelude::Problem::from(
                        CanonicalError::from(DomainError::StaleUnit { generation }),
                    );
                    problem.context["generation"] = serde_json::json!(generation);
                    return replay::finish(
                        tx,
                        ctx.subject_tenant_id(),
                        claim.as_ref(),
                        StatusCode::BAD_REQUEST,
                        &problem,
                    )
                    .await;
                }
                ApproveOutcome::Pending { have, need } => {
                    (ProductsVoteOutcome::Pending, Some(have), Some(need))
                }
                ApproveOutcome::Applied => (
                    match action {
                        Vote::Approve => ProductsVoteOutcome::Applied,
                        Vote::Reject => ProductsVoteOutcome::Rejected,
                        Vote::Withdraw => ProductsVoteOutcome::Withdrawn,
                    },
                    None,
                    None,
                ),
            };
            decision_audit(tx, &scope, &ctx, label.as_str(), &unit, found, note, now).await?;
            if matches!(outcome, ApproveOutcome::Applied) {
                unit = load(tx, &store, id).await?;
                g::decided(&outbox, tx, &store, &unit, ctx.subject_id()).await?;
            }
            let receipt = VoteReceipt {
                have,
                need,
                outcome: label,
                unit: with_decisions(tx, &store, unit).await?,
            };
            replay::finish(
                tx,
                ctx.subject_tenant_id(),
                claim.as_ref(),
                StatusCode::OK,
                &receipt,
            )
            .await
        })
    })
    .await;
    match result {
        Ok(receipt) => Ok(receipt),
        Err(TxError::GenerationMismatch { seen, current }) => Ok(g::generation_problem(
            DomainError::from(ApprovalError::GenerationMismatch { seen, current }).into(),
            current,
        )),
        Err(e) => Err(unit_tx_to_canonical(e)),
    }
}

/// A decision's audit row (P-D-193) for its outcome `label`, with the SKU lifecycle move it made
/// (P-D-213): `found` before the decision, and the lifecycle it left, read now.
#[allow(
    clippy::too_many_arguments,
    reason = "The row's actor, unit, outcome and move stay explicit at the one decision door"
)]
async fn decision_audit(
    tx: &DbTx<'_>,
    scope: &AccessScope,
    ctx: &SecurityContext,
    label: &str,
    unit: &Unit,
    found: bss_products_sdk::models::Lifecycle,
    note: Option<String>,
    now: OffsetDateTime,
) -> Result<(), TxError> {
    let action = match label {
        "refreshed" => "approval.refreshed",
        "pending" => "approval.vote",
        "rejected" => "approval.rejected",
        "withdrawn" => "approval.withdrawn",
        _ => "approval.approved",
    };
    let left = g::lifecycle(tx, ctx.subject_tenant_id(), unit.ref_id).await?;
    let moved = repo::LifecycleMove::between(found, left);
    g::audit(
        tx,
        scope,
        ctx,
        action,
        "approval_unit",
        unit.id,
        note,
        now,
        moved,
    )
    .await
}

/// Load the authorized unit's proposed content before external catalog resolution.
async fn review_content(
    state: &Arc<ApiState>,
    scope: &AccessScope,
    ctx: &SecurityContext,
    id: Uuid,
) -> Result<Option<SkuContent>, CanonicalError> {
    let scope = scope.clone();
    let ctx_tx = ctx.clone();
    // It enqueues nothing, but the subject it reads through takes the attempt's handle (P-D-221).
    events::transaction(
        &state.db.db(),
        &state.sink,
        category_tx_config(state),
        contention_db_err,
        move |tx, outbox| {
            let scope = scope.clone();
            let ctx = ctx_tx.clone();
            Box::pin(async move {
                let store = repo::ProductsApprovalStore {
                    scope,
                    tenant_id: ctx.subject_tenant_id(),
                };
                let unit = load(tx, &store, id).await?;
                if unit.state != UnitState::Pending {
                    return Err(ApprovalError::AlreadyDecided.into());
                }
                if unit.kind == KIND_SKU_RETIRE {
                    return Ok(None);
                }
                let sub = subject(&outbox, tx, &store, &ctx, &unit, None).await?;
                Ok(Some(proposed(&sub, tx, &unit).await?))
            })
        },
    )
    .await
    .map_err(tx_to_canonical)
}
