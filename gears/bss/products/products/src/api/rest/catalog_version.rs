//! The increment-request door — `design/06-catalog-version.md` §2 rule 1's
//! one contract on both bindings (`dod-request-door`,
//! `dod-increment-request-port`; P-D-15, P-D-52, P-D-56, P-D-81).
//!
//! # One gate, one core, two bindings
//!
//! [`enqueue_increment`] is the contract's whole synchronous path: the
//! `catalog_version x request` gate has already passed (each binding runs
//! it), the shape is judged, the source is judged against the registered
//! set, `requested_at` is stamped at ingress, and the queue's own UNIQUE is
//! the idempotency — no `products_idempotency` claim participates, the
//! migration's *"an idempotent replay is caught by the UNIQUE"*. The door
//! takes **no lease** and resolves **no version** (P-D-56 — the lease is the
//! coalescer's, the resolution the poll's), so nothing here waits on
//! anything, which is what lets it fit inside the smallest call budget a
//! consumer may configure.
//!
//! `POST /bss-products/v1/catalog-version-requests` is the out-of-process
//! binding; [`InProcessIncrementRequests`] is the in-process one, registered
//! in `ClientHub` at boot ([`crate::gear`]). Both pass the same gate, which
//! is the sentence `design/06` states and the reason the SDK trait takes the
//! caller's [`SecurityContext`].
//!
//! # The registered set, and the refusal's two channels
//!
//! [`REGISTERED_WIRE_SOURCES`] is the trigger set's wire-visible member
//! today: `pricing` (P-D-03's v1 registered set). The other two triggers the
//! design names — this gear's own bulk commits and the operator
//! catalog-publish act — are **in-crate writers** that enqueue through the
//! repository with their own source names when their doors land; neither
//! crosses this contract, so neither widens this roster.
//!
//! A source outside the set is refused **after** the grant passes —
//! `REQUEST_SOURCE_UNKNOWN`, a precondition on the request's content — and
//! the wire shape is P-D-52's: a `FailedPrecondition` whose violation is
//! typed `CATALOG_VERSION_REJECTED`, the discriminator the consumer's
//! `Rejected` arm matches on, while the audit row records the domain code.
//!
//! # The acknowledgement
//!
//! **202** with `{coalesced, catalog_version_id?}`: the assignment is
//! asynchronous by design, so the door acknowledges the demand rather than
//! answering a version; an idempotent replay of a request the coalescer has
//! already satisfied answers the same shape with the version filled. The
//! status is this door's own routine call — nothing in the set pins one —
//! and it is stated here so the `OpenAPI` surface and the tests hold the same
//! value.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-request-door:p1

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use bss_products_sdk::increments::{
    CommittedIncrement, IncrementAck, IncrementLane, IncrementRequest, IncrementRequests,
};

use crate::api::rest::{ApiState, repo_error_to_canonical, require_authenticated};
use crate::domain::canonical;
use crate::domain::error::DomainError;
use crate::domain::validation::ValidationReport;
use crate::infra::storage::repo::{self, RefusalSubject};

/// `OpenAPI` tag for the catalog-version surface's operations.
const TAG: &str = "BSS Products";

/// The wire-visible registered requester set (see the module doc).
pub(crate) const REGISTERED_WIRE_SOURCES: [&str; 1] = ["pricing"];

/// Build the Axum router for the increment-request door and register it with
/// the supplied `OpenAPI` registry.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::post("/bss-products/v1/catalog-version-requests")
        .operation_id("bss_products.request_catalog_version")
        .summary("Request a catalog-version increment")
        .description(
            "Enqueues one increment request on the demand queue and answers 202 with the \
             acknowledgement: the version assignment is asynchronous (the coalescer batches \
             demand per tenant), so the body carries `coalesced` and, once satisfied, the \
             `catalog_version_id`. Idempotent per `(tenant, source, request_key)`, the queue's \
             own key, so a retry answers the stored state rather than enqueueing a second \
             demand. Gates on `catalog_version x request` (S2S). A `source` outside the \
             registered set is refused AFTER the grant passes, as a `FailedPrecondition` \
             carrying a `CATALOG_VERSION_REJECTED` precondition violation (the consumer \
             projection's `Rejected` discriminator); a `bulk` request must name its \
             `operation_key`, and an `interactive` one must not.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<CreateIncrementRequestBody>(openapi, "The increment request.")
        .handler(request_catalog_version)
        .json_response_with_schema::<IncrementAckView>(
            openapi,
            StatusCode::ACCEPTED,
            "The acknowledgement.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi)
        .layer(Extension(state))
}

/// The request body: the entity minus `requested_at`, which is the door's.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct CreateIncrementRequestBody {
    /// The registered requester this demand belongs to.
    pub source: String,
    /// `interactive` or `bulk`.
    pub lane: String,
    /// The caller's idempotency handle.
    pub request_key: String,
    /// The bulk batch this request coalesces under; required exactly when
    /// `lane` is `bulk`.
    pub operation_key: Option<String>,
}

/// The acknowledgement body.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct IncrementAckView {
    /// `true` once the request's version has committed.
    pub coalesced: bool,
    /// The committed version, present exactly when `coalesced`.
    pub catalog_version_id: Option<i64>,
}

impl From<IncrementAck> for IncrementAckView {
    fn from(ack: IncrementAck) -> Self {
        Self {
            coalesced: ack.coalesced,
            catalog_version_id: ack.catalog_version_id,
        }
    }
}

/// One audited refusal of this door, CATALOG_VERSION-labelled.
async fn refuse_request(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    subject: String,
    refusal: DomainError,
) -> CanonicalError {
    let code = refusal.code();
    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: crate::authz::labels::CATALOG_VERSION,
            error_code: code,
        },
        RefusalSubject::Attempted(subject),
        CanonicalError::from(refusal),
    )
    .await
}

/// The contract's synchronous core, shared by both bindings: shape, source
/// roster, stamp, enqueue, answer. The caller has already passed the gate
/// and resolved `actor_ref`.
async fn enqueue_increment(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    request: IncrementRequest,
    now: DateTime<Utc>,
) -> Result<IncrementAck, CanonicalError> {
    let source = request.source.trim().to_owned();
    let request_key = request.request_key.trim().to_owned();
    let operation_key = request
        .operation_key
        .as_deref()
        .map(str::trim)
        .map(str::to_owned);
    let subject = format!("{source}/{request_key}");

    // -- shape (P-D-33's collected form). --
    let mut report = ValidationReport::new();
    if source.is_empty() {
        report.violate("VALIDATION", "source", "source must not be blank");
    }
    if request_key.is_empty() {
        report.violate("VALIDATION", "request_key", "request_key must not be blank");
    }
    match (request.lane, operation_key.as_deref()) {
        (IncrementLane::Bulk, None | Some("")) => {
            report.violate(
                "VALIDATION",
                "operation_key",
                "a bulk request must name its operation_key so the batch coalesces into one \
                 version",
            );
        }
        (IncrementLane::Interactive, Some(_)) => {
            report.violate(
                "VALIDATION",
                "operation_key",
                "operation_key is the bulk lane's batching operand; an interactive request \
                 carries none",
            );
        }
        _ => {}
    }
    if !report.is_empty() {
        return Err(refuse_request(
            state,
            scope,
            tenant_id,
            actor_ref,
            subject,
            DomainError::Validation(report),
        )
        .await);
    }

    // -- the registered set, judged after the grant (P-D-52). --
    if !REGISTERED_WIRE_SOURCES.contains(&source.as_str()) {
        let refusal = DomainError::RequestSourceUnknown(format!(
            "source \"{source}\" is not in the registered requester set; the registered set \
             is: {}",
            REGISTERED_WIRE_SOURCES.join(", ")
        ));
        return Err(refuse_request(state, scope, tenant_id, actor_ref, subject, refusal).await);
    }

    // -- stamp, enqueue, answer: the queue's UNIQUE is the idempotency. --
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
            "increment request connection: {e}"
        )))
    })?;
    let record = repo::enqueue_increment_request(
        &conn,
        scope,
        tenant_id,
        repo::NewIncrementRequest {
            source: &source,
            request_key: &request_key,
            lane: request.lane.as_str(),
            operation_key: operation_key.as_deref().filter(|key| !key.is_empty()),
            requested_at: now,
        },
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))?;

    Ok(IncrementAck {
        coalesced: record.state == "coalesced",
        catalog_version_id: record.satisfied_by_version_id,
    })
}

/// The `catalog_version x request` gate, shared by both bindings.
async fn request_scope(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    subject: String,
) -> Result<AccessScope, CanonicalError> {
    match crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::CATALOG_VERSION,
        crate::authz::actions::REQUEST,
        Some(tenant_id),
        None,
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
                    subject_kind: crate::authz::labels::CATALOG_VERSION,
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(subject),
                CatalogVersionResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await)
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            Err(crate::api::rest::authz_error_to_canonical(err, |reason| {
                CatalogVersionResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }))
        }
    }
}

use toolkit::api::canonical_prelude::resource_error;

/// The canonical-error identity of this door's own refusals.
#[resource_error(gts_id!("cf.bss.products.catalog_version.v1~"))]
struct CatalogVersionResource;

/// `POST /bss-products/v1/catalog-version-requests` — the out-of-process
/// binding.
async fn request_catalog_version(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<CreateIncrementRequestBody>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());

    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let subject = format!("{}/{}", body.source.trim(), body.request_key.trim());
    let scope = request_scope(&state, &enforcer, &ctx, tenant_id, actor_ref, subject).await?;

    // The lane string is part of the shape judgment, collected with the rest.
    let lane = match body.lane.trim() {
        "interactive" => IncrementLane::Interactive,
        "bulk" => IncrementLane::Bulk,
        other => {
            let mut report = ValidationReport::new();
            report.violate("VALIDATION", "lane", "lane must be interactive or bulk");
            let subject = format!("{}/{}", body.source.trim(), other);
            return Err(refuse_request(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                subject,
                DomainError::Validation(report),
            )
            .await);
        }
    };

    let ack = enqueue_increment(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        IncrementRequest {
            source: body.source,
            lane,
            request_key: body.request_key,
            operation_key: body.operation_key,
        },
        now,
    )
    .await?;

    Ok((StatusCode::ACCEPTED, Json(IncrementAckView::from(ack))).into_response())
}

/// The in-process binding, registered in `ClientHub` at boot — the default
/// deployment mode (P-D-15). Runs the identical gate and core.
pub(crate) struct InProcessIncrementRequests {
    /// The door's own state: database, outbox sink, retention.
    pub(crate) state: Arc<ApiState>,
    /// The platform PEP, the same instance the routers layer.
    pub(crate) enforcer: authz_resolver_sdk::PolicyEnforcer,
}

#[async_trait]
impl IncrementRequests for InProcessIncrementRequests {
    async fn request(
        &self,
        ctx: &SecurityContext,
        tenant_id: Uuid,
        request: IncrementRequest,
    ) -> Result<IncrementAck, CanonicalError> {
        let now = canonical::write_instant(Utc::now());
        let actor_ref = crate::api::rest::resolve_creator_actor_ref(
            &self.state,
            tenant_id,
            ctx.subject_id(),
            now,
        )
        .await?;
        let subject = format!("{}/{}", request.source.trim(), request.request_key.trim());
        let scope = request_scope(
            &self.state,
            &self.enforcer,
            ctx,
            tenant_id,
            actor_ref,
            subject,
        )
        .await?;
        enqueue_increment(&self.state, &scope, tenant_id, actor_ref, request, now).await
    }

    async fn committed(
        &self,
        ctx: &SecurityContext,
        tenant_id: Uuid,
        source: &str,
        request_key: &str,
    ) -> Result<Option<CommittedIncrement>, CanonicalError> {
        let now = canonical::write_instant(Utc::now());
        let actor_ref = crate::api::rest::resolve_creator_actor_ref(
            &self.state,
            tenant_id,
            ctx.subject_id(),
            now,
        )
        .await?;
        // The poll spends the same grant as the request: it reads the
        // caller's own demand row, which the request contract owns whole.
        let scope = request_scope(
            &self.state,
            &self.enforcer,
            ctx,
            tenant_id,
            actor_ref,
            format!("{source}/{request_key}"),
        )
        .await?;

        let conn = self.state.db.conn().map_err(|e| {
            repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
                "increment poll connection: {e}"
            )))
        })?;
        let record = repo::find_increment_request(&conn, &scope, tenant_id, source, request_key)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?;
        Ok(record.and_then(|row| {
            row.satisfied_by_version_id
                .map(|catalog_version_id| CommittedIncrement { catalog_version_id })
        }))
    }
}

#[cfg(test)]
#[path = "catalog_version_tests.rs"]
mod catalog_version_tests;
