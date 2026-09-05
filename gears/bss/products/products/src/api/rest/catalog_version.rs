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
//! # The freeze doors and the resolver (P-D-84)
//!
//! `POST …/{catalogVersionId}/acks` and `…/releases` are the participant's
//! own S2S doors (`catalog_version x ack` / `x release`, P-D-67), each an
//! **UPDATE, never an upsert** — the increment transaction seeds one
//! `pending` row per snapshot member, so the row's existence IS the
//! membership check and a non-member is refused `PARTICIPANT_UNKNOWN` (403:
//! the identity is the refusal's subject, a 404 would leak version
//! existence). Each door refreshes `freeze_state`'s derived cache in the
//! same transaction (P-D-73) under P-D-84's settled predicate, and each
//! writes an **audit row and no broker event** — these acts are inbound,
//! the audit row is the record. The release door does **not** stamp
//! `released_at`: that column is the force ceremony's alone (P-D-67).
//!
//! **The identity-binding half is owed, and deliberately.** The `DoD` binds
//! an ack to *"that participant's own service identity"*; nothing in the
//! platform maps a `SecurityContext` subject to a participant name, so the
//! door takes the participant from the body and holds membership — the
//! same posture the create door records for `brand_id` claims (*"there is
//! nothing on this door's `SecurityContext` to check a brand claim
//! against"*), owed to whoever adds a service-name claim.
//!
//! `GET …/catalog-versions/{catalogVersionId}?intent=…` is the
//! `IntentfulResolver` (P-D-84 arm 4's route) on `catalog_version x read` —
//! the single raising door of `CATALOG_VERSION_UNKNOWN`. `intent` is
//! required (`INTENT_REQUIRED` when absent); `browse` serves any committed
//! version at once; `posted` is refused `FREEZE_INCOMPLETE` while the
//! ledger holds a `pending` row and `VERSION_FORCED_INCOMPLETE` naming each
//! `not_frozen(forced)` participant. The response renders from the
//! **stored manifest** — never a re-collect — and re-verifies the stored
//! checksum before serving (`inst-rv-bytes`); a mismatch is the store
//! contradicting itself, a 500 and an operator alarm. When the caller
//! names its `bound_version` and it differs from the resolved one, the
//! response carries `(bound_version, resolved_version, diff_ref)` —
//! `dod-version-binding`'s surfacing, the diff door's ref grammar.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-request-door:p1
//! @cpt-dod:cpt-cf-bss-products-dod-ack-door:p1
//! @cpt-dod:cpt-cf-bss-products-dod-liveness-and-release:p1
//! @cpt-dod:cpt-cf-bss-products-dod-intentful-resolver:p1
//! @cpt-dod:cpt-cf-bss-products-dod-version-binding:p1

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
    let router = OperationBuilder::post("/bss-products/v1/catalog-version-requests")
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
        .register(Router::new(), openapi);
    let router = register_freeze_routes(router, openapi);
    register_resolver_route(router, openapi).layer(Extension(state))
}

/// Register the two participant doors — `…/acks` and `…/releases`.
fn register_freeze_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/bss-products/v1/catalog-versions/{id}/acks")
        .operation_id("bss_products.ack_catalog_version")
        .summary("Acknowledge a catalog version's freeze")
        .description(
            "Records the named participant's ack on the version's freeze ledger: an UPDATE of \
             the pending row the increment transaction seeded, so membership is the row's \
             existence and a non-member is refused PARTICIPANT_UNKNOWN (403). Idempotent per \
             (tenant, version, participant): a re-ack answers the standing state. Refreshes the \
             version's freeze_state cache in the same transaction; complete lands with the last \
             member's ack. Gates on catalog_version x ack (S2S). Audit-plane: writes an audit \
             row and emits no broker event.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The catalog version.")
        .json_request::<FreezeParticipantRequest>(openapi, "The acking participant.")
        .handler(ack_catalog_version)
        .json_response_with_schema::<FreezeEdgeView>(
            openapi,
            StatusCode::OK,
            "The participant's ledger state and the version's freeze state.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-products/v1/catalog-versions/{id}/releases")
        .operation_id("bss_products.release_catalog_version")
        .summary("Release a catalog version's freeze liveness")
        .description(
            "Records that the named participant holds no more live references to this version \
             (P-D-18's second half): pending or acked moves to released, settling exactly as an \
             ack under the completeness predicate, and released_at is NOT stamped, that column \
             being the force ceremony's alone. Membership, idempotency, the cache refresh, the \
             gate (catalog_version x release) and the audit posture mirror the ack door.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The catalog version.")
        .json_request::<FreezeParticipantRequest>(openapi, "The releasing participant.")
        .handler(release_catalog_version)
        .json_response_with_schema::<FreezeEdgeView>(
            openapi,
            StatusCode::OK,
            "The participant's ledger state and the version's freeze state.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    let router = OperationBuilder::post("/bss-products/v1/catalog-versions/{id}/force-completions")
        .operation_id("bss_products.force_complete_catalog_version")
        .summary("Force-complete a timed-out freeze (a two-person ceremony)")
        .description(
            "Closes a version's open freeze without its silent participants: every `pending` \
         registration becomes `not_frozen(forced)` and is stamped `released_at` in the same \
         statement, the version flips to `complete(forced)`, and `posted` resolution stays \
         refused VERSION_FORCED_INCOMPLETE until each forced participant acks or releases \
         through its own door. Gates on `catalog_version x force_complete`; the act is a \
         `GovernedLiveOp` under the stored approval host at the tenant's N (APPROVAL_REQUIRED \
         without a satisfied record), `quorumReduced` riding the record and the event. Writes \
         an audit row carrying the ceremony reference and emits FreezeForceCompleted in the \
         same transaction.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The catalog version whose freeze to force-complete.")
        .handler(force_complete_catalog_version)
        .json_response_with_schema::<FreezeLedgerView>(
            openapi,
            StatusCode::OK,
            "The version's ledger after the ceremony: per-participant frozen state.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    OperationBuilder::post("/bss-products/v1/freeze-participants")
        .operation_id("bss_products.change_freeze_participant")
        .summary("Register or retire a freeze participant")
        .description(
            "Moves the tenant's LIVE freeze-participant set - `op` is `register` or `retire`. A \
             `GovernedLiveOp` on `freeze_participant x write` under the stored approval host \
             (APPROVAL_REQUIRED without a satisfied record), material at the tenant's N. Emits \
             FreezeParticipantSetChanged and writes an audit row in the same transaction. Every \
             published version keeps its own participant_set_snapshot, so a change here never \
             re-resolves a historical freezeComplete.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<FreezeParticipantChangeRequest>(openapi, "The participant and the op.")
        .handler(change_freeze_participant)
        .json_response_with_schema::<FreezeParticipantView>(
            openapi,
            StatusCode::OK,
            "The participant and whether the set moved.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// Register the resolver — `GET …/catalog-versions/{id}`.
fn register_resolver_route(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-products/v1/catalog-versions/{id}")
        .operation_id("bss_products.resolve_catalog_version")
        .summary("Resolve a catalog version")
        .description(
            "The IntentfulResolver: requires intent (browse serves any committed version at \
             once; posted is refused FREEZE_INCOMPLETE while the ledger holds a pending row, \
             and VERSION_FORCED_INCOMPLETE naming each not_frozen(forced) participant). Renders \
             from the STORED manifest, never a re-collect, and re-verifies the stored \
             checksum before serving; the checksum is returned and verifiable. An unknown id is \
             CATALOG_VERSION_UNKNOWN (404), raised here for resolve and diff alike. When \
             bound_version is supplied and differs, the response carries bound_version, \
             resolved_version and diff_ref. Gates on catalog_version x read.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The catalog version to resolve.")
        .query_param("intent", false, "Required: browse or posted.")
        .query_param(
            "bound_version",
            false,
            "The caller's bound version, for the re-binding surface.",
        )
        .handler(resolve_catalog_version)
        .json_response_with_schema::<ResolvedVersionView>(
            openapi,
            StatusCode::OK,
            "The resolved version: metadata, manifest and checksum.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);
    OperationBuilder::get("/bss-products/v1/catalog-versions/{a}/diff/{b}")
        .operation_id("bss_products.diff_catalog_versions")
        .summary("Diff two catalog versions")
        .description(
            "Computed read-only from the two STORED manifests through the same version lookup as \
             resolve: entities added and removed, per-entity published-version deltas, and the \
             capture half - every capture kind whose stored content differs. Byte-stable for a \
             given pair, sorted throughout; no retention effect and no freeze-registration row. \
             An unknown id on either side is CATALOG_VERSION_UNKNOWN (404). Gates on \
             catalog_version x read.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("a", "The earlier catalog version.")
        .path_param("b", "The later catalog version.")
        .handler(diff_catalog_versions)
        .json_response_with_schema::<VersionDiffView>(
            openapi,
            StatusCode::OK,
            "The diff: entity deltas and changed capture kinds.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
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
            lane: request.lane,
            operation_key: operation_key.as_deref().filter(|key| !key.is_empty()),
            requested_at: now,
        },
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))?;

    Ok(IncrementAck {
        coalesced: record.state == crate::domain::states::RequestState::Coalesced,
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

use crate::api::rest::dto::{
    CreateIncrementRequestBody, FreezeEdgeView, FreezeParticipantRequest, IncrementAckView,
    ManifestCaptureView, ManifestEntryView, ResolvedVersionView,
};
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

/// The shared gate for the freeze doors and the resolver, one action each.
async fn catalog_version_scope(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    action: &'static str,
    subject: String,
) -> Result<AccessScope, CanonicalError> {
    match crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::CATALOG_VERSION,
        action,
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

/// One audited refusal of a freeze door or the resolver.
async fn refuse_cv(
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

/// Which edge a participant door drives.
#[derive(Clone, Copy)]
enum FreezeEdge {
    Ack,
    Release,
}

impl FreezeEdge {
    const fn target(self) -> &'static str {
        match self {
            Self::Ack => "acked",
            Self::Release => "released",
        }
    }
    const fn action(self) -> &'static str {
        match self {
            Self::Ack => crate::authz::actions::ACK,
            Self::Release => crate::authz::actions::RELEASE,
        }
    }
    const fn audit_action(self) -> &'static str {
        match self {
            Self::Ack => "catalog_version.freeze.ack",
            Self::Release => "catalog_version.freeze.release",
        }
    }
}

/// The two participant doors' shared body: gate, shape, the ledger edge
/// and the cache refresh in one transaction, the keyed audit row, the
/// answer.
async fn drive_freeze_edge(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    catalog_version_id: i64,
    body: FreezeParticipantRequest,
    edge: FreezeEdge,
) -> Result<Response, CanonicalError> {
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let participant = body.participant.trim().to_owned();
    let subject = format!("{catalog_version_id}/{participant}");

    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = catalog_version_scope(
        state,
        enforcer,
        ctx,
        tenant_id,
        actor_ref,
        edge.action(),
        subject.clone(),
    )
    .await?;

    if participant.is_empty() {
        let mut report = ValidationReport::new();
        report.violate("VALIDATION", "participant", "participant must not be blank");
        return Err(refuse_cv(
            state,
            &scope,
            tenant_id,
            actor_ref,
            subject,
            DomainError::Validation(report),
        )
        .await);
    }

    // The edge, the cache refresh and the act's audit row commit together
    // (P-D-73's same-transaction obligation).
    let scope_for_tx = scope.clone();
    let participant_for_tx = participant.clone();
    let outcome = state
        .db
        .db()
        .transaction_with_retry::<(
            repo::FreezeEdgeOutcome,
            Option<crate::domain::states::FreezeState>,
        ), toolkit_db::DbError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            crate::api::rest::contention_db_err,
            move |tx| {
                let scope = scope_for_tx.clone();
                let participant = participant_for_tx.clone();
                Box::pin(async move {
                    let outcome = match edge {
                        FreezeEdge::Ack => {
                            repo::ack_freeze_row(
                                tx,
                                &scope,
                                tenant_id,
                                catalog_version_id,
                                &participant,
                                now,
                            )
                            .await
                        }
                        FreezeEdge::Release => {
                            repo::release_freeze_row(
                                tx,
                                &scope,
                                tenant_id,
                                catalog_version_id,
                                &participant,
                            )
                            .await
                        }
                    }
                    .map_err(|e| toolkit_db::DbError::Sea(e.to_db_err()))?;

                    let freeze_state = if matches!(outcome, repo::FreezeEdgeOutcome::Flipped) {
                        let refreshed =
                            repo::refresh_freeze_state(tx, &scope, tenant_id, catalog_version_id)
                                .await
                                .map_err(|e| toolkit_db::DbError::Sea(e.to_db_err()))?;
                        // The `event → ack` meter per participant
                        // (`dod-posting-safe-observability`), from the
                        // version's publish instant to this ack.
                        if matches!(edge, FreezeEdge::Ack)
                            && let Some(version) = repo::find_catalog_version(
                                tx,
                                &scope,
                                tenant_id,
                                catalog_version_id,
                            )
                            .await
                            .map_err(|e| toolkit_db::DbError::Sea(e.to_db_err()))?
                        {
                            tracing::info!(
                                event = "freeze_ack_latency",
                                %tenant_id,
                                catalog_version_id,
                                participant = %participant,
                                latency_ms = now
                                    .signed_duration_since(version.published_at)
                                    .num_milliseconds(),
                                "bss-products: event -> ack"
                            );
                        }
                        repo::write_keyed_act_audit(
                            tx,
                            &scope,
                            repo::AuditCommon {
                                audit_id: Uuid::new_v4(),
                                tenant_id,
                                actor_ref,
                                action: edge.audit_action().to_owned(),
                                subject_kind: crate::authz::labels::CATALOG_VERSION.to_owned(),
                                reason: None,
                                correlation_id: crate::infra::events::correlation_id(),
                                written_at: now,
                            },
                            format!("{catalog_version_id}/{participant}"),
                        )
                        .await
                        .map_err(|e| toolkit_db::DbError::Sea(e.to_db_err()))?;
                        Some(refreshed)
                    } else {
                        None
                    };
                    Ok((outcome, freeze_state))
                })
            },
        )
        .await
        .map_err(|e| {
            repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
        })?;

    let (outcome, freeze_state) = outcome;
    match outcome {
        repo::FreezeEdgeOutcome::Flipped => Ok((
            StatusCode::OK,
            Json(FreezeEdgeView {
                participant,
                state: edge.target().to_owned(),
                // Flipped is the arm that refreshed, so the state is
                // always present here; rendered through the enum's own
                // spelling.
                freeze_state: freeze_state
                    .map(|state| state.as_str().to_owned())
                    .unwrap_or_default(),
            }),
        )
            .into_response()),
        repo::FreezeEdgeOutcome::AlreadyThere => {
            // The idempotent replay: the row already sits where this door
            // would put it, so the standing cache is read back unchanged.
            let conn = state.db.conn().map_err(|e| {
                repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
                    "freeze replay connection: {e}"
                )))
            })?;
            let version = repo::find_catalog_version(&conn, &scope, tenant_id, catalog_version_id)
                .await
                .map_err(|e| repo_error_to_canonical(&e))?;
            // The edge just matched this version's ledger row, so a missing
            // version row is the store contradicting itself — refused as
            // corruption, never papered over with a fabricated empty
            // freeze_state.
            let Some(version) = version else {
                return Err(repo_error_to_canonical(
                    &crate::infra::storage::RepoError::CorruptRow(format!(
                        "catalog version {catalog_version_id} of {tenant_id} has a freeze-ack \
                         row but no version row"
                    )),
                ));
            };
            Ok((
                StatusCode::OK,
                Json(FreezeEdgeView {
                    participant,
                    state: edge.target().to_owned(),
                    freeze_state: version.freeze_state.as_str().to_owned(),
                }),
            )
                .into_response())
        }
        repo::FreezeEdgeOutcome::NoRow => {
            let refusal = DomainError::ParticipantUnknown(format!(
                "\"{participant}\" is not in this version's snapshotted participant set"
            ));
            Err(refuse_cv(state, &scope, tenant_id, actor_ref, subject, refusal).await)
        }
        repo::FreezeEdgeOutcome::IllegalFrom(from) => {
            let refusal = DomainError::IllegalTransition {
                from,
                to: edge.target().to_owned(),
            };
            Err(refuse_cv(state, &scope, tenant_id, actor_ref, subject, refusal).await)
        }
    }
}

/// `POST /bss-products/v1/catalog-versions/{id}/acks`.
pub(crate) async fn ack_catalog_version(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path(catalog_version_id): axum::extract::Path<i64>,
    Json(body): Json<FreezeParticipantRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    drive_freeze_edge(
        &state,
        &enforcer,
        &ctx,
        catalog_version_id,
        body,
        FreezeEdge::Ack,
    )
    .await
}

/// `POST /bss-products/v1/catalog-versions/{id}/releases`.
pub(crate) async fn release_catalog_version(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path(catalog_version_id): axum::extract::Path<i64>,
    Json(body): Json<FreezeParticipantRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    drive_freeze_edge(
        &state,
        &enforcer,
        &ctx,
        catalog_version_id,
        body,
        FreezeEdge::Release,
    )
    .await
}

/// The resolver's query operands.
#[derive(Debug, serde::Deserialize)]
struct ResolveQuery {
    intent: Option<String>,
    bound_version: Option<i64>,
}

/// `GET /bss-products/v1/catalog-versions/{id}` — the `IntentfulResolver`.
async fn resolve_catalog_version(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path(catalog_version_id): axum::extract::Path<i64>,
    axum::extract::Query(query): axum::extract::Query<ResolveQuery>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let subject = catalog_version_id.to_string();

    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = catalog_version_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        crate::authz::actions::READ,
        subject.clone(),
    )
    .await?;

    // -- intent: required; junk is the ordinary shape refusal. --
    let intent = match query.intent.as_deref().map(str::trim) {
        None | Some("") => {
            let refusal = DomainError::IntentRequired(
                "resolution requires intent=browse or intent=posted".to_owned(),
            );
            return Err(refuse_cv(&state, &scope, tenant_id, actor_ref, subject, refusal).await);
        }
        Some("browse") => "browse",
        Some("posted") => "posted",
        Some(_) => {
            let mut report = ValidationReport::new();
            report.violate("VALIDATION", "intent", "intent must be browse or posted");
            return Err(refuse_cv(
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

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
            "resolver connection: {e}"
        )))
    })?;
    let Some(version) = repo::find_catalog_version(&conn, &scope, tenant_id, catalog_version_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
    else {
        let refusal = DomainError::CatalogVersionUnknown(format!(
            "no catalog version {catalog_version_id} in the caller's scope"
        ));
        return Err(refuse_cv(&state, &scope, tenant_id, actor_ref, subject, refusal).await);
    };

    // -- posted: the fail-closed intent (C5, P-D-19, P-D-84 arm 3). --
    if intent == "posted" {
        match version.freeze_state.as_str() {
            "complete" => {}
            "complete(forced)" => {
                let forced: Vec<String> =
                    repo::freeze_ack_rows(&conn, &scope, tenant_id, catalog_version_id)
                        .await
                        .map_err(|e| repo_error_to_canonical(&e))?
                        .into_iter()
                        .filter(|(_, s)| {
                            *s == crate::domain::states::FreezeAckState::NotFrozenForced
                        })
                        .map(|(p, _)| p)
                        .collect();
                let refusal = DomainError::VersionForcedIncomplete(format!(
                    "version {catalog_version_id} was force-completed; not frozen: {}",
                    forced.join(", ")
                ));
                return Err(
                    refuse_cv(&state, &scope, tenant_id, actor_ref, subject, refusal).await,
                );
            }
            _ => {
                let refusal = DomainError::FreezeIncomplete(format!(
                    "version {catalog_version_id}'s freeze ledger still holds pending rows"
                ));
                return Err(
                    refuse_cv(&state, &scope, tenant_id, actor_ref, subject, refusal).await,
                );
            }
        }
    }

    // -- the stored manifest, re-verified before it is served. --
    let (entries, captures) =
        repo::catalog_version_manifest_rows(&conn, &scope, tenant_id, catalog_version_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?;
    let participant_set: Vec<String> = captures
        .iter()
        .find(|(kind, _)| kind == "freeze_participant_set")
        .map(|(_, content)| {
            serde_json::from_str::<serde_json::Value>(content)
                .ok()
                .and_then(|value| {
                    value.as_array().map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let manifest = crate::infra::increment::VersionManifest {
        entries: entries.clone(),
        captures: captures.clone(),
        participant_set: participant_set.clone(),
    };
    let recomputed = manifest.checksum();
    if recomputed != version.checksum {
        return Err(repo_error_to_canonical(
            &crate::infra::storage::RepoError::CorruptRow(format!(
                "stored manifest of version {catalog_version_id} re-renders to {recomputed}, \
                 not the stored checksum {}",
                version.checksum
            )),
        ));
    }

    let (bound_version, resolved_version, diff_ref) = match query.bound_version {
        Some(bound) if bound != catalog_version_id => (
            Some(bound),
            Some(catalog_version_id),
            Some(format!("{bound}..{catalog_version_id}")),
        ),
        _ => (None, None, None),
    };

    Ok((
        StatusCode::OK,
        Json(ResolvedVersionView {
            catalog_version_id: version.catalog_version_id,
            checksum: version.checksum,
            digest_version: version.digest_version,
            published_at: version.published_at,
            freeze_complete: version.freeze_state == crate::domain::states::FreezeState::Complete,
            freeze_state: version.freeze_state.as_str().to_owned(),
            entries: entries
                .into_iter()
                .map(|entry| ManifestEntryView {
                    entity_kind: entry.entity_kind,
                    entity_id: entry.entity_id,
                    published_version: entry.published_version,
                })
                .collect(),
            captures: captures
                .into_iter()
                .map(|(capture_kind, content)| ManifestCaptureView {
                    capture_kind,
                    content,
                })
                .collect(),
            participant_set,
            bound_version,
            resolved_version,
            diff_ref,
        }),
    )
        .into_response())
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

// ---------------------------------------------------------------------------
// `06`'s three remaining doors — force-completion, the participant set, the
// diff (`dod-force-completion`, `dod-participant-set`, `dod-diff-door`;
// P-D-67 arm 6, P-D-148)
// ---------------------------------------------------------------------------

/// One participant's frozen state on a version.
#[toolkit_macros::api_dto(response)]
pub struct FreezeParticipantStateView {
    pub participant: String,
    /// `pending`, `acked`, `released` or `not_frozen(forced)`.
    pub state: String,
}

/// A version's ledger after force-completion.
#[toolkit_macros::api_dto(response)]
pub struct FreezeLedgerView {
    pub catalog_version_id: i64,
    /// `open`, `complete` or `complete(forced)`.
    pub freeze_state: String,
    /// The participants this ceremony forced.
    pub forced: Vec<String>,
    pub participants: Vec<FreezeParticipantStateView>,
    /// The ceremony reference the audit row and the forced registrations share.
    pub ceremony_ref: Uuid,
    /// Whether the ceremony's effective quorum was below the default of two.
    pub quorum_reduced: bool,
}

/// The participant-set door's body.
#[toolkit_macros::api_dto(request)]
pub struct FreezeParticipantChangeRequest {
    /// The participant's name, as it acks.
    pub participant: String,
    /// `register` or `retire`.
    pub op: String,
}

#[toolkit_macros::api_dto(response)]
pub struct FreezeParticipantView {
    pub participant: String,
    /// `registered` or `retired`.
    pub state: String,
    /// `false` when the set already read that way (an idempotent replay).
    pub changed: bool,
}

/// One entity's delta between two versions.
#[toolkit_macros::api_dto(response)]
pub struct EntityDeltaView {
    pub entity_kind: String,
    pub entity_id: Uuid,
    /// The version in `a`; `null` when added in `b`.
    pub from_published_version: Option<i64>,
    /// The version in `b`; `null` when removed in `b`.
    pub to_published_version: Option<i64>,
}

/// The diff of two stored manifests (`dod-diff-door`).
#[toolkit_macros::api_dto(response)]
pub struct VersionDiffView {
    pub a: i64,
    pub b: i64,
    pub added: Vec<EntityDeltaView>,
    pub removed: Vec<EntityDeltaView>,
    pub changed: Vec<EntityDeltaView>,
    /// The capture kinds whose stored content differs between the two.
    pub changed_captures: Vec<String>,
    /// The capture kinds present in one manifest only.
    pub added_captures: Vec<String>,
    pub removed_captures: Vec<String>,
}

/// The `GovernedLiveOp` subject a force-completion rides
/// (`dod-force-completion`; P-D-125 row 52's pin belongs to the subject — a
/// catalog version carries no `InternalRevision`, so the pin is `Unpinned`
/// like every live-op door's). `pub(crate)` for the tests' seeding double.
///
/// @cpt-dod:cpt-cf-bss-products-dod-force-completion:p1
pub(crate) fn force_completion_subject(
    tenant_id: Uuid,
    catalog_version_id: i64,
) -> crate::domain::governance::GateSubject {
    crate::domain::governance::GateSubject::governed_live_op(
        tenant_id,
        &format!("catalog_version/{catalog_version_id}/force_complete"),
        crate::domain::governance::SubjectPin::Unpinned,
    )
}

/// The `GovernedLiveOp` subject a participant-set change rides
/// (`dod-participant-set`).
///
/// @cpt-dod:cpt-cf-bss-products-dod-participant-set:p1
pub(crate) fn participant_change_subject(
    tenant_id: Uuid,
    participant: &str,
) -> crate::domain::governance::GateSubject {
    crate::domain::governance::GateSubject::governed_live_op(
        tenant_id,
        &format!("freeze_participant/{participant}"),
        crate::domain::governance::SubjectPin::Unpinned,
    )
}

enum CvTxError {
    Refused(DomainError),
    Repo(crate::infra::storage::RepoError),
    NotFound,
}

impl From<toolkit_db::DbError> for CvTxError {
    fn from(error: toolkit_db::DbError) -> Self {
        Self::Repo(crate::infra::storage::RepoError::Db(error.to_string()))
    }
}

fn cv_contention_db_err(error: &CvTxError) -> Option<&sea_orm::DbErr> {
    match error {
        CvTxError::Repo(crate::infra::storage::RepoError::Driver { source, .. }) => Some(source),
        CvTxError::Repo(_) | CvTxError::Refused(_) | CvTxError::NotFound => None,
    }
}

/// Resolve the stored host for one governed catalog-version act before its
/// transaction; a refusal is audited under the version label.
async fn authorize_cv_op(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    subject: crate::domain::governance::GateSubject,
    attempted: String,
) -> Result<crate::domain::governance::GateAuthorization, CanonicalError> {
    match crate::api::rest::authorize_live_op(state, scope, tenant_id, subject).await {
        Ok(authorization) => Ok(authorization),
        Err(crate::api::rest::HostError::Refused(refusal)) => {
            Err(refuse_cv(state, scope, tenant_id, actor_ref, attempted, refusal).await)
        }
        Err(crate::api::rest::HostError::Repo(error)) => Err(repo_error_to_canonical(&error)),
    }
}

async fn settle_cv_op(
    tx: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    authorization: &crate::domain::governance::GateAuthorization,
    now: DateTime<Utc>,
) -> Result<(), CvTxError> {
    repo::settle_authorization(tx, scope, tenant_id, authorization, now)
        .await
        .map(|_spent| ())
        .map_err(|error| match error {
            repo::SettleError::Refused(refusal) => CvTxError::Refused(refusal),
            repo::SettleError::Repo(error) => CvTxError::Repo(error),
        })
}

/// `quorumReduced` off the authorizing record's descriptor (P-D-13).
async fn quorum_reduced_of(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    authorization: &crate::domain::governance::GateAuthorization,
) -> Result<bool, CvTxError> {
    let Some(approval_id) = authorization.approval_ref() else {
        return Ok(false);
    };
    let record = repo::read_approval(runner, scope, tenant_id, approval_id)
        .await
        .map_err(CvTxError::Repo)?;
    Ok(record.is_some_and(|row| row.quorum_descriptor.contains("\"quorumReduced\":true")))
}

/// `POST /catalog-versions/{id}/force-completions`.
async fn force_complete_catalog_version(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path(catalog_version_id): axum::extract::Path<i64>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let attempted = catalog_version_id.to_string();
    let scope = catalog_version_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        crate::authz::actions::FORCE_COMPLETE,
        attempted.clone(),
    )
    .await?;
    let authorization = authorize_cv_op(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        force_completion_subject(tenant_id, catalog_version_id),
        attempted.clone(),
    )
    .await?;

    let outbox = state.sink.clone();
    let scope_tx = scope.clone();
    let authorization_tx = authorization.clone();
    let result = state
        .db
        .db()
        .transaction_with_retry::<FreezeLedgerView, CvTxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            cv_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let scope = scope_tx.clone();
                let authorization = authorization_tx.clone();
                Box::pin(async move {
                    let Some(version) =
                        repo::find_catalog_version(tx, &scope, tenant_id, catalog_version_id)
                            .await
                            .map_err(CvTxError::Repo)?
                    else {
                        return Err(CvTxError::NotFound);
                    };
                    if version.freeze_state != crate::domain::states::FreezeState::Open {
                        return Err(CvTxError::Refused(DomainError::IllegalTransition {
                            from: version.freeze_state.as_str().to_owned(),
                            to: "complete(forced)".to_owned(),
                        }));
                    }
                    let quorum_reduced =
                        quorum_reduced_of(tx, &scope, tenant_id, &authorization).await?;
                    settle_cv_op(tx, &scope, tenant_id, &authorization, now).await?;
                    let ceremony_ref = Uuid::now_v7();
                    let forced = repo::force_pending_freeze_rows(
                        tx,
                        &scope,
                        tenant_id,
                        catalog_version_id,
                        ceremony_ref,
                        now,
                    )
                    .await
                    .map_err(CvTxError::Repo)?;
                    let freeze_state =
                        repo::refresh_freeze_state(tx, &scope, tenant_id, catalog_version_id)
                            .await
                            .map_err(CvTxError::Repo)?;
                    // The ceremony's audit row carries the reference the forced
                    // registrations store (`dod-cv-audit`); the version is a
                    // counter, so its audit subject is the tenant-scoped v5 of it.
                    repo::write_ceremony_act_audit(
                        tx,
                        &scope,
                        repo::AuditCommon {
                            audit_id: Uuid::now_v7(),
                            tenant_id,
                            actor_ref,
                            action: "catalog_version.freeze.force_complete".to_owned(),
                            subject_kind: crate::authz::labels::CATALOG_VERSION.to_owned(),
                            reason: None,
                            correlation_id: crate::infra::events::correlation_id(),
                            written_at: now,
                        },
                        Uuid::new_v5(&tenant_id, catalog_version_id.to_string().as_bytes()),
                        None,
                        ceremony_ref,
                    )
                    .await
                    .map_err(CvTxError::Repo)?;
                    crate::infra::events::enqueue_catalog_version_event(
                        &outbox,
                        tx,
                        crate::infra::events::FREEZE_FORCE_COMPLETED_PAYLOAD_TYPE,
                        crate::infra::events::CatalogVersionEventBody {
                            tenant_id,
                            catalog_version_id: Some(catalog_version_id),
                            act: "force_completed",
                            participants: &forced,
                            changed_entities: &[],
                            satisfied_requests: 0,
                            checksum: None,
                            quorum_reduced: Some(quorum_reduced),
                        },
                        actor_ref,
                    )
                    .await
                    .map_err(|e| {
                        CvTxError::Repo(crate::infra::storage::RepoError::Db(e.to_string()))
                    })?;
                    let participants =
                        repo::freeze_ack_rows(tx, &scope, tenant_id, catalog_version_id)
                            .await
                            .map_err(CvTxError::Repo)?
                            .into_iter()
                            .map(|(participant, state)| FreezeParticipantStateView {
                                participant,
                                state: state.as_str().to_owned(),
                            })
                            .collect();
                    Ok(FreezeLedgerView {
                        catalog_version_id,
                        freeze_state: freeze_state.as_str().to_owned(),
                        forced,
                        participants,
                        ceremony_ref,
                        quorum_reduced,
                    })
                })
            },
        )
        .await;
    match result {
        Ok(view) => Ok((StatusCode::OK, Json(view)).into_response()),
        Err(CvTxError::NotFound) => Err(CatalogVersionResource::not_found(
            "no such catalog version in the caller's tenant",
        )
        .with_resource(attempted)
        .create()),
        Err(CvTxError::Refused(refusal)) => {
            Err(refuse_cv(&state, &scope, tenant_id, actor_ref, attempted, refusal).await)
        }
        Err(CvTxError::Repo(e)) => Err(repo_error_to_canonical(&e)),
    }
}

/// `POST /freeze-participants`.
async fn change_freeze_participant(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<FreezeParticipantChangeRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let participant = body.participant.trim().to_owned();
    let scope = match crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::FREEZE_PARTICIPANT,
        crate::authz::actions::WRITE,
        Some(tenant_id),
        None,
        true,
    )
    .await
    {
        Ok(scope) => scope,
        Err(crate::authz::AuthzError::Denied(reason)) => {
            let self_scope = AccessScope::for_tenant(tenant_id);
            return Err(crate::api::rest::audit_refusal_and_report(
                &state,
                &self_scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: crate::authz::labels::FREEZE_PARTICIPANT,
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(participant),
                CatalogVersionResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await);
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            return Err(crate::api::rest::authz_error_to_canonical(err, |reason| {
                CatalogVersionResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }));
        }
    };
    let register = match body.op.trim() {
        "register" => true,
        "retire" => false,
        _ => {
            let mut report = ValidationReport::new();
            report.violate("VALIDATION", "op", "op must be register or retire");
            return Err(refuse_cv(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                participant,
                DomainError::Validation(report),
            )
            .await);
        }
    };
    if participant.is_empty() {
        let mut report = ValidationReport::new();
        report.violate("VALIDATION", "participant", "participant must not be blank");
        return Err(refuse_cv(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            participant,
            DomainError::Validation(report),
        )
        .await);
    }
    let authorization = authorize_cv_op(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        participant_change_subject(tenant_id, &participant),
        participant.clone(),
    )
    .await?;

    let outbox = state.sink.clone();
    let scope_tx = scope.clone();
    let participant_tx = participant.clone();
    let authorization_tx = authorization.clone();
    let result = state
        .db
        .db()
        .transaction_with_retry::<bool, CvTxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            cv_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let scope = scope_tx.clone();
                let participant = participant_tx.clone();
                let authorization = authorization_tx.clone();
                Box::pin(async move {
                    settle_cv_op(tx, &scope, tenant_id, &authorization, now).await?;
                    let changed = if register {
                        repo::register_freeze_participant(tx, &scope, tenant_id, &participant, now)
                            .await
                    } else {
                        repo::retire_freeze_participant(tx, &scope, tenant_id, &participant).await
                    }
                    .map_err(CvTxError::Repo)?;
                    // A no-op (registering a registered participant, retiring an
                    // unknown one) spends the ceremony but writes no audit row and
                    // announces nothing: the set did not change.
                    if changed {
                        repo::write_keyed_act_audit(
                            tx,
                            &scope,
                            repo::AuditCommon {
                                audit_id: Uuid::now_v7(),
                                tenant_id,
                                actor_ref,
                                action: if register {
                                    "freeze_participant.register".to_owned()
                                } else {
                                    "freeze_participant.retire".to_owned()
                                },
                                subject_kind: crate::authz::labels::FREEZE_PARTICIPANT.to_owned(),
                                reason: None,
                                correlation_id: crate::infra::events::correlation_id(),
                                written_at: now,
                            },
                            participant.clone(),
                        )
                        .await
                        .map_err(CvTxError::Repo)?;
                        let moved = vec![participant.clone()];
                        crate::infra::events::enqueue_catalog_version_event(
                            &outbox,
                            tx,
                            crate::infra::events::FREEZE_PARTICIPANT_SET_CHANGED_PAYLOAD_TYPE,
                            crate::infra::events::CatalogVersionEventBody {
                                tenant_id,
                                catalog_version_id: None,
                                act: if register {
                                    "participant_registered"
                                } else {
                                    "participant_retired"
                                },
                                participants: &moved,
                                changed_entities: &[],
                                satisfied_requests: 0,
                                checksum: None,
                                quorum_reduced: None,
                            },
                            actor_ref,
                        )
                        .await
                        .map_err(|e| {
                            CvTxError::Repo(crate::infra::storage::RepoError::Db(e.to_string()))
                        })?;
                    }
                    Ok(changed)
                })
            },
        )
        .await;
    match result {
        Ok(changed) => Ok((
            StatusCode::OK,
            Json(FreezeParticipantView {
                participant,
                state: if register { "registered" } else { "retired" }.to_owned(),
                changed,
            }),
        )
            .into_response()),
        Err(CvTxError::NotFound) => Err(CatalogVersionResource::not_found("no such participant")
            .with_resource(participant)
            .create()),
        Err(CvTxError::Refused(refusal)) => {
            Err(refuse_cv(&state, &scope, tenant_id, actor_ref, participant, refusal).await)
        }
        Err(CvTxError::Repo(e)) => Err(repo_error_to_canonical(&e)),
    }
}

/// `GET /catalog-versions/{a}/diff/{b}` (`dod-diff-door`): computed from the
/// two stored manifests, sorted throughout, no writes.
///
/// @cpt-dod:cpt-cf-bss-products-dod-diff-door:p2
async fn diff_catalog_versions(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path((a, b)): axum::extract::Path<(i64, i64)>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let attempted = format!("{a}..{b}");
    let scope = catalog_version_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        crate::authz::actions::READ,
        attempted.clone(),
    )
    .await?;
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
            "diff connection: {e}"
        )))
    })?;
    let mut manifests = Vec::with_capacity(2);
    for id in [a, b] {
        if repo::find_catalog_version(&conn, &scope, tenant_id, id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?
            .is_none()
        {
            let refusal = DomainError::CatalogVersionUnknown(format!(
                "catalog version {id} is not one of this tenant's"
            ));
            return Err(refuse_cv(&state, &scope, tenant_id, actor_ref, attempted, refusal).await);
        }
        manifests.push(
            repo::catalog_version_manifest_rows(&conn, &scope, tenant_id, id)
                .await
                .map_err(|e| repo_error_to_canonical(&e))?,
        );
    }
    let (entries_b, captures_b) = manifests.pop().unwrap_or_default();
    let (entries_a, captures_a) = manifests.pop().unwrap_or_default();
    Ok((
        StatusCode::OK,
        Json(diff_manifests(
            a,
            b,
            &entries_a,
            &captures_a,
            &entries_b,
            &captures_b,
        )),
    )
        .into_response())
}

/// The pure diff: entities by `(kind, id)`, captures by kind; every list
/// sorted so the same pair renders the same bytes.
fn diff_manifests(
    a: i64,
    b: i64,
    entries_a: &[repo::SnapshotEntityRef],
    captures_a: &[(String, String)],
    entries_b: &[repo::SnapshotEntityRef],
    captures_b: &[(String, String)],
) -> VersionDiffView {
    use std::collections::BTreeMap;
    let index = |entries: &[repo::SnapshotEntityRef]| -> BTreeMap<(String, Uuid), i64> {
        entries
            .iter()
            .map(|e| ((e.entity_kind.clone(), e.entity_id), e.published_version))
            .collect()
    };
    let (ia, ib) = (index(entries_a), index(entries_b));
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for ((kind, id), version_b) in &ib {
        match ia.get(&(kind.clone(), *id)) {
            None => added.push(EntityDeltaView {
                entity_kind: kind.clone(),
                entity_id: *id,
                from_published_version: None,
                to_published_version: Some(*version_b),
            }),
            Some(version_a) if version_a != version_b => changed.push(EntityDeltaView {
                entity_kind: kind.clone(),
                entity_id: *id,
                from_published_version: Some(*version_a),
                to_published_version: Some(*version_b),
            }),
            Some(_) => {}
        }
    }
    for ((kind, id), version_a) in &ia {
        if !ib.contains_key(&(kind.clone(), *id)) {
            removed.push(EntityDeltaView {
                entity_kind: kind.clone(),
                entity_id: *id,
                from_published_version: Some(*version_a),
                to_published_version: None,
            });
        }
    }
    let ca: BTreeMap<&str, &str> = captures_a
        .iter()
        .map(|(k, c)| (k.as_str(), c.as_str()))
        .collect();
    let cb: BTreeMap<&str, &str> = captures_b
        .iter()
        .map(|(k, c)| (k.as_str(), c.as_str()))
        .collect();
    let changed_captures = cb
        .iter()
        .filter(|(kind, content)| ca.get(*kind).is_some_and(|before| before != *content))
        .map(|(kind, _)| (*kind).to_owned())
        .collect();
    let added_captures = cb
        .keys()
        .filter(|kind| !ca.contains_key(*kind))
        .map(|kind| (*kind).to_owned())
        .collect();
    let removed_captures = ca
        .keys()
        .filter(|kind| !cb.contains_key(*kind))
        .map(|kind| (*kind).to_owned())
        .collect();
    VersionDiffView {
        a,
        b,
        added,
        removed,
        changed,
        changed_captures,
        added_captures,
        removed_captures,
    }
}
