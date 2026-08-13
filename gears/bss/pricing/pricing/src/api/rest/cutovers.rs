//! `POST /bss-pricing/v1/plans/{planId}/cutovers` — the grandfathering cutover
//! (`inst-gc-api`, `inst-gc-return`, D-100).
//!
//! `api::rest::supersessions`' shape, deliberately, because the two acts differ in
//! three things and a reader should find those and nothing else. Every argument
//! that module makes about *this* layer holds here verbatim and is not restated:
//! why the body names a **row id** rather than the ten scope-key axes, why the
//! authz pair is `plan × write` and not `plan × publish`, why the body is parsed
//! **after** the gate, why both arms answer `202` with a token saying which, and
//! why the minted ids may be discarded.
//!
//! # What differs, and it is the whole of this module's own content
//!
//! **The answer carries a second row and a second window.** A supersession moves
//! one row onto a key; a cutover moves one on and **retains** another on a new
//! generation, so `copyPriceId` and `copyKey` are on the response and are not
//! nullable on the controlled arm — both drafts are staged there, because
//! `inst-gc-compose` clause (a) makes them the reviewer's subject.
//!
//! **There is no materiality question to report.** `inst-mat-registered` registers
//! a grandfathering cutover, so the act is material whatever a threshold policy
//! says and whatever delta it carries. The response therefore carries the verdict
//! for the record rather than as an explanation of *why* an approval was needed,
//! and no configured threshold can make this route commit on one principal.
//!
//! **The idempotency key is `(planId, key-set hash, cutover instant)`** (D-28), not
//! `(planId, scope key, instant)`. A retry naming a different selection is a
//! different act — see `infra::cutover::cutover_unit_ref` for why that protects the
//! caller rather than endangering them.

use std::sync::Arc;

use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::approvals::ApprovalView;
use crate::api::rest::auth_context::require_authenticated;
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::preconditions;
use crate::api::rest::prices::{PriceContentView, content_of};
use crate::api::rest::state::GovernanceState;
use crate::api::rest::windows::verdict_json;
use crate::domain::error::DomainError;
use crate::domain::scope_key::PlanId;
use crate::infra::cutover::{CutoverOutcome, CutoverPending, CutoverReceipt, CutoverRequest};
use crate::infra::storage::repo::price_repo;

/// The route's registered path template.
///
/// The literal is repeated in the `OperationBuilder` call below because DE0801
/// validates a literal argument and silently passes a `const` one;
/// `tests/module_test.rs` binds the two spellings together.
pub const PLAN_CUTOVERS: &str = "/bss-pricing/v1/plans/{planId}/cutovers";

/// The `OpenAPI` tag this surface is filed under — Slice 7's, as the window and
/// supersession routes are.
const TAG: &str = "BSS Pricing Windows";

/// The wire token for the submit arm — `api::rest::publish`'s, imported rather
/// than re-spelled (Z13-2).
///
/// `OUTCOME_SUBMITTED` is `pub(crate)` precisely so this surface does not carry a
/// second spelling: a client's `match` must not depend on which route answered it,
/// and a rename of the const there has to reach here. It is the **outcome**
/// vocabulary and not a lifecycle one — no column in this gear stores this word,
/// which is why `windows`, `overlays`, `bundles` and `customer_groups` all import
/// this same const while each keeps its own stored-state renderer.
const OUTCOME_SUBMITTED: &str = crate::api::rest::publish::OUTCOME_SUBMITTED;

/// The body of a cutover request.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct CutoverBody {
    /// The row being cut over — the key's current published row.
    pub predecessor_price_id: Uuid,
    /// The instant all three window operations pivot on. Strictly future at submit,
    /// and at least D-47's max batching delay ahead at the approval commit
    /// (`inst-gc-compose`).
    pub cutover_at: DateTime<Utc>,
    /// The successor row's whole content, in the authoring vocabulary.
    ///
    /// The **grandfathered copy's** content is not a member and cannot be: the copy
    /// is what retained subscribers keep paying, so it is the predecessor's content
    /// carried onto a generation of its own key. A body able to name it would be a
    /// body able to change what a retained subscriber pays without anybody
    /// superseding anything.
    pub successor: PriceContentView,
    /// The operator-supplied change reason **both** scheduled windows are recorded
    /// under.
    pub reason_code: String,
}

/// What a cutover answers with.
///
/// `outcome` is the only discriminator and the status is `202` on both arms, for
/// `api::rest::supersessions`' stated reason: neither arm can honestly answer 200.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct CutoverOutcomeView {
    /// `cut_over` | `submitted_for_approval`.
    pub outcome: String,
    /// The plan whose subject re-projects.
    pub plan_id: Uuid,
    /// The plan revision this act freezes — the plan's **current** one.
    pub revision: u64,
    /// The row that left, or would leave, the published plane.
    pub predecessor_price_id: Uuid,
    /// The successor draft, on **both** arms: the controlled arm stages it.
    pub successor_price_id: Uuid,
    /// The grandfathered copy's draft, on **both** arms for the same reason.
    pub copy_price_id: Uuid,
    /// The generation the copy stands on, canonically rendered — ten axes.
    ///
    /// On both arms, and it is the field a caller most needs from the controlled
    /// one: the generation is minted by the act rather than named by the request,
    /// so nothing else tells a caller which key their retained subscribers will
    /// resolve to.
    pub copy_key: String,
    /// The cutover instant, echoed so a retry can be built from the answer.
    pub cutover_at: DateTime<Utc>,
    /// The window whose end moves, or would move, to the cutover. `null` on the
    /// controlled arm, where no window moved.
    pub shortened_window_id: Option<Uuid>,
    /// The registry's pending handle. `null` on the controlled arm: no version was
    /// requested for an act that did not commit, and D-156 is why.
    pub pending_version_ref: Option<String>,
    /// The unit a second principal has to decide. `null` on the committed arm.
    pub approval: Option<ApprovalView>,
}

impl CutoverOutcomeView {
    fn of_receipt(receipt: &CutoverReceipt) -> Self {
        Self {
            outcome: "cut_over".to_owned(),
            plan_id: receipt.plan_id.get(),
            revision: receipt.revision,
            predecessor_price_id: receipt.predecessor_price_id,
            successor_price_id: receipt.successor_price_id,
            copy_price_id: receipt.copy_price_id,
            copy_key: receipt.copy_key.to_string(),
            cutover_at: receipt.cutover_at,
            shortened_window_id: Some(receipt.shortened_window_id),
            pending_version_ref: Some(receipt.pending_version_ref.clone()),
            approval: None,
        }
    }

    fn of_pending(pending: &CutoverPending, cutover_at: DateTime<Utc>, predecessor: Uuid) -> Self {
        Self {
            outcome: OUTCOME_SUBMITTED.to_owned(),
            plan_id: pending.plan_id.get(),
            revision: pending.revision,
            predecessor_price_id: predecessor,
            successor_price_id: pending.successor_price_id,
            copy_price_id: pending.copy_price_id,
            copy_key: pending.copy_key.to_string(),
            cutover_at,
            shortened_window_id: None,
            pending_version_ref: None,
            approval: Some(ApprovalView::from(&pending.approval)),
        }
    }
}

async fn cut_over_key(
    Extension(state): Extension<Arc<GovernanceState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    correlation: Option<Extension<CorrelationId>>,
    Path(plan_id): Path<Uuid>,
    body: axum::body::Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(correlation)?;
    let plan_id = PlanId::new(plan_id);
    let tenant = ctx.subject_tenant_id();

    // `plan × write`, the pair the supersession and the window mutations ask for, and
    // for the same reason: the entrance is what `publish` guards, and a cutover that
    // commits does so under an approval this route does not grant itself.
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(tenant),
        /* resource_id */ Some(plan_id.get()),
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    // Parsed after the gate — `api::rest::supersessions`' house rule, stated there.
    let request: CutoverBody = preconditions::parse_body(&body)?;

    let key = price_repo::load_scope_key(
        &state
            .db
            .conn()
            .map_err(|e| DomainError::Internal(format!("bss-pricing: cutover connection: {e}")))?,
        &scope,
        tenant,
        request.predecessor_price_id,
    )
    .await
    .map_err(|e| crate::infra::storage::repo_failure(&e))?
    .ok_or_else(|| DomainError::NotFound {
        subject: "price row".to_owned(),
        id: request.predecessor_price_id.to_string(),
    })?;
    if key.plan_id() != plan_id {
        return Err(DomainError::NotFound {
            subject: "price row on this plan".to_owned(),
            id: request.predecessor_price_id.to_string(),
        }
        .into());
    }

    let stamp = crate::api::rest::auth_context::audit_stamp(&ctx, Utc::now(), correlation);
    let cutover_at = request.cutover_at;
    let outcome = state
        .cutovers
        .cut_over(
            &ctx,
            &scope,
            tenant,
            CutoverRequest {
                predecessor_key: key,
                cutover_at,
                successor: content_of(&request.successor)?,
                // Four ids minted here, server-side, for the supersession's stated
                // reason: the surface has to name them in its answer before any is
                // durable, and the call after an approve discards the ones whose rows
                // an earlier attempt already staged.
                successor_price_id: Uuid::now_v7(),
                successor_window_id: Uuid::now_v7(),
                copy_price_id: Uuid::now_v7(),
                copy_window_id: Uuid::now_v7(),
                reason_code: request.reason_code,
            },
            verdict_json,
            stamp,
        )
        .await?;

    let view = match &outcome {
        CutoverOutcome::Committed(receipt) => CutoverOutcomeView::of_receipt(receipt),
        CutoverOutcome::SubmittedForApproval(pending) => {
            CutoverOutcomeView::of_pending(pending, cutover_at, request.predecessor_price_id)
        }
    };
    Ok((StatusCode::ACCEPTED, Json(view)).into_response())
}

/// Build the Axum router for the cutover surface and register its operation.
pub fn router(state: Arc<GovernanceState>, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::post("/bss-pricing/v1/plans/{planId}/cutovers")
        .operation_id("bss_pricing.cut_over_key")
        .summary("Reprice a canonical scope key and retain its subscribers, as one atomic unit")
        .description(
            "Composes the **grandfathering cutover** (D-100) over one canonical scope key: a \
             successor draft on the key's own axes, a **grandfathered copy** of the row being \
             closed on a new generation of that key, the current window's `effectiveTo` shortened \
             to the cutover instant, and both new windows scheduled open-ended from it - one \
             approval unit and one local ACID transaction, or nothing. Answers `202` on both \
             arms, `outcome` saying which: `submitted_for_approval` while a second principal has \
             not decided the unit, `cut_over` when it committed. \
             \
             **A cutover is always material** (`inst-mat-registered`), so no configured threshold \
             makes it commit on one principal; the verdict in the answer is the record of that \
             rather than an explanation. \
             \
             The body names the **current `priceId`** rather than the ten scope-key axes, and the \
             `{planId}` segment is checked against the row's own plan - a row of another plan \
             answers `404`. The copy's content is **not** a member of the body: it is the \
             predecessor's, carried onto the new generation, because that is what retained \
             subscribers keep paying. The instant must be strictly future at submit and at least \
             D-47's max batching delay ahead at the commit (`CUTOVER_INSTANT_PASSED`); the key \
             must hold coverage at it, a dormant key being a revival rather than a cutover \
             (`CUTOVER_GAP`); and an instant some generation of the key already carries is \
             refused (`DUPLICATE_SCOPE_KEY`), since every cutover mints its own generation. \
             \
             **No `Idempotency-Key` header**: the act is keyed by `(planId, key-set hash, cutover \
             instant)` (D-28), so a second identical request is answered with the *same* pending \
             unit, while a request naming a different selection is a different act. \
             \
             Both drafts **are** written on the controlled arm - they are the reviewer's subject - \
             while the flip, both publishes and all three window operations are not. Gates on \
             `plan` x `write`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan whose key is being cut over.")
        .json_request::<CutoverBody>(
            openapi,
            "The predecessor row, the cutover instant and the successor's content.",
        )
        .handler(cut_over_key)
        .json_response_with_schema::<CutoverOutcomeView>(
            openapi,
            StatusCode::ACCEPTED,
            "The unit committed, or a unit was opened over the two staged drafts; `outcome` says \
             which.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi)
        .layer(Extension(state))
        // D-178's correlation edge, per-router as every mutating surface applies it.
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}
