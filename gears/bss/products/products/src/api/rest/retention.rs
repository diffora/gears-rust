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
use crate::domain::taxonomy::{NoPiiPolicyDetector, PiiDetector, content_pii_block};
use crate::infra::storage::repo::{self, RefusalSubject};

/// The `OpenAPI` tag both doors register under.
const TAG: &str = "BSS Products";

/// What an audit row of this feature names as its subject kind.
const SUBJECT_KIND: &str = "identity_ref";

/// The canonical-error identity of the erasure surface's refusals.
#[resource_error(gts_id!("cf.bss.products.erasure.v1~"))]
struct ErasureResource;

/// The canonical-error identity of the compliance surface's refusals.
#[resource_error(gts_id!("cf.bss.products.compliance.v1~"))]
struct ComplianceResource;

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

/// The export's one query parameter.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportQuery {
    /// The principal to export.
    principal_ref: String,
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
                    subject_kind: SUBJECT_KIND,
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

/// Which of this feature's two grants a door spends.
#[derive(Clone, Copy)]
enum Gate {
    /// `erasure × execute`.
    Erasure,
    /// `compliance × export`.
    Compliance,
}

impl Gate {
    fn resource(self) -> authz_resolver_sdk::ResourceType {
        match self {
            Self::Erasure => crate::authz::resource_types::ERASURE,
            Self::Compliance => crate::authz::resource_types::COMPLIANCE,
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::Erasure => crate::authz::actions::EXECUTE,
            Self::Compliance => crate::authz::actions::EXPORT,
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
        }
    }
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
    // other reason-bearing door does. The registered detector admits
    // everything and says so; the day `dod-pii-detector` lands a real one this
    // is the line that changes.
    let detector: Arc<dyn PiiDetector + Send + Sync> = Arc::new(NoPiiPolicyDetector);
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
                            correlation_id: None,
                            written_at: now,
                        },
                        retired,
                    )
                    .await
                    .map_err(TxError::Repo)?;
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
    }
}

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

    if principal_ref.is_empty() {
        let mut report = crate::domain::validation::ValidationReport::new();
        report.violate(
            "VALIDATION",
            "principalRef",
            "principalRef must not be blank",
        );
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
                            reason: None,
                            correlation_id: None,
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
        .map_err(|TxError::Repo(_)| {
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

/// The retry loop classifies `sea-orm`'s own error, which `RepoError::Driver`
/// carries directly.
fn contention_db_err(error: &TxError) -> Option<&sea_orm::DbErr> {
    match error {
        TxError::Repo(crate::infra::storage::RepoError::Driver { source, .. }) => Some(source),
        TxError::Repo(_) => None,
    }
}

enum TxError {
    Repo(crate::infra::storage::RepoError),
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_tests;

impl From<toolkit_db::DbError> for TxError {
    fn from(error: toolkit_db::DbError) -> Self {
        Self::Repo(crate::infra::storage::RepoError::Db(error.to_string()))
    }
}
