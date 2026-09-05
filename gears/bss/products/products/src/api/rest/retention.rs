//! `10-retention-erasure`'s two wire doors: the erasure request and the
//! compliance identity export.
//!
//! # The erasure door must not resolve through the shared actor context
//!
//! `crate::api::rest::resolve_creator_actor_ref` mints on a miss, which is
//! right for the **caller** — an operator acting for the first time needs a
//! pseudonym — and wrong for the **subject**: an unknown principal would gain
//! a fresh live row and the door would answer success on a DSAR it never
//! served. So the caller's own ref comes from the shared context and the
//! subject's from `repo::tombstone_principal`, whose miss is the refusal.
//!
//! # The request names a `principal_ref`, and that is an owner call
//!
//! Owner call, 2026-09-03, on `features/retention-erasure.md` §7 row 24. The
//! alternative — an identity string — makes the refusal *"naming the
//! principal"* write personal data into its own audit row, which is the
//! failure `dod-pii-detector` forbids for `CONTENT_PII_BLOCKED`. The
//! person-to-pseudonym step belongs to whichever identity provider minted the
//! principal; this door is internal to the gear.
//!
//! The row's stated objection to that arm was measured false before it was
//! taken: `principal_ref` is `NOT NULL` and survives the tombstone by design
//! (P-D-49), so a repeat DSAR still resolves, and a **first** DSAR from a
//! principal that never held a ref resolves to *"no entries"* — a correct
//! answer, not a failure.
//!
//! # The reason is operator free text and goes through the write block
//!
//! `inst-av-pii-reason` enumerates *"every operator free-text `reason`"* and
//! `dod-pii-detector` obliges the whole set to raise the same code. The
//! erasure reason is one of them, so it passes `content_pii_block` before it
//! reaches an audit row — with the registered `NoPiiPolicyDetector` until
//! `dod-pii-detector` lands a real one, exactly as the product and SKU doors
//! do.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-erasure-door:p1
//! @cpt-dod:cpt-cf-bss-products-dod-compliance-export:p1
//! @cpt-dod:cpt-cf-bss-products-dod-retention-events:p1
//! @cpt-dod:cpt-cf-bss-products-dod-pii-allowlist:p1
//!
//! # Both audits are preconditions, in opposite directions
//!
//! The erasure's evidential row commits **inside** the tombstone's own
//! transaction: the act and its record stand or fall together, and a DSAR
//! whose evidence did not commit did not happen. The export's row commits in
//! a transaction **of its own, before anything is served** (P-D-34): a read
//! has no mutation transaction to join, and an access the registry did not
//! record is what individual auditing exists to prevent.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
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
use crate::domain::retention::RegistryPiiDetector;
use crate::domain::taxonomy::{PiiDetector, content_pii_block};
use crate::infra::storage::repo::{self, RefusalSubject};

/// The `OpenAPI` tag both doors register under.
const TAG: &str = "BSS Products";

/// What an audit row of this feature's identity acts names as its subject
/// kind.
const SUBJECT_KIND: &str = "identity_ref";

/// What an audit row of an allow-list act names as its subject kind.
const ALLOWLIST_SUBJECT_KIND: &str = "pii_allowlist";

/// The `GovernedLiveOp` target an allow-list mutation submits under.
///
/// A string rather than an `EntityRef` for `02`'s reason: `EntityKind` is
/// exactly `Product | Sku`, and an allow-list entry is neither, so
/// `GateSubject::governed_live_op` is the seam that takes it.
const ALLOWLIST_LIVE_OP_TARGET: &str = "pii_allowlist";

/// The canonical-error identity of the erasure surface's refusals.
#[resource_error(gts_id!("cf.bss.products.erasure.v1~"))]
struct ErasureResource;

/// The canonical-error identity of the compliance surface's refusals.
#[resource_error(gts_id!("cf.bss.products.compliance.v1~"))]
struct ComplianceResource;

/// The canonical-error identity of the allow-list surface's refusals.
#[resource_error(gts_id!("cf.bss.products.pii_allowlist.v1~"))]
struct AllowlistResource;

/// A DSAR erasure, as the caller states it.
#[toolkit_macros::api_dto(request)]
pub struct ErasureRequest {
    /// The pseudonym of the principal to erase, in this tenant.
    pub principal_ref: String,
    /// Why, for the evidential audit row. Operator free text, and inside the
    /// content-PII write block.
    pub reason: String,
}

/// What the erasure door answers on success.
#[toolkit_macros::api_dto(response)]
pub struct ErasureReceipt {
    /// The pseudonym that was retired. It stays in every immutable record.
    pub actor_ref: Uuid,
    /// When the tombstone was stamped.
    pub tombstoned_at: DateTime<Utc>,
}

/// One map entry, as the export renders it.
#[toolkit_macros::api_dto(response)]
pub struct IdentityEntryView {
    /// The pseudonym.
    pub actor_ref: Uuid,
    /// The identity, where one was stored and has not been destroyed.
    pub identity_payload: Option<String>,
    /// Set once, by erasure, and never cleared.
    pub tombstoned_at: Option<DateTime<Utc>>,
    /// When the ref was minted.
    pub first_seen_at: DateTime<Utc>,
    /// When an act last resolved it.
    pub last_seen_at: DateTime<Utc>,
}

/// The DSAR answer for one principal.
#[toolkit_macros::api_dto(response)]
pub struct IdentityExport {
    /// The principal the caller named, echoed so a stored response is
    /// self-describing.
    pub principal_ref: String,
    /// Every entry the principal has held here, tombstoned ones included.
    pub entries: Vec<IdentityEntryView>,
    /// The `audit_id`s of the rows carrying those refs. References, not rows:
    /// an audit row's `reason` is operator free text, and returning it would
    /// put a second copy of whatever it holds into the export.
    pub audit_references: Vec<Uuid>,
}

/// An allow-list entry as the caller offers it.
#[toolkit_macros::api_dto(request)]
pub struct AllowlistEntryRequest {
    /// The person-named string Legal admitted, as the operator typed it. It
    /// is normalized before it is stored, and the normalization is the whole
    /// of the match rule — see
    /// [`crate::domain::retention::normalize_allowlist_value`].
    pub value: String,
    /// Why Legal admitted it. Operator free text, and inside the content-PII
    /// write block.
    pub justification: String,
    /// The reference to the external Legal decision — the artifact, never a
    /// person. **Mandatory**: an entry offered without it is refused riding
    /// `01`'s `VALIDATION` naming the field (P-D-64).
    pub signed_off_by: String,
    /// When Legal signed off.
    pub signed_off_at: DateTime<Utc>,
}

/// What the sign-off door answers.
#[toolkit_macros::api_dto(response)]
pub struct AllowlistEntryReceipt {
    /// The entry's stable address, and the event's aggregate.
    pub entry_id: Uuid,
    /// The stored form the detector matches on, echoed so an operator can see
    /// what the normalization made of their input.
    pub value_normalized: String,
    /// `active`.
    pub state: String,
}

/// The revoke door's body.
#[toolkit_macros::api_dto(request)]
pub struct AllowlistOperationRequest {
    /// The only operation: `revoke`.
    pub op: String,
    /// Why. Operator free text, and inside the content-PII write block.
    pub reason: String,
}

/// One allow-list entry as the Legal review renders it.
#[toolkit_macros::api_dto(response)]
pub struct AllowlistEntryView {
    pub entry_id: Uuid,
    /// The stored, normalized value.
    pub value_normalized: String,
    pub justification: String,
    /// The reference to the external Legal decision.
    pub signed_off_by: String,
    pub signed_off_at: DateTime<Utc>,
    /// `active` or `revoked`. Revoked entries are **in** the review: a
    /// revocation is a state flip precisely so the sign-off that admitted the
    /// entry stays on record.
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The Legal review's answer.
#[toolkit_macros::api_dto(response)]
pub struct AllowlistExport {
    /// Every entry in this tenant, active and revoked, oldest first.
    pub entries: Vec<AllowlistEntryView>,
}

/// The export's query parameters.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportQuery {
    /// The principal to export.
    principal_ref: String,
    /// Why this access is being made — **required** (**P-D-133**, `10` row
    /// 11). The one surface that returns real identities may not be spent
    /// without a stated reason, and the reason lands on the access's own
    /// audit row rather than in a log line. Operator free text, so it rides
    /// the content-PII write block like every other.
    justification: String,
}

/// Register both doors.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = OperationBuilder::post("/bss-products/v1/erasure-requests")
        .operation_id("bss_products.execute_erasure_request")
        .summary("Erase an actor's identity in this tenant")
        .description(
            "Tombstones the named principal's live map entry in one transaction - the identity \
             payload is destroyed, `tombstoned_at` is stamped and `principal_ref` stands - and \
             writes the evidential audit row in the same transaction, under the eraser's own \
             pseudonymous ref. No immutable record is touched (C1). Erasure is per-tenant \
             (P-D-50): a principal appearing in several tenants needs one request per tenant. A \
             principal with no live ref here is refused `ERASURE_UNKNOWN_ACTOR`. The request \
             names a `principal_ref`, never a real-world identity, so a refusal naming the \
             principal writes no personal data into its own audit row.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<ErasureRequest>(openapi, "The principal to erase, and why.")
        .handler(execute_erasure)
        .json_response_with_schema::<ErasureReceipt>(
            openapi,
            StatusCode::OK,
            "The retired pseudonym and the instant it was retired.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::get("/bss-products/v1/compliance/identity-export")
        .operation_id("bss_products.export_identity_map")
        .summary("Export a principal's identity-map entries")
        .description(
            "Returns, per named principal, that principal's map entries - tombstoned ones \
             included, since a DSAR after an erasure must be able to see that the erasure \
             happened - plus the references of the audit rows carrying those refs. Spends \
             `compliance x export`, its own grant and never `audit x export`: this is the one \
             surface that returns real identities, and folding it into the audit grant would \
             hand every auditor the identities the pseudonymisation scheme exists to withhold. \
             Every access is audited individually, in its own transaction, before anything is \
             served.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .query_param("principalRef", true, "The principal to export.")
        .query_param(
            "justification",
            true,
            "Why this access is being made. Required: the one surface that returns real \
             identities is not served unreasoned, and this lands on the access's audit row.",
        )
        .handler(export_identity_map)
        .json_response_with_schema::<IdentityExport>(
            openapi,
            StatusCode::OK,
            "The principal's entries and its audit references.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/pii-allowlist-entries")
        .operation_id("bss_products.sign_off_pii_allowlist_entry")
        .summary("Sign a person-named string onto the PII allow-list")
        .description(
            "Records a Legal-signed-off entry the PII detector will admit. The mutation is a \
             `GovernedLiveOp` on `pii_allowlist x write` under the base approver quorum \
             (P-D-10): there is no gear-side Legal role, and what the gear proves is that a \
             Legal reference was recorded - never that Legal approved. An entry offered without \
             `signedOffBy` is refused riding `VALIDATION` naming the field (P-D-64). The value \
             is normalized before it is stored and the match is exact equality on that form, \
             never a pattern: a pattern would let one sign-off widen itself. A second ACTIVE \
             entry for the same normalized value is refused by \
             `uq_products_pii_allowlist_active`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<AllowlistEntryRequest>(openapi, "The entry, and its Legal reference.")
        .handler(sign_off_allowlist_entry)
        .json_response_with_schema::<AllowlistEntryReceipt>(
            openapi,
            StatusCode::OK,
            "The entry's id and the stored, normalized value.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router =
        OperationBuilder::post("/bss-products/v1/pii-allowlist-entries/{entryId}/operations")
            .operation_id("bss_products.operate_pii_allowlist_entry")
            .summary("Revoke a PII allow-list entry")
            .description(
                "`op: revoke` flips an active entry to `revoked`. **Never a DELETE** (P-D-47's \
         reasoning): a revoked entry keeps its sign-off on record, which is what makes the \
         paper control auditable, and the partial unique index scopes uniqueness to the active \
         rows so the same value may be signed off again later as its own row. An entry that is \
         not active is refused.",
            )
            .tag(TAG)
            .authenticated()
            .no_license_required()
            .path_param("entryId", "The entry to operate on.")
            .json_request::<AllowlistOperationRequest>(openapi, "The operation, and why.")
            .handler(operate_allowlist_entry)
            .json_response_with_schema::<AllowlistEntryReceipt>(
                openapi,
                StatusCode::OK,
                "The entry's id and its new state.",
            )
            .error_400(openapi)
            .error_401(openapi)
            .error_403(openapi)
            .error_404(openapi)
            .error_500(openapi)
            .error_503(openapi)
            .register(router, openapi);

    let router = OperationBuilder::get("/bss-products/v1/compliance/pii-allowlist")
        .operation_id("bss_products.export_pii_allowlist")
        .summary("Export the PII allow-list for the Legal review")
        .description(
            "Returns every entry in this tenant, active and revoked, oldest first - the review \
             `inst-pp-allowlist` obliges. Spends `compliance x export` and not \
             `pii_allowlist x write`: the table is a PII store by construction and takes the \
             identity map's posture (P-D-117 item 12), excluded from every export EXCEPT the \
             compliance surface, and a read served under a write grant would be the second.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(export_allowlist)
        .json_response_with_schema::<AllowlistExport>(
            openapi,
            StatusCode::OK,
            "Every entry in this tenant.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router.layer(Extension(state))
}

/// Authorize one of this feature's grants, auditing a denial as a refusal.
async fn retention_scope(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    gate: Gate,
    subject: String,
) -> Result<AccessScope, CanonicalError> {
    match crate::authz::access_scope(
        enforcer,
        ctx,
        &gate.resource(),
        gate.action(),
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
                    subject_kind: gate.subject_kind(),
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(subject),
                gate.permission_denied(reason),
            )
            .await)
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            Err(crate::api::rest::authz_error_to_canonical(err, |reason| {
                gate.permission_denied(reason)
            }))
        }
    }
}

/// Which of this feature's three grants a door spends.
#[derive(Clone, Copy)]
enum Gate {
    /// `erasure × execute`.
    Erasure,
    /// `compliance × export`.
    Compliance,
    /// `pii_allowlist × write`.
    Allowlist,
}

impl Gate {
    fn resource(self) -> authz_resolver_sdk::ResourceType {
        match self {
            Self::Erasure => crate::authz::resource_types::ERASURE,
            Self::Compliance => crate::authz::resource_types::COMPLIANCE,
            Self::Allowlist => crate::authz::resource_types::PII_ALLOWLIST,
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::Erasure => crate::authz::actions::EXECUTE,
            Self::Compliance => crate::authz::actions::EXPORT,
            Self::Allowlist => crate::authz::actions::WRITE,
        }
    }

    /// The subject kind an audit row of a door spending this grant names.
    fn subject_kind(self) -> &'static str {
        match self {
            Self::Erasure | Self::Compliance => SUBJECT_KIND,
            Self::Allowlist => ALLOWLIST_SUBJECT_KIND,
        }
    }

    fn permission_denied(self, reason: String) -> CanonicalError {
        match self {
            Self::Erasure => ErasureResource::permission_denied()
                .with_reason(reason)
                .create(),
            Self::Compliance => ComplianceResource::permission_denied()
                .with_reason(reason)
                .create(),
            Self::Allowlist => AllowlistResource::permission_denied()
                .with_reason(reason)
                .create(),
        }
    }
}

/// Build this feature's PII detector over the tenant's active allow-list.
///
/// **This is `dod-pii-detector`'s host swap, as a function rather than as a
/// literal at each door.** Every construction of `NoPiiPolicyDetector` in the
/// crate was its own `Arc::new(..)`, so "the registered detector" named a
/// phrase and not a registry, and swapping it meant finding six literals. A
/// door calls this instead, and the day the policy changes again there is one
/// line to change.
///
/// The read is outside any mutation transaction and before it, deliberately:
/// [`PiiDetector::inspect`] is synchronous so it cannot read a store itself,
/// and a detector built inside the act's transaction would hold that
/// transaction open across a read the act does not need.
///
/// A tenant with no entries gets an empty detector, which is a resolved state
/// and not a failed read — every person-shaped candidate is then undecidable,
/// which is the correct answer.
pub(crate) async fn tenant_pii_detector(
    state: &ApiState,
    tenant_id: Uuid,
) -> Result<Arc<dyn PiiDetector + Send + Sync>, CanonicalError> {
    // The tenant's own scope, not the caller's, and that is forced rather
    // than convenient: the list is an *input to a refusal*, no grant pair
    // `pii_allowlist × read` exists, and reading it under the caller's scope
    // would make every write-blocking door require a grant nobody holds. The
    // caller has already passed its own door's authz above; this read is the
    // gear consulting its own configuration inside that tenant, which is the
    // same self-scope a denial's audit row is written under.
    let scope = &AccessScope::for_tenant(tenant_id);
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let values = repo::active_allowlist_values(&conn, scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    Ok(Arc::new(RegistryPiiDetector::new(values)))
}

/// Refuse, audit the refusal, and answer.
async fn refuse(
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
            subject_kind: SUBJECT_KIND,
            error_code: code,
        },
        RefusalSubject::Attempted(subject),
        CanonicalError::from(refusal),
    )
    .await
}

/// @cpt-flow:cpt-cf-bss-products-flow-erasure:p1
async fn execute_erasure(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<ErasureRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let principal_ref = body.principal_ref.trim().to_owned();
    let reason = body.reason.trim().to_owned();
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = retention_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Erasure,
        principal_ref.clone(),
    )
    .await?;

    if principal_ref.is_empty() || reason.is_empty() {
        let mut report = crate::domain::validation::ValidationReport::new();
        if principal_ref.is_empty() {
            report.violate(
                "VALIDATION",
                "principalRef",
                "principalRef must not be blank",
            );
        }
        if reason.is_empty() {
            report.violate("VALIDATION", "reason", "reason must not be blank");
        }
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            principal_ref,
            DomainError::Validation(report),
        )
        .await);
    }

    // The reason is operator free text, so it rides the same write block every
    // other reason-bearing door does — now against this feature's own
    // detector rather than the permissive host.
    let detector = tenant_pii_detector(&state, tenant_id).await?;
    if let Err(blocked) = content_pii_block(detector.as_ref(), "reason", &reason) {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            principal_ref,
            DomainError::ContentPiiBlocked(blocked.into_detail()),
        )
        .await);
    }

    let audit_id = Uuid::now_v7();
    let scope_tx = scope.clone();
    let principal_tx = principal_ref.clone();
    let reason_tx = reason.clone();
    let sink_tx = state.sink.clone();
    let result = state
        .db
        .db()
        .transaction_with_retry::<Option<Uuid>, TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let principal_ref = principal_tx.clone();
                let reason = reason_tx.clone();
                let sink = sink_tx.clone();
                Box::pin(async move {
                    let Some(retired) =
                        repo::tombstone_principal(tx, &scope, tenant_id, &principal_ref, now)
                            .await
                            .map_err(TxError::Repo)?
                    else {
                        return Ok(None);
                    };
                    // Same transaction as the tombstone, deliberately: the act
                    // and its evidence stand or fall together.
                    repo::write_evidential_act_audit(
                        tx,
                        &scope,
                        repo::AuditCommon {
                            audit_id,
                            tenant_id,
                            actor_ref,
                            action: "erasure.execute".to_owned(),
                            subject_kind: SUBJECT_KIND.to_owned(),
                            reason: Some(reason),
                            correlation_id: crate::infra::events::correlation_id(),
                            written_at: now,
                        },
                        retired,
                    )
                    .await
                    .map_err(TxError::Repo)?;
                    // Same transaction again: `dod-retention-events` requires
                    // the event ride the act, and an `ActorErased` that
                    // committed beside a rolled-back tombstone would tell
                    // every cache to drop an identity that is still there.
                    crate::infra::events::enqueue_retention(
                        &sink,
                        tx,
                        crate::infra::events::retention_aggregate_id(tenant_id, &principal_ref),
                        crate::infra::events::ACTOR_ERASED_PAYLOAD_TYPE,
                        &crate::infra::events::RetentionEventBody {
                            tenant_id,
                            subject_ref: &principal_ref,
                            act: "erased",
                            erased_actor_ref: Some(retired),
                        },
                        actor_ref,
                    )
                    .await
                    .map_err(TxError::Events)?;
                    Ok(Some(retired))
                })
            },
        )
        .await;

    match result {
        Ok(Some(retired)) => Ok((
            StatusCode::OK,
            Json(ErasureReceipt {
                actor_ref: retired,
                tombstoned_at: now,
            }),
        )
            .into_response()),
        Ok(None) => Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            principal_ref.clone(),
            DomainError::ErasureUnknownActor(format!(
                "no live actor_ref for principal `{principal_ref}` in this tenant"
            )),
        )
        .await),
        Err(TxError::Repo(e)) => Err(repo_error_to_canonical(&e)),
        // The erasure is ungoverned and spends no record; the arm exists
        // because the tx error type is shared with the allow-list doors.
        Err(TxError::Refused(refusal)) => Err(CanonicalError::from(refusal)),
        Err(TxError::Events(e)) => Err(repo_error_to_canonical(
            &crate::infra::storage::RepoError::Db(format!("retention event: {e}")),
        )),
    }
}

/// @cpt-algo:cpt-cf-bss-products-algo-identity-map:p1
async fn export_identity_map(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Query(query): axum::extract::Query<ExportQuery>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let principal_ref = query.principal_ref.trim().to_owned();
    let justification = query.justification.trim().to_owned();
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = retention_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Compliance,
        principal_ref.clone(),
    )
    .await?;

    let mut report = crate::domain::validation::ValidationReport::new();
    if principal_ref.is_empty() {
        report.violate(
            "VALIDATION",
            "principalRef",
            "principalRef must not be blank",
        );
    }
    if justification.is_empty() {
        report.violate(
            "VALIDATION",
            "justification",
            "justification must not be blank: an export of real identities is not served \
             unreasoned",
        );
    }
    if !report.is_empty() {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            principal_ref,
            DomainError::Validation(report),
        )
        .await);
    }

    if let Err(blocked) = content_pii_block(
        tenant_pii_detector(&state, tenant_id).await?.as_ref(),
        "justification",
        &justification,
    ) {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            principal_ref,
            DomainError::ContentPiiBlocked(blocked.into_detail()),
        )
        .await);
    }

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let entries = repo::identity_entries_of_principal(&conn, &scope, tenant_id, &principal_ref)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let refs: Vec<Uuid> = entries.iter().map(|entry| entry.actor_ref).collect();
    let audit_references = repo::audit_refs_of_actors(&conn, &scope, tenant_id, &refs)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;

    // The audit commits in a transaction of its own and is a precondition of
    // serving (P-D-34). An access the registry did not record is exactly what
    // individual auditing exists to prevent, so a write failure serves
    // nothing.
    let scope_tx = scope.clone();
    let principal_tx = principal_ref.clone();
    let justification_tx = justification.clone();
    let audit_id = Uuid::now_v7();
    state
        .db
        .db()
        .transaction_with_retry::<(), TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let principal_ref = principal_tx.clone();
                let justification = justification_tx.clone();
                Box::pin(async move {
                    repo::write_audited_read_audit(
                        tx,
                        &scope,
                        repo::AuditCommon {
                            audit_id,
                            tenant_id,
                            actor_ref,
                            action: "compliance.export".to_owned(),
                            subject_kind: SUBJECT_KIND.to_owned(),
                            reason: Some(justification),
                            correlation_id: crate::infra::events::correlation_id(),
                            written_at: now,
                        },
                        principal_ref,
                    )
                    .await
                    .map_err(TxError::Repo)
                })
            },
        )
        .await
        .map_err(|_| {
            CanonicalError::from(DomainError::AuditUnavailable(
                "the compliance export's audit row could not be written, and an unaudited export \
                 is not served"
                    .to_owned(),
            ))
        })?;

    Ok((
        StatusCode::OK,
        Json(IdentityExport {
            principal_ref,
            entries: entries
                .into_iter()
                .map(|entry| IdentityEntryView {
                    actor_ref: entry.actor_ref,
                    identity_payload: entry.identity_payload,
                    tombstoned_at: entry.tombstoned_at,
                    first_seen_at: entry.first_seen_at,
                    last_seen_at: entry.last_seen_at,
                })
                .collect(),
            audit_references,
        }),
    )
        .into_response())
}

/// Submit an allow-list mutation to `05`'s live-op gate.
///
/// The shape is `02`'s and `05`'s `submit_to_gate`, deliberately: a third way
/// of asking one seam the same question is how three doors come to disagree
/// about what a live op is. `inst-mt-inputs` (d) registers this kind
/// **material**, so the day a store-backed host is registered this is where
/// the base approver quorum starts applying (P-D-10 — there is no gear-side
/// Legal role, and the quorum is the tenant's approvers).
///
/// The pin is [`SubjectPin::Unpinned`] — a live op has no entity head to
/// read a revision from, and P-D-125 row 52 folded the pin into the subject
/// (strand B, merged 2026-09-04; this call was reconstructed at that merge).
/// Refuse an allow-list act, audit the refusal under its own subject kind,
/// and answer.
async fn refuse_allowlist(
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
            subject_kind: ALLOWLIST_SUBJECT_KIND,
            error_code: code,
        },
        RefusalSubject::Attempted(subject),
        CanonicalError::from(refusal),
    )
    .await
}

/// `POST /bss-products/v1/pii-allowlist-entries`.
///
/// @cpt-flow:cpt-cf-bss-products-flow-pii-policy:p1
async fn sign_off_allowlist_entry(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<AllowlistEntryRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = retention_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Allowlist,
        ALLOWLIST_LIVE_OP_TARGET.to_owned(),
    )
    .await?;

    let value = body.value.trim().to_owned();
    let justification = body.justification.trim().to_owned();
    let signed_off_by = body.signed_off_by.trim().to_owned();
    let mut report = crate::domain::validation::ValidationReport::new();
    if value.is_empty() {
        report.violate("VALIDATION", "value", "value must not be blank");
    }
    if justification.is_empty() {
        report.violate(
            "VALIDATION",
            "justification",
            "justification must not be blank",
        );
    }
    // P-D-64: the missing mandatory member of an offered entry is a
    // shape-class refusal riding `01`'s VALIDATION with the violation naming
    // the field, and this feature mints no code of its own for it.
    if signed_off_by.is_empty() {
        report.violate(
            "VALIDATION",
            "signedOffBy",
            "signedOffBy must not be blank: an allow-list entry stands on a recorded Legal \
             sign-off reference, and an entry without one records nothing",
        );
    }
    let normalized = crate::domain::retention::normalize_allowlist_value(&value);
    if !value.is_empty() && normalized.is_empty() {
        report.violate(
            "VALIDATION",
            "value",
            "value normalizes to the empty string, which no detector candidate can equal",
        );
    }
    if !report.is_empty() {
        return Err(refuse_allowlist(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            ALLOWLIST_LIVE_OP_TARGET.to_owned(),
            DomainError::Validation(report),
        )
        .await);
    }

    // Both free-text fields ride the write block (P-D-117 item 12), against
    // the tenant's own list — so an operator may justify an entry using a
    // name the list already admits, and may not smuggle a second one in.
    let detector = tenant_pii_detector(&state, tenant_id).await?;
    for (field, text) in [
        ("justification", &justification),
        ("signedOffBy", &signed_off_by),
    ] {
        if let Err(blocked) = content_pii_block(detector.as_ref(), field, text) {
            return Err(refuse_allowlist(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                ALLOWLIST_LIVE_OP_TARGET.to_owned(),
                DomainError::ContentPiiBlocked(blocked.into_detail()),
            )
            .await);
        }
    }

    let authorization = match crate::api::rest::authorize_live_op(
        &state,
        &scope,
        tenant_id,
        crate::domain::governance::GateSubject::governed_live_op(
            tenant_id,
            ALLOWLIST_LIVE_OP_TARGET,
            crate::domain::governance::SubjectPin::Unpinned,
        ),
    )
    .await
    {
        Ok(authorization) => authorization,
        Err(crate::api::rest::HostError::Refused(refusal)) => {
            return Err(refuse_allowlist(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                ALLOWLIST_LIVE_OP_TARGET.to_owned(),
                refusal,
            )
            .await);
        }
        Err(crate::api::rest::HostError::Repo(error)) => {
            return Err(repo_error_to_canonical(&error));
        }
    };

    let entry_id = Uuid::now_v7();
    let audit_id = Uuid::now_v7();
    let entry = repo::NewAllowlistEntry {
        tenant_id,
        entry_id,
        value_normalized: normalized.clone(),
        justification: justification.clone(),
        signed_off_by: signed_off_by.clone(),
        signed_off_at: canonical::write_instant(body.signed_off_at),
        now,
    };
    let scope_tx = scope.clone();
    let sink_tx = state.sink.clone();
    let reason_tx = justification.clone();
    let authorization_tx = authorization.clone();
    let outcome = state
        .db
        .db()
        .transaction_with_retry::<(), TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let sink = sink_tx.clone();
                let entry = entry.clone();
                let reason = reason_tx.clone();
                let authorization = authorization_tx.clone();
                Box::pin(async move {
                    repo::settle_authorization(tx, &scope, tenant_id, &authorization, now)
                        .await
                        .map_err(|error| match error {
                            repo::SettleError::Refused(refusal) => TxError::Refused(refusal),
                            repo::SettleError::Repo(error) => TxError::Repo(error),
                        })?;
                    repo::insert_entry(tx, &scope, entry)
                        .await
                        .map_err(TxError::Repo)?;
                    write_allowlist_audit(
                        tx,
                        &scope,
                        AllowlistAudit {
                            audit_id,
                            tenant_id,
                            actor_ref,
                            entry_id,
                            action: "pii_allowlist.sign_off",
                            reason,
                            now,
                        },
                    )
                    .await?;
                    emit_allowlist_changed(&sink, tx, tenant_id, entry_id, "signed_off", actor_ref)
                        .await
                })
            },
        )
        .await;
    if let Err(error) = outcome {
        return Err(match error {
            TxError::Refused(refusal) => {
                refuse_allowlist(
                    &state,
                    &scope,
                    tenant_id,
                    actor_ref,
                    ALLOWLIST_LIVE_OP_TARGET.to_owned(),
                    refusal,
                )
                .await
            }
            other => tx_failure_to_canonical(other),
        });
    }

    Ok((
        StatusCode::OK,
        Json(AllowlistEntryReceipt {
            entry_id,
            value_normalized: normalized,
            state: crate::infra::storage::entity::pii_allowlist::STATE_ACTIVE.to_owned(),
        }),
    )
        .into_response())
}

/// `POST /bss-products/v1/pii-allowlist-entries/{entryId}/operations`.
async fn operate_allowlist_entry(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path(entry_id): axum::extract::Path<Uuid>,
    Json(body): Json<AllowlistOperationRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = retention_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Allowlist,
        entry_id.to_string(),
    )
    .await?;

    let op = body.op.trim().to_owned();
    let reason = body.reason.trim().to_owned();
    let mut report = crate::domain::validation::ValidationReport::new();
    if op != "revoke" {
        report.violate("VALIDATION", "op", "op must be `revoke`");
    }
    if reason.is_empty() {
        report.violate("VALIDATION", "reason", "reason must not be blank");
    }
    if !report.is_empty() {
        return Err(refuse_allowlist(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            entry_id.to_string(),
            DomainError::Validation(report),
        )
        .await);
    }

    let detector = tenant_pii_detector(&state, tenant_id).await?;
    if let Err(blocked) = content_pii_block(detector.as_ref(), "reason", &reason) {
        return Err(refuse_allowlist(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            entry_id.to_string(),
            DomainError::ContentPiiBlocked(blocked.into_detail()),
        )
        .await);
    }

    let authorization = match crate::api::rest::authorize_live_op(
        &state,
        &scope,
        tenant_id,
        crate::domain::governance::GateSubject::governed_live_op(
            tenant_id,
            ALLOWLIST_LIVE_OP_TARGET,
            crate::domain::governance::SubjectPin::Unpinned,
        ),
    )
    .await
    {
        Ok(authorization) => authorization,
        Err(crate::api::rest::HostError::Refused(refusal)) => {
            return Err(refuse_allowlist(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                entry_id.to_string(),
                refusal,
            )
            .await);
        }
        Err(crate::api::rest::HostError::Repo(error)) => {
            return Err(repo_error_to_canonical(&error));
        }
    };

    let audit_id = Uuid::now_v7();
    let scope_tx = scope.clone();
    let sink_tx = state.sink.clone();
    let reason_tx = reason.clone();
    let authorization_tx = authorization.clone();
    let outcome = state
        .db
        .db()
        .transaction_with_retry::<bool, TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let sink = sink_tx.clone();
                let reason = reason_tx.clone();
                let authorization = authorization_tx.clone();
                Box::pin(async move {
                    if !repo::revoke_entry(tx, &scope, tenant_id, entry_id, now)
                        .await
                        .map_err(TxError::Repo)?
                    {
                        return Ok(false);
                    }
                    // Spent only once the row is really revoked: a not-found
                    // answer must leave the record standing.
                    repo::settle_authorization(tx, &scope, tenant_id, &authorization, now)
                        .await
                        .map_err(|error| match error {
                            repo::SettleError::Refused(refusal) => TxError::Refused(refusal),
                            repo::SettleError::Repo(error) => TxError::Repo(error),
                        })?;
                    write_allowlist_audit(
                        tx,
                        &scope,
                        AllowlistAudit {
                            audit_id,
                            tenant_id,
                            actor_ref,
                            entry_id,
                            action: "pii_allowlist.revoke",
                            reason,
                            now,
                        },
                    )
                    .await?;
                    emit_allowlist_changed(&sink, tx, tenant_id, entry_id, "revoked", actor_ref)
                        .await?;
                    Ok(true)
                })
            },
        )
        .await;
    let revoked = match outcome {
        Ok(revoked) => revoked,
        Err(TxError::Refused(refusal)) => {
            return Err(refuse_allowlist(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                entry_id.to_string(),
                refusal,
            )
            .await);
        }
        Err(other) => return Err(tx_failure_to_canonical(other)),
    };

    if !revoked {
        // A 404 rather than a minted code: `dod-retention-error-taxonomy`
        // keeps this feature's owned roster at one, and "no such active
        // entry" is the resource-shaped refusal `01`'s canonical envelope
        // already carries. Never-existed and already-revoked answer the same
        // way, because from the caller's side they are the same fact.
        return Err(AllowlistResource::not_found(
            "no active allow-list entry with this id in this tenant",
        )
        .with_resource(entry_id.to_string())
        .create());
    }

    Ok((
        StatusCode::OK,
        Json(AllowlistEntryReceipt {
            entry_id,
            value_normalized: String::new(),
            state: crate::infra::storage::entity::pii_allowlist::STATE_REVOKED.to_owned(),
        }),
    )
        .into_response())
}

/// `GET /bss-products/v1/compliance/pii-allowlist`.
async fn export_allowlist(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = retention_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        Gate::Compliance,
        ALLOWLIST_LIVE_OP_TARGET.to_owned(),
    )
    .await?;

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let entries = repo::allowlist_entries(&conn, &scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;

    Ok((
        StatusCode::OK,
        Json(AllowlistExport {
            entries: entries
                .into_iter()
                .map(|entry| AllowlistEntryView {
                    entry_id: entry.entry_id,
                    value_normalized: entry.value_normalized,
                    justification: entry.justification,
                    signed_off_by: entry.signed_off_by,
                    signed_off_at: entry.signed_off_at,
                    state: entry.state,
                    created_at: entry.created_at,
                    updated_at: entry.updated_at,
                })
                .collect(),
        }),
    )
        .into_response())
}

/// The fields both allow-list audit rows carry.
struct AllowlistAudit {
    audit_id: Uuid,
    tenant_id: Uuid,
    actor_ref: Uuid,
    entry_id: Uuid,
    action: &'static str,
    reason: String,
    now: DateTime<Utc>,
}

/// One allow-list audit row, in the act's own transaction.
///
/// `write_eventless_act_audit` would be the wrong writer: this act **does**
/// emit an event. The row is P-D-21's class 3 — a committed act — and the
/// event beside it is `PiiAllowlistChanged`.
async fn write_allowlist_audit(
    tx: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    audit: AllowlistAudit,
) -> Result<(), TxError> {
    repo::write_evidential_act_audit(
        tx,
        scope,
        repo::AuditCommon {
            audit_id: audit.audit_id,
            tenant_id: audit.tenant_id,
            actor_ref: audit.actor_ref,
            action: audit.action.to_owned(),
            subject_kind: ALLOWLIST_SUBJECT_KIND.to_owned(),
            reason: Some(audit.reason),
            correlation_id: crate::infra::events::correlation_id(),
            written_at: audit.now,
        },
        audit.entry_id,
    )
    .await
    .map_err(TxError::Repo)
}

/// `PiiAllowlistChanged`, in the act's own transaction, partitioned on the
/// entry (**P-D-118** item 26).
async fn emit_allowlist_changed(
    sink: &crate::infra::broker::EventSink,
    tx: &(impl toolkit_db::secure::DBRunner + Sync),
    tenant_id: Uuid,
    entry_id: Uuid,
    act: &str,
    actor_ref: Uuid,
) -> Result<(), TxError> {
    let subject_ref = entry_id.to_string();
    crate::infra::events::enqueue_retention(
        sink,
        tx,
        entry_id,
        crate::infra::events::PII_ALLOWLIST_CHANGED_PAYLOAD_TYPE,
        &crate::infra::events::RetentionEventBody {
            tenant_id,
            subject_ref: &subject_ref,
            act,
            // No ref was retired: this arm's `erased_actor_ref` is `None` by
            // construction, and the acting principal rides `actor_ref`.
            erased_actor_ref: None,
        },
        actor_ref,
    )
    .await
    .map_err(TxError::Events)
}

/// Render a transaction's failure. The doors' own refusals never travel this
/// way — they carry the subject `refuse_allowlist` needs.
fn tx_failure_to_canonical(error: TxError) -> CanonicalError {
    match error {
        TxError::Refused(refusal) => CanonicalError::from(refusal),
        TxError::Repo(e) => repo_error_to_canonical(&e),
        TxError::Events(e) => repo_error_to_canonical(&crate::infra::storage::RepoError::Db(
            format!("retention event: {e}"),
        )),
    }
}

/// The retry loop classifies `sea-orm`'s own error, which `RepoError::Driver`
/// carries directly.
fn contention_db_err(error: &TxError) -> Option<&sea_orm::DbErr> {
    match error {
        TxError::Repo(crate::infra::storage::RepoError::Driver { source, .. }) => Some(source),
        TxError::Repo(_) | TxError::Events(_) | TxError::Refused(_) => None,
    }
}

enum TxError {
    /// The record the act was authorized on could not be spent by it.
    Refused(DomainError),
    Repo(crate::infra::storage::RepoError),
    /// The event could not be enqueued. It rides the act's transaction, so
    /// this rolls the act back rather than committing a tombstone whose
    /// cache-buster never reached a consumer.
    Events(crate::infra::events::EventsError),
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_tests;

impl From<toolkit_db::DbError> for TxError {
    fn from(error: toolkit_db::DbError) -> Self {
        Self::Repo(crate::infra::storage::RepoError::Db(error.to_string()))
    }
}
