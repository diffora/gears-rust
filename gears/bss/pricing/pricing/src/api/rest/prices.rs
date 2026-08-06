//! The price authoring plane: `POST`, `PATCH`, `DELETE` and the paginated list
//! (`design/03-price-structure.md` §5, D-125, D-140, D-141, D-148).
//!
//! # This surface does not validate a row's shape, and a reader would otherwise
//! # assume it does
//!
//! `MODEL_KIND_MISSING`, the tier-band family, `EVAL_POLICY_*`,
//! `AMOUNT_PLACEMENT_INVALID`, `LEVEL_*` and the rest of Slice 3's rules are
//! **publish-time** and none of them runs here. Authoring a shape-invalid draft
//! is legal: §4.2 puts the whole rule set at the publish pre-check, an author
//! assembles a row over several calls, and refusing an intermediate state at
//! save time would contradict that design outright.
//!
//! What this plane **can** refuse is what the store itself decides — a duplicate
//! canonical scope key (`DUPLICATE_SCOPE_KEY`, on the draft plane too since
//! D-148), a stale entity tag (`STALE_VERSION`), a horizon off its eligibility
//! class (`GRANDFATHER_UNTIL_FORBIDDEN`), an instant finer than the millisecond
//! quantum (`TIMESTAMP_PRECISION_EXCEEDED`), a value past its column
//! (`InvalidArgument`), and an edit of a frozen row (`LIFECYCLE_FORBIDDEN`) —
//! plus the two idempotency refusals the create's gate can raise. Every one of
//! those already exists; nothing here is minted. The two Slice-10 primitives
//! this surface refuses (below) mint nothing either — an unsupported field is a
//! malformed request under the Foundation validation envelope.
//!
//! # The `{planId}` segment is checked, not decorative
//!
//! `PATCH` and `DELETE` name a plan **and** a price, while the repository keys
//! on `price_id` alone. So both verbs verify the row actually belongs to the
//! named plan and answer 404 otherwise. A path that names a parent it does not
//! check is a path that lets a caller mutate a row through the wrong plan's URL
//! — and it makes the authz `resource_id` argument a fiction, since the gate is
//! asked about a resource the handler then does not confirm.
//!
//! # Two Slice-10 primitives are refused here, not validated
//!
//! `tierQualificationWindow` (D-40, D-60) and `includedAllowance` (D-45, D-130)
//! are members of [`PriceContentView`] and are refused with
//! [`DomainError::InvalidRequest`] the moment a request carries a non-null one.
//! The reason is that Slice 10 has landed *nothing else*: neither the ten
//! refusals `inst-ac-gate` / `inst-tt-forbidden` / `inst-tt-window-pair` /
//! `inst-tt-zero-band` / `inst-tt-fixture` state, nor the allowance compile
//! (`inst-ac-band`, `inst-ac-marker`, `inst-ac-carry`) that gives the declaration
//! its meaning. Storing a value this gear can neither judge nor honour is
//! precisely the state `inst-tt-forbidden` names when it says *an
//! accepted-but-ignored value would mask authoring errors* — and the projector
//! and the `ep-1` roster already carry both fields, so the day Slice 5 mounts
//! the publish route an unjudged allowance freezes into an immutable version.
//!
//! Building the ten refusals **without** the compile would be worse, not
//! better: a `graduated` row carrying `{100, none}` would pass all six
//! `inst-ac-gate` rules, publish, and then be billed from unit one — an
//! allowance accepted, *validated*, and silently ignored. That is the shape
//! D-149 clause (3), D-161 clause (1), D-167 clause (3) and D-168 clause (1)
//! each refuse one artifact over.
//!
//! The members **stay** on the view. D-174 clause (1) puts a member the gear
//! does not model outside the idempotency digest and therefore inside the replay
//! set, so deleting them would convert an accepted field into a *silently
//! ignored* one. The response half stays too: the domain model, the storage
//! round trip and the D-129 supersession guard all read both fields, and a read
//! that dropped them would lose a field that guard compares. No code is minted
//! — a field whose slice has not landed is a malformed request under the
//! Foundation validation envelope, the class D-141 and D-171 give an absent
//! `If-Match`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, Query};
use axum::http::header::{ETAG, LOCATION};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::HeaderMap, http::StatusCode};
use bss_fixtures::ModelKind;
use chrono::{DateTime, Utc};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_db::secure::{AccessScope, DbTx};
use toolkit_odata::Page;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::auth_context::{audit_stamp, require_authenticated};
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::cursor::{self, PageRequest};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::preconditions;
use crate::api::rest::state::AuthoringState;
use crate::domain::error::DomainError;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::price_record::{PriceContent, PriceRecord};
use crate::domain::price_row::{
    AggregationFunction, AggregationGranularity, BandTop, BillingGranularity, IncludedAllowance,
    PriceRow, QuantitySource, RolloverPolicy, TierAggregationWindow, TierBand,
    TierQualificationWindow, model_kind_wire,
};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use crate::infra::idempotent::{self, Guarded, GuardedRequest, TxFuture};
use crate::infra::storage::repo::{NewPriceDraft, price_repo};
use crate::infra::storage::repo_failure;

/// `OpenAPI` tag applied to every price operation (DE0205).
const TAG: &str = "BSS Pricing Price Rows";

/// The idempotency-gate operation name for the guarded create. Its own, so a
/// client key reused across the two guarded creates does not collide.
const CREATE_PRICE_OPERATION: &str = "bss_pricing.create_price";

/// A plan's price rows.
pub const PLAN_PRICES: &str = "/bss-pricing/v1/plans/{planId}/prices";
/// One price row under its plan.
pub const PLAN_PRICE: &str = "/bss-pricing/v1/plans/{planId}/prices/{priceId}";

/// The lifecycle states the **authoring** list answers with.
///
/// `draft` and `published` — the two an author works with, and the states a
/// `PATCH`, a `DELETE` or a supersession can be aimed at. `superseded` is
/// excluded because it is history that is no longer current on its key, and
/// `abandoned` because it is not a price-row state at all
/// (`03-price-structure.md` §4 has three states). The vocabulary comes from
/// [`LifecycleState`] rather than from string literals, exactly as
/// `infra::publish`'s `CANDIDATE_ROW_STATES` takes it, for the same reason: a
/// literal list is one a state added later goes missing from silently.
///
/// A caller that needs another set is asking for a filter nothing has designed;
/// it is reported rather than invented here.
const AUTHORING_STATES: &[LifecycleState] = &[LifecycleState::Draft, LifecycleState::Published];

// ---------------------------------------------------------------------------
// Views and requests.
// ---------------------------------------------------------------------------

/// The eight axes a row is filed under, as a caller authors them.
///
/// `price_overlay` is **not** a member: every row this gear authors carries
/// `base`, and partner / orgTier / brand overlays are separate overlay rows
/// rather than a value of this axis. Offering it would offer a choice the
/// authoring plane does not have.
///
/// `plan_id` is not a member either — it is the `{planId}` path segment, so a
/// body cannot name a plan the URL does not.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct ScopeKeyRequest {
    /// ISO 4217 currency.
    pub currency: String,
    /// The pricing region.
    pub region: String,
    /// The phase this row prices; the plan's terminal phase for a
    /// phase-invariant row (D-19).
    pub phase: Uuid,
    /// `all_subscriptions` | `new_subscriptions_only` | `existing_grandfathered`.
    pub price_eligibility: String,
    /// `recurring` | `usage` | `one_time` | `one_time_setup`.
    pub charge_kind: String,
    /// The grandfathering generation's cutover instant, or `null` for a row
    /// that retains nobody.
    pub cohort: Option<DateTime<Utc>>,
}

/// The eight axes as the store holds them.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ScopeKeyView {
    /// Axis 1.
    pub plan_id: Uuid,
    /// Axis 2.
    pub currency: String,
    /// Axis 3.
    pub region: String,
    /// Axis 4 — always `base` on an authored row.
    pub price_overlay: String,
    /// Axis 5.
    pub phase: Uuid,
    /// Axis 6.
    pub price_eligibility: String,
    /// Axis 7.
    pub charge_kind: String,
    /// Axis 8, `null` when the row retains nobody.
    pub cohort: Option<DateTime<Utc>>,
}

impl From<&ScopeKey> for ScopeKeyView {
    fn from(key: &ScopeKey) -> Self {
        Self {
            plan_id: key.plan_id().get(),
            currency: key.currency().as_str().to_owned(),
            region: key.region().as_str().to_owned(),
            price_overlay: key.price_overlay().as_str().to_owned(),
            phase: key.phase().get(),
            price_eligibility: key.price_eligibility().as_str().to_owned(),
            charge_kind: key.charge_kind().as_str().to_owned(),
            cohort: key.cohort().generation(),
        }
    }
}

/// One tier band.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct TierBandView {
    /// Inclusive lower bound.
    pub from_qty: u64,
    /// Exclusive upper bound, or `null` for the open top band (D-17: the top
    /// band is always open, and capping belongs to quotas).
    pub to_qty: Option<u64>,
    /// The unit price in ISO 4217 minor units.
    pub unit_price_minor: i64,
}

impl From<&TierBand> for TierBandView {
    fn from(band: &TierBand) -> Self {
        Self {
            from_qty: band.from_qty,
            to_qty: band.to_qty.closed_at(),
            unit_price_minor: band.unit_price_minor.get(),
        }
    }
}

/// The included allowance a usage row grants (D-45).
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct IncludedAllowanceView {
    /// How much is included.
    pub quantity: u64,
    /// `none` | `carry`.
    pub rollover_policy: String,
}

/// Everything about a row an open draft may still change.
///
/// It is a whole-content submission and not a patch, for the reason
/// [`PriceContent`]'s own doc gives: a price row's fields are not independent of
/// each other — moving `model_kind` from `graduated` to `flat` has to drop the
/// band set and set `amount_minor` in the *same* write, because every
/// intermediate state is one no Slice-3 rule can pass.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct PriceContentView {
    /// `flat` | `per_unit` | `graduated` | `volume` | `package`. Authored, never
    /// inferred — there is no implicit default at rating time.
    pub model_kind: Option<String>,
    /// The row's own amount, on the kinds that carry one.
    pub amount_minor: Option<i64>,
    /// The band set, ordered by lower bound on read.
    pub bands: Option<Vec<TierBandView>>,
    /// The package's block size.
    pub package_size: Option<u64>,
    /// The package's block price.
    pub package_price_minor: Option<i64>,
    /// Where a non-usage `per_unit` row's quantity comes from.
    pub quantity_source: Option<String>,
    /// The manual quantity, on a `manual` source.
    pub manual_quantity: Option<u64>,
    /// The meter a usage row prices.
    pub meter: Option<String>,
    /// The priced dimension within the meter; empty for a whole-meter line.
    pub dimension_key: Option<String>,
    /// The billable-unit quantization.
    pub billing_granularity: Option<String>,
    /// The tier counter's reset window.
    pub tier_aggregation_window: Option<String>,
    /// The D-40 tier-qualification window.
    pub tier_qualification_window: Option<String>,
    /// How the in-window quantity is derived (D-44).
    pub aggregation_function: Option<String>,
    /// The granule a non-`sum` window is cut into (D-44).
    pub aggregation_granularity: Option<String>,
    /// The `maxHold` bound on a non-`sum` row.
    pub max_hold_granules: Option<u64>,
    /// The plan-scoped included allowance (D-45).
    pub included_allowance: Option<IncludedAllowanceView>,
    /// Whether the authored amounts are tax-inclusive. Absent is `false`.
    pub tax_inclusive: Option<bool>,
    /// `advance` | `arrears` — Slice-6-owned, so a free string here.
    pub billing_timing: Option<String>,
    /// The named rounding policy this row resolves against.
    pub rounding_policy_ref: Option<String>,
    /// The grandfathering horizon. Only an `existing_grandfathered` row may
    /// carry one.
    pub grandfather_until: Option<DateTime<Utc>>,
    /// The predecessor this row supersedes on its key.
    pub supersedes_price_id: Option<Uuid>,
}

impl From<&PriceRecord> for PriceContentView {
    fn from(record: &PriceRecord) -> Self {
        let row = &record.row;
        Self {
            model_kind: row.model_kind.map(model_kind_wire).map(str::to_owned),
            amount_minor: row.amount_minor.map(MinorAmount::get),
            bands: Some(row.bands.iter().map(TierBandView::from).collect()),
            package_size: row.package_size,
            package_price_minor: row.package_price_minor.map(MinorAmount::get),
            quantity_source: row.quantity_source.map(|q| q.as_str().to_owned()),
            manual_quantity: row.manual_quantity,
            meter: row.meter.clone(),
            dimension_key: Some(row.dimension_key.clone()),
            billing_granularity: row.billing_granularity.map(|g| g.as_str().to_owned()),
            tier_aggregation_window: row.tier_aggregation_window.map(|w| w.as_str().to_owned()),
            tier_qualification_window: row.tier_qualification_window.map(|w| w.as_str().to_owned()),
            aggregation_function: row.aggregation_function.map(|f| f.as_str().to_owned()),
            aggregation_granularity: row.aggregation_granularity.map(|g| g.as_str().to_owned()),
            max_hold_granules: row.max_hold_granules,
            included_allowance: row.included_allowance.as_ref().map(|allowance| {
                IncludedAllowanceView {
                    quantity: allowance.quantity,
                    rollover_policy: allowance.rollover_policy.as_str().to_owned(),
                }
            }),
            tax_inclusive: Some(record.tax_inclusive),
            billing_timing: record.billing_timing.clone(),
            rounding_policy_ref: record.rounding_policy_ref.clone(),
            grandfather_until: record.grandfather_until,
            supersedes_price_id: record.supersedes_price_id,
        }
    }
}

/// One price row, whole.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct PriceRowView {
    /// The row's identity, minted by the surface at creation.
    pub price_id: Uuid,
    /// The eight axes it is filed under.
    pub scope_key: ScopeKeyView,
    /// What the row says.
    pub content: PriceContentView,
    /// `draft` | `published` | `superseded`.
    pub lifecycle_state: String,
    /// Pseudonymous principal id of the authoring actor.
    pub created_by: Uuid,
    /// When the row was authored, UTC.
    pub created_at_utc: DateTime<Utc>,
    /// The row's own optimistic-concurrency version — never the plan's (D-141),
    /// and the same number the `ETag` quotes.
    pub row_version: u64,
}

impl From<&PriceRecord> for PriceRowView {
    fn from(record: &PriceRecord) -> Self {
        Self {
            price_id: record.price_id,
            scope_key: ScopeKeyView::from(&record.scope_key),
            content: PriceContentView::from(record),
            lifecycle_state: record.lifecycle_state.as_str().to_owned(),
            created_by: record.created_by,
            created_at_utc: record.created_at_utc,
            row_version: record.row_version.get(),
        }
    }
}

/// A create: the key the row is filed under, and what it says.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct CreatePriceRequest {
    /// The seven axes a caller authors; the eighth is the `{planId}` segment.
    pub scope_key: ScopeKeyRequest,
    /// The row's whole content.
    pub content: PriceContentView,
}

/// An edit: the whole content, and optionally the key it must still be on.
///
/// **The scope key is immutable.** `PriceRepo::update_draft` cannot move it, so
/// a body naming a different one is refused rather than silently ignored: a key
/// decides which duplicate a row is, which supersession chain it joins and which
/// window covers it, and moving it would need the create-time duplicate check
/// re-run against a different key — which is what deleting the draft and
/// authoring another one already is.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct PatchPriceRequest {
    /// The key the caller believes the row is on. Optional; when present it must
    /// equal the stored one.
    pub scope_key: Option<ScopeKeyRequest>,
    /// The row's whole content, replacing what is there.
    pub content: PriceContentView,
}

/// The two pagination query parameters (D-125). Offset is not offered.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PricePageQuery {
    /// Rows per page; server default 100, hard cap 1,000.
    pub limit: Option<u64>,
    /// The opaque token a previous page returned.
    pub cursor: Option<String>,
}

/// Build the Axum router for the price surface and register its operations.
///
/// No route declares a 422: §3.3's status-rendering rule makes every
/// architectural 422 in the design set reach the wire as a 400 carrying its
/// code, so a 422 here would document a response no path can emit.
#[allow(
    clippy::too_many_lines,
    reason = "one builder chain per operation; flat is clearer than helpers that hide which route declares which response"
)]
pub fn router(state: Arc<AuthoringState>, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router = Router::new();

    router = OperationBuilder::post("/bss-pricing/v1/plans/{planId}/prices")
        .operation_id("bss_pricing.create_price")
        .summary("Create a draft price row on a canonical scope key")
        .description(
            "Creates a `draft` price row and its tier bands in one transaction, and answers \
             `201` with a `Location` header naming the row and an `ETag` carrying its own row \
             version. The `price_id` is minted by the server. An `Idempotency-Key` header is \
             required: the gate runs in the same transaction as the insert, so a retry carrying \
             the same key and body is answered the recorded response - the original price id \
             included - and a retry with a different body is refused \
             `IDEMPOTENCY_PAYLOAD_MISMATCH`. A key already held by a `draft` or `published` row \
             is refused `DUPLICATE_SCOPE_KEY` (on the draft plane too, since D-148). The row's \
             `charge_kind` is the key's, not the body's, so the response echoes what was stored \
             rather than what was sent. Slice-3 shape rules run at publish, not here.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan the row prices.")
        .param(crate::api::rest::plans::idempotency_key_param())
        .json_request::<CreatePriceRequest>(openapi, "The row's scope key and content.")
        .handler(create_price)
        .json_response_with_schema::<PriceRowView>(
            openapi,
            StatusCode::CREATED,
            "The newly created draft row.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::patch("/bss-pricing/v1/plans/{planId}/prices/{priceId}")
        .operation_id("bss_pricing.patch_price")
        .summary("Replace a draft row's content")
        .description(
            "Replaces the whole content of a `draft` row, band set included, under the \
             `If-Match` precondition (D-141). It is a whole-content submission rather than a \
             field patch because a row's fields are not independent: moving `model_kind` from \
             `graduated` to `flat` has to drop the bands and set `amount_minor` in the same \
             write. The canonical scope key is **immutable** - a body naming a different one is \
             `400`, never a silent no-op. A row belonging to a different plan than the `{planId}` \
             segment answers `404`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan the row belongs to.")
        .path_param("priceId", "The row to replace.")
        .param(crate::api::rest::plans::if_match_param(
            "On a price route the tag is the row's **own** version and nothing else (D-141: \
             never derived from the plan's), because the path addresses one row by id.",
        ))
        .json_request::<PatchPriceRequest>(openapi, "The row's whole new content.")
        .handler(patch_price)
        .json_response_with_schema::<PriceRowView>(
            openapi,
            StatusCode::OK,
            "The row as it stands after the edit.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::delete("/bss-pricing/v1/plans/{planId}/prices/{priceId}")
        .operation_id("bss_pricing.delete_price")
        .summary("Delete a never-published draft row")
        .description(
            "Deletes a `draft` row and its bands, under the `If-Match` precondition. The \
             precondition is D-141's whole point: this verb's idempotency cell used to be \
             empty, so a draft row could be destroyed under an unknown version - and what a \
             blind delete destroys is a concurrent editor's uncommitted work, not the row. A \
             published row is refused, never deleted (`inst-ps-nodelete`). A row belonging to a \
             different plan than the `{planId}` segment answers `404`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan the row belongs to.")
        .path_param("priceId", "The draft row to delete.")
        .param(crate::api::rest::plans::if_match_param(
            "On a price route the tag is the row's **own** version and nothing else (D-141).",
        ))
        .handler(delete_price)
        .no_content_response(
            StatusCode::NO_CONTENT,
            "The draft row and its bands are gone.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::get("/bss-pricing/v1/plans/{planId}/prices")
        .operation_id("bss_pricing.list_plan_prices")
        .summary("List a plan's authoring price rows (cursor-paginated)")
        .description(
            "One page of the plan's `draft` and `published` rows, in `price_id` order, with an \
             opaque `cursor` and a `limit` whose server default is 100 and whose hard cap is \
             1,000 (D-125). `next_cursor` is returned on every page until the result is \
             exhausted, so a client stops without issuing an extra request that returns an \
             empty page. `prev_cursor` is always `null`: D-125 specifies a forward walk only. \
             `superseded` rows are excluded - they are history and no longer current on their \
             key - and so is any state the price-row machine does not have.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("planId", "The plan whose rows are listed.")
        .query_param_typed(
            "limit",
            false,
            "Rows per page (default 100, hard cap 1,000)",
            "integer",
        )
        .query_param("cursor", false, "Opaque base64url pagination cursor")
        .handler(list_plan_prices)
        .json_response_with_schema::<Page<PriceRowView>>(
            openapi,
            StatusCode::OK,
            "One page of the plan's authoring price rows.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
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

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

/// `POST /plans/{planId}/prices`.
async fn create_price(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path(plan_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = write_scope(&enforcer, &ctx, plan_id.get(), tenant).await?;

    let body: CreatePriceRequest = preconditions::parse_body(&body)?;
    let client_key = preconditions::idempotency_key(&headers)?;
    let request_hash = preconditions::request_digest(&body)?;
    let key = scope_key_of(plan_id, &body.scope_key)?;
    let content = content_of(&body.content)?;
    let now = Utc::now();

    let guard = GuardedRequest {
        operation: CREATE_PRICE_OPERATION,
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
        move |txn: &DbTx<'_>| -> TxFuture<'_, PriceRecord> {
            Box::pin(async move {
                // Minted inside the guarded body for the reason the plan create
                // states: a replay must answer the FIRST caller's id.
                let draft = NewPriceDraft {
                    price_id: Uuid::now_v7(),
                    scope_key: key,
                    content,
                    created_by: actor,
                    created_at_utc: now,
                    correlation_id: correlation,
                };
                // `guarded`'s mutation speaks `DomainError`; the ladder is the
                // same one it used to apply here. See `plans.rs`'s note.
                price_repo::create_draft_on(txn, &scope_for_body, tenant, draft)
                    .await
                    .map_err(|e| repo_failure(&e))
            })
        },
        |record: &PriceRecord| {
            serde_json::to_value(PriceRowView::from(record)).map_err(|e| {
                DomainError::Internal(format!("cannot render the created price row: {e}"))
            })
        },
    )
    .await
    .map_err(CanonicalError::from)?;

    Ok(match outcome {
        Guarded::Performed(record) => created(plan_id, &record),
        Guarded::Replayed { status, body } => replayed(plan_id, status, &body),
    })
}

/// `PATCH /plans/{planId}/prices/{priceId}`.
async fn patch_price(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path((plan_id, price_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<([(axum::http::HeaderName, String); 1], Json<PriceRowView>), CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = write_scope(&enforcer, &ctx, price_id, tenant).await?;

    let body: PatchPriceRequest = preconditions::parse_body(&body)?;
    let expected = preconditions::if_match(&headers)?;
    let stored = row_of_plan(&state, &scope, tenant, plan_id, price_id).await?;
    if let Some(named) = &body.scope_key {
        // **Compared over the axes the wire can express** (D-196 clause 3).
        // `ScopeKeyRequest` has no `meter` member — the usage line is authored on
        // the *content* view and the door derives the ninth and tenth axes from
        // it — so a stored usage row's key always carries a line this body could
        // not have named. Comparing the two raw would refuse every `PATCH` that
        // echoes its own key on a metered row, which is the opposite of what this
        // immutability check is for. The stored line is therefore carried onto
        // the named key before the comparison: the axes the caller *can* state
        // must match, and the ones they cannot are taken from the row.
        let named = scope_key_of(plan_id, named)?.with_usage_line(
            stored.scope_key.meter().cloned(),
            stored.scope_key.dimension_key().clone(),
        )?;
        if named != stored.scope_key {
            return Err(CanonicalError::from(DomainError::InvalidRequest(
                "the canonical scope key is immutable; a row's key decides which duplicate it \
                 is, which supersession chain it joins and which window covers it. Delete this \
                 draft and author another one on the key you want"
                    .to_owned(),
            )));
        }
    }
    let content = content_of(&body.content)?;

    let updated = state
        .prices
        .update_draft(
            &scope,
            tenant,
            price_id,
            expected,
            content,
            audit_stamp(&ctx, Utc::now(), correlation),
        )
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;

    Ok(answer(&updated))
}

/// `DELETE /plans/{planId}/prices/{priceId}`.
async fn delete_price(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path((plan_id, price_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let plan_id = PlanId::new(plan_id);
    let scope = write_scope(&enforcer, &ctx, price_id, tenant).await?;

    let expected = preconditions::if_match(&headers)?;
    row_of_plan(&state, &scope, tenant, plan_id, price_id).await?;

    state
        .prices
        .delete_draft(
            &scope,
            tenant,
            price_id,
            expected,
            audit_stamp(&ctx, Utc::now(), correlation),
        )
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /plans/{planId}/prices`.
///
/// The gate is `plan x read` with `owner_tenant_id = None`, so the compiled
/// scope is the SQL filter and the whole walk is tenant-scoped by construction
/// rather than by a predicate this handler remembers to add.
async fn list_plan_prices(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(plan_id): Path<Uuid>,
    Query(query): Query<PricePageQuery>,
) -> Result<Json<Page<PriceRowView>>, CanonicalError> {
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

    let page = PageRequest::parse(query.limit, query.cursor.as_deref())?;
    // One row more than the page, so "is there another page" is answered without
    // a second query and without a page of `next_cursor` pointing at nothing.
    let probe = page.limit.saturating_add(1);
    let mut rows = state
        .prices
        .list_for_plan_page(
            &scope,
            ctx.subject_tenant_id(),
            plan_id,
            AUTHORING_STATES,
            page.after,
            probe,
        )
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;

    let has_more = u64::try_from(rows.len()).unwrap_or(u64::MAX) > page.limit;
    if has_more {
        rows.pop();
    }
    let next = has_more
        .then(|| rows.last().map(|row| row.price_id))
        .flatten();
    Ok(Json(Page {
        items: rows.iter().map(PriceRowView::from).collect(),
        page_info: cursor::page_info(next, page.limit),
    }))
}

// ---------------------------------------------------------------------------
// Shared pieces.
// ---------------------------------------------------------------------------

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

/// The row named by `{priceId}`, **confirmed to belong to `{planId}`**.
///
/// A row under the wrong plan's URL is answered exactly like an absent one: the
/// caller named a resource that does not exist at that address, and telling them
/// the row exists elsewhere would leak which plans hold which rows.
async fn row_of_plan(
    state: &AuthoringState,
    scope: &AccessScope,
    tenant: Uuid,
    plan_id: PlanId,
    price_id: Uuid,
) -> Result<PriceRecord, CanonicalError> {
    let found = state
        .prices
        .find(scope, tenant, price_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    match found {
        Some(record) if record.scope_key.plan_id() == plan_id => Ok(record),
        _ => Err(CanonicalError::from(DomainError::NotFound {
            subject: "price row".to_owned(),
            id: price_id.to_string(),
        })),
    }
}

/// 200 plus the row's own entity tag.
fn answer(record: &PriceRecord) -> ([(axum::http::HeaderName, String); 1], Json<PriceRowView>) {
    (
        [(ETAG, preconditions::etag(record.row_version))],
        Json(PriceRowView::from(record)),
    )
}

/// The 201 a performed create answers with.
fn created(plan_id: PlanId, record: &PriceRecord) -> Response {
    (
        StatusCode::CREATED,
        [
            (LOCATION, price_location(plan_id, record.price_id)),
            (ETAG, preconditions::etag(record.row_version)),
        ],
        Json(PriceRowView::from(record)),
    )
        .into_response()
}

/// The recorded answer a replay is handed back.
///
/// `Location` is rebuilt from the recorded body's `price_id`, which is a pure
/// function of data that body carries. No `ETag`: the dedup row stores a status
/// and a body and no headers, and the row's version may have moved since — a tag
/// rebuilt from a stale body would be a precondition token that looks valid and
/// is not.
fn replayed(plan_id: PlanId, status: i32, body: &serde_json::Value) -> Response {
    let status = u16::try_from(status)
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok())
        .unwrap_or(StatusCode::OK);
    let location = body
        .get("price_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .map(|price_id| price_location(plan_id, price_id));
    match location {
        Some(location) => (status, [(LOCATION, location)], Json(body.clone())).into_response(),
        None => (status, Json(body.clone())).into_response(),
    }
}

/// Where a row lives, spelled once.
fn price_location(plan_id: PlanId, price_id: Uuid) -> String {
    format!("/bss-pricing/v1/plans/{plan_id}/prices/{price_id}")
}

// ---------------------------------------------------------------------------
// Wire -> domain.
//
// Every token list below is `price_repo`'s, not a second one: the surface parses
// the same tokens off a request that the repository parses off a column, and two
// tables would be two answers to the same question.
// ---------------------------------------------------------------------------

/// Build the canonical scope key from the path's plan and the body's axes.
fn scope_key_of(plan_id: PlanId, key: &ScopeKeyRequest) -> Result<ScopeKey, DomainError> {
    ScopeKey::new(
        plan_id,
        CurrencyCode::new(&key.currency)?,
        Region::new(&key.region)?,
        PhaseId::new(key.phase),
        wire_token(
            "scope_key.price_eligibility",
            &key.price_eligibility,
            price_repo::PRICE_ELIGIBILITIES,
            PriceEligibility::as_str,
        )?,
        wire_token(
            "scope_key.charge_kind",
            &key.charge_kind,
            price_repo::CHARGE_KINDS,
            ChargeKind::as_str,
        )?,
        key.cohort.map_or(Cohort::None, Cohort::Generation),
    )
}

/// Build the row's content.
///
/// `charge_kind` is filled from the key by the repository, not from here: the
/// axis is the key's and the copy on [`PriceRow`] is a convenience the shape
/// rules read. A placeholder is passed and overwritten, which is why the
/// response echoes what was stored rather than what was sent.
///
/// `pub(crate)` for one second caller, [`crate::api::rest::supersessions`]: a
/// supersession's successor is a price row and its body carries the **same**
/// [`PriceContentView`], so a second conversion of that view would be a second
/// reading of what a price row's content is — including a second copy of
/// `refuse_unlanded_primitives`, which is the one guard a divergence there would
/// silently drop.
pub(crate) fn content_of(view: &PriceContentView) -> Result<PriceContent, DomainError> {
    refuse_unlanded_primitives(view)?;
    let bands = view
        .bands
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(band_of)
        .collect::<Result<Vec<_>, _>>()?;
    let row = PriceRow {
        charge_kind: ChargeKind::Recurring,
        model_kind: view
            .model_kind
            .as_deref()
            .map(|token| {
                wire_token(
                    "content.model_kind",
                    token,
                    &ModelKind::ALL,
                    model_kind_wire,
                )
            })
            .transpose()?,
        amount_minor: amount("content.amount_minor", view.amount_minor)?,
        bands,
        package_size: view.package_size,
        package_price_minor: amount("content.package_price_minor", view.package_price_minor)?,
        quantity_source: optional_token(
            "content.quantity_source",
            view.quantity_source.as_deref(),
            price_repo::QUANTITY_SOURCES,
            QuantitySource::as_str,
        )?,
        manual_quantity: view.manual_quantity,
        meter: view.meter.clone(),
        dimension_key: view.dimension_key.clone().unwrap_or_default(),
        billing_granularity: optional_token(
            "content.billing_granularity",
            view.billing_granularity.as_deref(),
            price_repo::BILLING_GRANULARITIES,
            BillingGranularity::as_str,
        )?,
        tier_aggregation_window: optional_token(
            "content.tier_aggregation_window",
            view.tier_aggregation_window.as_deref(),
            price_repo::TIER_AGGREGATION_WINDOWS,
            TierAggregationWindow::as_str,
        )?,
        tier_qualification_window: optional_token(
            "content.tier_qualification_window",
            view.tier_qualification_window.as_deref(),
            price_repo::TIER_QUALIFICATION_WINDOWS,
            TierQualificationWindow::as_str,
        )?,
        aggregation_function: optional_token(
            "content.aggregation_function",
            view.aggregation_function.as_deref(),
            price_repo::AGGREGATION_FUNCTIONS,
            AggregationFunction::as_str,
        )?,
        aggregation_granularity: optional_token(
            "content.aggregation_granularity",
            view.aggregation_granularity.as_deref(),
            price_repo::AGGREGATION_GRANULARITIES,
            AggregationGranularity::as_str,
        )?,
        max_hold_granules: view.max_hold_granules,
        included_allowance: view
            .included_allowance
            .as_ref()
            .map(|allowance| {
                Ok::<_, DomainError>(IncludedAllowance {
                    quantity: allowance.quantity,
                    rollover_policy: wire_token(
                        "content.included_allowance.rollover_policy",
                        &allowance.rollover_policy,
                        price_repo::ROLLOVER_POLICIES,
                        RolloverPolicy::as_str,
                    )?,
                })
            })
            .transpose()?,
    };
    Ok(PriceContent {
        row,
        tax_inclusive: view.tax_inclusive.unwrap_or(false),
        billing_timing: view.billing_timing.clone(),
        rounding_policy_ref: view.rounding_policy_ref.clone(),
        grandfather_until: view.grandfather_until,
        supersedes_price_id: view.supersedes_price_id,
    })
}

/// Refuse the two Slice-10 primitives until Slice 10 lands.
///
/// The refusal is **not value-conditioned**: `inst-tt-forbidden` refuses an
/// *explicit* window of any value, `current` included, so a check that only
/// caught `trailing_period` would accept the default spelled out and store a
/// field nothing judges. The same holds for the allowance under either rollover
/// policy — `none` needs the band compile and `carry` needs a
/// `pricing_plan_grant` row, and neither exists.
///
/// The message names the field and says why, because the caller's next action
/// differs from every other 400 on this surface: there is nothing to correct in
/// the request, the primitive is unsupported.
fn refuse_unlanded_primitives(view: &PriceContentView) -> Result<(), DomainError> {
    if view.tier_qualification_window.is_some() {
        return Err(DomainError::InvalidRequest(
            "content.tier_qualification_window is not supported yet: the gear can store the value \
             and cannot judge it. Slice 10's refusals (TIER_QUAL_ON_NON_TIERED, \
             TIER_QUAL_WINDOW_INCOMPATIBLE, TIER_QUAL_ZERO_BAND_LOCK, FIXTURE_MISSING) and the \
             trailing-tier fixture the window's publish block needs have not landed, so an \
             accepted window would be an authoring error nothing reports. Omit the field"
                .to_owned(),
        ));
    }
    if view.included_allowance.is_some() {
        return Err(DomainError::InvalidRequest(
            "content.included_allowance is not supported yet: the gear can store the declaration \
             and cannot honour it. Slice 10's allowance compile (the $0 band, the offset band set, \
             the display marker, the carry grant) and its six publish refusals have not landed, so \
             a stored allowance would be billed from the first unit. Omit the field"
                .to_owned(),
        ));
    }
    Ok(())
}

/// One band. An absent `to_qty` is the **open** top, not a missing value.
fn band_of(view: &TierBandView) -> Result<TierBand, DomainError> {
    Ok(TierBand {
        from_qty: view.from_qty,
        to_qty: view.to_qty.map_or(BandTop::Open, BandTop::Closed),
        unit_price_minor: MinorAmount::new(view.unit_price_minor)?,
    })
}

/// An amount, refused rather than coerced when it is negative: typed credit rows
/// are deliberately out of scope, so a negative price is a mistake.
fn amount(field: &str, raw: Option<i64>) -> Result<Option<MinorAmount>, DomainError> {
    raw.map(|value| {
        MinorAmount::new(value).map_err(|e| match e {
            DomainError::AmountNegative(_) => {
                DomainError::AmountNegative(format!("{field}: {value}"))
            }
            other => other,
        })
    })
    .transpose()
}

/// Read an optional wire token.
fn optional_token<T: Copy>(
    field: &str,
    token: Option<&str>,
    candidates: &[T],
    render: fn(T) -> &'static str,
) -> Result<Option<T>, DomainError> {
    token
        .map(|token| wire_token(field, token, candidates, render))
        .transpose()
}

/// Read a wire token back into the domain value that renders it.
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
#[path = "prices_tests.rs"]
mod prices_tests;
