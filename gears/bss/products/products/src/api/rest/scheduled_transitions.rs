//! Scheduled-transition doors (**P-D-134**): the GET surface
//! (`dod-deferred-intent`) and the governed cancel.
//!
//! `× write` is not minted: the retire doors write the rows under
//! `sku × write` / `product × write` today, so this module spends only
//! `scheduled_transition × read` and `× cancel`.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-deferred-intent:p1
//! @cpt-dod:cpt-cf-bss-products-dod-lifecycle-audit:p1

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::{ApiState, repo_error_to_canonical, require_authenticated};
use crate::domain::canonical;
use crate::domain::error::DomainError;
use crate::domain::governance::{
    GateMode, GateSubject, GateVerdict, GovernanceGate, NoMaterialityPolicyGate, SubjectPin,
};
use crate::domain::validation::ValidationReport;
use crate::infra::storage::repo::{self, RefusalSubject};

const TAG: &str = "BSS Products";
const SUBJECT_KIND: &str = "scheduled_transition";
const LIVE_OP_TARGET: &str = "scheduled_transition.cancel";

#[resource_error(gts_id!("cf.bss.products.scheduled_transition.v1~"))]
struct ScheduledTransitionResource;

/// Filter for [`GET /bss-products/v1/scheduled-transitions`].
#[derive(Debug, Default, serde::Deserialize)]
pub struct ListQuery {
    /// Stored state (`pending`, `deferred`, `applied`, …). Absent: every row.
    pub state: Option<String>,
}

/// One scheduled-transition row on the wire.
#[toolkit_macros::api_dto(response)]
pub struct ScheduledTransitionView {
    /// Surrogate key.
    pub transition_id: Uuid,
    /// `product` or `sku`.
    pub entity_kind: String,
    /// Subject entity id.
    pub entity_id: Uuid,
    /// `publish` or `retire`.
    pub kind: String,
    /// UTC activation instant.
    pub at: DateTime<Utc>,
    /// Stored run state.
    pub state: String,
    /// Runner outcome text, present on `applied|failed|deferred`.
    pub outcome_reason: Option<String>,
}

/// The list the GET answers.
#[toolkit_macros::api_dto(response)]
pub struct ScheduledTransitionList {
    /// Tenant-scoped rows, optionally filtered by `state`.
    pub items: Vec<ScheduledTransitionView>,
}

/// The cancel operation envelope. Only `cancel` is admitted.
#[toolkit_macros::api_dto(request)]
pub struct ScheduledTransitionOp {
    /// Must be `cancel`.
    pub op: String,
}

/// Register the two doors.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = OperationBuilder::get("/bss-products/v1/scheduled-transitions")
        .operation_id("bss_products.list_scheduled_transitions")
        .summary("List scheduled transitions")
        .description(
            "The deferred-intent surface this feature owns and `08` projects. \
             Filterable by state; each row carries `outcomeReason`. Tenant-scoped \
             through the ordinary pipeline under `scheduled_transition x read`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(list_scheduled_transitions)
        .json_response_with_schema::<ScheduledTransitionList>(
            openapi,
            StatusCode::OK,
            "The tenant's scheduled transitions.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .register(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/scheduled-transitions/{id}/operations")
        .operation_id("bss_products.scheduled_transition_operation")
        .summary("Operate on a scheduled transition")
        .description(
            "The governed cancel (`op: cancel`). Supersedes the row and its \
             intent. Spends `scheduled_transition x cancel`. A cancelled row \
             is one more state the runner never claims.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<ScheduledTransitionOp>(openapi, "The operation. Only `cancel` is admitted.")
        .handler(operate_scheduled_transition)
        .no_content_response(
            StatusCode::ACCEPTED,
            "The cancel was accepted and the row superseded.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .register(router, openapi);

    router.layer(Extension(state))
}

async fn list_scheduled_transitions(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Query(query): Query<ListQuery>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    // Collection read: the PDP derives the scope; `resource_id` is unset.
    // `owner_tenant_id` stays `None` the way [`super::products::get_product`]
    // does — the SQL filter then binds the caller's tenant.
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::SCHEDULED_TRANSITION,
        crate::authz::actions::READ,
        None,
        None,
        true,
    )
    .await
    .map_err(|e| {
        crate::api::rest::authz_error_to_canonical(e, |reason| {
            ScheduledTransitionResource::permission_denied()
                .with_reason(reason)
                .create()
        })
    })?;
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let rows = repo::list_scheduled_transitions(&conn, &scope, tenant_id, query.state.as_deref())
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let body = ScheduledTransitionList {
        items: rows
            .into_iter()
            .map(|row| ScheduledTransitionView {
                transition_id: row.transition_id,
                entity_kind: row.entity_kind,
                entity_id: row.entity_id,
                kind: row.kind,
                at: row.at,
                state: row.state,
                outcome_reason: row.outcome_reason,
            })
            .collect(),
    };
    Ok((StatusCode::OK, Json(body)).into_response())
}

async fn operate_scheduled_transition(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(transition_id): Path<Uuid>,
    Json(body): Json<ScheduledTransitionOp>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = cancel_scope(&state, &enforcer, &ctx, tenant_id, actor_ref, transition_id).await?;

    if body.op != "cancel" {
        let mut report = ValidationReport::new();
        report.violate(
            "VALIDATION",
            "op",
            format!("op {} is not admitted; only cancel is", body.op),
        );
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            DomainError::Validation(report),
        )
        .await);
    }

    if let Err(error) = submit_cancel_to_gate(tenant_id) {
        return Err(refuse(&state, &scope, tenant_id, actor_ref, error).await);
    }

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let found = repo::find_scheduled_transition(&conn, &scope, tenant_id, transition_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let Some(_) = found else {
        return Err(ScheduledTransitionResource::not_found(
            "no scheduled transition with this id in this tenant",
        )
        .with_resource(transition_id.to_string())
        .create());
    };
    let superseded =
        repo::supersede_scheduled_transition(&conn, &scope, tenant_id, transition_id, now)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?;
    if !superseded {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            DomainError::IllegalTransition {
                from: "terminal".to_owned(),
                to: "superseded".to_owned(),
            },
        )
        .await);
    }

    repo::write_eventless_act_audit(
        &conn,
        &scope,
        repo::AuditCommon {
            audit_id: Uuid::now_v7(),
            tenant_id,
            actor_ref,
            action: "scheduled_transition.cancel".to_owned(),
            subject_kind: SUBJECT_KIND.to_owned(),
            reason: Some("governed cancel".to_owned()),
            correlation_id: crate::infra::events::correlation_id(),
            written_at: now,
        },
        transition_id,
        None,
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))?;

    Ok(StatusCode::ACCEPTED.into_response())
}

fn submit_cancel_to_gate(tenant_id: Uuid) -> Result<(), DomainError> {
    let gate: Arc<dyn GovernanceGate + Send + Sync> = Arc::new(NoMaterialityPolicyGate);
    // `SubjectPin::Unpinned`: a live op has no entity head to read a revision
    // from, and the pin rides the subject since P-D-125 row 52 (strand B,
    // merged 2026-09-04; this call was reconstructed at that merge).
    match gate.evaluate(
        GateSubject::governed_live_op(tenant_id, LIVE_OP_TARGET, SubjectPin::Unpinned),
        GateMode::Gate,
    )? {
        GateVerdict::Authorized(_) => Ok(()),
        GateVerdict::Refused { reason } => Err(DomainError::ApprovalRequired(reason)),
    }
}

async fn cancel_scope(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    transition_id: Uuid,
) -> Result<AccessScope, CanonicalError> {
    match crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::SCHEDULED_TRANSITION,
        crate::authz::actions::CANCEL,
        Some(tenant_id),
        Some(transition_id),
        true,
    )
    .await
    {
        Ok(scope) => Ok(scope),
        Err(crate::authz::AuthzError::Denied(reason)) => {
            let self_scope = AccessScope::for_tenant(tenant_id);
            Err(crate::api::rest::audit_refusal_and_report(
                state,
                &self_scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: SUBJECT_KIND,
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(LIVE_OP_TARGET.to_owned()),
                ScheduledTransitionResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await)
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            Err(crate::api::rest::authz_error_to_canonical(err, |reason| {
                ScheduledTransitionResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }))
        }
    }
}

async fn refuse(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    refusal: DomainError,
) -> CanonicalError {
    let code = refusal.code();
    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: SUBJECT_KIND,
            error_code: code,
        },
        RefusalSubject::Attempted(LIVE_OP_TARGET.to_owned()),
        CanonicalError::from(refusal),
    )
    .await
}

#[cfg(test)]
#[path = "scheduled_transitions_tests.rs"]
mod scheduled_transitions_tests;
