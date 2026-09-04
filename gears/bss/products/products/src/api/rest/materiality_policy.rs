//! `05-governance`'s materiality-policy door — the tenant's `N`, its trigger
//! and its extra field set, mutated as a `GovernedLiveOp` (**P-D-112** arm 1;
//! `design/05-governance.md` C4, `inst-mt-policy-material`).
//!
//! # Why the policy has a door of its own rather than a configuration key
//!
//! C4: *"the policy's own mutation is material — the two-person rule's
//! foundation must not be single-person-editable"*. Configuration is
//! single-person-editable by construction, so a config home would put the
//! two-person rule's own foundation outside the two-person rule.
//! `inst-mt-once` compounds it: the evaluation runs *"against the policy in
//! force at the submission instant"*, and a process's configuration has no
//! historical value to re-read. Two independent clauses, one answer.
//!
//! # One verb, and the missing one is a grant rather than an oversight
//!
//! `PUT` replaces the tenant's single policy row. There is **no `GET`**, and
//! that is forced rather than chosen: `design/05` §3.2 mints
//! `materiality_policy × write` and **no read action**, so a read door would
//! have to spend a grant nobody declared. Minting one is a change to the
//! catalog, which this slice may not make on its own — see
//! `features/governance.md` §7 row 24, which asks precisely who mints a pair
//! when the owning slice names none.
//!
//! # The mutation goes through the gate, and today the gate lets it through
//!
//! `inst-mt-policy-material` makes the policy a `GovernedLiveOp` subject, so
//! this door submits to the governance gate exactly as `02`'s taxonomy ops
//! do. The registered host is still `NoMaterialityPolicyGate`, which
//! **authorizes and says so** rather than pretending to judge — the posture
//! every door in this gear takes. The day `05` registers a store-backed host
//! this door becomes ceremony-gated like the rest, which is the point: the
//! policy's own mutation must not be the one act that escapes the rule it
//! configures.
//!
//! The revision is `InternalRevision::new(0)`, for `02`'s stated reason: a
//! live op has no entity head and so no `If-Match` to pin, which is the
//! poverty §7 row 14 records about the entity-shaped columns on a non-entity
//! subject.
//!
//! # The `subject_kind` a policy mutation records
//!
//! **P-D-120** row 38 makes `materiality_policy` a subject kind of its own.
//! The gate subject this door builds is still `governed_live_op` — which is
//! what `inst-mt-policy-material` names it — because the record whose column
//! carries the sixth kind is written by the **submit door**, and a gate
//! question is not a record. The two are different objects and the row asked
//! about the second.
//!
//! # The provisioning clause, and why the marker is here now
//!
//! An earlier revision of this doc carried no `@cpt-dod` marker, on the
//! grounds that `dod-materiality-policy` obliges the value to take *"its
//! initial value from tenant provisioning"* and this gear has no tenant
//! registry to provision from. **P-D-135** (2026-09-04) reads that clause as
//! P-D-112's default: P-D-104 withdrew the registry the provisioning would
//! have run from, and a tenant's initial `N` **is** the default until the
//! tenant configures one. The absent row resolving to the default is that
//! provisioning rather than a substitute for it, so the clause is met and the
//! `DoD` ticks.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-materiality-policy:p1

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
use crate::domain::governance::{
    GateMode, GateSubject, GateVerdict, GovernanceGate, NoMaterialityPolicyGate,
};
use crate::domain::materiality::MaterialityPolicy;
use crate::domain::taxonomy::{NoPiiPolicyDetector, PiiDetector, content_pii_block};
use crate::infra::storage::repo::{self, RefusalSubject};

/// The `OpenAPI` tag this door registers under.
const TAG: &str = "BSS Products";

/// What an audit row of this door names as its subject kind.
const SUBJECT_KIND: &str = "materiality_policy";

/// The `GovernedLiveOp` target this mutation names.
///
/// A constant rather than a per-request string: the subject **is** the
/// tenant's one policy, so there is nothing to identify beyond the tenant the
/// gate already scopes by, and a caller-supplied target would be an operand
/// nobody validates.
const LIVE_OP_TARGET: &str = "materiality_policy";

/// The canonical-error identity of this door's refusals.
#[resource_error(gts_id!("cf.bss.products.materiality_policy.v1~"))]
struct MaterialityPolicyResource;

/// The policy a tenant is setting.
#[toolkit_macros::api_dto(request)]
pub struct MaterialityPolicyRequest {
    /// The tenant's own additions to the bucket registry. A column named here
    /// is material even where the registry does not tag it; the registry still
    /// runs first, so this set may raise a verdict and never skip a refusal.
    pub field_set: Vec<String>,
    /// The affected-entity count at or above which a batch act is material.
    pub affected_entity_trigger: u32,
    /// `N`, the approver count. **Zero is admitted** (**P-D-11**) and means
    /// this tenant publishes approver-less by policy — the record says so, and
    /// the descriptor carries `quorumReduced`.
    pub approver_count: u32,
    /// Why, for the audit row. Operator free text, and inside the content-PII
    /// write block.
    pub reason: String,
}

/// What the door answers on success.
#[toolkit_macros::api_dto(response)]
pub struct MaterialityPolicyReceipt {
    /// `N` as now in force.
    pub approver_count: u32,
    /// The trigger as now in force.
    pub affected_entity_trigger: u32,
    /// The field set as now in force.
    pub field_set: Vec<String>,
    /// When the mutation committed.
    pub updated_at: DateTime<Utc>,
}

/// Register the door.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = OperationBuilder::put("/bss-products/v1/materiality-policy")
        .operation_id("bss_products.set_materiality_policy")
        .summary("Set this tenant's materiality policy")
        .description(
            "Replaces the tenant's materiality policy - the extra field set, the affected-entity \
             trigger and the approver count `N` - as a governed live operation. The policy's own \
             mutation is material by C4, so that the holder of a configuration grant cannot \
             weaken the threshold that governs them; it spends `materiality_policy x write`, its \
             own pair and never a config administrator's general grant. `N` admits 0, which \
             means the tenant publishes approver-less by policy and every record says so. A \
             tenant that has never called this door has no row and resolves to the default - \
             `N = 2`, trigger 10, no extra fields - so the gear is enforceable at launch.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<MaterialityPolicyRequest>(openapi, "The policy to put in force.")
        .handler(set_materiality_policy)
        .json_response_with_schema::<MaterialityPolicyReceipt>(
            openapi,
            StatusCode::OK,
            "The policy now in force, and the instant it committed.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router.layer(Extension(state))
}

/// Authorize `materiality_policy × write`, auditing a denial as a refusal.
async fn policy_scope(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
) -> Result<AccessScope, CanonicalError> {
    match crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::MATERIALITY_POLICY,
        crate::authz::actions::WRITE,
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
                RefusalSubject::Attempted(LIVE_OP_TARGET.to_owned()),
                MaterialityPolicyResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await)
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            Err(crate::api::rest::authz_error_to_canonical(err, |reason| {
                MaterialityPolicyResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }))
        }
    }
}

/// Submit the mutation to the governance gate as a `GovernedLiveOp`.
///
/// The shape is `02`'s `submit_to_gate`, and deliberately so: a second way of
/// asking the same seam the same question is how two doors come to disagree
/// about what a live op is.
fn submit_to_gate(tenant_id: Uuid) -> Result<(), DomainError> {
    let gate: Arc<dyn GovernanceGate + Send + Sync> = Arc::new(NoMaterialityPolicyGate);
    match gate.evaluate(
        GateSubject::governed_live_op(tenant_id, LIVE_OP_TARGET),
        crate::domain::concurrency::InternalRevision::new(0),
        GateMode::Gate,
    )? {
        GateVerdict::Authorized(_) => Ok(()),
        GateVerdict::Refused { reason } => Err(DomainError::ApprovalRequired(reason)),
    }
}

/// Refuse, audit the refusal, and answer.
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

/// `PUT /bss-products/v1/materiality-policy`.
async fn set_materiality_policy(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<MaterialityPolicyRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = policy_scope(&state, &enforcer, &ctx, tenant_id, actor_ref).await?;

    let reason = body.reason.trim().to_owned();
    let fields: Vec<String> = body.field_set.iter().map(|f| f.trim().to_owned()).collect();
    let mut report = crate::domain::validation::ValidationReport::new();
    if reason.is_empty() {
        report.violate("VALIDATION", "reason", "reason must not be blank");
    }
    if fields.iter().any(String::is_empty) {
        report.violate(
            "VALIDATION",
            "fieldSet",
            "a field-set member must not be blank",
        );
    }
    if !report.is_empty() {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            DomainError::Validation(report),
        )
        .await);
    }

    // Operator free text, so it rides the same write block every other
    // reason-bearing door does. The registered detector admits everything and
    // says so.
    let detector: Arc<dyn PiiDetector + Send + Sync> = Arc::new(NoPiiPolicyDetector);
    if let Err(blocked) = content_pii_block(detector.as_ref(), "reason", &reason) {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            DomainError::ContentPiiBlocked(blocked.into_detail()),
        )
        .await);
    }

    // C4's ceremony. Today's host authorizes and records why; the day a
    // store-backed host is registered, this is where the policy's own
    // mutation starts needing the quorum it configures.
    if let Err(refusal) = submit_to_gate(tenant_id) {
        return Err(refuse(&state, &scope, tenant_id, actor_ref, refusal).await);
    }

    let policy = MaterialityPolicy::new(fields, body.affected_entity_trigger, body.approver_count);
    // The write and its audit row commit together: a governed mutation whose
    // record did not commit is a change nobody can review, which is the whole
    // reason C4 governs this object.
    let audit_id = Uuid::now_v7();
    let scope_tx = scope.clone();
    let policy_tx = policy.clone();
    let reason_tx = reason.clone();
    state
        .db
        .db()
        .transaction_with_retry::<(), TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let policy = policy_tx.clone();
                let reason = reason_tx.clone();
                Box::pin(async move {
                    repo::write_materiality_policy(tx, &scope, tenant_id, &policy, actor_ref, now)
                        .await
                        .map_err(TxError::Repo)?;
                    repo::write_eventless_act_audit(
                        tx,
                        &scope,
                        repo::AuditCommon {
                            audit_id,
                            tenant_id,
                            actor_ref,
                            action: "materiality_policy.write".to_owned(),
                            subject_kind: SUBJECT_KIND.to_owned(),
                            reason: Some(reason),
                            correlation_id: None,
                            written_at: now,
                        },
                        // The subject **is** the tenant: this table's key is
                        // `(tenant_id)` alone, so there is no other id to
                        // name, and minting one would put a value in the
                        // column that identifies nothing.
                        tenant_id,
                        None,
                    )
                    .await
                    .map_err(TxError::Repo)?;
                    Ok(())
                })
            },
        )
        .await
        .map_err(|TxError::Repo(e)| repo_error_to_canonical(&e))?;

    Ok((
        StatusCode::OK,
        Json(MaterialityPolicyReceipt {
            approver_count: policy.approver_count(),
            affected_entity_trigger: policy.affected_entity_trigger(),
            field_set: policy.field_set().to_vec(),
            updated_at: now,
        }),
    )
        .into_response())
}

/// This door's transaction error.
enum TxError {
    Repo(crate::infra::storage::RepoError),
}

impl From<toolkit_db::DbError> for TxError {
    fn from(error: toolkit_db::DbError) -> Self {
        Self::Repo(crate::infra::storage::RepoError::Db(error.to_string()))
    }
}

/// The retry loop classifies `sea-orm`'s own error, which `RepoError::Driver`
/// carries directly.
fn contention_db_err(error: &TxError) -> Option<&sea_orm::DbErr> {
    match error {
        TxError::Repo(crate::infra::storage::RepoError::Driver { source, .. }) => Some(source),
        TxError::Repo(_) => None,
    }
}

#[cfg(test)]
#[path = "materiality_policy_tests.rs"]
mod materiality_policy_tests;
