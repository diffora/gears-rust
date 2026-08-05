//! `GET/PUT /bss-pricing/v1/config/approval-threshold-policy` — the tenant's
//! per-currency approval-threshold policy (`design/05-governance.md` §5, D-10).
//!
//! # The `PUT` opens a unit; it does not apply a diff
//!
//! D-10 is direction-agnostic: **any** policy diff needs an independent second
//! `FinanceReviewer`, because the two-person rule's own foundation must not be
//! single-person-editable. Loosening a threshold obviously needs review;
//! tightening one does too, since a tightening that nobody reviewed is a denial of
//! service on the authoring plane dressed as prudence. So the `PUT` answers **202**
//! with the pending unit, and the proposed version becomes the tenant's policy when
//! that unit is approved — which is a fact `infra::threshold::effective_version`
//! reads off the approval store, not a column anything flips.
//!
//! **The bootstrap is reachable and is fail-safe, which is the sentence that makes
//! the whole arrangement work.** A tenant with no policy has everything material
//! (`inst-mat-failsafe`), so its *first* policy `PUT` is itself an always-material
//! act that needs a second principal. No tenant can therefore configure a threshold
//! without first completing an approved unit, and a single-reviewer tenant simply
//! stays at "everything material" rather than being locked out of the gear.
//!
//! # `approval_policy`, not `config`, and that is segregation of duties
//!
//! The authz catalog carries `cf.bss.pricing.approval_policy.v1~` as a resource
//! type of its own, deliberately separate from `config`: a config admin must not be
//! able to weaken the thresholds that govern their own changes. The label already
//! existed and no authz vocabulary is minted here. `FinanceReviewer` holds
//! `approval_policy × write/read` in the matrix, which is also what makes D-61
//! satisfiable — the approver of a policy unit can read what they are approving.
//!
//! # §5's Idempotency cell is **not** satisfied, and this is an open gap against
//! # D-171 rather than a settled divergence
//!
//! §5 gives this row the cell *"`ETag` + approval unit"*. The approval unit half is
//! implemented exactly. The `ETag` half is not implemented at all, and the argument
//! that used to stand here for why that was *fine* does not hold. It is restated as
//! the gap it is, for the owner to decide.
//!
//! **The premise that failed.** It read: *"a tenant's **first** proposal is made
//! against no prior version, so there is no tag the caller could echo … a mandatory
//! `If-Match` would make the bootstrap unreachable"*. The `GET` answers **200** with
//! `effective: null` for a tenant that has never had a version approved — that is this
//! module's own decision, argued three sections down — so the resource always has a
//! representation and therefore always has an entity tag. `If-Match` is satisfiable on
//! the first `PUT`; the bootstrap is not unreachable and never was.
//!
//! **The other premise is weak rather than false.** It is true that every `If-Match`
//! in this gear today names a [`RowVersion`] read off a mutable row's `row_version`
//! column, and that `pricing_approval_threshold` carries no such column. But an entity
//! tag need not be a row version: the **effective version number** and the **pending
//! unit id** are both to hand on the `GET`, and either would name the state a caller
//! read. What is owed is a decision about which, not a column the store lacks.
//!
//! **And the replacement does not fully cover what the cell is aimed at.**
//! `PENDING_CHANGE_UNIT_EXISTS` (409) refuses a second proposal while one is under
//! review, and every proposal that opens is reviewed by a second principal who is shown
//! its content (D-61) — which is a strong control and is why the gap is a gap and not a
//! hole. It leaves two things: two operators proposing *in sequence* (the first is
//! approved, the second is authored against a policy the operator read before that
//! happened, and nothing tells them the world moved), and the pending-unit guard's own
//! read-then-write race, which is a real lost-update window rather than a theoretical
//! one — see `ThresholdService::propose`.
//!
//! [`RowVersion`]: crate::domain::concurrency::RowVersion
//!
//! # No `404`, and no route-level `409` on the `GET`
//!
//! A tenant with no policy is answered `200` with `version: null` and no entries.
//! That is a **state** and not an absent resource: "unset" is the design set's own
//! bootstrap, `inst-mat-failsafe` is named for it, and answering 404 would tell an
//! operator the surface does not exist when what they actually need to know is that
//! everything is material.

use std::sync::Arc;

use axum::extract::Extension;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::StatusCode};
use chrono::{DateTime, Utc};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::approvals::{
    ApprovalView, MaterialityView, PinnedThresholdPolicyView, ThresholdEntryView,
};
use crate::api::rest::auth_context::{audit_stamp, require_authenticated};
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::state::GovernanceState;
use crate::domain::error::DomainError;
use crate::domain::materiality::triggers::Trigger;
use crate::domain::materiality::{self, ChangeSet, ThresholdBasis, ThresholdEntry};
use crate::domain::money::CurrencyCode;

/// `OpenAPI` tag applied to both operations (DE0205).
const TAG: &str = "BSS Pricing Governance";

/// The policy resource, as a `config` sub-resource (D-140: never a colon method).
///
/// The literal is repeated in both `OperationBuilder` calls below because DE0801
/// validates a **literal** argument and silently passes a `const` one; the two
/// spellings are pinned together by `tests/module_test.rs`'s route census.
pub const APPROVAL_THRESHOLD_POLICY: &str = "/bss-pricing/v1/config/approval-threshold-policy";

/// The largest basis-point threshold this surface accepts: 100%.
///
/// §6 says only `percent > 0` and names no ceiling, so this is a **decision** and
/// it is stated as one. A threshold above 100% is a threshold no price change can
/// reach — a doubling is `10_000` bp — so it is the two-person rule switched off by
/// arithmetic, which is exactly what
/// `chk_pricing_approval_threshold_percent_positive` refuses at the other end of
/// the range. It is also what keeps `percent_bp`'s domain `u32` inside the store's
/// signed 32-bit column with four orders of magnitude to spare.
pub const MAX_PERCENT_BP: u32 = 10_000;

// ---------------------------------------------------------------------------
// Views.
// ---------------------------------------------------------------------------

/// The tenant's policy as it stands: what is in force, and what is under review.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ThresholdPolicyView {
    /// The version currently in force, or `null` for a tenant that has never had
    /// one approved — under which **every** change is material (`inst-mat-failsafe`).
    pub effective: Option<PinnedThresholdPolicyView>,
    /// The unit reviewing a proposal, if one is open. A second `PUT` while this is
    /// present is refused `PENDING_CHANGE_UNIT_EXISTS` (409).
    pub pending_approval: Option<ApprovalView>,
}

/// A proposed policy version.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct PutThresholdPolicyRequest {
    /// When the thresholds start applying, once approved. UTC, millisecond
    /// precision (D-144).
    pub effective_from: DateTime<Utc>,
    /// The per-currency entries. **The whole policy**, not a patch: a version is a
    /// complete entry set, so a currency left out of this list is a currency with
    /// no threshold — which is material by `inst-mat-percurrency`'s fail-safe
    /// rather than a currency that keeps its old value.
    pub entries: Vec<ThresholdEntryView>,
}

/// What the `PUT` did: opened a unit over the proposed version.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ThresholdProposalView {
    /// The version this proposal minted. It is **not** in force yet.
    pub proposed: PinnedThresholdPolicyView,
    /// The always-material unit reviewing it (D-10).
    pub approval: ApprovalView,
}

// ---------------------------------------------------------------------------
// Router.
// ---------------------------------------------------------------------------

/// Build the Axum router for the two policy operations and register them.
pub fn router(state: Arc<GovernanceState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-pricing/v1/config/approval-threshold-policy")
        .operation_id("bss_pricing.get_approval_threshold_policy")
        .summary("Read the tenant's approval-threshold policy")
        .description(
            "The per-currency thresholds currently **in force** - the greatest proposed version \
             whose unit an independent `FinanceReviewer` approved and whose content still matches \
             what they signed - together with the proposal under review, if there is one. A \
             tenant that has never had a version approved is answered `200` with `effective: \
             null`, which is a state and not an absent resource: unset means the two-person rule \
             applies to **every** change (`inst-mat-failsafe`), and a currency with no entry in a \
             configured policy is material for the same reason. Gates on `approval_policy` x \
             `read`, which is deliberately not `config` x `read`: a config admin must not read or \
             move the policy that governs their own changes.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(get_threshold_policy)
        .json_response_with_schema::<ThresholdPolicyView>(
            openapi,
            StatusCode::OK,
            "The effective policy and the proposal under review.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    let router = OperationBuilder::put("/bss-pricing/v1/config/approval-threshold-policy")
        .operation_id("bss_pricing.put_approval_threshold_policy")
        .summary("Propose the tenant's approval-threshold policy")
        .description(
            "Writes the proposal as a **new version** and answers `202` with the always-material \
             approval unit reviewing it (D-10). It does **not** apply the diff: the version \
             becomes the tenant's policy when an independent `FinanceReviewer` approves that \
             unit, which is why the store is versioned and append-only rather than mutated in \
             place - under mutation in place the proposed content would have nowhere to live and \
             the approval's pin nothing to cover. The body is the **whole** policy and not a \
             patch. A tenant's first proposal is itself material under the fail-safe, so no \
             tenant can configure a threshold without completing an approved unit first. Shape \
             rules, all `THRESHOLD_INVALID` (400): keys are ISO 4217 codes, at least one entry, \
             no currency twice, exactly one of `absoluteMinor` / `percentBp` per entry, \
             `absoluteMinor` >= 0, and `percentBp` in `1..=10000`. A second proposal while one is \
             under review is `PENDING_CHANGE_UNIT_EXISTS` (409) - decide it or withdraw it. This \
             surface declares **no** `If-Match`: the store carries no row version and a tenant's \
             first proposal has no prior version to name, so a mandatory precondition would make \
             the bootstrap unreachable; see the module doc. Gates on `approval_policy` x `write`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<PutThresholdPolicyRequest>(openapi, "The proposed per-currency entries.")
        .handler(put_threshold_policy)
        .json_response_with_schema::<ThresholdProposalView>(
            openapi,
            StatusCode::ACCEPTED,
            "The proposal is open; the body names the version and the unit reviewing it.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    // D-178's edge, applied at this router's own tail for the reason every other
    // mutating router applies it at its own: a surface reachable without it cannot
    // build an `AuditStamp`.
    router
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

/// `GET /config/approval-threshold-policy`.
async fn get_threshold_policy(
    Extension(state): Extension<Arc<GovernanceState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
) -> Result<Json<ThresholdPolicyView>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = read_scope(&enforcer, &ctx).await?;
    let held = state
        .thresholds
        .state(&scope, ctx.subject_tenant_id())
        .await
        .map_err(CanonicalError::from)?;
    Ok(Json(ThresholdPolicyView {
        effective: held.effective.as_ref().map(PinnedThresholdPolicyView::from),
        pending_approval: held.pending.as_ref().map(ApprovalView::from),
    }))
}

/// `PUT /config/approval-threshold-policy`.
async fn put_threshold_policy(
    Extension(state): Extension<Arc<GovernanceState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Json(request): Json<PutThresholdPolicyRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let scope = write_scope(&enforcer, &ctx).await?;
    let tenant = ctx.subject_tenant_id();
    let now = Utc::now();

    let entries = parse_entries(&request.entries).map_err(CanonicalError::from)?;

    // **Evaluated, not asserted.** The verdict recorded on the unit is
    // `materiality::evaluate`'s over the registered trigger this act *is*
    // (`Trigger::ThresholdPolicyDiff`, D-10), rather than a `Material` value
    // written here — so the stored `materiality` jsonb of a policy unit is produced
    // by the same evaluator as every other unit's, and a reader comparing two units
    // is comparing two answers from one function. It passes no policy, which is
    // right for a reason beyond convenience: the trigger is examined at §3 step 4
    // before any threshold is consulted, so no configured policy can make this act
    // auto-publishable, which is D-10 exactly.
    let verdict = materiality::evaluate(
        &ChangeSet::of_act(Trigger::ThresholdPolicyDiff, Vec::new()),
        /* policy */ None,
        /* baseline */ None,
    );
    let materiality = serde_json::to_value(MaterialityView::from(verdict)).map_err(|e| {
        CanonicalError::from(DomainError::Internal(format!(
            "cannot render the materiality verdict: {e}"
        )))
    })?;

    let (version, record) = state
        .thresholds
        .propose(
            &scope,
            tenant,
            Uuid::now_v7(),
            request.effective_from,
            entries,
            materiality,
            audit_stamp(&ctx, now, correlation),
        )
        .await
        .map_err(CanonicalError::from)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(ThresholdProposalView {
            proposed: PinnedThresholdPolicyView::from(&version),
            approval: ApprovalView::from(&record),
        }),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// Request parsing — the shape rules §6 states and §5 codes `THRESHOLD_INVALID`.
// ---------------------------------------------------------------------------

/// The authored entries as the domain holds them.
///
/// Every refusal here is [`DomainError::ThresholdInvalid`], including the currency
/// one: [`CurrencyCode::new`] answers `CURRENCY_INVALID`, which is the right code
/// on a price row and the wrong one here — a caller of this route has no price row,
/// and a `currency` precondition violation would name a field they cannot find. The
/// error is therefore **re-coded**, not passed through, and the detail names the
/// offending code so the remedy is still actionable.
///
/// The order is the caller's, sorted by currency before it leaves: the pin is taken
/// over this rendering, so a policy proposed as `USD, EUR` and one proposed as
/// `EUR, USD` must be one version with one digest rather than two.
///
/// # Errors
/// [`DomainError::ThresholdInvalid`] on a code that is not ISO 4217 alpha-3, on an
/// entry setting neither or both bases, on a negative `absolute_minor`, and on a
/// `percent_bp` outside `1..=`[`MAX_PERCENT_BP`]. The empty-set and
/// duplicate-currency rules are [`ThresholdVersion::new`]'s and are refused inside
/// the proposal's transaction, where the version number they would have consumed is
/// still unminted.
fn parse_entries(authored: &[ThresholdEntryView]) -> Result<Vec<ThresholdEntry>, DomainError> {
    let refuse = |why: String| DomainError::ThresholdInvalid(why);
    let mut entries = Vec::with_capacity(authored.len());
    for entry in authored {
        let currency = CurrencyCode::new(&entry.currency).map_err(|_| {
            refuse(format!(
                "entries[{}]: a threshold key is an ISO 4217 alpha-3 code",
                entry.currency
            ))
        })?;
        let basis = match (entry.absolute_minor, entry.percent_bp) {
            (Some(minor), None) => {
                if minor < 0 {
                    return Err(refuse(format!(
                        "entries[{currency}]: absoluteMinor is {minor}; a negative threshold is \
                         below every change there is, which is the two-person rule switched off \
                         by arithmetic"
                    )));
                }
                ThresholdBasis::Absolute { minor }
            }
            (None, Some(bp)) => {
                if bp == 0 || bp > MAX_PERCENT_BP {
                    return Err(refuse(format!(
                        "entries[{currency}]: percentBp is {bp}; it must be in 1..={MAX_PERCENT_BP} \
                         (basis points, 10000 = 100%)"
                    )));
                }
                ThresholdBasis::Percent { bp }
            }
            (Some(_), Some(_)) => {
                return Err(refuse(format!(
                    "entries[{currency}]: absoluteMinor and percentBp are both set; a threshold \
                     has one basis, or the evaluator picks one with nothing saying which"
                )));
            }
            (None, None) => {
                return Err(refuse(format!(
                    "entries[{currency}]: neither absoluteMinor nor percentBp is set; an entry \
                     that thresholds nothing still counts as this currency having one, which is \
                     the fail-safe switched off by an empty row"
                )));
            }
        };
        entries.push(ThresholdEntry { currency, basis });
    }
    // Sorted here and nowhere else. `ThresholdVersion::new` keeps its caller's
    // order deliberately, so that the two producers of a version - this and the
    // store's `ORDER BY currency` - have one place each to be right rather than one
    // shared re-sort that hides a disagreement.
    entries.sort_by(|left, right| left.currency.as_str().cmp(right.currency.as_str()));
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Gates.
// ---------------------------------------------------------------------------

/// The `approval_policy × read` gate.
///
/// `resource_id` is `None` because the policy is the tenant's and has no id of its
/// own; the tenant axis is the scope's, as it is on the approval list.
async fn read_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::APPROVAL_POLICY,
        crate::authz::actions::READ,
        /* owner_tenant_id */ None,
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)
}

/// The `approval_policy × write` gate.
///
/// `owner_tenant_id = Some(caller's tenant)` because this is a write, for
/// `approvals::decide_scope`'s reason: the membership assertion is what refuses a
/// target outside the compiled scope, the degraded flat-`In` decision not
/// re-checking the property.
async fn write_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::APPROVAL_POLICY,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(ctx.subject_tenant_id()),
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)
}

#[cfg(test)]
#[path = "threshold_policy_tests.rs"]
mod threshold_policy_tests;
