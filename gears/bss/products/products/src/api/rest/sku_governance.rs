//! Governed submissions claim replay first, then fence and submit atomically.
//! @cpt-dod:cpt-cf-bss-products-dod-sku-retire-fenced:p1
//! @cpt-dod:cpt-cf-bss-products-dod-type-change-fenced:p1
use super::{
    ApiState, TxError, category_tx_config, contention_db_err,
    dto::{ProductsSkuSubmitRequest, SkuChangeRequest, SkuDto, SubmitReceipt},
    governance as g, json_body, replay, require_authenticated, tx_to_canonical,
    unit_tx_to_canonical,
};
use crate::{
    authz::{actions, resource_types},
    domain::{
        approvals::{
            Subject, change::SkuChange, check_note, publish::SkuPublish, retire::SkuRetire,
        },
        derived,
        sku::{SkuPatch, apply_patch},
        validation::ValidationReport,
    },
    infra::{events, idempotency::IdempotencyClaimInput, storage::repo},
};
use authz_resolver_sdk::PolicyEnforcer;
use axum::{
    Extension, Json, Router,
    extract::{Path, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use bss_approval::{Engine, SubmitRequest};
use bss_products_sdk::models::{Lifecycle, SkuContent};
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
        .description(
            "Submits a draft SKU for publication; at quorum 0 the submit is the publish. A usage \
             SKU's GTS ref is resolved through the usage-type catalog (P-D-184); a derived ref \
             (`products.derived/<code>@<n>`) is read from this gear's store, never from the \
             catalog, and the first publish pins it (P-D-232). Refusals include 400 \
             USAGE_NEEDS_METER, USAGE_TYPE_UNRESOLVED, DERIVED_USAGE_TYPE_UNKNOWN and \
             DERIVED_UNIT_MISMATCH, 400 NOTE_TOO_LONG, 403 USAGE_TYPE_FORBIDDEN, 409 NOT_A_DRAFT \
             and 503 USAGE_TYPE_UNAVAILABLE.",
        )
        .tag("SKU governance")
        .authenticated()
        .no_license_required()
        .path_param("id", "SKU id")
        .json_request::<ProductsSkuSubmitRequest>(
            openapi,
            "Optional: the submitter's note, at most 2000 characters (400 NOTE_TOO_LONG); stored \
             on the unit as submit_note and on the submit's history row (P-D-219)",
        )
        .request_optional()
        .param(replay::param())
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
        .description(
            "Submits a change of a published SKU for approval, effective from `effective_from`. \
             The texts it carries have the caps of the create (P-D-225), and the note at most \
             2000 characters. A usage SKU keeps the derived usage type it was first published on \
             (P-D-232): a change that moves it to another version, to a GTS ref, from a GTS ref, \
             or drops it (a type change included) is refused before any catalog is asked. \
             Refusals include 400 FIELD_TOO_LONG on a text over its cap, 400 NOTE_TOO_LONG on \
             the note, 400 DERIVED_PIN_IMMUTABLE, and 400 DERIVED_UNIT_MISMATCH for a unit other \
             than the pinned version's output unit.",
        )
        .tag("SKU governance")
        .authenticated()
        .no_license_required()
        .path_param("id", "SKU id")
        .json_request::<SkuChangeRequest>(openapi, "Request")
        .param(replay::param())
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
        .json_request::<ProductsSkuSubmitRequest>(
            openapi,
            "Optional: the submitter's note, at most 2000 characters (400 NOTE_TOO_LONG); stored \
             on the unit as submit_note and on the submit's history row (P-D-219)",
        )
        .request_optional()
        .param(replay::param())
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
        .param(replay::param())
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
    let scope = g::scope(&enforcer, &ctx, &resource_types::SKU, actions::SUBMIT).await?;
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
    let scope = g::scope(&enforcer, &ctx, &resource_types::SKU, actions::SUBMIT).await?;
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
    let scope = g::scope(&enforcer, &ctx, &resource_types::SKU, actions::SUBMIT).await?;
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
            .map(super::dto::ReferenceReceipt::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(TxError::Repo)?;
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
    let lifecycle = repo::fence_lifecycle(&s).map_err(TxError::Repo)?;
    if let Some(op) = s.fence_op_id
        && match kind {
            repo::Fence::Retire => lifecycle == Lifecycle::Retiring,
            repo::Fence::TypeChange => {
                s.type_change_pending
                    && matches!(lifecycle, Lifecycle::Published | Lifecycle::Deprecated)
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
    // The submitter's `note`, on each of the three doors: stored on the unit (`submit_note`,
    // P-D-219) and on the submit's audit row, which the history shows (P-D-213). The body's other
    // violations and the note's length are one stage (P-D-202).
    let (patch, date, note) = if matches!(kind, SubmitKind::Change) {
        let parsed: SkuChangeRequest = serde_json::from_value(payload.clone())
            .map_err(|e| CanonicalError::from(g::validation("body", e.to_string())))?;
        let (patch, mut report) = match SkuPatch::try_from(parsed.patch) {
            Ok(patch) => (patch, ValidationReport::new()),
            Err(report) => (SkuPatch::default(), report),
        };
        crate::domain::sku::check_patch(&mut report, &patch);
        check_note(parsed.note.as_deref(), &mut report);
        if !report.is_empty() {
            return Err(crate::domain::error::DomainError::Validation(report).into());
        }
        (
            patch,
            Some(parsed.effective_from.unwrap_or(now.date())),
            parsed.note,
        )
    } else {
        let parsed: ProductsSkuSubmitRequest = serde_json::from_value(payload.clone())
            .map_err(|e| CanonicalError::from(g::validation("body", e.to_string())))?;
        let mut report = ValidationReport::new();
        check_note(parsed.note.as_deref(), &mut report);
        if !report.is_empty() {
            return Err(crate::domain::error::DomainError::Validation(report).into());
        }
        (SkuPatch::default(), None, parsed.note)
    };
    let claim = replay::input(
        &state,
        &headers,
        format!("/bss-products/v1/skus/{id}/{}", kind.suffix()),
        &payload,
    )?;
    let conn = state.db.conn().map_err(|e| tx_to_canonical(e.into()))?;
    g::find(&conn, &scope, tenant, id)
        .await
        .map_err(tx_to_canonical)?;
    if let Some(response) = replay::lookup(&conn, tenant, claim.as_ref())
        .await
        .map_err(tx_to_canonical)?
    {
        return Ok(response);
    }
    execute(state, scope, ctx, id, kind, patch, date, note, now, claim).await
}
#[expect(
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
    note: Option<String>,
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
    // P-D-232: a change that moves a derived pin is refused before any catalog is asked. The rule
    // is `SkuChange::validate_change`'s, judged again in the transaction and at apply.
    if matches!(kind, SubmitKind::Change)
        && derived::pin_moves(
            current.usage_type_ref.as_deref(),
            proposed.usage_type_ref.as_deref(),
        )
    {
        return Err(derived::pin_immutable().into());
    }
    let usage = if matches!(kind, SubmitKind::Retire) {
        None
    } else {
        g::resolve(&state, &ctx, &proposed).await?
    };
    let (db, sink, config) = (
        state.db.db(),
        state.sink.clone(),
        category_tx_config(&state),
    );
    // The one setting the attempts read, copied once rather than an `Arc<ApiState>` per attempt
    // (RS-55).
    let ttl = state.fence_ttl_minutes;
    let receipt = events::transaction(&db, &sink, config, contention_db_err, move |tx, outbox| {
        let scope = scope.clone();
        let ctx = ctx.clone();
        let patch = patch.clone();
        let usage = usage.clone();
        let proposed = proposed.clone();
        let claim = claim.clone();
        let note = note.clone();
        Box::pin(async move {
            g::find(tx, &scope, tenant, id).await?;
            if let Some(response) = replay::begin(tx, tenant, claim.as_ref()).await? {
                return Ok(response);
            }
            // The authorized SKU anchors unit, policy and reference work.
            let scope = AccessScope::for_tenant(tenant);
            g::expire(tx, &scope, tenant, id, ttl, now).await?;
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
                && !patch
                    .r#type
                    .is_some_and(|proposed| proposed != current.r#type)
                && current.type_change_pending
            {
                return Err(g::conflict(
                    "SKU_FENCED",
                    "resume the type change or unfence it first",
                ));
            }
            // P-D-213: the lifecycle the submit found, and the one it leaves before any apply:
            // only a retire moves it, by its fence (a fence taken now, or the orphan resumed,
            // is `retiring` either way); a type-change fence moves none.
            let found = current.lifecycle;
            let fenced = if matches!(kind, SubmitKind::Retire) {
                Lifecycle::Retiring
            } else {
                found
            };
            let base = SkuPublish {
                scope: scope.clone(),
                tenant_id: tenant,
                outbox: outbox.clone(),
                actor: ctx.subject_id(),
                now,
                usage_type: usage,
            };
            let subject = match kind {
                SubmitKind::Publish => Subject::Publish(base),
                SubmitKind::Retire => Subject::Retire(SkuRetire {
                    base,
                    fence_op_id: fence(tx, &scope, tenant, id, repo::Fence::Retire, now).await?,
                }),
                SubmitKind::Change => {
                    let fence_op_id = if patch
                        .r#type
                        .is_some_and(|proposed| proposed != current.r#type)
                    {
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
                    note: note.as_deref(),
                    now,
                },
            )
            .await?;
            g::audit(
                tx,
                &ctx,
                "approval.submit",
                "approval_unit",
                submitted.unit.id,
                note,
                now,
                repo::LifecycleMove::between(found, fenced),
            )
            .await?;
            let after = g::find(tx, &scope, tenant, id).await?;
            if submitted.applied {
                g::audit(
                    tx,
                    &ctx,
                    "approval.applied",
                    "approval_unit",
                    submitted.unit.id,
                    None,
                    now,
                    repo::LifecycleMove::between(fenced, after.lifecycle),
                )
                .await?;
                g::decided(&outbox, tx, &store, &submitted.unit, ctx.subject_id()).await?;
            }
            let receipt = SubmitReceipt {
                applied: submitted.applied,
                unit: submitted.unit.into(),
                sku: after.into(),
            };
            replay::finish(tx, tenant, claim.as_ref(), StatusCode::OK, &receipt).await
        })
    })
    .await;
    match receipt {
        Ok(receipt) => Ok(receipt),
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
        Err(e) => Err(unit_tx_to_canonical(e)),
    }
}
async fn unfence(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<PolicyEnforcer>,
    ctx: Option<Extension<SecurityContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(ctx)?;
    let scope = g::scope(&enforcer, &ctx, &resource_types::SKU, actions::SUBMIT).await?;
    let claim = replay::input(
        &state,
        &headers,
        format!("/bss-products/v1/skus/{id}/unfence"),
        &serde_json::json!({}),
    )?;
    let sku = state
        .db
        .db()
        .transaction_with_retry(category_tx_config(&state), contention_db_err, move |tx| {
            let scope = scope.clone();
            let ctx = ctx.clone();
            let claim = claim.clone();
            Box::pin(async move {
                let found = g::find(tx, &scope, ctx.subject_tenant_id(), id)
                    .await?
                    .lifecycle;
                if let Some(response) =
                    replay::begin(tx, ctx.subject_tenant_id(), claim.as_ref()).await?
                {
                    return Ok(response);
                }
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
                    &ctx,
                    "sku.unfence",
                    "sku",
                    id,
                    None,
                    OffsetDateTime::now_utc(),
                    repo::LifecycleMove::between(found, sku.lifecycle),
                )
                .await?;
                replay::finish(
                    tx,
                    ctx.subject_tenant_id(),
                    claim.as_ref(),
                    StatusCode::OK,
                    &SkuDto::from(sku),
                )
                .await
            })
        })
        .await
        .map_err(tx_to_canonical)?;
    Ok(sku)
}
#[cfg(test)]
#[path = "sku_governance_tests.rs"]
mod sku_governance_tests;
