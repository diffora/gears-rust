//! Slice 7's window surface: the two reads and the three mutations §5 declares.
//!
//! - `GET /plans/{planId}/coverage` — the coverage/gap report an operator
//!   remediates from (`dod-window-coverage`).
//! - `GET /plans/{planId}/sellability` — the gate's surface, four of six
//!   predicates over one pinned delta (`dod-sellability`; see
//!   [`crate::domain::sellability`] for the split and
//!   [`get_plan_sellability`] for which version it answers from).
//! - `POST /prices/{priceId}/windows`, `PATCH`/`DELETE /price-windows/{windowId}` —
//!   each a publish unit (D-99): validation → pending `CatalogVersion` ref →
//!   plan-subject re-projection, answering **202**.
//!   `infra::window::WindowService` is what they call.
//!
//! **The two reads answer different questions about the same plane, and neither is
//! the other's summary.** Coverage is the *truth* side an operator acts on — every
//! key the plan touches, gaps included, over the rows a publish would produce.
//! Sellability is the *pinned* side a storefront reads — the keys a purchase binds
//! on one market, from the frozen version a consumer may pin, which is a different
//! row set at a different instant. A plan can be fully covered on the truth side
//! and unsellable on the pinned one, for the length of one batching delay.
//!
//! # The three mutations answer 202, and none of them answers 200
//!
//! A publish unit is not consumer-visible until `CatalogVersionPublished` + warm. A
//! 200 would tell the caller the coverage changed *for readers*, which it has not:
//! what changed is the truth side, and the read model still answers the previous
//! pin until the projector catches up. The body carries the **pending** handle for
//! the same reason — there is no committed `CatalogVersion` yet and there will not
//! be until the registry batches (D-47).
//!
//! # Two of the three mutations answer 202 **twice**, and D-62 is why
//!
//! The `DELETE` and a **shortening** `PATCH` are on `inst-mat-registered`'s
//! always-material trigger list, so neither commits on one principal. The first call
//! runs every check, opens a pinned approval unit and changes nothing —
//! `outcome = submitted_for_approval`, the stored interval echoed unmoved, no version
//! handle; the call after an independent approve commits — `outcome = mutated`. The
//! `POST` and a lengthening `PATCH` answer `mutated` on the first call, because they
//! are not on the trigger list and what would make *them* material is a threshold
//! policy G6 lands. [`crate::infra::window`]'s module doc carries the split and the
//! obligation.
//!
//! # The state is `GovernanceState`, coverage GET included
//!
//! The mutations need the window service, which holds the catalog-version registry
//! handle, and that handle is the criterion `api::rest::state` splits the two states
//! on. So the whole module moved rather than growing a second `pub fn router`: the
//! route census keys on `pub fn router` under `src/api/rest/**`, and a router named
//! anything else is precisely the hole `every_mounted_router_is_merged_into_both_censuses`
//! exists to close — G3 found a router that was registered, declared, catalogued for
//! authz and answering 404 on every request because the harness never mounted it.
//! One module, one router, one census entry.
//!
//! # `DELETE` takes no idempotency header, and that is §5's call
//!
//! §5's Idempotency column gives the `POST` a **client idempotency key** and the
//! `PATCH` an **`ETag`**, and leaves the `DELETE` cell empty — so this surface asks
//! for neither there, and the omission is the design set's rather than an oversight
//! to improve on. Reported as a divergence rather than fixed by adding a header:
//! a cancel is not naturally idempotent the way a publish is (a revision has one
//! edge out of `draft`; a window has one edge out of `scheduled`, which is
//! `window_repo::transition`'s narrowed `idempotent_arrival` refusing a second
//! cancellation outright — so a retried `DELETE` is answered `WINDOW_NOT_CANCELLABLE`
//! rather than replaying). That refusal is safe and legible, which is why the
//! divergence is reported and not patched.
//!
//! # Why this route exists when `precheck` already reports coverage
//!
//! `POST …/plans/{planId}/publish` runs the aggregate rule set and its refusal
//! already enumerates every uncovered key. This surface answers a different
//! question, and §5 files it under "operator remediation" for that reason:
//!
//! - **It needs no open draft revision.** `infra::publish::assemble` answers
//!   `NotFound` for a plan with nothing to publish, so a published plan whose
//!   coverage an operator wants to inspect — the ordinary case — cannot be
//!   pre-checked at all.
//! - **It reports state, not violations.** Every key, its intervals, its derived
//!   coverage end and its interior gaps, including the keys that are fine. A
//!   refusal names only what is wrong, which is the wrong shape for "what does
//!   this plan's time axis look like".
//!
//! What it deliberately does **not** report is `AVAILABILITY_OUTSIDE_COVERAGE`.
//! That rule judges the plan's `availableFrom`/`availableTo` against per-key
//! coverage, so it is a finding about the *plan* rather than a property of a key,
//! and the publish report is where it belongs. Naming it here as well would give
//! one violation two surfaces to disagree on.
//!
//! **DIVERGENCE (found 2026-08-05, reported not fixed): a cancel can move a published
//! plan's dates outside its own coverage, and the publish report is not always a
//! surface.** `AvailabilityInsideCoverage` is registered in the **publish** pipeline
//! only, and a window mutation runs the two rules `inst-fg-when` names — so cancelling
//! a plan's *first* window, which is legal whenever a later open-ended one keeps the key
//! covered, moves the coverage **start** past `availableFrom` with nothing re-checking.
//! The textual exclusion is defensible (`inst-fg-when` is step 2 of §3's Future-Gap
//! Detection, written about that section's two rules) and running the publish-time set
//! inside a mutation is not available either: `inst-wc-required` would refuse the very
//! mutation that fixes it.
//!
//! The sentence above — *"the publish report is its surface"* — is **not** the whole
//! answer, and D-128 is why: a retired plan can never publish again and a published plan
//! nobody revises gets no report, so for those the finding has no reader at all. What
//! finishes the argument is the **gate**: predicate (1) evaluates each bound key's
//! coverage at the caller's instant, so over the uncovered interval the plan-market is
//! `not_sellable` whatever its dates say. So the residue is an operator-visible
//! incoherence and not a sales hazard — nothing can be sold into the hole.
//! `sqlite_window_service.rs::a_cancel_can_move_a_published_plan_outside_its_own_coverage`
//! executes both halves.
//!
//! # The row set is the publish check's own
//!
//! [`CANDIDATE_ROW_STATES`] — the plan's `published` rows plus the `draft` rows a
//! publish would produce. Taking the published plane alone would make this
//! surface answer a different question from the one whose failure sent the
//! operator here, and the whole value of a remediation surface is that acting on
//! it makes the publish pass.

use std::sync::Arc;

use axum::extract::{Extension, Path, Query};
use axum::http::HeaderMap;
use axum::{Json, Router, body::Bytes, http::StatusCode};
use bss_pricing_sdk::CatalogVersion;
use chrono::{DateTime, Utc};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::approvals::{ApprovalView, MaterialityView};
use crate::api::rest::auth_context::require_authenticated;
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::plans::{idempotency_key_param, if_match_param};
use crate::api::rest::preconditions;
use crate::api::rest::state::GovernanceState;
use crate::domain::coverage::{self, CoverageReport, KeyCoverage};
use crate::domain::error::DomainError;
use crate::domain::money::CurrencyCode;
use crate::domain::read_model::SubjectRef;
use crate::domain::scope_key::{PlanId, Region};
use crate::domain::sellability::{
    PredicateAnswer, PredicateOutcome, SellabilityFacts, SellabilitySurface,
};
use crate::domain::window::{CoverageEnd, WindowInterval};
use crate::infra::publish::CANDIDATE_ROW_STATES;
use crate::infra::storage::repo::{pin_frontier_repo, price_repo, read_model_repo, window_repo};
use crate::infra::storage::repo_failure;
use crate::infra::window::{PendingApproval, WindowMutationOutcome, WindowMutationReceipt};

/// `OpenAPI` tag applied to the window operations (DE0205 requires a tag and a
/// summary on every registered operation).
const TAG: &str = "BSS Pricing Price Windows";

/// The per-key coverage report (§5, `dod-window-coverage`).
///
/// The literal is repeated in the `OperationBuilder` call below because DE0801
/// validates a **literal** argument and silently passes a `const` one, so the
/// route-shape rule only binds where the literal is; the two spellings are pinned
/// together by `tests/module_test.rs`'s route census.
pub const PLAN_COVERAGE: &str = "/bss-pricing/v1/plans/{planId}/coverage";

/// The scheduling collection of one price row (§5, `POST`).
///
/// The literal is repeated in the `OperationBuilder` call for [`PLAN_COVERAGE`]'s
/// reason: DE0801 validates a literal and silently passes a `const`.
pub const PRICE_WINDOWS: &str = "/bss-pricing/v1/prices/{priceId}/windows";

/// One window, addressed by its own id (§5, `PATCH` and `DELETE`).
///
/// **Not nested under the price row**, which is §5's shape and not a shortcut: a
/// window id is unique on its own, so nesting would put a second identifier in the
/// path that the server would have to check agrees with the first — the arm
/// `require_same_revision` exists for on the price routes, bought for nothing here.
pub const PRICE_WINDOW: &str = "/bss-pricing/v1/price-windows/{windowId}";

/// The sellability surface the joint gate reads (§5, `dod-sellability`).
///
/// The literal is repeated in the `OperationBuilder` call for [`PLAN_COVERAGE`]'s
/// reason: DE0801 validates a literal and silently passes a `const`.
pub const PLAN_SELLABILITY: &str = "/bss-pricing/v1/plans/{planId}/sellability";

/// The three query parameters §5 names, each optional at the type and **required
/// in the handler**.
///
/// Optional here so that an absent or unparseable value is refused through the
/// canonical ladder naming the parameter, rather than by axum's own extractor
/// rejection — which answers a bare `400` with no problem document at all, the
/// shape `a_malformed_plan_id_never_reaches_the_handler` pins for a path segment
/// and which is right there (this gear never answered) and wrong here (it did).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SellabilityQuery {
    /// The instant to evaluate at, RFC 3339.
    pub at: Option<String>,
    /// The currency half of the bound market, ISO 4217.
    pub currency: Option<String>,
    /// The region half of the bound market.
    pub region: Option<String>,
}

/// One canonical scope key's coverage, as the wire renders it.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct KeyCoverageView {
    /// The key's canonical rendering — the eight axes, pipe-separated, exactly as
    /// the violation subjects of a publish refusal name it.
    ///
    /// The **same string** on both surfaces on purpose: an operator who was told
    /// `WINDOW_COVERAGE_MISSING` on a key has to be able to find that key here
    /// without transcribing eight axes.
    pub scope_key: String,
    /// Does a billable row of this plan sit on the key? `false` on a key only
    /// history occupies, which is reported and never refused.
    pub required: bool,
    /// Does the key hold an **active or scheduled** window — `inst-wc-required`'s
    /// own predicate?
    ///
    /// Deliberately not derivable from [`KeyCoverageView::coverage_end`]: a key
    /// whose only window expired has a coverage end and no live window, and it is
    /// the second of those that decides whether a publish passes.
    pub covered: bool,
    /// How far the key's coverage runs.
    pub coverage_end: CoverageEndView,
    /// Every window of the key, ordered, **every state included** — a cancelled
    /// one among them, because "why did this key lose its successor" is the
    /// question this surface exists to answer.
    pub intervals: Vec<WindowIntervalView>,
    /// The uncovered intervals **between** two of the key's windows.
    ///
    /// A void with no successor is never here: `inst-fg-detect` is an interior
    /// check and cannot see one, which is `inst-fg-trailing`'s whole reason for
    /// existing.
    pub interior_gaps: Vec<CoverageGapView>,
}

/// Where a key's coverage stops, as the three-armed answer the domain carries.
///
/// `kind` and a nullable instant rather than a bare nullable instant, because
/// `null` would have to serve two **opposite** answers: a key covered forever
/// passes every horizon predicate and a key covered not at all fails all of them.
/// The read-model payload declares the same shape for the same reason.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct CoverageEndView {
    /// `uncovered` | `ends` | `open_ended`.
    pub kind: String,
    /// The exclusive instant coverage ends at, non-null only on `ends`.
    pub at: Option<DateTime<Utc>>,
}

impl From<CoverageEnd> for CoverageEndView {
    fn from(end: CoverageEnd) -> Self {
        Self {
            kind: end.as_str().to_owned(),
            at: end.at(),
        }
    }
}

/// One window interval of a key.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct WindowIntervalView {
    /// Inclusive start, UTC.
    pub effective_from: DateTime<Utc>,
    /// Exclusive end, UTC; `null` is open-ended.
    pub effective_to: Option<DateTime<Utc>>,
    /// `scheduled` | `active` | `expired` | `cancelled`.
    pub state: String,
}

impl From<&WindowInterval> for WindowIntervalView {
    fn from(interval: &WindowInterval) -> Self {
        Self {
            effective_from: interval.effective_from,
            effective_to: interval.effective_to,
            state: interval.state.as_str().to_owned(),
        }
    }
}

/// One uncovered interval, half-open `[gapStart, gapEnd)`.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct CoverageGapView {
    /// Inclusive start of the hole — the instant the preceding window stopped
    /// covering.
    pub gap_start: DateTime<Utc>,
    /// Exclusive end — the instant the next window starts.
    pub gap_end: DateTime<Utc>,
}

/// The body of `POST /prices/{priceId}/windows`.
///
/// `effectiveTo` is `Option` and its absence means **open-ended**, which is the
/// store's own reading: a window with no end covers every instant from its start.
/// It is deliberately not a required field with a sentinel — an open-ended window
/// is the ordinary case for a plan's first window, and a sentinel instant would be
/// an instant somebody eventually compares against.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct ScheduleWindowRequest {
    /// Inclusive start, UTC, strictly in the future (`inst-ws-future-start`,
    /// D-63).
    pub effective_from: DateTime<Utc>,
    /// Exclusive end, UTC; absent is open-ended.
    pub effective_to: Option<DateTime<Utc>>,
    /// Why the window was scheduled — free text the store keeps with the row.
    pub reason_code: String,
}

/// The body of `PATCH /price-windows/{windowId}`.
///
/// One field, and the `Option` carries the same meaning it does on the schedule:
/// absent means **remove the bound**, making the window open-ended. That is an
/// extension rather than a shortening, so it passes the trailing-void floor by
/// construction — a key covered forever satisfies every floor there is.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct AdjustWindowRequest {
    /// The new exclusive end, UTC; absent makes the window open-ended.
    pub effective_to: Option<DateTime<Utc>>,
}

/// What a window mutation answers with — the pending handle and the row as stored.
///
/// It carries no `catalogVersion`, and that absence is the 202: the committed
/// number does not exist yet. A field for it would be a field every success left
/// null, which is the shape D-161 refuses one artifact over.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct WindowMutationView {
    /// The window the mutation created or moved.
    pub window_id: Uuid,
    /// The plan whose subject re-projects.
    pub plan_id: Uuid,
    /// The price row the window is bound to.
    pub price_id: Uuid,
    /// The plan revision the re-projection freezes — the current one (D-99).
    pub revision: u64,
    /// The registry handle the projector resolves into a `CatalogVersion`.
    ///
    /// **`null` on the controlled arm**, and that is the one thing about it worth
    /// stating: a D-62 mutation that opened a unit requested no version, because it
    /// wrote nothing that could need addressing. A handle there would name an
    /// assignment nothing will ever commit — the orphan D-156 moves the request
    /// *inside* the transaction to avoid. `PublishReceiptView` types it the same way
    /// for the same reason.
    pub pending_version_ref: Option<String>,
    /// Inclusive start, UTC.
    pub effective_from: DateTime<Utc>,
    /// Exclusive end, UTC; `null` is open-ended.
    pub effective_to: Option<DateTime<Utc>>,
    /// `scheduled` | `active` | `expired` | `cancelled`.
    pub state: String,
}

impl From<WindowMutationReceipt> for WindowMutationView {
    fn from(receipt: WindowMutationReceipt) -> Self {
        Self {
            window_id: receipt.window_id,
            plan_id: receipt.plan_id.get(),
            price_id: receipt.price_id,
            revision: receipt.revision,
            pending_version_ref: Some(receipt.pending_version_ref),
            effective_from: receipt.effective_from,
            effective_to: receipt.effective_to,
            state: receipt.state.as_str().to_owned(),
        }
    }
}

impl From<&PendingApproval> for WindowMutationView {
    /// The window as the controlled arm leaves it — **unmoved**, and with no handle.
    fn from(pending: &PendingApproval) -> Self {
        Self {
            window_id: pending.window_id,
            plan_id: pending.plan_id.get(),
            price_id: pending.price_id,
            revision: pending.revision,
            pending_version_ref: None,
            effective_from: pending.effective_from,
            effective_to: pending.effective_to,
            state: pending.state.as_str().to_owned(),
        }
    }
}

/// The wire token for the arm that mutated.
///
/// Its sibling is `api::rest::publish`'s `OUTCOME_SUBMITTED`, imported rather than
/// re-spelled: one word for one act across both surfaces.
const OUTCOME_MUTATED: &str = "mutated";

/// What a window mutation did: it ran, or it opened a unit and stood still.
///
/// One response type across the two arms, for the reason `PublishOutcomeView` gives —
/// a generated client has one thing to deserialize and reads `outcome` to know which
/// arm it got. Here both arms are **202**, which is D-99 rather than an oversight: a
/// mutation that ran is not consumer-visible until its version commits and warms, and
/// a mutation that opened a unit has not run, so neither can honestly answer 200.
/// `outcome` is therefore the only discriminator, which is exactly why it is a field
/// and not an inference from which halves are null.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct WindowMutationOutcomeView {
    /// `mutated` | `submitted_for_approval`.
    pub outcome: String,
    /// The window as it stands **after** the call — on both arms, and on the
    /// controlled one it is the row unchanged. A caller therefore reads the truth of
    /// the interval either way rather than having to infer it from the outcome.
    pub window: WindowMutationView,
    /// Why a second principal was required, on the controlled arm. `null` on the arm
    /// that mutated: nothing about a schedule or an extension is material yet, and a
    /// verdict there would be a verdict nobody evaluated.
    pub materiality: Option<MaterialityView>,
    /// The pinned unit this call opened, on the controlled arm.
    pub approval: Option<ApprovalView>,
}

/// The plan's whole time axis, one entry per canonical scope key.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct PlanCoverageView {
    /// The plan the report is about.
    pub plan_id: Uuid,
    /// Every key: the ones a billable row sits on, and the ones only windows
    /// mention. Ordered by the key's canonical rendering, so two reads of one
    /// state read identically.
    pub keys: Vec<KeyCoverageView>,
}

/// One predicate's answer, as the wire renders it.
///
/// **`answer` is a token and there is no boolean anywhere in this document**, which
/// is D-99 reaching the wire: what a consumer is given is intervals, states, a
/// derived coverage end and per-predicate tokens, so no field of it is an answer to
/// a question about the reader's clock.
///
/// `detail` and `owed_to` are both nullable and never both set: the first belongs to
/// `failed` and the second to `not_evaluable`. Two fields rather than one, because
/// the whole of D-167 clause (3) is that a consumer can tell *"this predicate is
/// false"* from *"this version cannot evaluate this predicate"* — and one merged
/// message field would have made the difference a matter of reading English.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct PredicateAnswerView {
    /// The design set's own number for the predicate, 1 to 6.
    pub ordinal: u8,
    /// The predicate's name.
    pub predicate: String,
    /// `satisfied` | `failed` | `not_evaluable`.
    pub answer: String,
    /// Why the predicate is false, on `failed` and nowhere else.
    pub detail: Option<String>,
    /// What owes the fact this predicate reads, on `not_evaluable` and nowhere
    /// else — the slice, or the version.
    pub owed_to: Option<String>,
}

impl From<&PredicateOutcome> for PredicateAnswerView {
    fn from(outcome: &PredicateOutcome) -> Self {
        let (detail, owed_to) = match &outcome.answer {
            PredicateAnswer::Satisfied => (None, None),
            PredicateAnswer::Failed { detail } => (Some(detail.clone()), None),
            PredicateAnswer::NotEvaluable { owed_to } => (None, Some((*owed_to).to_owned())),
        };
        Self {
            ordinal: outcome.predicate.ordinal(),
            predicate: outcome.predicate.as_str().to_owned(),
            answer: outcome.answer.as_str().to_owned(),
            detail,
            owed_to,
        }
    }
}

/// One bound key's sellability: its frozen window facts and its per-key
/// predicates.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct KeySellabilityView {
    /// The key's canonical rendering — the **same string** the coverage report and
    /// a publish refusal use, so an operator matches keys across the three
    /// surfaces without transcribing eight axes.
    pub scope_key: String,
    /// How far this key's coverage runs, as the three-armed answer.
    pub coverage_end: CoverageEndView,
    /// The key's frozen intervals and their states, exactly as the version froze
    /// them — what lets a consumer re-derive the predicate at another instant
    /// without a second call.
    pub intervals: Vec<WindowIntervalView>,
    /// One answer per per-key predicate.
    pub predicates: Vec<PredicateAnswerView>,
}

/// The plan's sellability on one market at one instant.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct PlanSellabilityView {
    /// The plan asked about.
    pub plan_id: Uuid,
    /// The instant the answer is about — the caller's own, echoed so a stored
    /// response cannot be mistaken for one about a different moment.
    pub at: DateTime<Utc>,
    /// The currency half of the bound market.
    pub currency: String,
    /// The region half of the bound market.
    pub region: String,
    /// The pin-eligible version the answer was read from, `null` when none carries
    /// this plan.
    pub catalog_version: Option<u64>,
    /// The D-94 conjunction: `sellable` | `not_sellable` | `not_evaluable`.
    ///
    /// **`not_evaluable` is not a yes.** With two of the six predicates unanswerable
    /// by any version this gear can project, it is what a fully-covered, in-dates,
    /// published plan answers today: the gate saying it is not yet a gate.
    pub verdict: String,
    /// The plan-level predicates.
    pub predicates: Vec<PredicateAnswerView>,
    /// Every key the purchase binds on this market, eligibility-resolved.
    /// **Empty means not sellable**, never vacuously sellable.
    pub keys: Vec<KeySellabilityView>,
}

impl From<&SellabilitySurface> for PlanSellabilityView {
    fn from(surface: &SellabilitySurface) -> Self {
        Self {
            plan_id: surface.plan_id.get(),
            at: surface.at,
            currency: surface.currency.as_str().to_owned(),
            region: surface.region.as_str().to_owned(),
            catalog_version: surface.catalog_version.map(CatalogVersion::get),
            verdict: surface.plan_market_verdict().as_str().to_owned(),
            predicates: surface
                .plan_answers
                .iter()
                .map(PredicateAnswerView::from)
                .collect(),
            keys: surface
                .keys
                .iter()
                .map(|key| KeySellabilityView {
                    scope_key: key.scope_key.to_string(),
                    coverage_end: CoverageEndView::from(key.coverage_end),
                    intervals: key.intervals.iter().map(WindowIntervalView::from).collect(),
                    predicates: key.answers.iter().map(PredicateAnswerView::from).collect(),
                })
                .collect(),
        }
    }
}

impl From<&KeyCoverage> for KeyCoverageView {
    fn from(entry: &KeyCoverage) -> Self {
        Self {
            scope_key: entry.scope_key().to_string(),
            required: entry.required,
            covered: entry.has_live_window(),
            coverage_end: CoverageEndView::from(entry.coverage_end()),
            intervals: entry
                .windows
                .intervals
                .iter()
                .map(WindowIntervalView::from)
                .collect(),
            interior_gaps: entry
                .interior_gaps()
                .into_iter()
                .map(|(gap_start, gap_end)| CoverageGapView { gap_start, gap_end })
                .collect(),
        }
    }
}

/// Build the Axum router for the window surface and register its operations.
///
/// The declared error responses are the ones this path can produce: 401, 403, 503
/// (the PDP unreachable or the store unavailable, both fail closed) and 500. **No
/// 400** — the only input is a path `Uuid` axum itself rejects before the handler,
/// and there is no body and no query. **No 404**: a plan with no rows and no
/// windows is answered `200` with an empty key list, which is a state and not an
/// absent resource, and a plan outside the caller's scope reads the same way
/// rather than leaking whose plans exist. **No 422** — the design set's 422s are
/// architectural and reach the wire as 400s carrying their code
/// (`infra::error_mapping`).
pub fn router(state: Arc<GovernanceState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-pricing/v1/plans/{planId}/coverage")
        .operation_id("bss_pricing.get_plan_coverage")
        .summary("Read a plan's per-scope-key window coverage and gap report")
        .description(
            "The plan's time axis, one entry per canonical scope key: the key's window intervals \
             and states, its derived coverage end, whether it holds an active or scheduled window \
             at all, and every uncovered interval between two of its windows. The row set is the \
             publish check's own - the plan's `published` rows plus the `draft` rows a publish \
             would produce - so a key this report calls uncovered is exactly a key \
             `WINDOW_COVERAGE_MISSING` would name, under the same canonical rendering. A void \
             with no successor is never reported here: gap detection is an interior check and \
             cannot see one by construction. A plan with no rows and no windows is answered `200` \
             with an empty list; a plan outside the caller's scope reads the same way. Gates on \
             `plan` x `read`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan whose coverage is reported.")
        .handler(get_plan_coverage)
        .json_response_with_schema::<PlanCoverageView>(
            openapi,
            StatusCode::OK,
            "The plan's per-key coverage and gap report.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    let router = OperationBuilder::post("/bss-pricing/v1/prices/{priceId}/windows")
        .operation_id("bss_pricing.schedule_price_window")
        .summary("Schedule a window on a price row")
        .description(
            "Schedules a `[effectiveFrom, effectiveTo)` window on the row's canonical scope key \
             and answers `202` with the pending `CatalogVersion` handle. It is a **publish unit** \
             (D-99): the mutation re-projects the plan subject and is consumer-visible only at \
             `CatalogVersionPublished` + warm, so a `200` would claim the coverage changed for \
             readers when it has not. `effectiveFrom` must be strictly in the future \
             (`WINDOW_START_IN_PAST`), the interval must not intersect a scheduled or active \
             sibling on the same key (`WINDOW_OVERLAP`, 409 - siblings include every price row of \
             the plan on that key, not just this one), and the resulting interval set must open no \
             hole between two of the key's windows (`WINDOW_GAP`). Adjacency is legal: \
             `effectiveTo = next.effectiveFrom` is not a gap, the intervals being half-open. An \
             `Idempotency-Key` header is required (D-171, and S5's own idempotency column for this \
             surface). \
             \
             **A schedule is not on `inst-mat-registered`'s trigger list, so what decides whether \
             it needs a second principal is the tenant's approval-threshold policy.** It moves an \
             interval and no money, so its per-row delta is zero and reaches no bar - which leaves \
             `inst-mat-percurrency`'s fail-safe half as the only rule that can answer: a tenant \
             with no configured entry for the row's currency gets `outcome = \
             submitted_for_approval`, a pinned unit and **no window at all**. With an entry, \
             `outcome = mutated` and the body names the pending version handle. Gates on `plan` x \
             `write`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("priceId", "The price row the window applies to.")
        .param(idempotency_key_param())
        .json_request::<ScheduleWindowRequest>(openapi, "The interval and its reason code.")
        .handler(schedule_window)
        .json_response_with_schema::<WindowMutationOutcomeView>(
            openapi,
            StatusCode::ACCEPTED,
            "The window is scheduled, or a unit was opened and nothing was written; `outcome` says \
             which.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = mounted_window_mutations(router, openapi);

    let router = OperationBuilder::get("/bss-pricing/v1/plans/{planId}/sellability")
        .operation_id("bss_pricing.get_plan_sellability")
        .summary("Read a plan's sellability on one market at one instant")
        .description(
            "The sellability gate's surface, answered from the tenant's pin-eligible frontier. It \
             exposes **intervals, states and a derived coverage end per bound key, and never a \
             point-in-time boolean**: `at` is the caller's instant, so a consumer re-derives the \
             answer at another instant from the same frozen version and the time-driven window \
             transitions need no re-projection. Four of the six predicates are answered here - the \
             active-window-plus-coverage-horizon predicate, committed-version addressability, the \
             availability dates and the plan lifecycle state; the GA-gate flags and the registry \
             `sellable` flag answer `not_evaluable` and name the slice that owes each, so a \
             consumer can tell a false predicate from an unanswerable one. The verdict is the \
             conjunction over **every** key the purchase binds on the market, eligibility-resolved \
             and with grandfathered generations excluded: one failing key makes the plan-market \
             not sellable, never partially sellable, and a market the plan binds no key on is not \
             sellable either. `not_evaluable` is **not** a yes. All three query parameters are \
             required. A plan no pin-eligible version carries is answered `200` with the \
             committed-version predicate failed, which is also what a plan outside the caller's \
             scope reads. Gates on `plan` x `read`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan whose sellability is answered.")
        .query_param(
            "at",
            true,
            "The instant to evaluate at, RFC 3339 (required). Percent-encode a numeric offset: `+` \
             is a space under form-urlencoding, so `...+00:00` written literally arrives mangled \
             and is refused. `...Z` needs no encoding.",
        )
        .query_param(
            "currency",
            true,
            "The bound market's currency, ISO 4217 (required)",
        )
        .query_param("region", true, "The bound market's region (required)")
        .handler(get_plan_sellability)
        .json_response_with_schema::<PlanSellabilityView>(
            openapi,
            StatusCode::OK,
            "The plan's per-key sellability and the plan-market verdict.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    // D-178's edge, applied where the routes are rather than where the routers
    // merge, so it travels with them: a mutation reachable without it cannot build
    // an `AuditStamp`, and `require_correlation` answers 500 rather than minting a
    // second value per record. The two GETs write nothing and need none, but
    // the layer is per-router and costs them only a header read.
    router
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}

/// Register the two window mutations D-62 controls — the `PATCH` and the `DELETE`.
///
/// Split out of [`router`] because the two descriptions are long enough to put that
/// function over the 200-line lint, and split **here** rather than anywhere else
/// because these are the two operations that share a shape: both answer 202 twice,
/// both carry `inst-mat-registered`'s two-person control, and both hand back
/// [`WindowMutationOutcomeView`]. The five literals stay literals — DE0801 validates a
/// literal argument and silently passes a `const` one.
fn mounted_window_mutations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::patch("/bss-pricing/v1/price-windows/{windowId}")
        .operation_id("bss_pricing.adjust_price_window")
        .summary("Move a window's future effectiveTo")
        .description(
            "Moves a **future** `effectiveTo` - shorten, extend, or drop the bound entirely by \
             omitting the field - under the `If-Match` precondition, and answers `202`. A \
             **publish unit** (D-99); a shorten is additionally always-material (D-62). Three \
             things are frozen and each answers `WINDOW_HISTORICAL_IMMUTABLE` (409): an `expired` \
             or `cancelled` window, a stored end that has already passed, and a target end that is \
             not in the future - the last two because rewriting an end that has passed rewrites \
             what was chargeable. A move that would leave the key uncovered with no successor is \
             `WINDOW_TRAILING_VOID`: the key must stay covered through the later of its current \
             coverage end and now plus the longest billing cycle sold on it. That check is \
             **fail-closed with no exemption** (D-182) - the D-79 Subscriptions lane that would \
             decide the exemption has no client, no contract type and no counterpart gear here, and \
             D-131 makes a lane that cannot answer the subscribers-present case. \
             \
             **A shortening move takes two principals** (D-62, `inst-mat-registered`): the first \
             call opens a pinned approval unit, answers `202` with \
             `outcome = submitted_for_approval` and **changes nothing** - the stored end is echoed \
             unmoved and no version handle is issued - and the move commits on the call made after \
             an independent principal approves that unit. An **extension** removes no coverage and \
             is not on the trigger list, so it is the threshold policy that decides it: with an \
             entry for the row's currency it commits on the first call (`outcome = mutated`), and \
             with none `inst-mat-failsafe` answers and it opens a unit like a shorten. The \
             sentence here used to say an extension commits on the first call full stop, which was \
             true only while no policy could exist. Gates on `plan` x `write`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("windowId", "The window whose end is moved.")
        .param(if_match_param(
            "On a window route the tag is the window row's **own** version and nothing else, \
             because the path addresses one window by id - the same rule D-141 states for a price \
             row, and never the plan's tag.",
        ))
        .json_request::<AdjustWindowRequest>(openapi, "The new end, or nothing for open-ended.")
        .handler(adjust_window)
        .json_response_with_schema::<WindowMutationOutcomeView>(
            openapi,
            StatusCode::ACCEPTED,
            "The end moved, or a unit was opened and nothing moved; `outcome` says which.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    OperationBuilder::delete("/bss-pricing/v1/price-windows/{windowId}")
        .operation_id("bss_pricing.cancel_price_window")
        .summary("Cancel a not-yet-active window")
        .description(
            "Cancels a `scheduled` window and answers `202`, emitting `PriceWindowCancelled`. A \
             **publish unit** (D-99) and always-material (D-62). It is a **state flip, never a \
             deletion**: the store refuses `DELETE` on the row unconditionally, because a \
             cancelled schedule is a fact an auditor is entitled to see - which is also why a \
             second cancellation of one window is refused `WINDOW_NOT_CANCELLABLE` (409) rather \
             than answered `Ok`. Any state but `scheduled` gets that same refusal, and an active \
             window is shortened through its `effectiveTo` instead, which is the operation the \
             state machine leaves. A cancel that would leave the key uncovered with no successor is \
             `WINDOW_TRAILING_VOID`, fail-closed with no exemption (D-182). It takes **no** \
             `Idempotency-Key` and **no** `If-Match`: S5's idempotency column for this surface is \
             empty, and the refusal of a second cancellation is what stands in for one. \
             \
             **It takes two principals** (D-62, `inst-mat-registered`): the first call runs every \
             check, opens a pinned approval unit and answers `202` with \
             `outcome = submitted_for_approval` while the window **stays `scheduled`** - this is \
             the act D-62 exists to stop one operator performing alone, a cancelled successor \
             silently reverting an approved price change. The flip happens on the call made after \
             an independent principal approves the unit, and its audit record names that unit. \
             Gates on `plan` x `write`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("windowId", "The window to cancel.")
        .handler(cancel_window)
        .json_response_with_schema::<WindowMutationOutcomeView>(
            openapi,
            StatusCode::ACCEPTED,
            "The window is cancelled, or a unit was opened and nothing moved; `outcome` says which.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// The `plan x write` gate, spelled once for the three mutating routes.
///
/// `resource_id` is the **plan**, not the price row or the window, because that is
/// the object the catalog's endpoint map puts the action on: `plan x write` is a
/// grant over a plan, and asking the PDP about a window id it has no label for
/// would fail closed for every caller. Resolving the plan is therefore part of the
/// gate rather than part of the mutation, which is why both window handlers below
/// read their subject's key before they ask.
async fn window_write_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    plan_id: PlanId,
    tenant: Uuid,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(tenant),
        /* resource_id */ Some(plan_id.get()),
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)
}

/// Resolve the plan a subject belongs to, under a scope the caller already holds.
///
/// # Why the mutating routes ask the PDP twice, and why both asks are `write`
///
/// `PATCH` and `DELETE` address `/price-windows/{windowId}` — §5's shape, and the
/// right one, a window id being unique on its own. But the `plan x write` gate
/// needs a **plan** id as its `resource_id`, because that is the object the authz
/// catalog's endpoint map puts the action on; asking the PDP about a window id it
/// carries no label for would fail closed for every caller. So something has to
/// turn one id into the other before the per-plan gate can be asked.
///
/// That lookup is a read of the tenant's catalog, so it must run under a compiled
/// scope: unscoped, a principal could tell from the difference between `404` and
/// `403` which window ids exist in tenants they cannot see. The scope it runs under
/// is a **tenant-level `plan x write`** — `resource_id: None` — and *not* a
/// `plan x read`, for two reasons that point the same way. It is the honest
/// question ("may this principal write plans here at all?"), and it keeps the
/// route's **first** PDP ask equal to the pair the catalog names for it, which is
/// the property `every_route_asks_the_catalogued_pair` exists to hold. A route
/// whose first ask were `read` would behave identically under a fixture granting
/// both and one granting neither, and only what it *asked* would separate them.
///
/// The second ask then narrows to the resolved plan. Two asks of one pair is one
/// gate evaluated twice, never a gate widened.
async fn resolve_plan(
    state: &GovernanceState,
    scope: &AccessScope,
    tenant: Uuid,
    subject: Lookup,
) -> Result<PlanId, CanonicalError> {
    let conn = state.db.conn().map_err(|e| {
        CanonicalError::internal(format!("bss-pricing: window lookup: {e}")).create()
    })?;
    let key = match subject {
        Lookup::Price(price_id) => price_repo::load_scope_key(&conn, scope, tenant, price_id)
            .await
            .map_err(|e| CanonicalError::from(repo_failure(&e)))?
            .ok_or_else(|| {
                CanonicalError::from(crate::domain::error::DomainError::NotFound {
                    subject: "price row".to_owned(),
                    id: price_id.to_string(),
                })
            })?,
        Lookup::Window(window_id) => {
            window_repo::find(&conn, scope, tenant, window_id)
                .await
                .map_err(|e| CanonicalError::from(repo_failure(&e)))?
                .ok_or_else(|| {
                    CanonicalError::from(crate::domain::error::DomainError::NotFound {
                        subject: "price window".to_owned(),
                        id: window_id.to_string(),
                    })
                })?
                .scope_key
        }
    };
    Ok(key.plan_id())
}

/// The tenant-level `plan x write` scope the subject lookup runs under.
async fn tenant_write_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant: Uuid,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(tenant),
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)
}

/// Which identifier the path gave us.
#[derive(Clone, Copy, Debug)]
enum Lookup {
    Price(Uuid),
    Window(Uuid),
}

/// `POST /prices/{priceId}/windows`: schedule a window, answering 202.
async fn schedule_window(
    Extension(state): Extension<Arc<GovernanceState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    correlation: Option<Extension<CorrelationId>>,
    Path(price_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<WindowMutationOutcomeView>), CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(correlation)?;
    // The key is required (D-171) and is read before anything is resolved, so a
    // caller who omitted it is told that rather than being told a price id does
    // not exist.
    let _key = preconditions::idempotency_key(&headers)?;
    let request: ScheduleWindowRequest = preconditions::parse_body(&body)?;
    let tenant = ctx.subject_tenant_id();
    let coarse = tenant_write_scope(&enforcer, &ctx, tenant).await?;
    let plan_id = resolve_plan(&state, &coarse, tenant, Lookup::Price(price_id)).await?;
    let scope = window_write_scope(&enforcer, &ctx, plan_id, tenant).await?;

    let stamp = crate::api::rest::auth_context::audit_stamp(&ctx, Utc::now(), correlation);
    let outcome = state
        .windows
        .schedule(
            &ctx,
            &scope,
            tenant,
            price_id,
            // Server-minted, like a price row's id and for the same reason: the
            // surface has to be able to name the window in its answer before the
            // row is durable.
            Uuid::now_v7(),
            request.effective_from,
            request.effective_to,
            request.reason_code,
            stamp,
        )
        .await?;
    answer(&state, &scope, tenant, outcome, stamp).await
}

/// `PATCH /price-windows/{windowId}`: move a future end, answering 202.
async fn adjust_window(
    Extension(state): Extension<Arc<GovernanceState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    correlation: Option<Extension<CorrelationId>>,
    Path(window_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<WindowMutationOutcomeView>), CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(correlation)?;
    // Required and parsed here; the comparison itself is the store's, inside the
    // writing transaction (D-176 — a tag compared out here is a hint).
    let _expected = preconditions::if_match(&headers)?;
    let request: AdjustWindowRequest = preconditions::parse_body(&body)?;
    let tenant = ctx.subject_tenant_id();
    let coarse = tenant_write_scope(&enforcer, &ctx, tenant).await?;
    let plan_id = resolve_plan(&state, &coarse, tenant, Lookup::Window(window_id)).await?;
    let scope = window_write_scope(&enforcer, &ctx, plan_id, tenant).await?;

    let stamp = crate::api::rest::auth_context::audit_stamp(&ctx, Utc::now(), correlation);
    let outcome = state
        .windows
        .adjust_effective_to(&ctx, &scope, tenant, window_id, request.effective_to, stamp)
        .await?;
    answer(&state, &scope, tenant, outcome, stamp).await
}

/// `DELETE /price-windows/{windowId}`: cancel a scheduled window, answering 202.
///
/// **No header preconditions at all**, which is §5's Idempotency column for this
/// surface — see the module doc for why that is reported as a divergence rather
/// than improved on.
async fn cancel_window(
    Extension(state): Extension<Arc<GovernanceState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    correlation: Option<Extension<CorrelationId>>,
    Path(window_id): Path<Uuid>,
) -> Result<(StatusCode, Json<WindowMutationOutcomeView>), CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(correlation)?;
    let tenant = ctx.subject_tenant_id();
    let coarse = tenant_write_scope(&enforcer, &ctx, tenant).await?;
    let plan_id = resolve_plan(&state, &coarse, tenant, Lookup::Window(window_id)).await?;
    let scope = window_write_scope(&enforcer, &ctx, plan_id, tenant).await?;

    let stamp = crate::api::rest::auth_context::audit_stamp(&ctx, Utc::now(), correlation);
    let outcome = state
        .windows
        .cancel(&ctx, &scope, tenant, window_id, stamp)
        .await?;
    answer(&state, &scope, tenant, outcome, stamp).await
}

/// Render a window mutation's outcome, opening the unit the controlled arm needs.
///
/// **The unit is opened here rather than inside the mutation**, and the split is
/// D-176's the other way up: what has to be inside the mutating transaction is the
/// *decision* — is this act on D-62's trigger list, and does an approved unit cover it
/// — because that decision reads the stored row. Opening the unit writes nothing to the
/// window plane, so it can be a second transaction; and it has to be, because
/// `ApprovalService::submit_window_mutation` takes the pin inside its own transaction,
/// which is what makes the pin a pin rather than a value handed between two.
///
/// It runs under the **same** `plan × write` scope the mutation ran under: opening the
/// unit is part of performing the act, not a second authority, and asking a different
/// pair here would make the gate depend on which arm the caller happened to get.
///
/// Both arms answer **202**; [`WindowMutationOutcomeView`] says why.
async fn answer(
    state: &GovernanceState,
    scope: &AccessScope,
    tenant: Uuid,
    outcome: WindowMutationOutcome,
    stamp: crate::domain::audit::AuditStamp,
) -> Result<(StatusCode, Json<WindowMutationOutcomeView>), CanonicalError> {
    match outcome {
        WindowMutationOutcome::Committed(receipt) => Ok((
            StatusCode::ACCEPTED,
            Json(WindowMutationOutcomeView {
                outcome: OUTCOME_MUTATED.to_owned(),
                window: WindowMutationView::from(*receipt),
                materiality: None,
                approval: None,
            }),
        )),
        WindowMutationOutcome::SubmittedForApproval(pending) => {
            let view = MaterialityView::from(pending.verdict);
            let record = state
                .approvals
                .submit_window_mutation(
                    scope,
                    tenant,
                    pending.window_id,
                    // The **price row**, which is how the unit resolves its plan and
                    // its held key: a schedule's window does not exist yet, so the
                    // window id names nothing readable at this point.
                    pending.price_id,
                    // Server-minted per request, exactly as the publish route mints
                    // it: the caller names the window, never the unit.
                    Uuid::now_v7(),
                    serde_json::to_value(&view).map_err(|e| {
                        CanonicalError::from(DomainError::Internal(format!(
                            "cannot render the materiality verdict: {e}"
                        )))
                    })?,
                    stamp,
                    // The subject the mutation resolved. For a schedule it is the
                    // act itself and carries no window id, so the call made after
                    // the approve reproduces it (D-184 clause (2)).
                    &pending.subject_ref,
                )
                .await?;
            Ok((
                StatusCode::ACCEPTED,
                Json(WindowMutationOutcomeView {
                    outcome: crate::api::rest::publish::OUTCOME_SUBMITTED.to_owned(),
                    window: WindowMutationView::from(pending.as_ref()),
                    materiality: Some(view),
                    approval: Some(ApprovalView::from(&record)),
                }),
            ))
        }
    }
}

/// `GET /plans/{planId}/coverage`: the per-key coverage report.
///
/// `owner_tenant_id` is `None` — this is a read, so the PDP derives the scope from
/// the subject and its roles rather than trusting a caller-supplied tenant, and
/// the compiled scope is the SQL filter. `resource_id` names the plan, which is
/// what makes the gate answerable per plan rather than per tenant, and
/// `require_constraints = true` so an unconstrained allow fail-closes instead of
/// exposing every tenant's catalog. The pair is `plan` x `read`, the same one
/// `list_plan_prices` asks for, and **no new authz vocabulary**: the catalog
/// already carries every label and action this needs.
async fn get_plan_coverage(
    Extension(state): Extension<Arc<GovernanceState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(plan_id): Path<Uuid>,
) -> Result<Json<PlanCoverageView>, CanonicalError> {
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
    let conn = state.db.conn().map_err(|e| {
        CanonicalError::internal(format!("bss-pricing: plan coverage: {e}")).create()
    })?;

    let report = plan_coverage(&conn, &scope, tenant, plan_id).await?;

    Ok(Json(PlanCoverageView {
        plan_id: plan_id.get(),
        keys: report.keys.iter().map(KeyCoverageView::from).collect(),
    }))
}

/// The plan's coverage, read from the store.
///
/// **The billable key set and the window plane are read separately and neither
/// filters the other**, which is `coverage::check`'s own contract: a key a
/// billable row sits on is reported even with no window (uncovered), and a key
/// only a window mentions is reported too, because "why did this key lose its
/// successor" is a question about a plane rather than about a row.
///
/// The window read is `window_repo::list_for_plan` taken whole — every state, over
/// every price row of the plan whatever its lifecycle state — for the reason
/// `infra::publish::window_plane` states at length: coverage is validation over a
/// hypothetical, and the readers that restrict to `PROJECTED_ROW_STATES` are the
/// ones asserting facts to a consumer.
async fn plan_coverage(
    runner: &impl DBRunner,
    scope: &toolkit_db::secure::AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
) -> Result<CoverageReport, CanonicalError> {
    let rows = price_repo::load_for_plan(runner, scope, tenant, plan_id, CANDIDATE_ROW_STATES)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    let billable: Vec<_> = rows
        .iter()
        .filter(|record| coverage::is_billable(record))
        .map(|record| record.scope_key.clone())
        .collect();

    let records = window_repo::list_for_plan(runner, scope, tenant, plan_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    let windows = crate::domain::window::group_by_key_seeded(
        billable.iter().cloned(),
        records.into_iter().map(|w| {
            (
                w.scope_key,
                WindowInterval::new(w.effective_from, w.effective_to, w.state),
            )
        }),
    );

    Ok(coverage::check(&billable, &windows))
}

/// `GET /plans/{planId}/sellability`: the gate's surface on one market at one
/// instant.
///
/// The gate is `plan` x `read`, the same pair `get_plan_coverage` and
/// `list_plan_prices` ask for, and **no new authz vocabulary**: the catalog
/// already carries every label and action this needs.
///
/// # Which version it answers from, and why there is no `version=` parameter
///
/// §5's signature is `?at=&currency=&region=` and names **no version**, so the
/// surface reads the tenant's own **pin-eligible frontier** and resolves the plan's
/// subject at it (§4.4). Adding a parameter would be minting a surface the design
/// set does not declare, and it would let a caller ask about a version they are not
/// entitled to pin.
///
/// The consequence is a property that cannot be asserted through this route: *"a
/// consumer pinned to the pre-cancel version still reports the old coverage"* is
/// about two versions at once, and this surface only ever answers at one. It is
/// therefore asserted **at the store**, against two stored payloads, in
/// `tests/sqlite_sellability.rs` - said here so the missing parameter reads as the
/// design set's shape rather than as an oversight.
///
/// # The frontier and the delta are two reads, and that is not a race
///
/// If the frontier advances between them the answer is computed at the older
/// version, which is a version the caller could legitimately have pinned a moment
/// earlier - an older answer, never an incoherent one. A frozen version never
/// changes what it says, which is what makes the pair safe to read apart.
async fn get_plan_sellability(
    Extension(state): Extension<Arc<GovernanceState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(plan_id): Path<Uuid>,
    Query(query): Query<SellabilityQuery>,
) -> Result<Json<PlanSellabilityView>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let plan_id = PlanId::new(plan_id);
    // Parsed before the gate is asked, so a caller who omitted a parameter is told
    // that rather than being told they may not read a plan - the shape
    // `schedule_window` reads its idempotency key in.
    let (at, currency, region) = market_of(&query)?;
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
    let conn = state.db.conn().map_err(|e| {
        CanonicalError::internal(format!("bss-pricing: plan sellability: {e}")).create()
    })?;

    let facts = sellability_facts(&conn, &scope, tenant, plan_id).await?;
    let surface = SellabilitySurface::of_delta(&facts, at, &currency, &region);
    Ok(Json(PlanSellabilityView::from(&surface)))
}

/// The three query parameters, or the refusal that names the missing one.
///
/// **All three are required**, which §5 does not state either way and is decided
/// here. The market is what the answer is *about*, so neither half of it has a
/// defensible default; and `at` is required rather than defaulted to this server's
/// clock because the whole contract of this surface is that the instant is the
/// caller's - a defaulted one would put the server's clock inside an answer the
/// caller reads as being about their own moment. An absent or unparseable value is
/// a malformed request under the validation envelope: **no new code**, which is the
/// rule D-171 clause (5) already states for an absent precondition.
///
/// The D-144 millisecond quantum is deliberately **not** checked here. It governs
/// *authored* instants - the ones this gear stores and later matches for equality -
/// and `at` is neither stored nor matched; refusing a finer-grained read parameter
/// would spend a validation on a value that leaves no trace.
///
/// # A numeric offset has to be percent-encoded, and that is refused rather than
/// repaired
///
/// `+` is a **space** under form-urlencoding, so an `at` written `...+00:00`
/// literally in a query string arrives as `...00:00` with a space where the sign
/// was - a string that is not an instant in any format. It is refused: repairing it
/// would mean guessing which character the caller meant, and answering about a
/// moment they did not name. `...Z` needs no encoding and is what the parameter's
/// own description tells a client to send.
fn market_of(
    query: &SellabilityQuery,
) -> Result<(DateTime<Utc>, CurrencyCode, Region), DomainError> {
    let required = |value: &Option<String>, name: &str| {
        value.clone().ok_or_else(|| {
            DomainError::InvalidRequest(format!(
                "the `{name}` query parameter is required on the sellability surface"
            ))
        })
    };
    let at = required(&query.at, "at")?;
    let at = DateTime::parse_from_rfc3339(&at)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|e| {
            DomainError::InvalidRequest(format!(
                "the `at` query parameter is not an RFC 3339 instant: {e}"
            ))
        })?;
    Ok((
        at,
        CurrencyCode::new(&required(&query.currency, "currency")?)?,
        Region::new(&required(&query.region, "region")?)?,
    ))
}

/// The plan's facts at the tenant's pin-eligible frontier.
///
/// **Both absences answer the same way** - a tenant with no frontier at all, and a
/// frontier that carries no delta for this plan - because they are one fact to a
/// consumer: this pin does not carry this plan's content. The surface answers
/// predicate (2) failed for it, which is the fail-closed direction, rather than
/// synthesising a plan with no rows (a different and answerable statement) or a
/// 404 (which would tell an unauthorized caller which plan ids exist).
async fn sellability_facts(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
) -> Result<SellabilityFacts, CanonicalError> {
    let Some(frontier) = pin_frontier_repo::read_at(runner, scope, tenant)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?
    else {
        return Ok(SellabilityFacts::NotAddressable { plan_id });
    };
    let Some(delta) = read_model_repo::delta_at(
        runner,
        scope,
        tenant,
        &SubjectRef::Plan(plan_id.get()),
        frontier.catalog_version,
    )
    .await
    .map_err(|e| CanonicalError::from(repo_failure(&e)))?
    else {
        return Ok(SellabilityFacts::NotAddressable { plan_id });
    };
    read_model_repo::sellability_facts(&delta).map_err(|e| CanonicalError::from(repo_failure(&e)))
}
