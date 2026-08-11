//! The mass-repricing run's two surfaces
//! (`design/12-operator-efficiency.md` §5, §3 `inst-mr-api`, `inst-mr-journal`,
//! `inst-mr-return`, `inst-mp-journal`, `inst-mp-grandfathered`; D-88, D-134,
//! D-261, D-293, D-307).
//!
//! # What a run built here does, and — as plainly as possible — what it does not
//!
//! `POST /repricing-runs` validates the changeover instant, expands the selector
//! over the tenant's **published** rows, refuses `RUN_SELECTOR_EMPTY` if that
//! expansion is empty, opens a [`BulkKind::Repricing`] run, and freezes the row
//! set into `pricing_repricing_journal`. Then it stops. **The run rests in
//! `validating` and goes no further**: nothing here applies a price, opens a batch
//! approval, evaluates materiality, coalesces a `CatalogVersion`, or aborts. A run
//! this module opened will sit in `validating` until the apply exists, and reading
//! it back through the `GET` will show exactly that, with every journal row
//! `pending`.
//!
//! That is a deliberate slice and not an oversight, so the three debts are named
//! rather than left to be discovered:
//!
//! * **The apply** (`inst-mr-apply`, `inst-mr-validate-scope`) owes the per-plan
//!   transaction D-134 requires — successor rows, their outbox records and the
//!   journal's `pending -> applied` flips in one commit per plan.
//! * **The batch approval** (`inst-bs-approval`) still cannot be opened:
//!   `chk_pricing_approval_subject_kind` admits `bulk_operation` as of
//!   `m20260802_000065`, but that widening is the narrower half of D-158's pair —
//!   storable, not yet stored. The unit that would open one, `inst-bs-approval`
//!   itself, is unwired, so the `validating -> awaiting_approval` edge still has
//!   no writer.
//!
//!   **Paid 2026-08-11:** this arm's other gap — that opening a run wrote **no
//!   audit record at all** — is closed. `AuditSubjectKind::BulkOperation` exists,
//!   and `open_run_in` appends a `create` record on the run's own chain
//!   (`audit_repo::bulk_operation_chain`), `subject_ref` its `operation_id`
//!   (`audit_repo::bulk_operation_ref`). The bulk import's open owes the
//!   identical record still; this change gave the token a writer on the
//!   repricing side only.
//! * **`inst-mp-grandfathered` clause 2** owes the per-row refusal of an
//!   explicitly-selected grandfathered row. The *selector* half is built (see
//!   [`RunSelector::admits_grandfathered`]): a selector that does not name the
//!   eligibility axis excludes the class. A selector that names it outright still
//!   expands over those rows and freezes them into the journal — because dropping
//!   them would be the silent skip the clause forbids, and a journal row cannot be
//!   *born* `failed` (D-261: the insert trigger admits only `pending`). The apply
//!   is therefore the only place that refusal can be written.
//!
//! # `run_id` is the idempotency column, and it is also the address
//!
//! §5's Idempotency cell for `POST /repricing-runs` is `run_id` — not a client
//! key header, which is why this surface declares **no** `Idempotency-Key` and no
//! `If-Match`. The run's `client_key` column holds it, so the store's
//! `(tenant_id, kind, client_key)` unique index is what makes a second `POST`
//! under one `run_id` answer the run it opened, exactly as
//! [`crate::api::rest::bulk_imports`] replays its own key.
//!
//! `GET /repricing-runs/{runId}` addresses the run by that same `run_id` rather
//! than by the server-minted `operation_id`, and that is the substantive choice
//! here. `inst-mr-return` makes the `GET` the progress endpoint; the caller who
//! needs it most is the one whose `POST` timed out and who therefore never saw a
//! response — and a progress endpoint keyed on an id that only the response
//! carries is unreachable for exactly that caller. Both ids are on the view, so
//! nothing is hidden.
//!
//! **The two spellings of "run id" are not the same value, and the collision is
//! the schema's rather than this module's.**
//! `pricing_repricing_journal.run_id` is a foreign key to
//! `pricing_bulk_operation.operation_id` — the *minted* id — while the API's
//! `run_id` is the caller's token in `client_key`. Nothing here can rename a
//! column; what it can do is never pass one where the other belongs, which is why
//! every journal call below takes `run.operation_id` explicitly.
//!
//! # The instant is checked at [`ChangeoverMoment::Submit`] and nowhere else here
//!
//! `inst-mr-api` gives the run's changeover the same two floors `inst-su-instant`
//! gives a supersession: strictly future at submit, and a whole
//! [`MAX_BATCHING_DELAY`](crate::domain::supersession::MAX_BATCHING_DELAY) ahead
//! at the **approval commit**. Only the first is this arm's — the second belongs
//! to a commit that does not exist — and the floor is
//! [`check_changeover_instant`]'s rather than a second copy of the bound, because
//! a hand-maintained second floor is how two mechanisms come to disagree about one
//! SLO. D-144's quantum is checked first, in `plan_supersession`'s order and for a
//! sharper version of its reason: the run freezes **one** instant for every
//! selected row, so a microsecond of precision that survived to the apply would
//! not fail one supersession, it would fail all of them.
//!
//! # `Bytes`, not `Json<T>`
//!
//! The body is parsed **after** the authz gate, this directory's standing
//! discipline: `Json<T>` runs as an extractor before the handler, so a caller
//! outside the scope with a malformed body would learn their body was malformed
//! instead of learning they were denied.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::StatusCode};
use chrono::{DateTime, Utc};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::auth_context::require_authenticated;
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::overlays::{AmountRequest, adjustment_of};
use crate::api::rest::preconditions;
use crate::api::rest::prices::optional_token;
use crate::api::rest::state::AuthoringState;
use crate::domain::audit::{AuditAction, AuditSubjectKind};
use crate::domain::bulk::BulkKind;
use crate::domain::error::DomainError;
use crate::domain::money::CurrencyCode;
use crate::domain::overlay::Adjustment;
use crate::domain::repricing::RunSelector;
use crate::domain::scope_key::{
    ChargeKind, Cohort, DimensionKey, Meter, PhaseId, PlanId, PriceEligibility, Region,
};
use crate::domain::supersession::{ChangeoverMoment, check_changeover_instant};
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::repricing_journal_repo::NewJournalRow;
use crate::infra::storage::repo::{
    BulkOperationRecord, NewAuditEntry, NewBulkOperation, audit_repo, bulk_repo, price_repo,
    repricing_journal_repo,
};
use crate::infra::storage::repo_failure;

const TAG: &str = "BSS Pricing Mass Repricing";

/// `POST` — open a mass-repricing run.
pub const REPRICING_RUNS: &str = "/bss-pricing/v1/repricing-runs";

/// `GET` — the run and its journal (`inst-mr-return`'s progress endpoint).
pub const REPRICING_RUN: &str = "/bss-pricing/v1/repricing-runs/{runId}";

/// Which published rows the run acts on: any subset of the canonical key's axes.
///
/// **Spelled as [`ScopeKeyRequest`] spells them**, field for field, with two
/// differences that are both the difference between a key and a filter: every
/// member is optional, because an absent axis is one the run does not constrain;
/// and the usage pair `(meter, dimensionKey)` is here, because D-196 authors that
/// pair on a row's *content* while a selector is choosing among rows already
/// filed under it.
///
/// `plan_id` is a member rather than a path segment for
/// [`BulkImportRowRequest`](crate::api::rest::bulk_imports::BulkImportRowRequest)'s
/// reason: a run may span plans, and `pricing_bulk_operation` carries a tenant and
/// no plan.
///
/// [`ScopeKeyRequest`]: crate::api::rest::prices::ScopeKeyRequest
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct RepricingSelectorRequest {
    /// Axis 1. Absent selects across plans; named, it is also the axis D-134
    /// groups the apply's transactions by.
    pub plan_id: Option<Uuid>,
    /// Axis 2, ISO 4217 — §2's *"a currency segment"* is this member alone.
    pub currency: Option<String>,
    /// Axis 3.
    pub region: Option<String>,
    /// Axis 5. Axis 4, `priceOverlay`, is absent by construction: every row this
    /// gear authors is on the `base` plane.
    pub phase: Option<Uuid>,
    /// Axis 6 — `all_subscriptions | new_subscriptions_only |
    /// existing_grandfathered`.
    ///
    /// **The one axis whose absence narrows the run rather than widening it.**
    /// Leaving it out excludes `existing_grandfathered`
    /// (`inst-mp-grandfathered`); naming it is how an operator asks for that
    /// class, and what they then get is the per-row refusal the apply owes.
    pub price_eligibility: Option<String>,
    /// Axis 7 — `recurring | usage | one_time | one_time_setup`.
    pub charge_kind: Option<String>,
    /// Axis 8: the grandfathering generation's cutover instant.
    ///
    /// `null` is **unconstrained** here, where on
    /// [`ScopeKeyRequest`](crate::api::rest::prices::ScopeKeyRequest) it is *a row
    /// that retains nobody*. The second meaning is not lost: `cohort != none` if
    /// and only if `priceEligibility == existing_grandfathered`, so the rows that
    /// retain nobody are selected through `price_eligibility` instead.
    pub cohort: Option<DateTime<Utc>>,
    /// Axis 9 (D-196). A published metering unit.
    pub meter: Option<String>,
    /// Axis 10 (D-196). The empty string selects the **undimensioned** rows,
    /// whose column holds the empty-tuple sentinel rather than a `NULL`.
    pub dimension_key: Option<String>,
}

/// What the run does to every selected row's amount.
///
/// The four members are [`OverlayLineRequest`]'s, spelled identically and parsed
/// by [`adjustment_of`] — the same function, not a copy. `inst-mr-api` names the
/// adjustment without defining it, and a second vocabulary for "what to do to a
/// price" is the hazard the overlay plane's own line parser already exists to
/// prevent.
///
/// [`OverlayLineRequest`]: crate::api::rest::overlays::OverlayLineRequest
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct RepricingAdjustmentRequest {
    /// `markup | discount | fixed`.
    pub adjustment_kind: String,
    /// `percent_bp | amount`. **Declared, never inferred** (D-08).
    pub magnitude_kind: String,
    /// The basis-points magnitude, on a `percent_bp` adjustment.
    pub adjustment_value: Option<i64>,
    /// The per-currency values, on an `amount` adjustment.
    #[serde(default)]
    pub amounts: Vec<AmountRequest>,
}

/// `inst-mr-api`'s four fields, and nothing else.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct RepricingRunRequest {
    /// The caller's name for this run, and §5's idempotency column for the
    /// surface. It becomes the run's `client_key`, so a second `POST` under it
    /// answers the run it opened rather than opening a second one.
    pub run_id: Uuid,
    /// Which published rows to act on.
    pub selector: RepricingSelectorRequest,
    /// What to do to their amounts.
    pub adjustment: RepricingAdjustmentRequest,
    /// **One** instant for every row of the run (D-88). Strictly future at
    /// submit; the approval commit holds it to a whole batching delay.
    pub changeover: DateTime<Utc>,
}

/// One selected row and how far it got (`inst-mr-journal`).
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct RepricingJournalRowView {
    /// The published row this run selected.
    pub price_id: Uuid,
    /// `pending | applied | failed`.
    ///
    /// **`not-attempted` is not among them.** D-261 makes that a *rendering* of a
    /// `pending` row on a report, never a stored value, which is what lets a
    /// re-drive tell "never reached" from "decided".
    pub state: String,
    /// Why it did not apply. Present exactly on `failed`.
    pub failure_reason: Option<String>,
    /// The successor the apply created. Present exactly on `applied`.
    pub applied_price_id: Option<Uuid>,
    /// When that commit landed.
    pub applied_at: Option<String>,
}

/// A run and its journal — `inst-mr-return`'s progress endpoint, whole.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct RepricingRunView {
    /// The caller's `run_id`, which is what the `GET` is addressed by.
    pub run_id: String,
    /// The run's durable row name, and what the journal's own `run_id` column
    /// holds. Both are on the view because they are two different values.
    pub operation_id: Uuid,
    /// One of §4's seven states. A run this arm opened reads `validating` and
    /// stays there: see the module doc.
    pub state: String,
    /// The frozen run parameters — selector, adjustment, changeover, and how many
    /// rows the expansion found.
    pub report: serde_json::Value,
    /// When the run was submitted.
    pub submitted_at: String,
    /// When it ended; absent while it is still going.
    pub completed_at: Option<String>,
    /// The frozen row set, in `price_id` order.
    pub journal: Vec<RepricingJournalRowView>,
}

/// What this surface needs.
pub struct ApiState {
    /// The authoring plane's state. No field of its own, for
    /// [`crate::api::rest::bulk_imports::ApiState`]'s reason: the expansion is a
    /// free function over a runner and the run's own statements are too, so what
    /// is wanted here is the provider and the scope every read goes through.
    pub authoring: Arc<AuthoringState>,
}

/// Build the router for the run's two surfaces.
pub fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let mut router = Router::new();

    router = OperationBuilder::post(REPRICING_RUNS)
        .operation_id("bss_pricing.open_repricing_run")
        .summary("Open a mass-repricing run")
        .description(
            "Expands the selector over the tenant's published price rows, freezes that row set \
             into the run's journal, and answers `202` with the run ref. The selector is any \
             subset of the canonical scope key's axes; rows in the `existing_grandfathered` class \
             are excluded unless the selector names that class, since they are immutable in \
             price. A selector matching no published row is refused `400` \
             `RUN_SELECTOR_EMPTY` and opens nothing, so the `run_id` stays available for a \
             corrected selector. The changeover instant must be strictly in the future. \
             Idempotency is the `run_id`: a second call under one answers the run it opened. \
             Progress is at `GET /bss-pricing/v1/repricing-runs/{runId}`. The run rests in \
             `validating`: applying the adjustment, the batch approval and the abort are not \
             built.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<RepricingRunRequest>(openapi, "The run to open.")
        .handler(open_repricing_run)
        .json_response_with_schema::<RepricingRunView>(
            openapi,
            StatusCode::ACCEPTED,
            "The run, with its frozen journal.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    router = OperationBuilder::get(REPRICING_RUN)
        .operation_id("bss_pricing.read_repricing_run")
        .summary("Read a mass-repricing run's progress")
        .description(
            "The run and its per-row journal, addressed by the `run_id` the caller opened it \
             under. Gates on `plan` x `read`: a run's journal is price data, and an operator who \
             may read prices may read what a run did to them.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("runId", "The run_id the run was opened under.")
        .handler(read_repricing_run)
        .json_response_with_schema::<RepricingRunView>(
            openapi,
            StatusCode::OK,
            "The run and its journal.",
        )
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    // D-178's edge. The `POST` writes — a run row and a journal row per selected
    // price — so it travels with the routes rather than with whoever merges them.
    router
        .layer(Extension(state))
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}

/// `POST /repricing-runs`.
async fn open_repricing_run(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    // **For a value now, not only for its refusal.** Until 2026-08-11 this arm wrote
    // no audit record — there was no `AuditSubjectKind` for a bulk operation — so the
    // ask here was only D-178's edge: a router mounted without it must answer 403
    // rather than 500 at the first record a later arm writes. `open_run_in` now
    // writes that record itself, on this same request, and D-178 clause (2) requires
    // every record and outbox row one operator call produces to carry **one**
    // correlation id — so the value taken here is the one that record carries.
    let correlation_id = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let scope = write_scope(&enforcer, &ctx).await?;
    // Parsed after the gate; see the module doc for why `Json<T>` is not used.
    let body: RepricingRunRequest = preconditions::parse_body(&body)?;
    let client_key = body.run_id.to_string();

    let conn =
        state.authoring.db.conn().map_err(|e| {
            CanonicalError::from(DomainError::Internal(format!("repricing run: {e}")))
        })?;

    // **The replay comes before every judgement below**, and that order is the
    // contract rather than an optimisation: a replay answers what the first call
    // answered, so a run accepted yesterday is still answered today even though its
    // changeover has since fallen behind the submit floor and its selector may now
    // match different rows. Re-judging either would turn a retry into a refusal of
    // work that was already accepted -- which is the one conclusion idempotency
    // exists to prevent, for exactly the client that retried on a timeout.
    if let Some(existing) =
        bulk_repo::find_by_client_key(&conn, &scope, tenant, BulkKind::Repricing, &client_key)
            .await
            .map_err(|e| CanonicalError::from(repo_failure(&e)))?
    {
        let view = run_view(&conn, &scope, tenant, &existing).await?;
        return Ok((StatusCode::ACCEPTED, Json(view)).into_response());
    }

    let selector = selector_of(&body.selector)?;
    let adjustment = adjustment_of(
        &body.adjustment.adjustment_kind,
        &body.adjustment.magnitude_kind,
        body.adjustment.adjustment_value,
        &body.adjustment.amounts,
    )?;
    // D-144's quantum before the distance, `plan_supersession`'s order: a malformed
    // instant is not an instant whose distance is worth measuring.
    crate::domain::instant::check_quantum("changeover", body.changeover)?;
    let now = Utc::now();
    check_changeover_instant(body.changeover, now, ChangeoverMoment::Submit)?;

    let selected = price_repo::load_published_for_selector(&conn, &scope, tenant, &selector)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    if selected.is_empty() {
        // **Refused before anything is opened, so the `run_id` is not spent.** That
        // is the one place this surface diverges from `bulk_imports`, whose Phase-1
        // refusal deliberately keeps its key: there the run has to exist because it
        // holds the per-row report the refusal points at, and `inst-bs-reject` rests
        // on a spent key being auditable. Here there is no report to hold -- nothing
        // was selected, so there is nothing to record per row -- and the remedy is
        // to correct one field of the request. Charging an operator a fresh `run_id`
        // for a mistyped region would be a cost with nothing bought by it. The
        // design set does not settle this; it is stated here because it is a choice.
        return Err(CanonicalError::from(DomainError::RunSelectorEmpty(
            empty_selector_detail(&selector),
        )));
    }

    let report = frozen_report(&selector, &adjustment, body.changeover, selected.len());
    let run = open_run(
        &state.authoring.db,
        &scope,
        tenant,
        NewBulkOperation {
            operation_id: Uuid::now_v7(),
            tenant_id: tenant,
            kind: BulkKind::Repricing,
            client_key,
            report,
            submitted_by: ctx.subject_id(),
            submitted_at: now,
        },
        selected,
        correlation_id,
    )
    .await?;

    let view = run_view(&conn, &scope, tenant, &run).await?;
    Ok((StatusCode::ACCEPTED, Json(view)).into_response())
}

/// `GET /repricing-runs/{runId}`.
async fn read_repricing_run(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<RepricingRunView>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant = ctx.subject_tenant_id();
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::READ,
        /* owner_tenant_id */ None,
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    let conn =
        state.authoring.db.conn().map_err(|e| {
            CanonicalError::from(DomainError::Internal(format!("repricing run: {e}")))
        })?;
    let run = bulk_repo::find_by_client_key(
        &conn,
        &scope,
        tenant,
        BulkKind::Repricing,
        &run_id.to_string(),
    )
    .await
    .map_err(|e| CanonicalError::from(repo_failure(&e)))?
    .ok_or_else(|| {
        CanonicalError::from(DomainError::NotFound {
            subject: "repricing run".to_owned(),
            id: run_id.to_string(),
        })
    })?;
    Ok(Json(run_view(&conn, &scope, tenant, &run).await?))
}

/// Open the run, freeze its row set and record it, **in one transaction**.
///
/// The atomicity is required rather than tidy, and
/// [`repricing_journal_repo::open_rows`] says so from its own side: it opens no
/// transaction by design (D-134 makes the run's transaction unit the plan) and
/// obliges the expansion to hold one across the whole set. Without it a partial
/// freeze leaves a run whose completion predicate — *no `pending` rows remain* —
/// is satisfiable by rows that were never selected, and a run open with no journal
/// at all is a run that reports success having touched nothing. The audit record
/// belongs in the same transaction for D-14's reason every writer in this crate
/// gives: a crash between the insert and the record must not produce one without
/// the other.
async fn open_run(
    db: &toolkit_db::DBProvider<toolkit_db::DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    new: NewBulkOperation,
    selected: Vec<Uuid>,
    correlation_id: Uuid,
) -> Result<BulkOperationRecord, CanonicalError> {
    let scope = scope.clone();
    let (_, outcome) = db
        .db()
        .in_transaction::<BulkOperationRecord, RepoError, _>(move |txn| {
            Box::pin(async move {
                open_run_in(txn, &scope, tenant_id, new, &selected, correlation_id).await
            })
        })
        .await;
    outcome
        .map_err(|err| {
            err.into_domain(|infra| {
                RepoError::Db(format!("bss-pricing: repricing run transaction: {infra}"))
            })
        })
        .map_err(|e| CanonicalError::from(repo_failure(&e)))
}

/// [`open_run`]'s body, on the transaction's runner.
async fn open_run_in(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    new: NewBulkOperation,
    selected: &[Uuid],
    correlation_id: Uuid,
) -> Result<BulkOperationRecord, RepoError> {
    let actor_principal_id = new.submitted_by;
    let recorded_at = new.submitted_at;
    let kind = new.kind;
    let row_count = selected.len();
    let run = bulk_repo::open(runner, scope, new).await?;
    // **`tenant_id` is written here and never taken from the request.** The
    // journal's only foreign key covers `run_id`, so nothing in the schema stops a
    // journal row carrying a foreign tenant — `NewJournalRow` states the same
    // obligation from the other end.
    let rows: Vec<NewJournalRow> = selected
        .iter()
        .map(|price_id| NewJournalRow {
            run_id: run.operation_id,
            price_id: *price_id,
            tenant_id,
        })
        .collect();
    repricing_journal_repo::open_rows(runner, scope, &rows).await?;

    // The debt `repricing_runs`' own module doc named: until `AuditSubjectKind`
    // carried a `BulkOperation` member, opening a run wrote no audit record at
    // all. `subject_ref` is `audit_repo::bulk_operation_ref(run.operation_id)`,
    // so the audit record and the batch approval `inst-bs-approval` will one day
    // open name this run identically — D-158's alignment, paid in advance of the
    // approval writer that does not exist yet.
    audit_repo::append(
        runner,
        scope,
        NewAuditEntry {
            tenant_id,
            chain_id: audit_repo::bulk_operation_chain(run.operation_id),
            recorded_at,
            actor_principal_id,
            action: AuditAction::Create,
            subject_kind: AuditSubjectKind::BulkOperation,
            subject_ref: audit_repo::bulk_operation_ref(run.operation_id),
            before_state: None,
            after_state: Some(serde_json::json!({
                "kind": kind.as_str(),
                "state": run.state.as_str(),
                "rowCount": row_count,
            })),
            approval_ref: None,
            correlation_id,
        },
    )
    .await?;

    Ok(run)
}

/// Why the expansion came back empty, in the operator's terms.
///
/// Two selectors are empty for a reason their own axes do not show, and for those
/// two a bare *"matched nothing"* sends an author hunting the catalog for rows
/// that are there:
///
/// * one naming **no axis at all** is the whole published catalog, so an empty
///   result means the tenant has none outside the retained class;
/// * one naming a **cohort** without naming `existing_grandfathered` is empty *by
///   construction*, whatever the tenant has published. A cohort exists only on the
///   retained class — `check_cohort_eligibility`'s biconditional — and that class
///   is excluded until the eligibility axis names it (`inst-mp-grandfathered`), so
///   the two conditions cannot both hold. Naming the class beside the cohort is
///   the request the author meant, and the message says so rather than leaving
///   them to derive it from two rules in different documents.
fn empty_selector_detail(selector: &RunSelector) -> String {
    let because = if selector.is_unconstrained() {
        " and named no axis at all, so this tenant has no published row outside the retained \
         grandfathered class"
    } else if selector.cohort.is_some_and(|cohort| !cohort.is_none())
        && !selector.admits_grandfathered()
    {
        ". A cohort names a retained generation, and the retained class is excluded until the \
         price_eligibility axis names existing_grandfathered, so this selector is empty whatever \
         is published; name the class beside the cohort"
    } else {
        ""
    };
    format!(
        "the selector matched no published price row{because}; a run over an empty set completes \
         the moment it opens, which would report a mass adjustment that never happened"
    )
}

/// The run's parameters, frozen onto its report.
///
/// `report` is the **only** column `pricing_bulk_operation` has for this: §6 gives
/// a run `kind`, `state`, `client_key`, `report`, `submitted_by` and timestamps,
/// and the apply needs the adjustment and the instant that were accepted. So they
/// go here, and they are rendered from the **parsed domain values** rather than
/// echoed from the request — an echo would store a currency the parse
/// uppercased and a region it trimmed, i.e. a record of what was sent instead of a
/// record of what was selected against.
///
/// `rows` is deliberately **not** in it. The row set is the journal's
/// (`inst-mr-journal`), and a second copy on the report would be a second answer
/// to "what did this run select" that no transition keeps in step.
fn frozen_report(
    selector: &RunSelector,
    adjustment: &Adjustment,
    changeover: DateTime<Utc>,
    selected: usize,
) -> serde_json::Value {
    let amounts: serde_json::Value = adjustment.amounts().map_or_else(
        || serde_json::json!({}),
        |set| {
            set.iter()
                .map(|(currency, value)| (currency.as_str().to_owned(), serde_json::json!(value)))
                .collect()
        },
    );
    serde_json::json!({
        "selector": {
            "plan_id": selector.plan_id.map(|p| p.get().to_string()),
            "currency": selector.currency.as_ref().map(CurrencyCode::as_str),
            "region": selector.region.as_ref().map(Region::as_str),
            "phase": selector.phase.map(|p| p.get().to_string()),
            "price_eligibility": selector.price_eligibility.map(PriceEligibility::as_str),
            "charge_kind": selector.charge_kind.map(ChargeKind::as_str),
            "cohort": selector.cohort.map(|c| c.to_string()),
            "meter": selector.meter.as_ref().map(Meter::as_str),
            "dimension_key": selector.dimension_key.as_ref().map(DimensionKey::as_str),
        },
        "adjustment": {
            "adjustment_kind": adjustment.kind(),
            "magnitude_kind": adjustment.magnitude_kind(),
            "adjustment_value": adjustment.percent_bp(),
            "amounts": amounts,
        },
        "changeover": changeover.to_rfc3339(),
        "selected": selected,
    })
}

/// The run and its journal, as one view.
///
/// The journal is read on **every** rendering, including the `POST`'s, because
/// `inst-mr-return` promises a run ref *and* a progress endpoint and a `202` whose
/// journal came from the caller's own request rather than from the store would be
/// a receipt for a freeze that may not have happened. That is the defect D-225
/// records for the overlay submit's `202`.
async fn run_view(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    run: &BulkOperationRecord,
) -> Result<RepricingRunView, CanonicalError> {
    let journal = repricing_journal_repo::list_for_run(runner, scope, tenant_id, run.operation_id)
        .await
        .map_err(|e| CanonicalError::from(repo_failure(&e)))?;
    Ok(RepricingRunView {
        run_id: run.client_key.clone(),
        operation_id: run.operation_id,
        state: run.state.as_str().to_owned(),
        report: run.report.clone(),
        submitted_at: run.submitted_at.to_rfc3339(),
        completed_at: run.completed_at.map(|at| at.to_rfc3339()),
        journal: journal
            .into_iter()
            .map(|row| RepricingJournalRowView {
                price_id: row.price_id,
                state: row.state.as_str().to_owned(),
                failure_reason: row.failure_reason,
                applied_price_id: row.applied_price_id,
                applied_at: row.applied_at.map(|at| at.to_rfc3339()),
            })
            .collect(),
    })
}

/// The submitted selector as the domain's.
///
/// Each axis is validated by the **same** constructor the authoring plane uses, so
/// a selector cannot name a value no row could have been filed under: a blank
/// region, a two-letter currency or a token outside the eligibility enumeration is
/// refused here rather than silently matching nothing and being reported as
/// `RUN_SELECTOR_EMPTY` — which would send an operator hunting for missing rows
/// when what they have is a typo.
fn selector_of(request: &RepricingSelectorRequest) -> Result<RunSelector, DomainError> {
    Ok(RunSelector {
        plan_id: request.plan_id.map(PlanId::new),
        currency: request
            .currency
            .as_deref()
            .map(CurrencyCode::new)
            .transpose()?,
        region: request.region.as_deref().map(Region::new).transpose()?,
        phase: request.phase.map(PhaseId::new),
        price_eligibility: optional_token(
            "selector.price_eligibility",
            request.price_eligibility.as_deref(),
            price_repo::PRICE_ELIGIBILITIES,
            PriceEligibility::as_str,
        )?,
        charge_kind: optional_token(
            "selector.charge_kind",
            request.charge_kind.as_deref(),
            price_repo::CHARGE_KINDS,
            ChargeKind::as_str,
        )?,
        cohort: request.cohort.map(Cohort::Generation),
        meter: request.meter.as_deref().map(Meter::new).transpose()?,
        dimension_key: request.dimension_key.as_deref().map(DimensionKey::new),
    })
}

/// The `plan x write` gate the `POST` takes.
///
/// `resource_id = None` for `bulk_imports`' reason, sharpened by D-281: a run
/// spans plans and its selector names a *set*, so there is no single resource to
/// name — and an id-shaped constraint could not filter these tables anyway.
async fn write_scope(
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
) -> Result<AccessScope, CanonicalError> {
    crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(ctx.subject_tenant_id()),
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)
}
