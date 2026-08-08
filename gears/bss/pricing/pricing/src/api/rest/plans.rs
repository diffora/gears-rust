//! The plan authoring plane: `GET`, `POST`, `PATCH` and `POST …/abandon`
//! (`design/02-plan-definition.md` §5, D-140, D-141, D-145, D-146).
//!
//! Every route here gates on `plan x write` - or `plan x read` for the read  -
//! **before** it touches a repository (`01-foundation.md` §2.2), and the pair is
//! taken from [`crate::authz`] rather than spelled here, so a route gated on the
//! wrong action is a test failure rather than a reading of source order.
//!
//! # What this surface validates, and what it deliberately does not
//!
//! Nothing here judges a plan's **shape**. The billing-cycle matrix, the phase
//! graph, the add-on rules and the descriptor set are all validated at
//! **publish** (§4.2 step 2), and authoring an incomplete draft is legal by
//! design - an author assembles a plan over several calls, and refusing an
//! intermediate state would make the pre-check-at-publish design unreachable. So
//! a `PATCH` that leaves a plan with no `billing_cycle` succeeds, and the plan
//! is simply not publishable yet.
//!
//! # The refusals this plane can raise, all of them Foundation-owned
//!
//! `STALE_VERSION` (409), `LIFECYCLE_FORBIDDEN` (400 carrying its code),
//! `OPEN_DRAFT_REVISION_EXISTS` (409), `PLAN_RETIRED_NO_SUCCESSOR` (400),
//! `PLAN_ABANDONED_NO_SUCCESSOR` (400), `IDEMPOTENCY_PAYLOAD_MISMATCH` (409),
//! `IDEMPOTENCY_KEY_IN_FLIGHT` (409). The design set calls several of those 422s
//! and they arrive as **400s carrying their code** - §3.3 states it once for the
//! whole set: the platform has no 422 category, and the code string is the
//! discriminator a consumer matches on, not the status. No route here declares a
//! 422 response.

use std::collections::BTreeMap;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path};
use axum::http::header::{ETAG, LOCATION};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::HeaderMap, http::StatusCode};
use chrono::{DateTime, Utc};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::operation_builder::{ParamLocation, ParamSpec};
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_db::secure::{AccessScope, DbTx};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::auth_context::{audit_stamp, require_authenticated};
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::preconditions::{self, RevisionTag};
use crate::api::rest::state::AuthoringState;
use crate::domain::audit::AuditStamp;
use crate::domain::concurrency::RowVersion;
use crate::domain::contracts::{
    EntitlementGrants, GrantSet, PlanChangeContract, UsageCounterOnPlanChange,
};
use crate::domain::error::DomainError;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::plan::{PlanRevision, PlanShapePatch};
use crate::domain::plan_shape::{
    AddonRule, BillingCycle, CustomIntervalUnit, DescriptorSet, Frequency, PhaseKind, PlanPhase,
};
use crate::domain::scope_key::{PhaseId, PlanId};
use crate::infra::idempotent::{self, Guarded, GuardedRequest, TxFuture};
use crate::infra::storage::repo::{NewPlanDraft, PlanRepo, plan_repo};
use crate::infra::storage::{RepoError, repo_failure};

/// `OpenAPI` tag applied to every plan operation (DE0205).
const TAG: &str = "BSS Pricing Plans";

/// The plan collection.
///
/// The literal is repeated in the `OperationBuilder` call below because DE0801
/// validates a **literal** argument and silently passes a `const` one - so the
/// route shape rule only binds where the literal is. The two spellings are
/// pinned together by `plans_tests::the_router_registers_exactly_the_declared_paths`,
/// which is what keeps the second spelling from drifting.
pub const PLANS: &str = "/bss-pricing/v1/plans";
/// One plan, by id.
pub const PLAN: &str = "/bss-pricing/v1/plans/{planId}";
/// The abandon action, as a sub-resource segment (D-140: never a colon method).
pub const PLAN_ABANDON: &str = "/bss-pricing/v1/plans/{planId}/abandon";

/// The `If-Match` header, declared so a generated client knows it is mandatory.
///
/// `OperationBuilder` has `path_param` and `query_param` but no header builder,
/// so the spec is written out — the sibling credstore surface does the same at
/// `api/rest/routes.rs`. **Declaring it is not decoration**: without it a client
/// generated from this spec cannot satisfy the precondition on any mutating
/// verb, and learns of it from a 400. The same discipline this gear already
/// applies to responses, applied to requests.
pub(crate) fn if_match_param(subject: &str) -> ParamSpec {
    ParamSpec {
        name: "If-Match".to_owned(),
        location: ParamLocation::Header,
        required: true,
        description: Some(format!(
            "Mandatory optimistic-concurrency precondition (RFC 9110), and D-141's rule that \
             every mutating verb on a draft presents its `ETag`. {subject} A mismatch is `409` \
             `STALE_VERSION`; an absent or malformed header is `400`. Weak validators, the \
             wildcard `*` and tag lists are all refused - a wildcard is an unconditional \
             mutation, which is what the precondition exists to prevent."
        )),
        param_type: "string".to_owned(),
    }
}

/// The `Idempotency-Key` header, likewise.
pub(crate) fn idempotency_key_param() -> ParamSpec {
    ParamSpec {
        name: "Idempotency-Key".to_owned(),
        location: ParamLocation::Header,
        required: true,
        description: Some(
            "Mandatory client idempotency key (S2/S3 API surface, foundation step 1). Bounded \
             at 255 printable-ASCII characters. The key is claimed in the same transaction as \
             the mutation it guards, so a retry carrying the same key and body is answered the \
             recorded response verbatim, and one carrying the same key with a different body \
             is `409` `IDEMPOTENCY_PAYLOAD_MISMATCH`. An absent header is `400`: an unguarded \
             create on a governed authoring plane is the retry hazard the gate exists for."
                .to_owned(),
        ),
        param_type: "string".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Views. `*View` rather than `*Dto`, carrying `api_dto` anyway (DE0203/DE0204
// do not reach the name, and the macro is what fixes the wire shape to
// snake_case and registers the schema).
// ---------------------------------------------------------------------------

/// A recurring frequency, with the custom interval that rides its variant.
///
/// One object rather than three sibling members, because the domain type
/// [`Frequency`] cannot represent a fixed frequency carrying an interval and the
/// wire must not be able to either: three flat members would let a caller send
/// `monthly` with an interval, which is the pairing
/// `chk_pricing_plan_custom_interval_pairing` exists to refuse.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct FrequencyView {
    /// The frequency token (`monthly`, `quarterly`, `semiannual`, `annual`,
    /// `custom_every_n`).
    pub kind: String,
    /// The interval count, set on `custom_every_n` and on nothing else.
    pub custom_interval_n: Option<u32>,
    /// The interval unit (`days`, `months`), likewise.
    pub custom_interval_unit: Option<String>,
}

impl From<Frequency> for FrequencyView {
    fn from(frequency: Frequency) -> Self {
        let (n, unit) = match frequency {
            Frequency::CustomEveryN { n, unit } => (Some(n), Some(unit.as_str().to_owned())),
            Frequency::Monthly
            | Frequency::Quarterly
            | Frequency::Semiannual
            | Frequency::Annual => (None, None),
        };
        Self {
            kind: frequency.as_str().to_owned(),
            custom_interval_n: n,
            custom_interval_unit: unit,
        }
    }
}

/// One phase of a revision's phase chain.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct PlanPhaseView {
    /// Stable across revisions (D-83): the `phase` scope-key axis is filed under
    /// it, so a copy-forward keeps the id rather than minting a new one.
    pub phase_id: Uuid,
    /// `trial` | `intro` | `evergreen`.
    pub kind: String,
    /// Position in the chain.
    pub ordinal: i32,
    /// The phase this one converts into; `null` on the terminal phase.
    pub converts_to_phase_id: Option<Uuid>,
    /// How long the phase lasts; `null` on the terminal phase.
    pub phase_duration_days: Option<u32>,
    /// The trial length a storefront displays, on a `trial` phase.
    pub display_trial_days: Option<u32>,
}

impl From<PlanPhase> for PlanPhaseView {
    fn from(phase: PlanPhase) -> Self {
        Self {
            phase_id: phase.phase_id.get(),
            kind: phase.kind.as_str().to_owned(),
            ordinal: phase.ordinal,
            converts_to_phase_id: phase
                .converts_to_phase_id
                .map(crate::domain::scope_key::PhaseId::get),
            phase_duration_days: phase.phase_duration_days,
            display_trial_days: phase.display_trial_days,
        }
    }
}

/// One add-on composition rule.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct AddonRuleView {
    /// The add-on SKU this rule is about.
    pub addon_sku_id: Uuid,
    /// Whether the add-on must be taken.
    pub required: bool,
    /// Selection-time lower bound.
    pub min_qty: Option<u32>,
    /// Selection-time upper bound.
    pub max_qty: Option<u32>,
    /// Selection-time quantity step.
    pub step_qty: Option<u32>,
    /// An alternative price for this add-on when taken with this plan.
    pub price_override_ref: Option<Uuid>,
    /// Add-on SKU ids this one requires.
    pub depends_on: Vec<Uuid>,
    /// Add-on SKU ids this one excludes.
    pub conflicts_with: Vec<Uuid>,
}

impl From<AddonRule> for AddonRuleView {
    fn from(rule: AddonRule) -> Self {
        Self {
            addon_sku_id: rule.addon_sku_id,
            required: rule.required,
            min_qty: rule.min_qty,
            max_qty: rule.max_qty,
            step_qty: rule.step_qty,
            price_override_ref: rule.price_override_ref,
            depends_on: rule.depends_on,
            conflicts_with: rule.conflicts_with,
        }
    }
}

/// The revision's billing descriptor set.
///
/// Its **absence** is `null` on [`PlanView::descriptor_set`], and an attached
/// set whose three named members are all empty is an object with three nulls.
/// The distinction is kept on the wire because the store keeps it - an
/// unattached set has no row at all - and because `DESCRIPTOR_INCOMPLETE` is
/// asked of an attached set: collapsing the two would make "nobody attached one"
/// and "somebody attached an empty one" the same publish input.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct DescriptorSetView {
    /// The invoice line template Billing renders from.
    pub invoice_line_template: Option<String>,
    /// The general-ledger code the posting lands on.
    pub gl_code: Option<String>,
    /// How the plan's charges are composed into invoice lines.
    pub itemization_rule: Option<String>,
    /// Extra descriptor keys a deployment's required-set names (P5).
    ///
    /// A `BTreeMap` rather than a `HashMap`, and the reason is the idempotency
    /// digest: `serde_json` renders a `BTreeMap` in key order and a `HashMap` in
    /// whatever order the process happens to hash to, so a retry would digest
    /// differently from its own first attempt (see `api::rest::preconditions`).
    pub additional: std::collections::BTreeMap<String, String>,
}

impl From<DescriptorSet> for DescriptorSetView {
    fn from(set: DescriptorSet) -> Self {
        Self {
            invoice_line_template: set.invoice_line_template,
            gl_code: set.gl_code,
            itemization_rule: set.itemization_rule,
            additional: set.additional,
        }
    }
}

/// One plan revision, whole: its own columns and its three child sets.
///
/// The child sets are here rather than on sub-resources of their own because
/// S2 §5's `PATCH` cell names those four facets as the plan's shape - a read
/// that omitted them could not round-trip a patch, and an author would have to
/// guess what a `PATCH` was about to replace.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct PlanView {
    /// The plan this revision belongs to.
    pub plan_id: Uuid,
    /// The revision number. An **identity, not a counter** (D-145): the sequence
    /// may have gaps where a draft was abandoned.
    pub revision: u64,
    /// `draft` | `abandoned` | `published` | `superseded` | `retired`. Named on
    /// the wire so a caller never has to infer which revision it was given.
    pub lifecycle_state: String,
    /// The catalog SKU this plan realizes, when one is bound.
    pub sku_id: Option<Uuid>,
    /// The plan's tier (a registry-owned taxonomy, so a string).
    pub plan_tier: Option<String>,
    /// `one_time` | `recurring` | `usage` | `hybrid`.
    pub billing_cycle: Option<String>,
    /// The recurring frequency, interval and all.
    pub frequency: Option<FrequencyView>,
    /// Whether the tier diverges from the parent SKU's under an audited
    /// override.
    pub plan_tier_override: bool,
    /// Minimum purchasable quantity (one-time plans).
    pub purchase_min_qty: Option<u64>,
    /// Maximum purchasable quantity (one-time plans).
    pub purchase_max_qty: Option<u64>,
    /// The Billing invoice-layout hint (D-96).
    pub invoice_grouping_key: Option<String>,
    /// Start of the availability window, UTC.
    pub available_from: Option<DateTime<Utc>>,
    /// End of the availability window, UTC.
    pub available_to: Option<DateTime<Utc>>,
    /// Pseudonymous principal id of the authoring actor.
    pub created_by: Uuid,
    /// When the revision was created, UTC.
    pub created_at_utc: DateTime<Utc>,
    /// The revision's optimistic-concurrency version - the same number the
    /// `ETag` header quotes, carried in the body too so a client that cannot see
    /// response headers can still submit a precondition.
    pub row_version: u64,
    /// The revision's phase chain, in the order the store returns.
    pub phases: Vec<PlanPhaseView>,
    /// The revision's add-on composition rules.
    pub addon_rules: Vec<AddonRuleView>,
    /// The revision's descriptor set, or `null` when none is attached.
    pub descriptor_set: Option<DescriptorSetView>,
}

impl PlanView {
    /// Compose a revision with its three child sets.
    fn new(
        revision: PlanRevision,
        phases: Vec<PlanPhase>,
        addon_rules: Vec<AddonRule>,
        descriptor_set: Option<DescriptorSet>,
    ) -> Self {
        Self {
            plan_id: revision.plan_id.get(),
            revision: revision.revision,
            lifecycle_state: revision.lifecycle_state.as_str().to_owned(),
            sku_id: revision.sku_id,
            plan_tier: revision.plan_tier,
            billing_cycle: revision
                .billing_cycle
                .map(|cycle| cycle.as_str().to_owned()),
            frequency: revision.frequency.map(FrequencyView::from),
            plan_tier_override: revision.plan_tier_override,
            purchase_min_qty: revision.purchase_min_qty,
            purchase_max_qty: revision.purchase_max_qty,
            invoice_grouping_key: revision.invoice_grouping_key,
            available_from: revision.available_from,
            available_to: revision.available_to,
            created_by: revision.created_by,
            created_at_utc: revision.created_at_utc,
            row_version: revision.row_version.get(),
            phases: phases.into_iter().map(PlanPhaseView::from).collect(),
            addon_rules: addon_rules.into_iter().map(AddonRuleView::from).collect(),
            descriptor_set: descriptor_set.map(DescriptorSetView::from),
        }
    }
}

/// The plan shape a create authors, and the one facet a `PATCH` may move.
///
/// Every member is optional because a plan is assembled over several calls: the
/// shape rules run at **publish**, not at save, so an unfinished draft is a
/// legal draft (§4.2 step 2).
#[derive(Debug, Clone, Default)]
#[toolkit_macros::api_dto(request, response)]
pub struct PlanShapeRequest {
    /// Bind the plan to a catalog SKU.
    pub sku_id: Option<Uuid>,
    /// The plan's tier (a registry-owned taxonomy, so a free string).
    pub plan_tier: Option<String>,
    /// `one_time` | `recurring` | `usage` | `hybrid`.
    pub billing_cycle: Option<String>,
    /// The recurring frequency, interval and all.
    pub frequency: Option<FrequencyView>,
    /// Declare or withdraw the audited tier override (P3).
    pub plan_tier_override: Option<bool>,
    /// Minimum purchasable quantity (one-time plans).
    pub purchase_min_qty: Option<u64>,
    /// Maximum purchasable quantity (one-time plans).
    pub purchase_max_qty: Option<u64>,
    /// The Billing invoice-layout hint (D-96).
    pub invoice_grouping_key: Option<String>,
    /// Start of the availability window, UTC.
    pub available_from: Option<DateTime<Utc>>,
    /// End of the availability window, UTC.
    pub available_to: Option<DateTime<Utc>>,
    /// The entitlement grant set (Slice 6, §6, D-41): the plan-level feature
    /// flags and quotas, the `PlanTier` they resolved from when they did, and
    /// any per-phase sets keyed by `phaseId`.
    ///
    /// Sent **whole or not at all**, for the change contract's reason below.
    pub entitlement_grants: Option<EntitlementGrantsRequest>,
    /// The plan-change contract (Slice 6, §6): the published `planId`s a
    /// self-service change may travel to, the comparability rank that
    /// classifies one, and D-113's tier-`Q` continuity flag.
    ///
    /// Sent **whole or not at all**, which is the shape
    /// [`PlanShapePatch::change_contract`] gives it: K4 ties the rank to whether
    /// the edge list names anyone, so a caller able to move one member alone
    /// could express a state no publish accepts.
    pub change_contract: Option<PlanChangeContractRequest>,
}

/// The entitlement grant set on the wire.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct EntitlementGrantsRequest {
    /// The `PlanTier` policy this set resolved from, when it did. Carried for
    /// auditability beside the resolved set, never instead of it
    /// (`inst-gs-resolved`).
    pub plan_tier_ref: Option<String>,
    /// Plan-level `featureFlag: bool` entries.
    pub feature_flags: Option<BTreeMap<String, bool>>,
    /// Plan-level `quotaKey: value` entries.
    pub quotas: Option<BTreeMap<String, i64>>,
    /// Optional per-phase sets, keyed by `phaseId` (D-41). Every key is checked
    /// against the plan's own phase schedule at publish, not here.
    pub per_phase: Option<BTreeMap<Uuid, GrantSetRequest>>,
}

/// One grant set on the wire.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct GrantSetRequest {
    /// `featureFlag: bool` entries.
    pub feature_flags: Option<BTreeMap<String, bool>>,
    /// `quotaKey: value` entries.
    pub quotas: Option<BTreeMap<String, i64>>,
}

/// The plan-change contract on the wire.
///
/// `request, response` rather than `request`, because
/// [`PlanShapeRequest`] is both and a member of a response DTO has to be
/// serializable too.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct PlanChangeContractRequest {
    /// Explicit published `planId`s. **Omit the field entirely** to state the
    /// fail-safe — no self-service change (`inst-pc-failsafe`) — which is not
    /// the same as sending an empty list.
    pub allowed_change_targets: Option<Vec<Uuid>>,
    /// The tenant-wide comparability rank (K4).
    pub comparability_rank: Option<i32>,
    /// `reset` (default) | `carry`. Omitted is `reset`, which is D-113's
    /// ratified default and the safe direction.
    pub usage_counter_on_plan_change: Option<String>,
}

/// A `PATCH` body: **exactly one** of the four facets S2 §5 names.
///
/// # Why one and not four
///
/// The reason is structural rather than stylistic.
/// [`PlanShapeRepo::replace_phases`](crate::infra::storage::repo::PlanShapeRepo::replace_phases),
/// `replace_addon_rules` and `set_descriptor_set` each compare-and-swap on the
/// **revision's** row version and each bump it - the child sets carry no tag of
/// their own, deliberately, so two authors editing different facets of one draft
/// cannot both satisfy one precondition. A two-facet patch would therefore match
/// the caller's tag on the first mutation and could not match it on the second,
/// and the two are separate transactions with a visible half-applied state in
/// between.
///
/// So more than one facet is [`DomainError::InvalidRequest`] (400, no new code),
/// and **this is a divergence from S2 §5**, whose `PATCH` purpose names four
/// facets in one verb while the storage layer versions them against one tag each
/// of them advances. A coherent multi-facet patch needs a composite operation
/// nobody has designed; it is reported rather than approximated.
#[derive(Debug, Clone, Default)]
#[toolkit_macros::api_dto(request, response)]
pub struct PatchPlanRequest {
    /// The plan's own columns.
    pub shape: Option<PlanShapeRequest>,
    /// The whole phase chain, replaced wholesale.
    pub phases: Option<Vec<PlanPhaseView>>,
    /// The whole add-on rule set, replaced wholesale.
    pub addon_rules: Option<Vec<AddonRuleView>>,
    /// The billing descriptor set, attached or replaced.
    pub descriptor_set: Option<DescriptorSetView>,
}

/// Build the Axum router for the plan surface and register its operations.
///
/// The declared responses are the ones each path can actually produce. **No 422
/// anywhere**: §3.3's status-rendering rule makes every architectural 422 in the
/// design set reach the wire as a 400 carrying its code, so a 422 in an
/// `OpenAPI` registration here would document a response no path can emit.
#[allow(
    clippy::too_many_lines,
    reason = "one builder chain per operation; flat is clearer than helpers that hide which route declares which response"
)]
pub fn router(state: Arc<AuthoringState>, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router = Router::new();

    router = OperationBuilder::post("/bss-pricing/v1/plans")
        .operation_id("bss_pricing.create_plan")
        .summary("Create a draft plan")
        .description(
            "Creates a plan by writing its revision `0` in `draft`, and answers `201` with a \
             `Location` header naming the new plan and an `ETag` carrying the revision's row \
             version. The plan id is **minted by the server**, so a caller never has to choose \
             one and a retry can never create a second plan under a name the client picked. \
             An `Idempotency-Key` header is **required** (S2 API surface, foundation step 1): the gate is the \
             first step of the mutation and runs in the same transaction as it, so a retry \
             carrying the same key and the same body is answered the recorded response - the \
             original plan id included: and a retry carrying the same key and a different body \
             is refused `IDEMPOTENCY_PAYLOAD_MISMATCH`. Nothing here judges the plan's shape: \
             the Slice-2 rules run at publish.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .param(idempotency_key_param())
        .json_request::<PlanShapeRequest>(openapi, "The plan's initial shape.")
        .handler(create_plan)
        .json_response_with_schema::<PlanView>(
            openapi,
            StatusCode::CREATED,
            "The plan's newly created revision `0`.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::patch("/bss-pricing/v1/plans/{planId}")
        .operation_id("bss_pricing.patch_plan")
        .summary("Edit a plan's open draft revision")
        .description(
            "Applies **exactly one** facet - the plan's own columns, its phase chain, its \
             add-on rule set, or its descriptor set - to the plan's open draft revision, under \
             the `If-Match` precondition. Two facets in one body is `400`: each child mutator \
             compare-and-swaps on the revision's row version and bumps it, so the second could \
             not match the tag the first advanced. When the plan holds **no** open draft, a \
             successor revision is opened from its current one and the patch lands on that \
             (D-90); the `If-Match` is then tested against the current revision. A retired plan \
             answers `PLAN_RETIRED_NO_SUCCESSOR`, a plan that already holds a draft answers \
             `OPEN_DRAFT_REVISION_EXISTS`, and a plan whose every revision is `abandoned` \
             answers `PLAN_ABANDONED_NO_SUCCESSOR` - its id is spent, so mint a new plan.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan to edit.")
        .param(if_match_param(
            "On a plan route the tag names **both** the revision it was read from and that \
             revision's version, joined by a hyphen (for example `\"3-7\"`), because \
             `/plans/{planId}` does not name a revision and a version alone would be applied \
             to whichever revision this call resolves.",
        ))
        .json_request::<PatchPlanRequest>(openapi, "Exactly one facet of the plan's shape.")
        .handler(patch_plan)
        .json_response_with_schema::<PlanView>(
            openapi,
            StatusCode::OK,
            "The revision as it stands after the edit.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::post("/bss-pricing/v1/plans/{planId}/abandon")
        .operation_id("bss_pricing.abandon_plan_draft")
        .summary("Discard a plan's open draft revision")
        .description(
            "Flips the plan's open draft revision to the terminal `abandoned` state under the \
             `If-Match` precondition, dropping its phase, add-on-rule and descriptor-set copies \
             in the same transaction (D-145). It is **not** a delete and the verb is therefore \
             not `DELETE`: the row survives as a tombstone so the `revision` number it consumed \
             stays consumed and `(planId, revision)` never names two rows over a plan's \
             lifetime. The precondition is what an unconditional abandon would destroy - a \
             concurrent editor's uncommitted work. A plan holding no open draft answers \
             `LIFECYCLE_FORBIDDEN`; a plan whose every revision is already `abandoned` answers \
             `PLAN_ABANDONED_NO_SUCCESSOR`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan whose open draft is discarded.")
        .param(if_match_param(
            "On a plan route the tag names **both** the revision it was read from and that \
             revision's version, joined by a hyphen (for example `\"3-7\"`); here it must name \
             the plan's open draft.",
        ))
        .handler(abandon_plan_draft)
        .json_response_with_schema::<PlanView>(
            openapi,
            StatusCode::OK,
            "The tombstoned revision, at its last row version.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::get("/bss-pricing/v1/plans/{planId}")
        .operation_id("bss_pricing.get_plan")
        .summary("Read a plan's authoring revision")
        .description(
            "Returns the plan's **open draft** revision when it has one, and its **current** \
             revision (`published` or `retired`) otherwise, with its phase chain, its add-on \
             rules and its descriptor set. This is the authoring read (`plan` x `read`); the \
             published content a consumer resolves comes from the read model, which is a \
             different contract. `lifecycle_state` and `revision` name which revision was \
             answered, so a caller never infers it. The `ETag` header carries the revision's row \
             version and is what the mutating verbs take as `If-Match`. A plan outside the \
             caller's scope reads exactly like an absent one (404, no existence leak).",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan to read.")
        .handler(get_plan)
        .json_response_with_schema::<PlanView>(
            openapi,
            StatusCode::OK,
            "The plan's open draft revision, or its current revision.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    // D-178's edge, applied here rather than where the routers are merged so it
    // travels with the routes: a surface reachable without it cannot build an
    // `AuditStamp`, and `correlation::require_correlation` answers 500 rather
    // than minting a second value per record.
    router
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}

/// `GET /plans/{planId}`.
///
/// **Which revision it answers, and why that is a decision rather than a
/// lookup.** A plan is a chain, and at most two of its revisions are reachable
/// for authoring: the open `draft`, and the current one
/// ([`LifecycleState::is_current_revision`](crate::domain::lifecycle::LifecycleState::is_current_revision),
/// never a hand-written state list). The draft wins when there is one, because
/// an author editing a plan is working on the draft and a read that answered the
/// published revision would hand them a body their next `PATCH` would not match.
/// With no draft the current revision is the only thing a caller can act on  -
/// a `PATCH` opens a successor from it.
///
/// The gate is `plan x read` with `owner_tenant_id = None`, so the PDP derives
/// the scope from the subject rather than from anything the caller sent, and the
/// compiled scope is the SQL filter. `require_constraints = true` so an
/// unconstrained allow fail-closes instead of exposing every tenant's plans.
async fn get_plan(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(plan_id): Path<Uuid>,
) -> Result<([(axum::http::HeaderName, String); 1], Json<PlanView>), CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let plan_id = PlanId::new(plan_id);
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::READ,
        /* owner_tenant_id */ None,
        /* resource_id */ Some(plan_id.get()),
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    let tenant = ctx.subject_tenant_id();
    let revision = authoring_revision(&state.plans, &scope, tenant, plan_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?
        .ok_or_else(|| CanonicalError::from(not_readable(plan_id)))?;

    let number = revision.revision;
    let view = read_shape(&state, &scope, tenant, plan_id, number, revision)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    Ok(([(ETAG, plan_tag(&view))], Json(view)))
}

/// The revision an authoring caller is working with: the open draft, else the
/// current revision.
async fn authoring_revision(
    plans: &PlanRepo,
    scope: &toolkit_db::secure::AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
) -> Result<Option<PlanRevision>, RepoError> {
    if let Some(draft) = plans.find_open_draft(scope, tenant, plan_id).await? {
        return Ok(Some(draft));
    }
    plans.find_current(scope, tenant, plan_id).await
}

/// Attach a revision's three child sets.
async fn read_shape(
    state: &AuthoringState,
    scope: &toolkit_db::secure::AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
    revision: u64,
    row: PlanRevision,
) -> Result<PlanView, RepoError> {
    let phases = state
        .shapes
        .list_phases(scope, tenant, plan_id, revision)
        .await?;
    let addon_rules = state
        .shapes
        .list_addon_rules(scope, tenant, plan_id, revision)
        .await?;
    let descriptor_set = state
        .shapes
        .find_descriptor_set(scope, tenant, plan_id, revision)
        .await?;
    Ok(PlanView::new(row, phases, addon_rules, descriptor_set))
}

/// The "absent, or not yours, or holding nothing an author can act on" refusal.
///
/// One answer for all three, which is the posture `frontier.rs` already
/// documents: a 403 for a foreign tenant's plan would confirm that the plan
/// exists, and the catalog is commercially sensitive.
fn not_readable(plan_id: PlanId) -> DomainError {
    DomainError::NotFound {
        subject: "plan".to_owned(),
        id: plan_id.to_string(),
    }
}

// ---------------------------------------------------------------------------
// The writes.
// ---------------------------------------------------------------------------

/// `POST /plans`.
///
/// The gate is `plan x write` with `owner_tenant_id = Some(caller's tenant)`, so
/// `access_scope`'s membership assertion refuses a target outside the compiled
/// scope - the degraded flat-`In` decision does not re-check the property, and a
/// write is the one direction where "the PDP will have filtered it" is not true.
///
/// **The surface mints the plan id** (`plan_repo`'s `NewPlanDraft` says so):
/// a repository that generated ids would make an idempotent retry create a
/// second plan, and the id has to be in a `Location` header before the row is
/// durable. Minting happens **inside** the guarded body, so a replay answers the
/// recorded body carrying the **original** id rather than a fresh one - which is
/// the entire reason the response body is stored.
///
/// A replay reconstructs `Location` from the recorded body's `plan_id` and sets
/// **no** `ETag`: the location is a pure function of an id the record carries,
/// while the revision's row version may have moved since the record was written,
/// and a stale tag is a precondition token that looks valid and is not.
async fn create_plan(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(tenant),
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    // Parsed after the gate, not before it: `prices.rs` orders it this way and a
    // module doc asserting "the gate before the repository" reads as two
    // disciplines if the two modules differ on where the body is read.
    let body: PlanShapeRequest = preconditions::parse_body(&body)?;
    let client_key = preconditions::idempotency_key(&headers)?;
    let request_hash = preconditions::request_digest(&body)?;
    let draft_shape = shape_of(&body)?;
    let now = Utc::now();

    let guard = GuardedRequest {
        operation: CREATE_PLAN_OPERATION,
        client_key,
        request_hash,
        tenant_id: tenant,
        status: StatusCode::CREATED.as_u16().into(),
        now,
    };
    let scope_for_body = scope.clone();
    let actor = ctx.subject_id();
    let outcome = idempotent::guarded(
        &state.db,
        &state.idempotency,
        &scope,
        guard,
        move |txn: &DbTx<'_>| -> TxFuture<'_, PlanRevision> {
            Box::pin(async move {
                // Minted here rather than before the claim: a replay must answer
                // the first caller's id, and an id minted outside the guarded
                // body would be a second one nobody was ever told about.
                let draft = draft_shape.into_draft(
                    PlanId::new(Uuid::now_v7()),
                    tenant,
                    actor,
                    now,
                    correlation,
                );
                // Mapped here rather than by the gate: `guarded` now takes a
                // mutation that speaks the gear's rejection vocabulary, because a
                // pipeline richer than one repository call has refusals no `RepoError`
                // can carry. The ladder is the same one, at the call site that knows
                // what it produced.
                plan_repo::create_draft_on(txn, &scope_for_body, draft)
                    .await
                    .map_err(|e| repo_failure(&e))
            })
        },
        |revision: &PlanRevision| {
            let view = PlanView::new(revision.clone(), Vec::new(), Vec::new(), None);
            serde_json::to_value(&view)
                .map_err(|e| DomainError::Internal(format!("cannot render the created plan: {e}")))
        },
    )
    .await
    .map_err(CanonicalError::from)?;

    Ok(match outcome {
        Guarded::Performed(revision) => created(&revision),
        Guarded::Replayed { status, body } => replayed(status, &body),
    })
}

/// `PATCH /plans/{planId}`.
async fn patch_plan(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path(plan_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<([(axum::http::HeaderName, String); 1], Json<PlanView>), CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    // Once, at the top: on the arm that opens a successor this call writes
    // **two** records, and D-178 clause (2) is that they carry one value.
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = write_scope(&enforcer, &ctx, plan_id.get(), tenant).await?;

    let body: PatchPlanRequest = preconditions::parse_body(&body)?;
    let asserted = preconditions::if_match_revision(&headers)?;
    let facet = Facet::of(body)?;
    let stamp = audit_stamp(&ctx, Utc::now(), correlation);

    // The revision the patch lands on, and the version the store will match.
    // It takes the **same** stamp: the successor it may open is the first of the
    // two records this one call writes.
    let target = target_revision(&state, &scope, tenant, plan_id, asserted, stamp).await?;
    let revision = target.revision;
    let expected = target.expected;

    match facet {
        Facet::Shape(shape) => {
            state
                .plans
                .update_draft(&scope, tenant, plan_id, revision, expected, shape, stamp)
                .await
        }
        Facet::Phases(phases) => {
            state
                .shapes
                .replace_phases(&scope, tenant, plan_id, revision, expected, phases, stamp)
                .await
        }
        Facet::AddonRules(rules) => {
            state
                .shapes
                .replace_addon_rules(&scope, tenant, plan_id, revision, expected, rules, stamp)
                .await
        }
        Facet::DescriptorSet(set) => {
            state
                .shapes
                .set_descriptor_set(&scope, tenant, plan_id, revision, expected, set, stamp)
                .await
        }
    }
    .map_err(|e| CanonicalError::from(repo_failure(&e)))?;

    answer_revision(&state, &scope, tenant, plan_id, revision).await
}

/// `POST /plans/{planId}/abandon`.
///
/// The revision abandoned is the plan's **open draft**, resolved here - which is
/// why the discrimination S2 §5 asks for lives on this surface and not in the
/// repository: `abandon_draft` names a revision, and the question "does this
/// *plan* hold a draft" is about the plan.
async fn abandon_plan_draft(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path(plan_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<([(axum::http::HeaderName, String); 1], Json<PlanView>), CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = write_scope(&enforcer, &ctx, plan_id.get(), tenant).await?;
    let asserted = preconditions::if_match_revision(&headers)?;

    let draft = state
        .plans
        .find_open_draft(&scope, tenant, plan_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    let Some(draft) = draft else {
        // No draft: either the id is spent, or the plan has a current revision
        // and there is simply nothing to discard, or it does not exist at all.
        return Err(CanonicalError::from(
            no_open_draft(&state.plans, &scope, tenant, plan_id, "abandon").await,
        ));
    };

    let revision = draft.revision;
    // The tag has to name the revision it was read from. Without it the caller's
    // version is applied to whatever revision the plan currently offers, and a
    // tag minted against revision N satisfies the swap on N+1 - a freshly opened
    // successor starts at version 0, the value every first draft carries.
    require_same_revision(plan_id, asserted, revision).map_err(CanonicalError::from)?;
    let expected = asserted.version;
    state
        .plans
        .abandon_draft(
            &scope,
            tenant,
            plan_id,
            revision,
            expected,
            audit_stamp(&ctx, Utc::now(), correlation),
        )
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;

    answer_revision(&state, &scope, tenant, plan_id, revision).await
}

// ---------------------------------------------------------------------------
// The pieces the three writes share.
// ---------------------------------------------------------------------------

/// The idempotency-gate operation name for `POST /plans`.
///
/// A per-route constant, so one client key reused across two verbs does not
/// collide: `operation` is part of the dedup table's composite key.
const CREATE_PLAN_OPERATION: &str = "bss_pricing.create_plan";

/// The `plan x write` gate, spelled once for the three mutating routes.
async fn write_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    resource_id: Uuid,
    tenant: Uuid,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(tenant),
        /* resource_id */ Some(resource_id),
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)
}

/// Which of the four facets a `PATCH` carries.
enum Facet {
    /// The plan's own columns.
    Shape(PlanShapePatch),
    /// The whole phase chain.
    Phases(Vec<PlanPhase>),
    /// The whole add-on rule set.
    AddonRules(Vec<AddonRule>),
    /// The descriptor set.
    DescriptorSet(DescriptorSet),
}

impl Facet {
    /// Exactly one facet, parsed. See [`PatchPlanRequest`] for why one.
    fn of(body: PatchPlanRequest) -> Result<Self, DomainError> {
        let named = usize::from(body.shape.is_some())
            + usize::from(body.phases.is_some())
            + usize::from(body.addon_rules.is_some())
            + usize::from(body.descriptor_set.is_some());
        if named != 1 {
            return Err(DomainError::InvalidRequest(format!(
                "a PATCH carries exactly one of `shape`, `phases`, `addon_rules` or \
                 `descriptor_set`; this one carries {named}. Each of the four \
                 compare-and-swaps on the revision's own row version and advances it, so two \
                 in one request could not both satisfy one `If-Match`"
            )));
        }
        if let Some(shape) = body.shape {
            return Ok(Self::Shape(shape_patch(&shape)?));
        }
        if let Some(phases) = body.phases {
            return Ok(Self::Phases(
                phases.iter().map(phase_of).collect::<Result<_, _>>()?,
            ));
        }
        if let Some(rules) = body.addon_rules {
            return Ok(Self::AddonRules(rules.into_iter().map(addon_of).collect()));
        }
        let set = body
            .descriptor_set
            .ok_or_else(|| DomainError::InvalidRequest("no facet named".to_owned()))?;
        Ok(Self::DescriptorSet(descriptor_of(set)))
    }
}

/// The revision a `PATCH` lands on, and the version the store will match.
struct PatchTarget {
    /// The revision number.
    revision: u64,
    /// The version the compare-and-swap is submitted against.
    expected: RowVersion,
}

/// Resolve the patch target: the open draft, or a successor opened from the
/// current revision.
///
/// # The tag names its subject, and both arms check it
///
/// `/plans/{planId}` does not name a revision, so a `PATCH` resolves one for
/// itself - and if the caller's tag carried only a version, that version would
/// be applied to whatever this resolution produced. The counters are unrelated:
/// a freshly opened successor stands at `RowVersion::new(0)`, exactly where the
/// plan's first draft stood, so a tag read against revision *N* would satisfy
/// the compare-and-swap on *N+1*. That is D-145's lost update arriving through
/// the **revision** instead of the version, and D-145 closed it on the storage
/// side precisely so a transport could not reopen it.
///
/// So both arms compare the tag's revision to the one they resolved: the
/// open-draft arm requires the tag to name that draft, and the successor arm
/// requires it to name the **current** revision (which is what the caller read)
/// before anything is opened. A mismatch is `STALE_VERSION` (409), the code the
/// design set already gives a caller working from a read it did not refresh.
///
/// # The successor arm's version check is a comparison, not a swap
///
/// `PlanRepo::open_revision` takes no `expected` version - nothing in the
/// design set gives it one, and the operation it performs is an insert rather
/// than an update, so there is no row for a compare-and-swap to match on. The
/// caller's `If-Match` is therefore tested here against the current revision's
/// row version, which leaves a **TOCTOU window** between that read and the
/// insert: a concurrent publish or retirement landing inside it is caught by
/// `open_revision`'s own refusals (`PLAN_RETIRED_NO_SUCCESSOR`,
/// `OPEN_DRAFT_REVISION_EXISTS`, and `uq_pricing_plan_open_draft` deciding the
/// race), but a concurrent edit that merely moved the current revision's tag is
/// not - the successor would open from a revision the caller had not read. This
/// is stated rather than hidden behind a comparison that looks atomic; closing
/// it needs an `expected` parameter on `open_revision`, which is a repository
/// change and a divergence this group reports rather than makes.
async fn target_revision(
    state: &AuthoringState,
    scope: &AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
    asserted: RevisionTag,
    stamp: AuditStamp,
) -> Result<PatchTarget, CanonicalError> {
    let draft = state
        .plans
        .find_open_draft(scope, tenant, plan_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    if let Some(draft) = draft {
        require_same_revision(plan_id, asserted, draft.revision).map_err(CanonicalError::from)?;
        return Ok(PatchTarget {
            revision: draft.revision,
            expected: asserted.version,
        });
    }

    let current = state
        .plans
        .find_current(scope, tenant, plan_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    let Some(current) = current else {
        return Err(CanonicalError::from(
            spent_or_absent(&state.plans, scope, tenant, plan_id).await,
        ));
    };
    // The tag has to have been read from THIS revision, and to carry its
    // version. The first check is what stops a tag minted against a revision
    // that has since been superseded from opening a successor it never saw; the
    // second is the ordinary staleness test, taken here because the insert below
    // has no row to match it against.
    require_same_revision(plan_id, asserted, current.revision).map_err(CanonicalError::from)?;
    crate::domain::concurrency::require_match(current.row_version, asserted.version)
        .map_err(CanonicalError::from)?;

    let opened = state
        .plans
        .open_revision(scope, tenant, plan_id, stamp)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    Ok(PatchTarget {
        revision: opened.revision,
        expected: opened.row_version,
    })
}

/// Refuse a precondition minted against a **different revision** of this plan.
///
/// `STALE_VERSION` rather than a new code, and it is the same refusal in
/// substance: the caller is working from a read it did not refresh, and what has
/// moved on is *which revision the plan offers for editing* rather than the
/// version of the one it offered before. The message names both revisions,
/// because that difference is the diagnosis - a caller one revision behind
/// published in between, and a caller naming a revision the plan never had is
/// pasting a tag from somewhere else.
pub(crate) fn require_same_revision(
    plan_id: PlanId,
    asserted: RevisionTag,
    resolved: u64,
) -> Result<(), DomainError> {
    if asserted.revision == resolved {
        return Ok(());
    }
    Err(DomainError::StaleVersion(format!(
        "plan {plan_id}: the precondition names revision {}, and the revision this call acts \
         on is {resolved}; re-read the plan and submit against the tag it answers",
        asserted.revision
    )))
}

/// The entity tag a plan route hands back: the revision, and its version.
fn plan_tag(view: &PlanView) -> String {
    preconditions::revision_etag(view.revision, RowVersion::new(view.row_version))
}

/// A plan with no current revision and no open draft: spent, or absent.
///
/// The discrimination S2 §5 owes this group. It is two reads, and the second one
/// is what keeps the refusal honest.
///
/// An empty chain is absent (404). A non-empty chain with no current revision
/// and no draft should be one whose every revision is `abandoned` — those are the
/// only states left once `draft`, `published` and `retired` are excluded, and a
/// `superseded` revision implies a successor that is current. **Should**, not
/// must: nothing in the schema forbids a chain whose greatest revision is
/// `superseded` with no successor, and telling an operator such a plan's "id is
/// spent, so mint a new plan" would be specific and false. So the greatest
/// revision's state is read, and a chain that is not tipped by a tombstone is
/// reported as the invariant breach it is rather than dressed as a lifecycle
/// refusal.
async fn spent_or_absent(
    plans: &PlanRepo,
    scope: &AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
) -> DomainError {
    let highest = match plans.max_revision(scope, tenant, plan_id).await {
        Err(e) => return repo_failure(&e),
        Ok(None) => return not_readable(plan_id),
        Ok(Some(highest)) => highest,
    };
    match plans.find_revision(scope, tenant, plan_id, highest).await {
        Err(e) => repo_failure(&e),
        Ok(Some(row)) if row.lifecycle_state == LifecycleState::Abandoned => {
            DomainError::PlanAbandonedNoSuccessor(format!(
                "plan {plan_id} holds only abandoned revisions; its id is spent, so mint a \
                 new plan"
            ))
        }
        Ok(other) => DomainError::Internal(format!(
            "plan {plan_id} holds revision {highest} in state {}, with no current revision and \
             no open draft; the chain is unreachable and no refusal describes it",
            other.map_or_else(
                || "(absent)".to_owned(),
                |row| row.lifecycle_state.to_string()
            )
        )),
    }
}

/// Why a verb that needs the plan's open draft found none.
///
/// Three answers, and they are three because the operator's next action differs:
/// a spent plan is `PLAN_ABANDONED_NO_SUCCESSOR` (mint a new plan), a plan with
/// a current revision and no draft is `LIFECYCLE_FORBIDDEN` - S2 §5 in as many
/// words, and D-146's line about the code that holds refusals describing no
/// alternative action - and a plan with nothing at all is a 404.
///
/// **Shared with the publish mount**, which is why it takes a verb and a
/// repository rather than the abandon's state: S2 §5 gives `POST …/abandon` and
/// `POST …/publish` the same three refusals over the same missing subject, and
/// two spellings of one discrimination would be two chances for one of them to
/// answer a bare 404 for a plan whose id is spent.
pub(crate) async fn no_open_draft(
    plans: &PlanRepo,
    scope: &AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
    verb: &str,
) -> DomainError {
    match plans.find_current(scope, tenant, plan_id).await {
        Err(e) => repo_failure(&e),
        Ok(Some(current)) => DomainError::LifecycleForbidden(format!(
            "plan {plan_id} holds no open draft revision to {verb}; its current revision {} \
             is {}",
            current.revision, current.lifecycle_state
        )),
        Ok(None) => spent_or_absent(plans, scope, tenant, plan_id).await,
    }
}

/// Read the revision back with its child sets and answer 200 + `ETag`.
///
/// A fresh read rather than the value the mutation returned, because a facet
/// mutation answers only the revision row while the response carries all four
/// facets - and because under a concurrent second edit the store's reading is
/// the only honest one.
async fn answer_revision(
    state: &AuthoringState,
    scope: &AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
    revision: u64,
) -> Result<([(axum::http::HeaderName, String); 1], Json<PlanView>), CanonicalError> {
    let row = state
        .plans
        .find_revision(scope, tenant, plan_id, revision)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?
        .ok_or_else(|| CanonicalError::from(not_readable(plan_id)))?;
    let view = read_shape(state, scope, tenant, plan_id, revision, row)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    Ok(([(ETAG, plan_tag(&view))], Json(view)))
}

/// The 201 a performed create answers with.
fn created(revision: &PlanRevision) -> Response {
    let view = PlanView::new(revision.clone(), Vec::new(), Vec::new(), None);
    let tag = plan_tag(&view);
    (
        StatusCode::CREATED,
        [
            (LOCATION, format!("{PLANS}/{}", revision.plan_id)),
            (ETAG, tag),
        ],
        Json(view),
    )
        .into_response()
}

/// The recorded answer a replay is handed back, verbatim.
///
/// `Location` is rebuilt from the **recorded body**, which is a pure function of
/// an id that body carries. No `ETag`: the dedup row stores a status and a body
/// and no headers, and the revision's row version may have moved since the
/// record was written - a tag rebuilt from a stale body would be a precondition
/// token that looks valid and is not.
fn replayed(status: i32, body: &serde_json::Value) -> Response {
    let status = u16::try_from(status)
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok())
        .unwrap_or(StatusCode::OK);
    let location = body
        .get("plan_id")
        .and_then(serde_json::Value::as_str)
        .map(|plan_id| format!("{PLANS}/{plan_id}"));
    match location {
        Some(location) => (status, [(LOCATION, location)], Json(body.clone())).into_response(),
        None => (status, Json(body.clone())).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Wire -> domain. Every one of these refuses rather than coerces: a token the
// domain does not know is a request this gear cannot interpret, which is the
// Foundation validation envelope's own case and mints no code.
// ---------------------------------------------------------------------------

/// A create's shape, parsed, and still missing the five things only the handler
/// knows: the minted id, the tenant, the actor, the authoring instant and the
/// request's correlation.
struct DraftShape {
    /// The catalog SKU this plan realizes.
    sku_id: Option<Uuid>,
    /// The plan's tier.
    plan_tier: Option<String>,
    /// The plan's billing cycle.
    billing_cycle: Option<BillingCycle>,
    /// The recurring frequency, interval and all.
    frequency: Option<Frequency>,
    /// The audited tier override (P3); absent means no override, because the
    /// column is `NOT NULL DEFAULT false` and "nobody said" and "no override"
    /// are the same claim about a plan.
    plan_tier_override: bool,
    /// Minimum purchasable quantity.
    purchase_min_qty: Option<u64>,
    /// Maximum purchasable quantity.
    purchase_max_qty: Option<u64>,
    /// The Billing invoice-layout hint.
    invoice_grouping_key: Option<String>,
    /// Start of the availability window.
    available_from: Option<DateTime<Utc>>,
    /// End of the availability window.
    available_to: Option<DateTime<Utc>>,
}

impl DraftShape {
    /// Bind the shape to the identity and provenance the surface supplies.
    ///
    /// `created_at_utc` is the request's instant rather than the database's:
    /// §2.2 makes the catalog author-driven, so the authoring instant belongs to
    /// the call and not to whichever node evaluated `now()`.
    fn into_draft(
        self,
        plan_id: PlanId,
        tenant_id: Uuid,
        created_by: Uuid,
        created_at_utc: DateTime<Utc>,
        correlation_id: Uuid,
    ) -> NewPlanDraft {
        NewPlanDraft {
            plan_id,
            tenant_id,
            created_by,
            created_at_utc,
            sku_id: self.sku_id,
            plan_tier: self.plan_tier,
            billing_cycle: self.billing_cycle,
            frequency: self.frequency,
            plan_tier_override: self.plan_tier_override,
            purchase_min_qty: self.purchase_min_qty,
            purchase_max_qty: self.purchase_max_qty,
            invoice_grouping_key: self.invoice_grouping_key,
            available_from: self.available_from,
            available_to: self.available_to,
            // The authoring surface never sets lineage: `POST /plans` creates an
            // authored plan by definition, and a caller able to claim a source
            // could forge provenance for a plan it never copied. The clone path
            // is the only writer (`inst-cl-copy`).
            cloned_from: None,
            correlation_id,
        }
    }
}

/// Parse a create's shape.
fn shape_of(body: &PlanShapeRequest) -> Result<DraftShape, DomainError> {
    Ok(DraftShape {
        sku_id: body.sku_id,
        plan_tier: body.plan_tier.clone(),
        billing_cycle: billing_cycle_of(body.billing_cycle.as_deref())?,
        frequency: frequency_of(body.frequency.as_ref())?,
        plan_tier_override: body.plan_tier_override.unwrap_or(false),
        purchase_min_qty: body.purchase_min_qty,
        purchase_max_qty: body.purchase_max_qty,
        invoice_grouping_key: body.invoice_grouping_key.clone(),
        available_from: body.available_from,
        available_to: body.available_to,
    })
}

/// Parse a patch's shape facet.
///
/// **Nine of these fields cannot be *cleared* through a `PATCH`**, because
/// [`PlanShapePatch`] carries a single `Option` per field and an omitted member
/// and an explicit `null` are therefore the same value. The double option that
/// would separate them is a change to the domain patch type and to
/// `plan_repo::patched_columns`, not to this surface - the surface is correct
/// without it - and it is reported rather than approximated here. What a caller
/// can do meanwhile is replace a value, or discard the whole draft revision
/// through `POST …/abandon`, which keeps the revision number it consumed
/// (D-145).
fn shape_patch(body: &PlanShapeRequest) -> Result<PlanShapePatch, DomainError> {
    Ok(PlanShapePatch {
        sku_id: body.sku_id,
        plan_tier: body.plan_tier.clone(),
        billing_cycle: billing_cycle_of(body.billing_cycle.as_deref())?,
        frequency: frequency_of(body.frequency.as_ref())?,
        plan_tier_override: body.plan_tier_override,
        purchase_min_qty: body.purchase_min_qty,
        purchase_max_qty: body.purchase_max_qty,
        invoice_grouping_key: body.invoice_grouping_key.clone(),
        available_from: body.available_from,
        available_to: body.available_to,
        entitlement_grants: body.entitlement_grants.as_ref().map(read_grants),
        change_contract: body
            .change_contract
            .as_ref()
            .map(read_change_contract)
            .transpose()?,
    })
}

/// Read the wire grant set into the domain value.
///
/// Nothing is refused here: the shape carries no closed vocabulary, and the one
/// check the design states — that every per-phase key names a phase of the
/// plan's own schedule — is a publish rule (`inst-gs-perphase`), because the
/// phase schedule is a *different* facet of the same revision and a `PATCH` may
/// legitimately arrive before it.
fn read_grants(request: &EntitlementGrantsRequest) -> EntitlementGrants {
    EntitlementGrants {
        plan_tier_ref: request.plan_tier_ref.clone(),
        plan_level: GrantSet {
            feature_flags: request.feature_flags.clone().unwrap_or_default(),
            quotas: request.quotas.clone().unwrap_or_default(),
        },
        per_phase: request
            .per_phase
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|(id, set)| {
                (
                    id,
                    GrantSet {
                        feature_flags: set.feature_flags.unwrap_or_default(),
                        quotas: set.quotas.unwrap_or_default(),
                    },
                )
            })
            .collect(),
    }
}

/// Read the wire plan-change contract into the domain value.
///
/// The only thing refused here is a token outside D-113's pair. The edge list
/// and the rank are judged at **publish** rather than at this boundary, because
/// a draft is allowed to be unfinished and because `inst-pc-targets` needs the
/// published-plan lookup this surface does not hold — the same division
/// `inst-pi-required` draws for the proration set.
fn read_change_contract(
    request: &PlanChangeContractRequest,
) -> Result<PlanChangeContract, DomainError> {
    Ok(PlanChangeContract {
        allowed_change_targets: request.allowed_change_targets.clone(),
        comparability_rank: request.comparability_rank,
        usage_counter_on_plan_change: match request.usage_counter_on_plan_change.as_deref() {
            // Omitted is D-113's ratified default, not a missing field: absence
            // **is** `reset`, so there is nothing to refuse.
            None => UsageCounterOnPlanChange::Reset,
            Some(token) => UsageCounterOnPlanChange::ALL
                .iter()
                .copied()
                .find(|candidate| candidate.as_str() == token)
                .ok_or_else(|| {
                    DomainError::InvalidRequest(format!(
                        "change_contract.usage_counter_on_plan_change `{token}` is not one of \
                         reset, carry"
                    ))
                })?,
        },
    })
}

/// Parse a billing-cycle token.
fn billing_cycle_of(raw: Option<&str>) -> Result<Option<BillingCycle>, DomainError> {
    raw.map(|token| {
        wire_token(
            "billing_cycle",
            token,
            BillingCycle::ALL,
            BillingCycle::as_str,
        )
    })
    .transpose()
}

/// Parse a frequency, interval included.
///
/// The interval is required on the custom variant and refused on every other, so
/// the pairing `Frequency` cannot represent is one this surface cannot accept
/// either - rather than being discovered by a `CHECK` and reported as a storage
/// failure.
fn frequency_of(raw: Option<&FrequencyView>) -> Result<Option<Frequency>, DomainError> {
    let Some(view) = raw else {
        return Ok(None);
    };
    let kind = wire_token(
        "frequency.kind",
        &view.kind,
        Frequency::ALL,
        Frequency::as_str,
    )?;
    match kind {
        Frequency::CustomEveryN { .. } => {
            let (Some(n), Some(unit)) =
                (view.custom_interval_n, view.custom_interval_unit.as_deref())
            else {
                return Err(DomainError::InvalidRequest(
                    "frequency `custom_every_n` carries `custom_interval_n` and \
                     `custom_interval_unit`; a token alone does not say what interval was \
                     authored"
                        .to_owned(),
                ));
            };
            Ok(Some(Frequency::CustomEveryN {
                n,
                unit: wire_token(
                    "frequency.custom_interval_unit",
                    unit,
                    CustomIntervalUnit::ALL,
                    CustomIntervalUnit::as_str,
                )?,
            }))
        }
        fixed => {
            if view.custom_interval_n.is_some() || view.custom_interval_unit.is_some() {
                return Err(DomainError::InvalidRequest(format!(
                    "frequency `{}` carries no interval; the interval belongs to \
                     `custom_every_n` alone",
                    view.kind
                )));
            }
            Ok(Some(fixed))
        }
    }
}

/// Parse one phase.
fn phase_of(view: &PlanPhaseView) -> Result<PlanPhase, DomainError> {
    Ok(PlanPhase {
        phase_id: PhaseId::new(view.phase_id),
        kind: wire_token("phases.kind", &view.kind, PhaseKind::ALL, PhaseKind::as_str)?,
        ordinal: view.ordinal,
        converts_to_phase_id: view.converts_to_phase_id.map(PhaseId::new),
        phase_duration_days: view.phase_duration_days,
        display_trial_days: view.display_trial_days,
    })
}

/// Parse one add-on rule. Nothing here can fail: every member is already the
/// type the domain holds, and the rules that judge them run at publish.
fn addon_of(view: AddonRuleView) -> AddonRule {
    AddonRule {
        addon_sku_id: view.addon_sku_id,
        required: view.required,
        min_qty: view.min_qty,
        max_qty: view.max_qty,
        step_qty: view.step_qty,
        price_override_ref: view.price_override_ref,
        depends_on: view.depends_on,
        conflicts_with: view.conflicts_with,
    }
}

/// Parse a descriptor set.
fn descriptor_of(view: DescriptorSetView) -> DescriptorSet {
    DescriptorSet {
        invoice_line_template: view.invoice_line_template,
        gl_code: view.gl_code,
        itemization_rule: view.itemization_rule,
        additional: view.additional,
    }
}

/// Read a wire token back into the domain value that renders it.
///
/// The candidate list is always the enum's own `ALL`, never a copy: a
/// hand-written table is one a variant added later goes missing from while
/// everything still compiles, and what that produces is a perfectly legal
/// request refused as unknown.
fn wire_token<T: Copy>(
    field: &str,
    token: &str,
    candidates: &[T],
    render: fn(T) -> &'static str,
) -> Result<T, DomainError> {
    candidates
        .iter()
        .copied()
        .find(|candidate| render(*candidate) == token)
        .ok_or_else(|| {
            let known: Vec<&str> = candidates.iter().copied().map(render).collect();
            DomainError::InvalidRequest(format!(
                "{field} `{token}` is not one of {}",
                known.join(", ")
            ))
        })
}

#[cfg(test)]
#[path = "plans_tests.rs"]
mod plans_tests;
