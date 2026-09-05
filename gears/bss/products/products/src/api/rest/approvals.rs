//! The ceremony's three doors — submit, decide, and break-glass elevation
//! (`design/05-governance.md` §2 and §3.2; **P-D-120** row 12).
//!
//! # Why one module for three routes
//!
//! P-D-120 measured the hole: `approval × submit`, `approval × decide` and
//! `breakglass × elevate` were all **minted** and **no approval door
//! existed** — the only route the feature declared was the inbox `GET`. So
//! every one of `05`'s door `DoD`s was blocked on routes nobody had built,
//! and the grants were never the open half.
//!
//! The three land together because they are one ceremony read three ways: a
//! record is opened, decided, and — on the platform side — bypassed under a
//! recorded elevation. Splitting them across modules would put the record's
//! lifecycle in three files with three copies of its subject vocabulary.
//!
//! # The shapes are the corpus's, not invented here
//!
//! `POST /bss-products/v1/approvals`,
//! `POST /bss-products/v1/approvals/{approvalId}/decisions` and
//! `POST /bss-products/v1/breakglass-sessions` — the
//! collection-POST-plus-act-subresource shape **P-D-67**, **P-D-87**,
//! **P-D-90** and **P-D-106** set across this gear.
//!
//! # What the submit door reads rather than accepts
//!
//! For an **entity** subject the snapshot is **not** a request field. The
//! door reads the head and renders it through the very function the publish
//! door freezes with (`products::product_content`,
//! `skus::sku_version_content`), so the bytes an approver signs are the bytes
//! a later publish produces. A caller-supplied snapshot would let a
//! submission describe content the publish never writes, which is
//! `dod-stored-snapshot`'s defect arriving through the request instead of
//! through the diff.
//!
//! For the **non-entity** kinds it is the caller's, and that is **P-D-120**
//! row 14's own answer: `content_snapshot` *is* the op payload,
//! `internal_revision` is the op's own pin or `0` where the subject has no
//! counter, and `diff_basis` is `NULL` there being no published version to
//! diff against.
//!
//! # Roles are claims, and their absence is reported as absence
//!
//! **P-D-134** row 25: a principal's role is on no surface today; when the
//! platform's PDP encodes it in `token_scopes`, `APPROVER_ROLE_REQUIRED` and
//! P-D-131's per-set predicate read it from there. Until then a caller holds
//! **no role claim**, and [`roles_from_claims`] answers the empty set —
//! never a synthesised `CatalogAdmin`. A door that defaulted the role would
//! close a material ceremony on two principals holding neither C1 role,
//! which is exactly what `domain::approval::BaseRoleSet`'s own doc records as
//! the defect it was built to remove.
//!
//! # No broker event on submit, and that absence is declared
//!
//! `dod-governance-events` names the events this feature emits, and a
//! **submission is not one of them**: `design/05` §2 gives `ApprovalDecided`
//! on either verdict and `BreakGlassElevated` on an open, and names no
//! submission or supersession event. That is an explicit no-event
//! declaration rather than an omission — a consumer learns of a pending
//! record from the inbox queue, which is a read surface, and a supersession
//! is a consequence of a write the Foundation already announces.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-decide:p1
//! @cpt-dod:cpt-cf-bss-products-dod-breakglass-open:p1
//! @cpt-dod:cpt-cf-bss-products-dod-stored-snapshot:p1
//! @cpt-dod:cpt-cf-bss-products-dod-pii-on-reasons:p1
//! @cpt-cf-bss-products-flow-submit
//! @cpt-cf-bss-products-flow-decide
//! @cpt-cf-bss-products-flow-breakglass

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, TimeDelta, Utc};
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::{ApiState, repo_error_to_canonical, require_authenticated};
use crate::domain::approval::{ApprovalState, ApproverRole, diff_basis_for};
use crate::domain::canonical;
use crate::domain::concurrency::InternalRevision;
use crate::domain::error::DomainError;
use crate::domain::governance::{ApprovalId, EntityRef, GateSubject, SubjectKind, SubjectPin};
use crate::domain::materiality::{
    EnumeratedOp, LiveOpEdit, MaterialAct, MaterialLiveOp, MaterialityEvaluator,
};
use crate::domain::taxonomy::{PiiDetector, content_pii_block};
use crate::domain::validation::ValidationReport;
use crate::infra::events::{self, GovernanceEventBody};
use crate::infra::storage::repo::{
    self, ApprovalPath, ApprovalStoreError, DecisionVerdict, NewApproval, NewDecision,
    NewElevation, RefusalSubject, VersionedEntityKind,
};

/// The `OpenAPI` tag this door registers under.
const TAG: &str = "BSS Products";

/// What an approval door's audit row names as its subject kind.
const AUDIT_SUBJECT_APPROVAL: &str = "approval";

/// What the elevation door's audit row names as its subject kind.
const AUDIT_SUBJECT_BREAKGLASS: &str = "breakglass";

/// The canonical-error identity of the two approval routes' refusals.
#[resource_error(gts_id!("cf.bss.products.approval.v1~"))]
struct ApprovalResource;

/// The canonical-error identity of the elevation route's refusals.
#[resource_error(gts_id!("cf.bss.products.breakglass.v1~"))]
struct BreakglassResource;

/// Which of the two resources a refusal is attributed to.
///
/// The three routes share one [`authorize`] helper — one authorization shape,
/// not two — but a denial must still carry the resource the caller was
/// actually refused on: a `breakglass x elevate` denial reported against
/// `cf.bss.products.approval.v1~` would send an operator to the wrong grant.
#[derive(Copy, Clone)]
enum Door {
    /// The submit and decide routes.
    Approval,
    /// The elevation route.
    Breakglass,
}

impl Door {
    /// This door's own 403.
    fn permission_denied(self, reason: String) -> CanonicalError {
        match self {
            Self::Approval => ApprovalResource::permission_denied()
                .with_reason(reason)
                .create(),
            Self::Breakglass => BreakglassResource::permission_denied()
                .with_reason(reason)
                .create(),
        }
    }
}

// ---------------------------------------------------------------------------
// Wire shapes
// ---------------------------------------------------------------------------

/// A change submitted for approval.
#[toolkit_macros::api_dto(request)]
pub struct SubmitApprovalRequest {
    /// One of `products_approval.subject_kind`'s six tokens.
    pub subject_kind: String,
    /// The subject's own reference, as the store stores it: `product/{id}` or
    /// `sku/{id}` for an entity, and the kind's own identifier otherwise.
    pub subject_ref: String,
    /// Whether the change touches a finance-material field. The caller's,
    /// because `inst-gv-finance-predicate` names three columns the bucket
    /// registry does not carry.
    pub finance_material: bool,
    /// The op payload, for the **non-entity** kinds only. An entity
    /// submission's snapshot is read from the head and this field is refused
    /// on it, so a caller cannot describe content the publish will not write.
    pub content_snapshot: Option<String>,
    /// The op's own pin, for the non-entity kinds only. Omitted means `0`,
    /// which P-D-120 row 14 gives as *"no pin"* — the column exists to detect
    /// a stale submission, and an op with no counter cannot go stale.
    pub internal_revision: Option<i64>,
    /// The author's own override acknowledgment, admitted **only** at
    /// effective quorum zero (**P-D-68** arm 1).
    pub author_override_ack: Option<String>,
}

/// What a submission answers.
#[toolkit_macros::api_dto(response)]
pub struct SubmitApprovalReceipt {
    /// The record's id, which the decide route addresses.
    pub approval_id: Uuid,
    /// The state the record was **born** in — `satisfied` at `required = 0`
    /// (**P-D-119** row 31), `pending` above it.
    pub state: String,
    /// The **effective** count, never the raw configured `N`.
    pub required: u32,
    /// The raw `N` in force at this instant.
    pub configured_quorum: u32,
    /// Whether the finance lens is demanded of the approver set.
    pub finance_required: bool,
    /// Whether the effective count sits below the retained default of two,
    /// whatever the cause (**P-D-120** rows 15 and 39).
    pub quorum_reduced: bool,
}

/// `GET /approvals?state=pending`'s query. Only `pending` is admitted: the
/// inbox is the open queue, and finalized records are read by id.
#[derive(Debug, Default, serde::Deserialize)]
pub struct ApprovalsQuery {
    pub state: Option<String>,
}

/// The common inbox envelope (`inst-gv-queue`): one card per pending record,
/// oldest first. Merge-compatibility with pricing's queue is
/// `12-consumer-contracts`' half to assert.
#[toolkit_macros::api_dto(response)]
pub struct ApprovalInbox {
    pub items: Vec<ApprovalInboxCard>,
}

/// One pending record as the inbox shows it.
#[toolkit_macros::api_dto(response)]
pub struct ApprovalInboxCard {
    pub approval_id: Uuid,
    pub subject_ref: String,
    pub subject_kind: String,
    pub state: String,
    /// Pseudonymous.
    pub submitter: Uuid,
    pub submitted_at: DateTime<Utc>,
    pub quorum: ApprovalInboxQuorum,
    /// The per-kind diff payload: the stored snapshot and the published
    /// version it is diffed against (`None` for a first publish or a
    /// non-entity kind).
    pub content_snapshot: String,
    pub diff_basis: Option<i64>,
}

/// The card's quorum block. `required` is the record's **effective** count —
/// `N` for a material change, `min(N, 1)` for a non-material one — never the
/// raw `N`, so a card cannot show "2 required" for a record that closes on
/// one; `configured_quorum` carries the raw `N` when a surface needs it
/// (P-D-11). `predicate_unsatisfiable` stays visible where the finance lens
/// could not be demanded (P-D-120 row 14, `dod-quorum-evaluator`).
#[toolkit_macros::api_dto(response)]
pub struct ApprovalInboxQuorum {
    pub required: u32,
    /// Distinct approving principals so far.
    pub satisfied: u32,
    pub finance_required: bool,
    pub predicate_unsatisfiable: Option<String>,
    pub quorum_reduced: bool,
    pub configured_quorum: u32,
}

/// One principal's verdict.
#[toolkit_macros::api_dto(request)]
pub struct DecisionRequest {
    /// `approved` or `rejected`.
    pub verdict: String,
    /// Operator free text. **Mandatory on a rejection** and inside the
    /// content-PII write block either way.
    pub reason: Option<String>,
    /// The lint findings this approver acknowledged, by name.
    pub override_acknowledgments: Option<String>,
}

/// What a decision answers.
#[toolkit_macros::api_dto(response)]
pub struct DecisionReceipt {
    /// The record's state **after** the verdict.
    pub state: String,
    /// How many distinct eligible principals have approved.
    pub counted: u32,
    /// How many the stored descriptor requires.
    pub required: u32,
}

/// An elevation being opened.
#[toolkit_macros::api_dto(request)]
pub struct BreakglassRequest {
    /// The tenant whose data the session reaches.
    pub target_tenant_id: Uuid,
    /// Mandatory, and inside the content-PII write block.
    pub reason: String,
    /// The platform-side handle for the two-person ceremony. Present with
    /// both approvers, or absent for the post-hoc path.
    pub two_person_approval_ref: Option<Uuid>,
    /// The first approving platform principal.
    pub approver_a: Option<Uuid>,
    /// The second, distinct from the first.
    pub approver_b: Option<Uuid>,
}

/// What an elevation answers.
#[toolkit_macros::api_dto(response)]
pub struct BreakglassReceipt {
    /// The session's own id, which an elevated call names in its header.
    pub session_id: Uuid,
    /// The window's start, inclusive.
    pub valid_from: DateTime<Utc>,
    /// The window's end, **exclusive**.
    pub valid_until: DateTime<Utc>,
    /// `two_person` or `post_hoc`.
    pub path: String,
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the three doors.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = Router::new();
    let router = OperationBuilder::post("/bss-products/v1/approvals")
        .operation_id("bss_products.submit_approval")
        .summary("Submit a change for approval")
        .description(
            "Opens an `ApprovalRecord` for one subject, superseding whatever open record that \
             subject held. The materiality evaluator runs **once**, here, against the policy in \
             force at this instant, and the quorum descriptor it produces is stored rather than \
             re-derived: `required = N` for a material change and `min(N, 1)` for a non-material \
             one, alongside the raw configured `N`. For an entity subject the submitted content \
             is read from the head and rendered the way the publish door freezes it, never taken \
             from the request. A tenant at `N = 0` gets a record born `satisfied`, which is what \
             publishing approver-less by policy means.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<SubmitApprovalRequest>(openapi, "The change being submitted.")
        .handler(submit_approval)
        .json_response_with_schema::<SubmitApprovalReceipt>(
            openapi,
            StatusCode::CREATED,
            "The record's id, the state it was born in, and its stored quorum descriptor.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/approvals/{approvalId}/decisions")
        .operation_id("bss_products.decide_approval")
        .summary("Approve or reject an approval record")
        .description(
            "Appends one decision row carrying the approver principal, the verdict, the instant \
             and a mandatory reason on reject, then re-evaluates the stored descriptor. A \
             rejection finalizes the record and leaves the subject exactly as it was - there is \
             no `published -> draft` edge in this gear. An approval that meets the descriptor by \
             distinct eligible principals flips the record `satisfied` in the same transaction. \
             `ApprovalDecided` is emitted on either verdict.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<DecisionRequest>(openapi, "The verdict being cast.")
        .handler(decide_approval)
        .json_response_with_schema::<DecisionReceipt>(
            openapi,
            StatusCode::OK,
            "The record's state after the verdict, and where the count stands.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/breakglass-sessions")
        .operation_id("bss_products.open_breakglass_session")
        .summary("Open a break-glass elevation session")
        .description(
            "Opens a time-boxed, read-only elevation over one named target tenant, carrying a \
             mandatory reason and either two distinct platform approvers or a post-hoc review \
             obligation. The window comes from `breakglass_window_hours` and is never renewed - a \
             new session is a new ceremony. v1 is read and audit-export only: every write under \
             an elevation is refused `BREAKGLASS_WRITE_FORBIDDEN`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<BreakglassRequest>(openapi, "The elevation being opened.")
        .handler(open_breakglass)
        .json_response_with_schema::<BreakglassReceipt>(
            openapi,
            StatusCode::CREATED,
            "The session's id and the window it is valid in.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::get("/bss-products/v1/approvals")
        .operation_id("bss_products.list_pending_approvals")
        .summary("The pending-approvals inbox")
        .description(
            "`state=pending` only. Each card carries the record's **effective** count as \
             `required` and the raw configured `N` as `configured_quorum`, so a card cannot \
             show \"2 required\" for a record that closes on one (`inst-gv-queue`, P-D-11); \
             `satisfied` counts distinct approving principals; `predicate_unsatisfiable` stays \
             visible where the finance lens could not be demanded. Tenant-scoped under \
             `approval x read`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(list_pending_approvals)
        .json_response_with_schema::<ApprovalInbox>(
            openapi,
            StatusCode::OK,
            "The tenant's pending records, oldest first.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .register(router, openapi);

    router.layer(Extension(state))
}

/// `GET /bss-products/v1/approvals?state=pending` (`dod-inbox-envelope`).
///
/// @cpt-dod:cpt-cf-bss-products-dod-inbox-envelope:p1
async fn list_pending_approvals(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Query(query): axum::extract::Query<ApprovalsQuery>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = authorize(
        &state,
        &enforcer,
        &ctx,
        AuthzAsk {
            tenant_id,
            actor_ref,
            resource: &crate::authz::resource_types::APPROVAL,
            action: crate::authz::actions::READ,
            door: Door::Approval,
            audit_subject_kind: AUDIT_SUBJECT_APPROVAL,
            attempted: "pending",
        },
    )
    .await?;
    if query.state.as_deref() != Some("pending") {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            AUDIT_SUBJECT_APPROVAL,
            "pending".to_owned(),
            violation(
                "state",
                "the inbox lists `state=pending` only; a finalized record is read by its id",
            ),
        )
        .await);
    }
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let pending = repo::pending_approvals_with_progress(&conn, &scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let mut items = Vec::with_capacity(pending.len());
    for entry in pending {
        let record = entry.record;
        let descriptor = crate::domain::approval::descriptor_from_stored(&record.quorum_descriptor)
            .map_err(|e| {
                repo_error_to_canonical(&crate::infra::storage::RepoError::CorruptRow(format!(
                    "approval {} quorum_descriptor: {e}",
                    record.approval_id
                )))
            })?;
        items.push(ApprovalInboxCard {
            approval_id: record.approval_id,
            subject_ref: record.subject_ref,
            subject_kind: record.subject_kind,
            state: record.state,
            submitter: record.submitter,
            submitted_at: record.submitted_at,
            quorum: ApprovalInboxQuorum {
                required: descriptor.required(),
                satisfied: entry.satisfied,
                finance_required: descriptor.finance_required(),
                predicate_unsatisfiable: descriptor
                    .predicate_unsatisfiable()
                    .map(|predicate| predicate.as_str().to_owned()),
                quorum_reduced: descriptor.quorum_reduced(),
                configured_quorum: descriptor.configured_quorum(),
            },
            content_snapshot: record.content_snapshot,
            diff_basis: record.diff_basis,
        });
    }
    Ok((StatusCode::OK, Json(ApprovalInbox { items })).into_response())
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The roles a caller's `token_scopes` claim (**P-D-134** row 25).
///
/// **Absent is empty, never permissive.** No surface carries a role today, so
/// every caller resolves to `[]` until the platform's PDP encodes one — and
/// `[]` is what `BaseRoleSet::CatalogAdminOrFinanceReviewer` refuses. That is
/// the honest answer: the check says *"no role claim"*, never *"role held"*.
///
/// The wildcard `*` is a **permission** grant and not a role. Reading it as
/// one would give every test context both C1 roles and make the refusal
/// unreachable, which is the fail-open this function exists to avoid.
fn roles_from_claims(scopes: &[String]) -> Vec<ApproverRole> {
    let mut roles = Vec::new();
    for scope in scopes {
        match scope.as_str() {
            s if s == ApproverRole::CatalogAdmin.as_str() => {
                roles.push(ApproverRole::CatalogAdmin);
            }
            s if s == ApproverRole::FinanceReviewer.as_str() => {
                roles.push(ApproverRole::FinanceReviewer);
            }
            _ => {}
        }
    }
    roles.sort_unstable();
    roles.dedup();
    roles
}

/// What a door is asking the policy point, in one value.
///
/// A struct rather than five more arguments: `tenant_id` and `actor_ref` are
/// both `Uuid` and both would compile transposed, and `action`,
/// `audit_subject_kind` and `attempted` are three strings a call site could
/// order wrongly without the compiler noticing.
struct AuthzAsk<'a> {
    /// The tenant the grant is asked about.
    tenant_id: Uuid,
    /// The pseudonymous actor a denial's audit row attributes to.
    actor_ref: Uuid,
    /// The resource type, from the authz catalog.
    resource: &'a authz_resolver_sdk::pep::ResourceType,
    /// The action spent.
    action: &'static str,
    /// Which resource a denial is reported against.
    door: Door,
    /// The `subject_kind` the refusal's audit row names.
    audit_subject_kind: &'static str,
    /// What the caller attempted, for the audit row's subject.
    attempted: &'a str,
}

/// Authorize one `(resource, action)` pair, auditing a denial as a refusal.
async fn authorize(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    ask: AuthzAsk<'_>,
) -> Result<AccessScope, CanonicalError> {
    let AuthzAsk {
        tenant_id,
        actor_ref,
        resource,
        action,
        door,
        audit_subject_kind,
        attempted,
    } = ask;
    match crate::authz::access_scope(enforcer, ctx, resource, action, Some(tenant_id), None, true)
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
                    subject_kind: audit_subject_kind,
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(attempted.to_owned()),
                door.permission_denied(reason),
            )
            .await)
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            Err(crate::api::rest::authz_error_to_canonical(err, |reason| {
                door.permission_denied(reason)
            }))
        }
    }
}

/// Refuse, audit the refusal, and answer.
async fn refuse(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    subject_kind: &'static str,
    attempted: String,
    refusal: DomainError,
) -> CanonicalError {
    let code = refusal.code();
    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind,
            error_code: code,
        },
        RefusalSubject::Attempted(attempted),
        CanonicalError::from(refusal),
    )
    .await
}

/// One violation, as a `DomainError` the ladder renders by its own code.
fn violation(field: &'static str, detail: impl Into<String>) -> DomainError {
    let mut report = ValidationReport::new();
    report.violate("VALIDATION", field, detail);
    DomainError::Validation(report)
}

/// Run `02`'s write-block hook over one operator free-text reason.
///
/// `dod-pii-on-reasons`' two stored reasons — the decision's and the
/// elevation's — go through here, and **only** those two: **P-D-120** row 35
/// narrowed the `DoD` off the submission reason, `products_approval` having
/// no `reason` column and a column for text nobody writes being the wrong
/// fix.
///
/// The detector is **`10-retention-erasure`'s**, over the acting tenant's
/// Legal-signed-off allow-list (`dod-pii-detector`), and it is the caller's
/// to build: the read that builds it is asynchronous and
/// `PiiDetector::inspect` deliberately is not, so a detector constructed here
/// would make a synchronous rule reach a store.
///
/// It was `NoPiiPolicyDetector` when this door landed — the seventh
/// construction site of a host slice 10 had already replaced at six — and the
/// census that should have caught it named its files by hand. Both are fixed;
/// the census now discovers its own population.
fn pii_block(detector: &(dyn PiiDetector + Send + Sync), reason: &str) -> Result<(), DomainError> {
    content_pii_block(detector, "reason", reason)
        .map_err(|blocked| DomainError::ContentPiiBlocked(blocked.into_detail()))
}

/// The subject's override conditions at submission (`dod-override-ceremony`'s
/// operand, P-D-148): the dry-run lint's finding codes for the head, which is
/// exactly what the `validate` door answers (P-D-125 row 14) — `01`'s
/// pipeline for a Product, `03`'s and `07`'s rechecks folded in for a SKU,
/// plus the uncomposed-bundle condition `design/05` names.
async fn lint_conditions(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    entity: EntityRef,
) -> Result<Vec<String>, crate::infra::storage::RepoError> {
    let findings = match entity.entity_kind {
        bss_products_sdk::models::EntityKind::Sku => {
            crate::api::rest::skus::lint_sku_publish(
                runner,
                scope,
                entity.tenant_id,
                entity.entity_id,
            )
            .await?
        }
        bss_products_sdk::models::EntityKind::Product => {
            crate::api::rest::products::lint_product_publish(
                runner,
                scope,
                entity.tenant_id,
                entity.entity_id,
            )
            .await?
        }
    };
    let mut codes: Vec<String> = findings
        .into_iter()
        .map(|finding| finding.code)
        // Only the overridable class is a condition; a hard refusal stays a
        // report line the ceremony cannot change (P-D-148).
        .filter(|code| crate::domain::approval::OVERRIDE_CONDITION_CODES.contains(&code.as_str()))
        .collect();
    codes.sort();
    codes.dedup();
    Ok(codes)
}

/// The six subject kinds, parsed from the wire token the store stores.
fn parse_subject_kind(token: &str) -> Option<SubjectKind> {
    SubjectKind::ALL.into_iter().find(|k| k.as_str() == token)
}

/// The entity a `product/{id}` or `sku/{id}` reference names.
fn parse_entity_ref(tenant_id: Uuid, reference: &str) -> Option<EntityRef> {
    let (kind, id) = reference.split_once('/')?;
    let entity_kind = match kind {
        "product" => bss_products_sdk::models::EntityKind::Product,
        "sku" => bss_products_sdk::models::EntityKind::Sku,
        _ => return None,
    };
    Some(EntityRef {
        tenant_id,
        entity_kind,
        entity_id: Uuid::parse_str(id).ok()?,
    })
}

/// This module's transaction error.
enum TxError {
    Store(ApprovalStoreError),
    Repo(crate::infra::storage::RepoError),
    Events(events::EventsError),
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
        TxError::Repo(crate::infra::storage::RepoError::Driver { source, .. })
        | TxError::Store(ApprovalStoreError::Repo(crate::infra::storage::RepoError::Driver {
            source,
            ..
        })) => Some(source),
        TxError::Repo(_) | TxError::Store(_) | TxError::Events(_) => None,
    }
}

// ---------------------------------------------------------------------------
// POST /bss-products/v1/approvals
// ---------------------------------------------------------------------------

/// What the door resolved about the subject before the store ran.
struct Submission {
    subject: GateSubject,
    internal_revision: i64,
    content_snapshot: String,
    diff_basis: Option<i64>,
    act: ActSpec,
    /// The lint findings the subject carries at submission, by code — the
    /// descriptor's sixth name (`dod-quorum-descriptor`) and what approvers
    /// acknowledge by name (`dod-override-ceremony`; P-D-148). Computed by
    /// the dry-run lint for an entity subject, empty otherwise.
    override_conditions: Vec<String>,
}

/// The act, in an **owned** shape.
///
/// `MaterialAct::EntityPublish` borrows its touched-column slice, and the
/// transaction closure is higher-ranked over the transaction's lifetime, so
/// nothing borrowed from outside it can be captured. Owning the names here
/// and re-borrowing them **inside** the closure is what keeps the act a
/// borrow rather than a leak — an earlier revision reached for
/// `Vec::leak`, which would have leaked one allocation per submission for the
/// life of the process.
enum ActSpec {
    /// An entity re-publish, with the columns that differ from the last
    /// frozen version.
    EntityPublish {
        kind: bss_products_sdk::models::EntityKind,
        touched: Vec<String>,
    },
    /// Every other kind: no borrow to re-tie.
    Owned(MaterialAct<'static>),
}

impl ActSpec {
    /// Re-borrow the owned names, so the act can be handed to the evaluator.
    ///
    /// Takes the scratch vector as an argument rather than building it, so
    /// the `&str`s outlive the returned value — a helper that owned the
    /// vector would hand back a reference into its own frame.
    fn as_act<'a>(&'a self, scratch: &'a mut Vec<&'a str>) -> MaterialAct<'a> {
        match self {
            Self::EntityPublish { kind, touched } => {
                scratch.extend(touched.iter().map(String::as_str));
                MaterialAct::EntityPublish {
                    kind: *kind,
                    touched: scratch.as_slice(),
                }
            }
            Self::Owned(act) => act.clone(),
        }
    }
}

/// Read an **entity** subject's head and render the snapshot the publish door
/// would freeze, with the touched-column set the evaluator judges.
///
/// # The touched set is measured against the last published version
///
/// `inst-mt-inputs` (a) judges an entity re-publish *"by the buckets of the
/// columns it touched"*, and the only place that set exists is the difference
/// between the head as it stands and the version last frozen. A first publish
/// has no basis, so **every** column counts as touched, which is right: it is
/// all new content and the slice makes a first publish material outright.
pub(crate) async fn resolve_entity_subject(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    entity: EntityRef,
) -> Result<Option<(String, i64, Option<i64>, Vec<String>)>, crate::infra::storage::RepoError> {
    use bss_products_sdk::models::EntityKind;

    let (content, revision, versioned) = match entity.entity_kind {
        EntityKind::Product => {
            let Some(head) =
                repo::find_product(runner, scope, entity.tenant_id, entity.entity_id).await?
            else {
                return Ok(None);
            };
            let collections = repo::frozen_collections(
                runner,
                scope,
                entity.tenant_id,
                "product",
                entity.entity_id,
            )
            .await?;
            (
                crate::api::rest::products::product_content(&head, &collections),
                head.internal_revision,
                VersionedEntityKind::Product,
            )
        }
        EntityKind::Sku => {
            let Some(head) =
                repo::find_sku(runner, scope, entity.tenant_id, entity.entity_id).await?
            else {
                return Ok(None);
            };
            let collections =
                repo::frozen_collections(runner, scope, entity.tenant_id, "sku", entity.entity_id)
                    .await?;
            (
                crate::api::rest::skus::sku_version_content(&head, &collections.values),
                head.internal_revision,
                VersionedEntityKind::Sku,
            )
        }
    };
    let snapshot = canonical::canonical_rendering(&content, canonical::Absence::Omit);
    let basis =
        repo::latest_entity_version(runner, scope, entity.tenant_id, versioned, entity.entity_id)
            .await?;

    let touched = match &basis {
        // Every key that differs from the frozen content, plus every key the
        // frozen content had and the head no longer does: a removal is a
        // touch, and a set built from the head alone would miss it.
        Some((_, frozen)) => match canonical::decode_rendering(frozen) {
            Ok(mut before) => {
                // The frozen row is §4.3's *complete* set — a name with no value
                // is a `null` member (`Absence::Null`) — while `snapshot` is the
                // parsed shape that omits it (`Absence::Omit`, P-D-34). The two
                // spell one fact two ways; dropping the null members makes the
                // diff like-for-like, so an unmetered SKU's `metering_unit`
                // is not "touched" on every re-publish (which had raised
                // `CorrectableTouch` against a column nobody moved).
                before.retain(|_, value| !value.is_null());
                let mut after = canonical::decode_rendering(&snapshot).unwrap_or_default();
                after.retain(|_, value| !value.is_null());
                let mut names: Vec<String> = after
                    .iter()
                    .filter(|(key, value)| before.get(*key) != Some(*value))
                    .map(|(key, _)| key.clone())
                    .collect();
                names.extend(
                    before
                        .keys()
                        .filter(|key| !after.contains_key(*key))
                        .cloned(),
                );
                names.sort_unstable();
                names.dedup();
                names
            }
            // A basis this gear wrote and cannot read back is not a
            // first publish, and treating it as one would judge the whole
            // content as new. The whole key set is the fail-closed answer:
            // it can only raise the verdict, never lower it.
            Err(_) => canonical::decode_rendering(&snapshot)
                .unwrap_or_default()
                .keys()
                .cloned()
                .collect(),
        },
        // A first publish: every column is new, so every column counts as
        // touched — **except bucket ii**. P-D-41 admits `metering_unit` and
        // `usage_type_ref` through the save door while `published_version = 0`,
        // so their first appearance is not a correction, and judging them as
        // one would refuse every usage SKU's first publish with
        // `CorrectableTouch` (P-D-142). After first publish the head guard
        // admits no bucket-ii save, so the exclusion has no second case.
        None => canonical::decode_rendering(&snapshot)
            .unwrap_or_default()
            .keys()
            .filter(|column| {
                crate::domain::bucket::classify(entity.entity_kind, column)
                    .ok()
                    .and_then(crate::domain::bucket::FieldClass::bucket)
                    != Some(crate::domain::bucket::FieldBucket::Correctable)
            })
            .cloned()
            .collect(),
    };
    Ok(Some((
        snapshot,
        revision,
        diff_basis_for(basis.map(|(version, _)| version)),
        touched,
    )))
}

/// The act a non-entity subject is judged as.
///
/// Each kind maps to the input `inst-mt-inputs` declares for it, and the map
/// is exhaustive so a seventh kind cannot arrive without saying which input
/// judges it.
const fn non_entity_act(kind: SubjectKind) -> MaterialAct<'static> {
    match kind {
        // `02`/`03`'s envelope. `Registered` rather than the display-label
        // exception: a rename arrives through the taxonomy doors' own
        // envelope, and this door has no field naming which edit a live op
        // carries — see the module doc's note on P-D-121 row 17.
        SubjectKind::GovernedLiveOp => MaterialAct::LiveOp {
            kind: MaterialLiveOp::TaxonomyOp,
            edit: LiveOpEdit::Registered,
        },
        // The policy's own mutation is material in either direction (C4).
        SubjectKind::MaterialityPolicy => MaterialAct::PolicyMutation,
        // `06`'s inbound composition clear and `07`'s immutable-field
        // correction both reach `published` on the head (P-D-14; the
        // correction door re-publishes through `01`'s publish door), so both
        // are judged as input (b)'s lifecycle transition. One arm rather than
        // two identical ones: they answer the same act because they *are* the
        // same act seen from two slices.
        //
        // `EntityPublish` joins them, and is **unreachable** here: the entity
        // arm resolves its own act from the head's touched columns before
        // this function runs. It shares the arm rather than carrying an
        // identical copy or a `panic!` — the act named is the true one for a
        // publish, so an unreachable arm that becomes reachable answers
        // correctly instead of aborting a request.
        SubjectKind::SystemSignal | SubjectKind::SkuCorrection | SubjectKind::EntityPublish => {
            MaterialAct::Enumerated(EnumeratedOp::LifecycleTransition(
                bss_products_sdk::models::LifecycleState::Published,
            ))
        }
        // `09`'s batch is judged against the configured trigger. The count is
        // the batch's own and the door does not have it, so the conservative
        // operand is `u32::MAX`: a batch is material unless a caller that
        // knows the count says otherwise, and no caller can lower it here.
        SubjectKind::BulkBatch => MaterialAct::BatchAct { affected: u32::MAX },
    }
}

/// `POST /bss-products/v1/approvals`.
#[allow(clippy::too_many_lines)] // the door's one sequence: authz, parse, resolve, gate, write
async fn submit_approval(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<SubmitApprovalRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = authorize(
        &state,
        &enforcer,
        &ctx,
        AuthzAsk {
            tenant_id,
            actor_ref,
            resource: &crate::authz::resource_types::APPROVAL,
            action: crate::authz::actions::SUBMIT,
            door: Door::Approval,
            audit_subject_kind: AUDIT_SUBJECT_APPROVAL,
            attempted: &body.subject_ref,
        },
    )
    .await?;

    let Some(kind) = parse_subject_kind(&body.subject_kind) else {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            AUDIT_SUBJECT_APPROVAL,
            body.subject_ref.clone(),
            violation(
                "subjectKind",
                format!(
                    "{} is outside chk_products_approval_subject_kind's roster",
                    body.subject_kind
                ),
            ),
        )
        .await);
    };
    // A `system_signal` record is written by the signal consumer at
    // submission, born `satisfied` with the signal as its principal (P-D-14,
    // P-D-120 row 14) — never by a caller of this door, who would otherwise be
    // minting a directly consumable record with no human behind it.
    if matches!(kind, SubjectKind::SystemSignal) {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            AUDIT_SUBJECT_APPROVAL,
            body.subject_ref.clone(),
            violation(
                "subjectKind",
                "system_signal records are written by the signal consumer, not submitted",
            ),
        )
        .await);
    }

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
    })?;
    let submission = match resolve_submission(&conn, &scope, tenant_id, kind, &body).await {
        Ok(Ok(submission)) => submission,
        Ok(Err(refusal)) => {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                AUDIT_SUBJECT_APPROVAL,
                body.subject_ref.clone(),
                refusal,
            )
            .await);
        }
        Err(e) => return Err(repo_error_to_canonical(&e)),
    };

    // `inst-mt-once`: the policy read here is the policy **in force at the
    // submission instant**, and the store evaluates against it. An absent row
    // is the default and only a failed read is unresolvable (P-D-112 arm 2).
    let policy = repo::resolve_materiality_policy(&conn, &scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    // Held as a value the closure captures, so the evaluator's borrow lives
    // as long as the transaction it is used in. `Resolution` is `Copy` over a
    // reference and the reference is what must outlive the closure.
    let resolved: Option<crate::domain::materiality::MaterialityPolicy> = match policy {
        crate::domain::materiality::Resolution::Resolved(p) => Some(p),
        crate::domain::materiality::Resolution::Unresolvable => None,
    };
    // The store refuses an unresolvable policy through the codeless arm, so
    // the count is never read on that path.
    let configured = resolved.as_ref().map_or(
        0,
        crate::domain::materiality::MaterialityPolicy::approver_count,
    );

    let approval_id = ApprovalId::new(Uuid::now_v7());
    let audit_id = Uuid::now_v7();
    let scope_tx = scope.clone();
    let subject_tx = submission.subject.clone();
    let snapshot_tx = submission.content_snapshot.clone();
    let override_conditions = submission.override_conditions.clone();
    let ack_tx = body.author_override_ack.clone();
    let conditions_tx = override_conditions.clone();
    // The finance-material operand (`dod-finance-materiality`,
    // `dod-finance-predicate`; P-D-146): a publish that touches either
    // accounting code is Finance's whatever the caller said — the caller's
    // flag can add a reason the registry cannot see, never subtract one.
    let finance_material = body.finance_material
        || matches!(
            &submission.act,
            ActSpec::EntityPublish { touched, .. }
                if crate::domain::recognized::is_finance_material(touched)
        );
    let act_tx = Arc::new(submission.act);
    let attempted = body.subject_ref.clone();
    let answered = state
        .db
        .db()
        .transaction_with_retry::<repo::Submitted, TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let subject = subject_tx.clone();
                let snapshot = snapshot_tx.clone();
                let ack = ack_tx.clone();
                let override_conditions = conditions_tx.clone();
                let act = act_tx.clone();
                let resolved = resolved.clone();
                Box::pin(async move {
                    let evaluator = MaterialityEvaluator::new(resolved.as_ref().map_or(
                        crate::domain::materiality::Resolution::Unresolvable,
                        crate::domain::materiality::Resolution::Resolved,
                    ));
                    let mut scratch: Vec<&str> = Vec::new();
                    let act = act.as_act(&mut scratch);
                    let answered = repo::submit_approval(
                        tx,
                        &scope,
                        NewApproval {
                            approval_id,
                            subject: &subject,
                            internal_revision: submission.internal_revision,
                            content_snapshot: &snapshot,
                            diff_basis: submission.diff_basis,
                            act: &act,
                            evaluator,
                            finance_material,
                            approver_count: configured,
                            submitter: actor_ref,
                            author_override_ack: ack.as_deref(),
                            override_conditions: override_conditions.clone(),
                        },
                        now,
                    )
                    .await
                    .map_err(TxError::Store)?;
                    repo::write_eventless_act_audit(
                        tx,
                        &scope,
                        repo::AuditCommon {
                            audit_id,
                            tenant_id,
                            actor_ref,
                            action: "approval.submit".to_owned(),
                            subject_kind: AUDIT_SUBJECT_APPROVAL.to_owned(),
                            // No operator free text: P-D-120 row 35 narrowed
                            // `dod-pii-on-reasons` off the submission, and
                            // `products_approval` has no reason column.
                            reason: None,
                            correlation_id: events::correlation_id(),
                            written_at: now,
                        },
                        approval_id.get(),
                        Some(submission.internal_revision),
                    )
                    .await
                    .map_err(TxError::Repo)?;
                    Ok(answered)
                })
            },
        )
        .await;

    let answered = match answered {
        Ok(answered) => answered,
        Err(TxError::Store(ApprovalStoreError::Refused(refusal))) => {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                AUDIT_SUBJECT_APPROVAL,
                attempted,
                refusal,
            )
            .await);
        }
        Err(TxError::Store(ApprovalStoreError::Repo(e)) | TxError::Repo(e)) => {
            return Err(repo_error_to_canonical(&e));
        }
        Err(TxError::Events(e)) => {
            return Err(repo_error_to_canonical(
                &crate::infra::storage::RepoError::Db(e.to_string()),
            ));
        }
    };

    Ok((
        StatusCode::CREATED,
        Json(SubmitApprovalReceipt {
            approval_id: answered.approval_id.get(),
            state: answered.state.as_str().to_owned(),
            required: answered.descriptor.required(),
            configured_quorum: answered.descriptor.configured_quorum(),
            finance_required: answered.descriptor.finance_required(),
            quorum_reduced: answered.descriptor.quorum_reduced(),
        }),
    )
        .into_response())
}

/// Resolve the subject, the snapshot, the pin and the act, per kind.
///
/// The outer `Result` is the storage channel and the inner one the refusal
/// channel: a head that does not exist is a 404-shaped refusal, and a driver
/// failure is not a refusal at all.
async fn resolve_submission(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    kind: SubjectKind,
    body: &SubmitApprovalRequest,
) -> Result<Result<Submission, DomainError>, crate::infra::storage::RepoError> {
    if kind == SubjectKind::EntityPublish {
        if body.content_snapshot.is_some() {
            return Ok(Err(violation(
                "contentSnapshot",
                "an entity submission's snapshot is read from the head, so that an approver \
                 signs the bytes the publish will freeze (dod-stored-snapshot)",
            )));
        }
        let Some(entity) = parse_entity_ref(tenant_id, &body.subject_ref) else {
            return Ok(Err(violation(
                "subjectRef",
                "an entity_publish subject is product/{id} or sku/{id}",
            )));
        };
        let Some((snapshot, revision, basis, touched)) =
            resolve_entity_subject(runner, scope, entity).await?
        else {
            // **No `NotFound` variant is minted for this.** The gear's code
            // taxonomy is closed and `05` §3.3 declares no not-found code; a
            // subject that names no head is a request whose content cannot be
            // processed, which is what `VALIDATION` already is. Minting a
            // variant to spell a 404 would open a roster two decisions keep
            // closed for a case the ladder already renders.
            return Ok(Err(violation(
                "subjectRef",
                format!("no head for {} in this tenant", body.subject_ref),
            )));
        };
        let override_conditions = lint_conditions(runner, scope, entity).await?;
        return Ok(Ok(Submission {
            subject: GateSubject::entity_publish(entity, InternalRevision::new(revision)),
            internal_revision: revision,
            content_snapshot: snapshot,
            diff_basis: basis,
            act: ActSpec::EntityPublish {
                kind: entity.entity_kind,
                touched,
            },
            override_conditions,
        }));
    }

    let Some(snapshot) = body.content_snapshot.clone() else {
        return Ok(Err(violation(
            "contentSnapshot",
            "a non-entity subject carries the op payload as its snapshot (P-D-120 row 14)",
        )));
    };
    Ok(Ok(Submission {
        subject: GateSubject {
            tenant_id,
            kind,
            reference: body.subject_ref.clone(),
            // The op's own pin, or none. **P-D-125** row 52 puts the shape on
            // the kind, and this door has one wire field for it, so a caller
            // that supplies a number gets `PinnedRevision` and one that does
            // not gets `Unpinned` — never `Revision(0)`, which would claim a
            // head revision the subject does not have.
            pin: body
                .internal_revision
                .map_or(SubjectPin::Unpinned, SubjectPin::PinnedRevision),
        },
        // The op's own pin, or `0` where the subject has no counter: the
        // column exists to detect a stale submission and an op with no
        // counter cannot go stale (P-D-120 row 14).
        internal_revision: body.internal_revision.unwrap_or(0),
        content_snapshot: snapshot,
        // NULL: there is no published version to diff against.
        diff_basis: None,
        act: ActSpec::Owned(non_entity_act(kind)),
        override_conditions: Vec::new(),
    }))
}

// ---------------------------------------------------------------------------
// POST /bss-products/v1/approvals/{approvalId}/decisions
// ---------------------------------------------------------------------------

/// `POST /bss-products/v1/approvals/{approvalId}/decisions`.
///
/// # The role check runs before the row is appended, not after
///
/// `products_approval_decision` is **append-only outright** — no `UPDATE`, no
/// `DELETE`, on both engines — so a verdict from an ineligible principal
/// cannot be taken back once written. `APPROVER_ROLE_REQUIRED` is therefore
/// raised before the insert; checking afterwards would leave a permanent row
/// that every later evaluator has to remember to discount.
#[allow(clippy::too_many_lines)] // the door's one sequence: authz, parse, resolve, gate, write
async fn decide_approval(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(approval_id): Path<Uuid>,
    Json(body): Json<DecisionRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let attempted = approval_id.to_string();
    let scope = authorize(
        &state,
        &enforcer,
        &ctx,
        AuthzAsk {
            tenant_id,
            actor_ref,
            resource: &crate::authz::resource_types::APPROVAL,
            action: crate::authz::actions::DECIDE,
            door: Door::Approval,
            audit_subject_kind: AUDIT_SUBJECT_APPROVAL,
            attempted: &attempted,
        },
    )
    .await?;
    let record = ApprovalId::new(approval_id);

    let verdict = match body.verdict.as_str() {
        "approved" => DecisionVerdict::Approved,
        "rejected" => DecisionVerdict::Rejected,
        other => {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                AUDIT_SUBJECT_APPROVAL,
                attempted,
                violation(
                    "verdict",
                    format!("{other} is not a verdict: chk_products_approval_decision_verdict admits approved and rejected"),
                ),
            )
            .await);
        }
    };

    let reason = body.reason.as_ref().map(|r| r.trim().to_owned());
    // `dod-pii-on-reasons`' first stored reason. Refused **before the row is
    // written**, which is the whole reach erasure has over these records:
    // they are never edited and erasure is a map-only tombstone.
    let detector = crate::api::rest::retention::tenant_pii_detector(&state, tenant_id).await?;
    if let Some(reason) = reason.as_deref()
        && let Err(blocked) = pii_block(detector.as_ref(), reason)
    {
        {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                AUDIT_SUBJECT_APPROVAL,
                attempted,
                blocked,
            )
            .await);
        }
    }

    // **C1's base role set, read from the claim and never defaulted**
    // (P-D-119 rows 13 and 30, P-D-134 row 25). An approver holding neither
    // named role is not an eligible approver, so their verdict never reaches
    // the append-only table.
    let roles = roles_from_claims(ctx.token_scopes());
    if roles.is_empty() {
        return Err(refuse(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            AUDIT_SUBJECT_APPROVAL,
            attempted,
            DomainError::ApproverRoleRequired(format!(
                "principal {actor_ref} carries no role claim: C1 requires an approver holding \
                 {} or {}, and the platform's policy point encodes neither on any surface today \
                 (P-D-134 row 25)",
                ApproverRole::CatalogAdmin.as_str(),
                ApproverRole::FinanceReviewer.as_str()
            )),
        )
        .await);
    }

    let audit_id = Uuid::now_v7();
    let scope_tx = scope.clone();
    let reason_tx = reason.clone();
    // The ceremony (`dod-override-ceremony`; P-D-148): where the record
    // carries override conditions, an approving decision acknowledges each by
    // name — an informed override, never a blind one. Read before the
    // transaction: the conditions were fixed at submission and do not move.
    if verdict == DecisionVerdict::Approved {
        refuse_unacknowledged_conditions(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            attempted.clone(),
            record,
            body.override_acknowledgments.as_deref(),
        )
        .await?;
    }
    let acks_tx = body.override_acknowledgments.clone();
    let roles_tx = roles.clone();
    let sink = state.sink.clone();
    let outcome = state
        .db
        .db()
        .transaction_with_retry::<Decided, TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let reason = reason_tx.clone();
                let acks = acks_tx.clone();
                let roles = roles_tx.clone();
                let sink = sink.clone();
                Box::pin(async move {
                    let outcome = repo::record_decision(
                        tx,
                        &scope,
                        NewDecision {
                            tenant_id,
                            approval_id: record,
                            approver_principal: actor_ref,
                            verdict,
                            reason: reason.as_deref(),
                            override_acknowledgments: acks.as_deref(),
                        },
                        actor_ref,
                        now,
                    )
                    .await
                    .map_err(TxError::Store)?;

                    // **P-D-120 row 11: this transaction writes
                    // `state = satisfied`**, on the decision that meets the
                    // descriptor. The evaluator computes whether it is met;
                    // the flip is the store's, here, so the fact and the row
                    // that makes it true commit together.
                    let settled =
                        repo::settle_quorum(tx, &scope, tenant_id, record, &roles, actor_ref, now)
                            .await
                            .map_err(TxError::Store)?;

                    repo::write_eventless_act_audit(
                        tx,
                        &scope,
                        repo::AuditCommon {
                            audit_id,
                            tenant_id,
                            actor_ref,
                            action: "approval.decide".to_owned(),
                            subject_kind: AUDIT_SUBJECT_APPROVAL.to_owned(),
                            reason: reason.clone(),
                            correlation_id: events::correlation_id(),
                            written_at: now,
                        },
                        record.get(),
                        None,
                    )
                    .await
                    .map_err(TxError::Repo)?;

                    // `ApprovalDecided` on **either** verdict, in the
                    // mutating transaction. A refusal here travels as an
                    // error, so the transaction rolls back and the verdict is
                    // not recorded either — the event is the success-path
                    // announcement (P-D-21) and an act it cannot announce did
                    // not happen.
                    events::enqueue_governance(
                        &sink,
                        tx,
                        record.get(),
                        events::APPROVAL_DECIDED_PAYLOAD_TYPE,
                        &GovernanceEventBody {
                            tenant_id,
                            act: "decided",
                            approval_id: Some(record.get()),
                            session_id: None,
                            verdict: Some(verdict.as_str()),
                            state: Some(settled.state.as_str()),
                            target_tenant_id: None,
                        },
                        actor_ref,
                    )
                    .await
                    .map_err(TxError::Events)?;

                    Ok(Decided {
                        state: settled.state,
                        counted: settled.counted,
                        required: settled.required,
                        finalized: outcome == repo::DecisionOutcome::Finalized,
                    })
                })
            },
        )
        .await;

    let decided = match outcome {
        Ok(decided) => decided,
        Err(TxError::Store(ApprovalStoreError::Refused(refusal))) => {
            return Err(refuse(
                &state,
                &scope,
                tenant_id,
                actor_ref,
                AUDIT_SUBJECT_APPROVAL,
                attempted,
                refusal,
            )
            .await);
        }
        Err(TxError::Store(ApprovalStoreError::Repo(e)) | TxError::Repo(e)) => {
            return Err(repo_error_to_canonical(&e));
        }
        Err(TxError::Events(e)) => {
            return Err(repo_error_to_canonical(
                &crate::infra::storage::RepoError::Db(e.to_string()),
            ));
        }
    };
    let _ = decided.finalized;

    Ok((
        StatusCode::OK,
        Json(DecisionReceipt {
            state: decided.state.as_str().to_owned(),
            counted: decided.counted,
            required: decided.required,
        }),
    )
        .into_response())
}

/// The by-name half of `dod-override-ceremony`: an approving decision on a
/// record whose stored descriptor names override conditions must acknowledge
/// every one of them in `override_acknowledgments`, else the decision is
/// refused `VALIDATION` on that field, naming the codes not acknowledged. The
/// conditions were fixed at submission and are read outside the decision's
/// transaction because they do not move.
async fn refuse_unacknowledged_conditions(
    state: &ApiState,
    scope: &toolkit_db::secure::AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    attempted: String,
    record: ApprovalId,
    acknowledgments: Option<&str>,
) -> Result<(), CanonicalError> {
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
            "decision connection: {e}"
        )))
    })?;
    let stored = repo::read_approval(&conn, scope, tenant_id, record)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let Some(stored) = stored else {
        return Ok(());
    };
    let Ok(descriptor) = crate::domain::approval::descriptor_from_stored(&stored.quorum_descriptor)
    else {
        return Ok(());
    };
    let missing = descriptor.unacknowledged(acknowledgments);
    if missing.is_empty() {
        return Ok(());
    }
    Err(refuse(
        state,
        scope,
        tenant_id,
        actor_ref,
        AUDIT_SUBJECT_APPROVAL,
        attempted,
        violation(
            "override_acknowledgments",
            format!(
                "this subject carries override conditions an approver acknowledges by name; \
                 not named: {}",
                missing.join(", ")
            ),
        ),
    )
    .await)
}

/// What the decide transaction answered.
struct Decided {
    state: ApprovalState,
    counted: u32,
    required: u32,
    finalized: bool,
}

// ---------------------------------------------------------------------------
// POST /bss-products/v1/breakglass-sessions
// ---------------------------------------------------------------------------

/// Which of `inst-bg-open`'s two paths the request names.
///
/// A function rather than a `match` inside the handler, because the rule is
/// `chk_products_breakglass_path`'s **exclusivity** stated at the door: the
/// two-person path names its reference and both approvers, the post-hoc path
/// names none of the three, and any other combination is a caller error the
/// engine would otherwise report as a constraint name.
fn approval_path(body: &BreakglassRequest) -> Result<ApprovalPath, DomainError> {
    match (
        body.two_person_approval_ref,
        body.approver_a,
        body.approver_b,
    ) {
        (Some(reference), Some(approver_a), Some(approver_b)) => Ok(ApprovalPath::TwoPerson {
            reference,
            approver_a,
            approver_b,
        }),
        (None, None, None) => Ok(ApprovalPath::PostHoc),
        _ => Err(violation(
            "twoPersonApprovalRef",
            "the two-person path names its reference and both platform approvers; the post-hoc \
             path names none of the three (chk_products_breakglass_path)",
        )),
    }
}

/// `POST /bss-products/v1/breakglass-sessions`.
///
/// # The window is read, never inlined
///
/// `breakglass_window_hours` is `ProductsConfig`'s (**P-D-132**, interim 4,
/// zero refused at boot) and reaches the door through [`ApiState`] for the
/// reason every other configured number does: a door reads per-request state,
/// never a configuration source of its own. **No renewal** — a session is not
/// extended and a new session is a new two-person ceremony.
///
/// # A failed alert must not leave a silent session
///
/// `dod-breakglass-open`'s hardest clause. The alert is a `tracing::warn!`
/// with a stable event name — the gear's own alert channel, the one
/// `gear.rs`'s loops already use — and it is raised **after** the transaction
/// commits, deliberately: a macro that cannot fail cannot leave the session
/// silent, and folding an infallible emission into the transaction would only
/// make the commit depend on a subscriber. The event `BreakGlassElevated`
/// rides the transaction itself, so an elevation that could not be announced
/// does not open.
async fn open_breakglass(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<BreakglassRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let caller_tenant = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, caller_tenant, ctx.subject_id(), now)
            .await?;
    let attempted = body.target_tenant_id.to_string();
    // The grant is checked against the **caller's** tenant: an elevation is a
    // platform act, and asking the policy point about the target would let a
    // principal who holds `breakglass x elevate` nowhere obtain it by naming
    // a tenant where somebody else does.
    let scope = authorize(
        &state,
        &enforcer,
        &ctx,
        AuthzAsk {
            tenant_id: caller_tenant,
            actor_ref,
            resource: &crate::authz::resource_types::BREAKGLASS,
            action: crate::authz::actions::ELEVATE,
            door: Door::Breakglass,
            audit_subject_kind: AUDIT_SUBJECT_BREAKGLASS,
            attempted: &attempted,
        },
    )
    .await?;

    let reason = body.reason.trim().to_owned();
    if reason.is_empty() {
        return Err(refuse(
            &state,
            &scope,
            caller_tenant,
            actor_ref,
            AUDIT_SUBJECT_BREAKGLASS,
            attempted,
            violation("reason", "an elevation carries a mandatory reason"),
        )
        .await);
    }
    // `dod-pii-on-reasons`' second stored reason, judged against the
    // **caller's** allow-list: an elevation is authored in the caller's
    // tenant, and the target tenant's Legal sign-offs are not the caller's to
    // spend.
    let detector = crate::api::rest::retention::tenant_pii_detector(&state, caller_tenant).await?;
    if let Err(blocked) = pii_block(detector.as_ref(), &reason) {
        return Err(refuse(
            &state,
            &scope,
            caller_tenant,
            actor_ref,
            AUDIT_SUBJECT_BREAKGLASS,
            attempted,
            blocked,
        )
        .await);
    }

    let path = match approval_path(&body) {
        Ok(path) => path,
        Err(refusal) => {
            return Err(refuse(
                &state,
                &scope,
                caller_tenant,
                actor_ref,
                AUDIT_SUBJECT_BREAKGLASS,
                attempted,
                refusal,
            )
            .await);
        }
    };

    // **The grant is the caller's tenant; the write scope is the target's.**
    // `breakglass_session` is `Scopable` on `target_tenant` — the session is a
    // platform record, but the thing it grants access TO is one tenant, so a
    // tenant-scoped read must not see another's elevations. Writing the row
    // under the caller's own scope fails `scope_with_model` on every
    // cross-tenant elevation, which is every elevation this door exists for.
    let write_scope = AccessScope::for_tenant(body.target_tenant_id);
    let session_id = Uuid::now_v7();
    let valid_until = now
        + TimeDelta::try_hours(i64::from(state.breakglass_window_hours))
            .unwrap_or_else(|| TimeDelta::try_hours(4).unwrap_or_default());
    let audit_id = Uuid::now_v7();
    let scope_tx = write_scope;
    let reason_tx = reason.clone();
    let target = body.target_tenant_id;
    let sink = state.sink.clone();
    let opened = state
        .db
        .db()
        .transaction_with_retry::<(), TxError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            contention_db_err,
            move |tx| {
                let scope = scope_tx.clone();
                let reason = reason_tx.clone();
                let sink = sink.clone();
                Box::pin(async move {
                    repo::open_breakglass_session(
                        tx,
                        &scope,
                        NewElevation {
                            session_id,
                            principal: actor_ref,
                            target_tenant: target,
                            valid_from: now,
                            valid_until,
                            path,
                            opened_at: now,
                        },
                        &reason,
                    )
                    .await
                    .map_err(TxError::Repo)?;
                    repo::write_eventless_act_audit(
                        tx,
                        &scope,
                        repo::AuditCommon {
                            audit_id,
                            tenant_id: target,
                            actor_ref,
                            action: "breakglass.open".to_owned(),
                            subject_kind: AUDIT_SUBJECT_BREAKGLASS.to_owned(),
                            reason: Some(reason.clone()),
                            correlation_id: events::correlation_id(),
                            written_at: now,
                        },
                        session_id,
                        None,
                    )
                    .await
                    .map_err(TxError::Repo)?;
                    events::enqueue_governance(
                        &sink,
                        tx,
                        session_id,
                        events::BREAK_GLASS_ELEVATED_PAYLOAD_TYPE,
                        &GovernanceEventBody {
                            tenant_id: target,
                            act: "elevated",
                            approval_id: None,
                            session_id: Some(session_id),
                            verdict: None,
                            state: None,
                            target_tenant_id: Some(target),
                        },
                        actor_ref,
                    )
                    .await
                    .map_err(TxError::Events)?;
                    Ok(())
                })
            },
        )
        .await;

    match opened {
        Ok(()) => {}
        Err(TxError::Store(ApprovalStoreError::Refused(refusal))) => {
            return Err(refuse(
                &state,
                &scope,
                caller_tenant,
                actor_ref,
                AUDIT_SUBJECT_BREAKGLASS,
                attempted,
                refusal,
            )
            .await);
        }
        Err(TxError::Store(ApprovalStoreError::Repo(e)) | TxError::Repo(e)) => {
            return Err(repo_error_to_canonical(&e));
        }
        Err(TxError::Events(e)) => {
            return Err(repo_error_to_canonical(
                &crate::infra::storage::RepoError::Db(e.to_string()),
            ));
        }
    }

    // The distinct alert channel, raised after the commit. The post-hoc path
    // carries the review obligation the SLA measures.
    let path_token = if matches!(path, ApprovalPath::TwoPerson { .. }) {
        "two_person"
    } else {
        "post_hoc"
    };
    tracing::warn!(
        event = "products_breakglass_elevated",
        session_id = %session_id,
        target_tenant = %target,
        path = path_token,
        review_sla_hours = state.breakglass_review_sla_hours,
        "a break-glass elevation session opened"
    );

    Ok((
        StatusCode::CREATED,
        Json(BreakglassReceipt {
            session_id,
            valid_from: now,
            valid_until,
            path: path_token.to_owned(),
        }),
    )
        .into_response())
}

#[cfg(test)]
#[path = "approvals_tests.rs"]
mod approvals_tests;
