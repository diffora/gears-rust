//! The `PriceOverlay` authoring surface — `design/09-price-overlays.md` §5,
//! `inst-pl-author` / `inst-pl-validate` / `inst-pl-return` / `inst-pl-commit`.
//!
//! Three routes: author or edit an overlay draft, submit it, and list them.
//!
//! # A save lands a **draft only**, and nothing publishes from it
//!
//! `inst-pl-return` is explicit (2026-07-28 review fix, confirmed 2026-07-31):
//! the `POST`/`PATCH` answers 201/200 and the overlay is a `draft`. Publishing
//! is the submit's, and the submit is **always material** (D-50) — overlay
//! creation, line add or remove, magnitude, kind, audience, precedence, dating
//! and disclosure changes all route through the Slice 5 approval workflow before
//! anything publishes. That is why this module's `POST` never touches the
//! lifecycle and why the submit answers **202**.
//!
//! # `PATCH` addresses a resource, so it is not the collection `POST`
//!
//! §5 spells both halves as `POST/PATCH /bss-pricing/v1/price-overlays` — one
//! path, two methods, the subject in the body. This mounts the `PATCH` as
//! `/bss-pricing/v1/price-overlays/{overlayId}` instead, for the reason Slice 8
//! reported for its own composition route (owed entry B-10): a precondition
//! addresses a resource, and a collection that answers `If-Match` is a
//! collection pretending to be one. The divergence is in the owed register
//! rather than smoothed over.
//!
//! # What `If-Match` carries here, and why it is not the plan's
//!
//! An overlay has **no plan**: D-92 gives it a revision chain of its own, so the
//! entity tag is the overlay revision's `"<revision>-<version>"` (D-170's shape)
//! and not a plan revision's. `BundleRepo`'s composition borrows the plan's tag
//! because a bundle rides a plan; nothing here does.
//!
//! # 422 does not exist on this platform
//!
//! §5 types seven of the nine overlay codes 422, and that notation is
//! architectural: `CanonicalError` renders `InvalidArgument`,
//! `FailedPrecondition` and `OutOfRange` all as **400**, and the **code string**
//! is the discriminator a consumer matches on. Those seven travel inside the
//! `ValidationFailed` envelope, one violation per failing rule, which is what
//! makes an overlay remediable in one pass.
//!
//! **The other two do not, and that is §5's own typing rather than a choice
//! here.** `PRECEDENCE_DUPLICATE` and `OVERLAY_INTERVAL_OVERLAP` are typed
//! **409**: they are conflicts on state a sibling overlay holds, which a caller
//! resolves by re-reading rather than by editing their own request. So
//! [`conflict_of`](crate::domain::overlay_rules::conflict_of) lifts either of
//! them out of the report and answers the typed conflict, and everything else
//! goes in the envelope. No route here declares a 422.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, Query};
use axum::http::header::{ETAG, LOCATION};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::HeaderMap, http::StatusCode};
use chrono::{DateTime, Utc};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::auth_context::{audit_stamp, require_authenticated};
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::preconditions;
use crate::api::rest::state::AuthoringState;
use crate::domain::error::DomainError;
use crate::domain::money::CurrencyCode;
use crate::domain::overlay::{
    Adjustment, AmountSet, Disclosure, LineKey, Magnitude, OverlayInterval, OverlayLine,
    ScopeClass, ScopeSelector, ScopeValue, TargetRef, TargetSku, TaxBasis,
};
use crate::domain::overlay_rules::{
    OverlayCandidate, check_magnitudes, check_tax_basis_declared, conflict_of, validate,
};
use crate::domain::scope_key::PlanId;
use crate::infra::storage::repo::{NewOverlay, OverlayRecord};

const TAG: &str = "BSS Pricing Overlays";

/// `POST` — author an overlay draft; `GET` — list them.
pub const PRICE_OVERLAYS: &str = "/bss-pricing/v1/price-overlays";
/// `PATCH` — replace an open draft revision's whole line set.
pub const PRICE_OVERLAY_BY_ID: &str = "/bss-pricing/v1/price-overlays/{overlayId}";
/// `POST` — submit the draft for the always-material approval unit.
pub const PRICE_OVERLAY_SUBMIT: &str = "/bss-pricing/v1/price-overlays/{overlayId}/submit";

// ---------------------------------------------------------------------------
// Wire types. The wire is `snake_case` — `toolkit_macros::api_dto` does not
// rename — so the design set's `orgTier` is `org_tier` here and everywhere.
// ---------------------------------------------------------------------------

/// `POST /bss-pricing/v1/price-overlays`.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct CreateOverlayRequest {
    /// `partner | org_tier | brand | region | customer_group | global`.
    pub scope_class: String,
    /// The taxonomy value. **Absent exactly when the class is `global`** — the
    /// pairing is unrepresentable in the domain, so a mismatch is refused here.
    pub scope_value: Option<String>,
    /// L2's explicit precedence, unique within the class among published
    /// overlays.
    pub precedence: i32,
    /// `inclusive | exclusive | delegated_tariffs`. **Optional on the wire and
    /// required in substance**: L5 says the basis MUST be declared and
    /// `TAX_BASIS_UNDECLARED` is what an absent one is told. Modelling it as
    /// `Option` is what makes that code reachable — a required field would be
    /// refused by the deserializer with a message the design set does not own.
    pub tax_basis: Option<String>,
    /// `restricted | public`. Absent means `restricted`, L6's fail-closed
    /// default.
    pub disclosure: Option<String>,
    /// Inclusive start of the overlay's own interval.
    pub effective_from: Option<DateTime<Utc>>,
    /// Exclusive end of it.
    pub effective_to: Option<DateTime<Utc>>,
    /// The plans the lines may target.
    pub target_plan_ids: Vec<Uuid>,
    /// The adjustment lines, whole. At least one (D-42).
    pub lines: Vec<OverlayLineRequest>,
}

/// `PATCH /bss-pricing/v1/price-overlays/{overlayId}` — the line set, whole.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct ReplaceLinesRequest {
    /// The revision being edited. Must be the open draft.
    pub revision: u64,
    /// The lines, **replaced and never merged**: every D-42 rule quantifies over
    /// the set, so a partial update leaves nothing the validator can evaluate.
    pub lines: Vec<OverlayLineRequest>,
}

/// One authored adjustment line.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct OverlayLineRequest {
    /// The line's identity. Absent mints one; supplying it is how an edit keeps
    /// a line's identity stable across revisions (D-92).
    pub line_id: Option<Uuid>,
    /// `None` is the **list-default line**, which applies to every target.
    pub plan_id: Option<Uuid>,
    /// Optional narrowing. Requires `plan_id`.
    pub target_sku: Option<String>,
    /// The grandfathered generation this line filters to (D-78). Requires
    /// `plan_id`.
    pub cohort: Option<DateTime<Utc>>,
    /// `markup | discount | fixed`.
    pub adjustment_kind: String,
    /// `percent_bp | amount`. **Declared, never inferred** (D-08).
    pub magnitude_kind: String,
    /// The basis-points magnitude, on a `percent_bp` line.
    pub adjustment_value: Option<i64>,
    /// The per-currency values, on an `amount` line.
    #[serde(default)]
    pub amounts: Vec<AmountRequest>,
}

/// One currency's value on an amount-based line.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct AmountRequest {
    /// ISO 4217.
    pub currency: String,
    /// The magnitude, in the currency's ISO 4217 minor unit.
    pub value_minor: i64,
}

/// `GET /bss-pricing/v1/price-overlays` — the query.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ListOverlaysQuery {
    /// Narrow to one scope class.
    pub scope_class: Option<String>,
}

/// One overlay revision, as the read surface renders it.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct OverlayView {
    /// The overlay's identity.
    pub price_overlay_id: Uuid,
    /// Which revision this is.
    pub revision: u64,
    /// `draft | published | superseded`.
    pub lifecycle_state: String,
    /// Its scope class.
    pub scope_class: String,
    /// Its scope value, absent for `global`.
    pub scope_value: Option<String>,
    /// Its precedence.
    pub precedence: i32,
    /// Its declared basis.
    pub tax_basis: String,
    /// Its exposure flag.
    pub disclosure: String,
    /// Inclusive start of its own interval.
    pub effective_from: Option<DateTime<Utc>>,
    /// Exclusive end of it.
    pub effective_to: Option<DateTime<Utc>>,
    /// The plans its lines may target.
    pub target_plan_ids: Vec<Uuid>,
    /// Its lines.
    pub lines: Vec<OverlayLineView>,
}

/// One line, as the read surface renders it.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct OverlayLineView {
    /// The line's identity, stable across revisions that do not change it.
    pub line_id: Uuid,
    /// Its target plan, absent on the list-default line.
    pub plan_id: Option<Uuid>,
    /// Its SKU narrowing.
    pub target_sku: Option<String>,
    /// The generation it filters to.
    pub cohort: Option<DateTime<Utc>>,
    /// `markup | discount | fixed`.
    pub adjustment_kind: String,
    /// `percent_bp | amount`.
    pub magnitude_kind: String,
    /// The bp magnitude, on a percent line.
    pub adjustment_value: Option<i64>,
    /// The per-currency values, on an amount line.
    pub amounts: Vec<AmountRequest>,
}

/// What a list read answers with.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct OverlayListView {
    /// The overlays, ordered by precedence then id then revision.
    pub overlays: Vec<OverlayView>,
}

/// What an edit answers with.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct OverlayAcceptedView {
    /// The overlay that was written.
    pub price_overlay_id: Uuid,
    /// The revision it now stands at.
    pub revision: u64,
}

/// What a submit answers with.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct SubmitAcceptedView {
    /// The overlay submitted.
    pub price_overlay_id: Uuid,
    /// The revision submitted.
    pub revision: u64,
    /// Why this submit is material. **Always** `alwaysMaterialTrigger` (D-50):
    /// an overlay line has no per-currency baseline to threshold, so the G1
    /// no-delta rule applies and no threshold can make it immaterial. Stated in
    /// the response because an operator who expected auto-publish under a
    /// configured threshold needs the reason, not only the outcome.
    pub materiality: String,
    /// The advisory findings the pipeline raised. Warnings never block, and this
    /// is the channel that makes them advisory — the `ValidationFailed` envelope
    /// exists only on the rejecting path, so a warning carried only there would
    /// be computed and discarded (the defect D-197 records for the plan plane).
    pub warnings: Vec<AdvisoryView>,
}

/// One advisory finding.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct AdvisoryView {
    /// The machine-readable code — `FIXED_LINE_DISCARDS_STACK`, `TARGET_RETIRED`.
    pub code: String,
    /// What it is about.
    pub subject: String,
    /// What the author is told.
    pub detail: String,
}

// ---------------------------------------------------------------------------
// Conversions.
// ---------------------------------------------------------------------------

/// The scope selector, refusing every pairing the domain makes unrepresentable.
fn scope_of(class: &str, value: Option<&str>) -> Result<ScopeSelector, DomainError> {
    let class = ScopeClass::parse(class).ok_or_else(|| {
        DomainError::InvalidRequest(format!(
            "scope_class `{class}` is not one of partner, org_tier, brand, region, \
             customer_group, global"
        ))
    })?;
    match (class, value) {
        (ScopeClass::Global, None) => Ok(ScopeSelector::Global),
        (ScopeClass::Global, Some(_)) => Err(DomainError::InvalidRequest(
            "the global scope class carries no scope_value: it selects every payer of the \
             tenant, so there is no value for a taxonomy to declare"
                .to_owned(),
        )),
        (_, None) => Err(DomainError::InvalidRequest(format!(
            "a {class}-scoped overlay must name a scope_value; only the global class has none"
        ))),
        (_, Some(raw)) => {
            let value = ScopeValue::new(raw).ok_or_else(|| {
                DomainError::InvalidRequest(
                    "scope_value may not be blank: the empty string is the store's sentinel \
                     for the classless scope"
                        .to_owned(),
                )
            })?;
            ScopeSelector::scoped(class, value).ok_or_else(|| {
                DomainError::Internal(
                    "scope pairing refused after its class was checked".to_owned(),
                )
            })
        }
    }
}

/// One authored line, with every pairing §6 spends a `CHECK` on refused here by
/// construction.
fn line_of(request: &OverlayLineRequest) -> Result<OverlayLine, DomainError> {
    let key = match (request.plan_id, request.target_sku.as_deref()) {
        (None, None) => LineKey::list_default(),
        (None, Some(_)) => {
            return Err(DomainError::InvalidRequest(
                "a target_sku line must name its plan_id: a bare SKU is ambiguous per \
                 (currency, region)"
                    .to_owned(),
            ));
        }
        (Some(plan), None) => LineKey::for_plan(PlanId::new(plan)),
        (Some(plan), Some(raw)) => {
            let sku = TargetSku::new(raw).ok_or_else(|| {
                DomainError::InvalidRequest("target_sku may not be blank".to_owned())
            })?;
            LineKey::for_sku(PlanId::new(plan), sku)
        }
    };
    let key = match request.cohort {
        None => key,
        Some(cohort) => key.for_cohort(cohort).ok_or_else(|| {
            DomainError::InvalidRequest(
                "a cohort line must name its plan_id: the cohort is validated against the \
                 line's target plan, which the list-default line does not have (D-78)"
                    .to_owned(),
            )
        })?,
    };

    let amounts = {
        let mut set = Vec::with_capacity(request.amounts.len());
        for amount in &request.amounts {
            set.push((CurrencyCode::new(&amount.currency)?, amount.value_minor));
        }
        AmountSet::new(set)
    };

    let magnitude = match request.magnitude_kind.as_str() {
        "percent_bp" => Magnitude::PercentBp(request.adjustment_value.ok_or_else(|| {
            DomainError::InvalidRequest(
                "a percent_bp line must carry an adjustment_value: the magnitude's type is \
                 declared and never inferred (D-08)"
                    .to_owned(),
            )
        })?),
        "amount" => {
            if request.adjustment_value.is_some() {
                return Err(DomainError::InvalidRequest(
                    "an amount line must not carry an adjustment_value: its magnitude is money \
                     and lives per currency (D-08)"
                        .to_owned(),
                ));
            }
            Magnitude::Amount(amounts.clone())
        }
        other => {
            return Err(DomainError::InvalidRequest(format!(
                "magnitude_kind `{other}` is neither percent_bp nor amount"
            )));
        }
    };

    let adjustment = match request.adjustment_kind.as_str() {
        "markup" => Adjustment::Markup(magnitude),
        "discount" => Adjustment::Discount(magnitude),
        "fixed" => {
            if request.magnitude_kind != "amount" {
                return Err(DomainError::InvalidRequest(
                    "a fixed line is always amount-based: it replaces the running amount with \
                     an absolute price, and a percentage of the amount it replaces evaluates to \
                     nothing (D-138)"
                        .to_owned(),
                ));
            }
            Adjustment::Fixed(amounts)
        }
        other => {
            return Err(DomainError::InvalidRequest(format!(
                "adjustment_kind `{other}` is none of markup, discount, fixed"
            )));
        }
    };

    Ok(OverlayLine {
        line_id: request.line_id.unwrap_or_else(Uuid::now_v7),
        key,
        adjustment,
    })
}

/// Every authored line, with D-67's ranges checked **before** the store sees
/// them.
///
/// The range is the one rule that needs no world, and D-67 says it fails save as
/// well as publish. Checking it here is not belt-and-braces: the store's two
/// `CHECK`s fire on the INSERT and reach the caller as a driver error — a **500**
/// for a request whose whole remedy is to correct one number. See
/// [`check_magnitudes`] for the measurement.
fn lines_of(requests: &[OverlayLineRequest]) -> Result<Vec<OverlayLine>, DomainError> {
    let lines: Vec<OverlayLine> = requests
        .iter()
        .map(line_of)
        .collect::<Result<_, DomainError>>()?;
    let report = check_magnitudes(&lines);
    if report.is_publishable() {
        Ok(lines)
    } else {
        Err(DomainError::ValidationFailed(report))
    }
}

fn view_of(record: &OverlayRecord) -> OverlayView {
    OverlayView {
        price_overlay_id: record.price_overlay_id,
        revision: record.revision,
        lifecycle_state: record.lifecycle_state.as_str().to_owned(),
        scope_class: record.scope.class().as_str().to_owned(),
        scope_value: record.scope.value().map(|v| v.as_str().to_owned()),
        precedence: record.precedence,
        tax_basis: record.tax_basis.as_str().to_owned(),
        disclosure: record.disclosure.as_str().to_owned(),
        effective_from: record.interval.from,
        effective_to: record.interval.to,
        target_plan_ids: record.target_ref.plans.iter().map(|p| p.get()).collect(),
        lines: record.lines.iter().map(line_view_of).collect(),
    }
}

fn line_view_of(line: &OverlayLine) -> OverlayLineView {
    OverlayLineView {
        line_id: line.line_id,
        plan_id: line.key.plan_id().map(PlanId::get),
        target_sku: line.key.target_sku().map(|s| s.as_str().to_owned()),
        cohort: line.key.cohort(),
        adjustment_kind: line.adjustment.kind().to_owned(),
        magnitude_kind: line.adjustment.magnitude_kind().to_owned(),
        adjustment_value: line.adjustment.percent_bp(),
        amounts: line.adjustment.amounts().map_or_else(Vec::new, |set| {
            set.iter()
                .map(|(currency, value_minor)| AmountRequest {
                    currency: currency.as_str().to_owned(),
                    value_minor,
                })
                .collect()
        }),
    }
}

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

async fn create_overlay(
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
        &crate::authz::resource_types::PRICE_OVERLAY,
        crate::authz::actions::WRITE,
        Some(tenant),
        None,
        true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    // Parsed after the gate, as `plans.rs`, `prices.rs` and `bundles.rs` order it.
    let body: CreateOverlayRequest = preconditions::parse_body(&body)?;
    let _client_key = preconditions::idempotency_key(&headers)?;

    let selector = scope_of(&body.scope_class, body.scope_value.as_deref())?;
    // L5's *silence fails*, on its own entry point: `tax_basis` is NOT NULL in
    // the store and `TaxBasis` is closed, so this is the only place the code is
    // reachable. It is raised as a one-violation report rather than as a generic
    // bad request, so `TAX_BASIS_UNDECLARED` reaches the wire as the
    // discriminator §5 declares rather than buried in a message.
    let tax_basis = match body.tax_basis.as_deref() {
        None => {
            let code = check_tax_basis_declared(None)
                .err()
                .unwrap_or("TAX_BASIS_UNDECLARED");
            let mut report = crate::domain::validation::ValidationReport::default();
            report.violate(
                code,
                "tax_basis",
                "an overlay must declare its tax basis or explicitly delegate it to Tariffs; \
                 silence fails (L5)",
            );
            return Err(DomainError::ValidationFailed(report).into());
        }
        Some(token) => TaxBasis::parse(token).ok_or_else(|| {
            DomainError::InvalidRequest(format!(
                "tax_basis `{token}` is none of inclusive, exclusive, delegated_tariffs"
            ))
        })?,
    };
    let disclosure = match body.disclosure.as_deref() {
        None => Disclosure::Restricted,
        Some(token) => Disclosure::parse(token).ok_or_else(|| {
            DomainError::InvalidRequest(format!(
                "disclosure `{token}` is neither restricted nor public"
            ))
        })?,
    };
    let lines = lines_of(&body.lines)?;

    let price_overlay_id = Uuid::now_v7();
    let stamp = audit_stamp(&ctx, Utc::now(), correlation);
    let revision = state
        .overlays
        .create(
            &scope,
            NewOverlay {
                price_overlay_id,
                tenant_id: tenant,
                scope: selector,
                precedence: body.precedence,
                interval: OverlayInterval {
                    from: body.effective_from,
                    to: body.effective_to,
                },
                tax_basis,
                disclosure,
                target_ref: TargetRef {
                    plans: body
                        .target_plan_ids
                        .iter()
                        .map(|p| PlanId::new(*p))
                        .collect(),
                },
            },
            lines,
            stamp,
        )
        .await
        .map_err(|e| crate::infra::storage::repo_failure(&e))?;

    // 201, and a **draft**: nothing publishes from a save (`inst-pl-return`).
    let mut response = (
        StatusCode::CREATED,
        Json(OverlayAcceptedView {
            price_overlay_id,
            revision,
        }),
    )
        .into_response();
    response.headers_mut().insert(
        LOCATION,
        format!("/bss-pricing/v1/price-overlays/{price_overlay_id}")
            .parse()
            .map_err(|_| DomainError::InvalidRequest("unrenderable location".to_owned()))?,
    );
    response.headers_mut().insert(
        ETAG,
        preconditions::revision_etag(revision, crate::domain::concurrency::RowVersion::new(0))
            .parse()
            .map_err(|_| DomainError::InvalidRequest("unrenderable entity tag".to_owned()))?,
    );
    Ok(response)
}

async fn replace_lines(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path(price_overlay_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PRICE_OVERLAY,
        crate::authz::actions::WRITE,
        Some(tenant),
        Some(price_overlay_id),
        true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    let body: ReplaceLinesRequest = preconditions::parse_body(&body)?;
    // The **overlay revision's** tag, not a plan's: an overlay has no plan.
    let tag = preconditions::if_match_revision(&headers)?;
    let lines = lines_of(&body.lines)?;

    let stamp = audit_stamp(&ctx, Utc::now(), correlation);
    let row_version = state
        .overlays
        .replace_lines(
            &scope,
            tenant,
            price_overlay_id,
            tag.revision,
            i64::try_from(tag.version.get()).unwrap_or(i64::MAX),
            lines,
            stamp,
        )
        .await
        .map_err(|e| crate::infra::storage::repo_failure(&e))?;

    let mut response = Json(OverlayAcceptedView {
        price_overlay_id,
        revision: body.revision,
    })
    .into_response();
    response.headers_mut().insert(
        ETAG,
        preconditions::revision_etag(
            tag.revision,
            crate::domain::concurrency::RowVersion::new(
                u64::try_from(row_version).unwrap_or_default(),
            ),
        )
        .parse()
        .map_err(|_| DomainError::InvalidRequest("unrenderable entity tag".to_owned()))?,
    );
    Ok(response)
}

async fn submit_overlay(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path(price_overlay_id): Path<Uuid>,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let _correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PRICE_OVERLAY,
        crate::authz::actions::WRITE,
        Some(tenant),
        Some(price_overlay_id),
        true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    let request: SubmitOverlayRequest = preconditions::parse_body(&body)?;
    let record = state
        .overlays
        .load(&scope, tenant, price_overlay_id, request.revision)
        .await
        .map_err(|e| crate::infra::storage::repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "price overlay revision".to_owned(),
            id: format!("{price_overlay_id}/{}", request.revision),
        })?;

    // `inst-pl-validate`: the whole rule set, aggregate, over the world as it
    // stands **now**. §4.2 runs the same set again inside the publish commit,
    // because the world moves between the two.
    let world = state
        .overlays
        .world_for(&scope, tenant, &record)
        .await
        .map_err(|e| crate::infra::storage::repo_failure(&e))?;
    let candidate = OverlayCandidate {
        price_overlay_id: record.price_overlay_id,
        revision: record.revision,
        scope: record.scope.clone(),
        precedence: record.precedence,
        interval: record.interval,
        tax_basis: record.tax_basis,
        disclosure: record.disclosure,
        target_ref: record.target_ref.clone(),
        lines: record.lines.clone(),
        world,
    };
    let report = validate(&candidate);

    // §5 types two of the nine codes **409** and the rest as architectural 422s.
    // A conflict is lifted out of the envelope so a caller reads the status the
    // design set assigns it; see the module doc.
    if let Some(violation) = conflict_of(&report) {
        return Err(match violation.code.as_str() {
            crate::domain::overlay_rules::PRECEDENCE_DUPLICATE => {
                DomainError::PrecedenceDuplicate(violation.detail.clone())
            }
            _ => DomainError::OverlayIntervalOverlap(violation.detail.clone()),
        }
        .into());
    }
    if !report.is_publishable() {
        return Err(DomainError::ValidationFailed(report).into());
    }

    // 202, per `inst-pl-commit`: the submit opens the always-material Slice 5
    // approval unit (D-50) and the approved overlay is then a publish unit
    // through the Foundation engine (D-06). **Neither is wired here** — see the
    // module-level note in the hand-back; what this route does today is
    // validate, and it answers 202 because that is the status the act has.
    Ok((
        StatusCode::ACCEPTED,
        Json(SubmitAcceptedView {
            price_overlay_id,
            revision: record.revision,
            materiality: "alwaysMaterialTrigger".to_owned(),
            warnings: report
                .warnings
                .iter()
                .map(|w| AdvisoryView {
                    code: w.code.clone(),
                    subject: w.subject.clone(),
                    detail: w.detail.clone(),
                })
                .collect(),
        }),
    )
        .into_response())
}

/// `POST /bss-pricing/v1/price-overlays/{overlayId}/submit`.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct SubmitOverlayRequest {
    /// The revision to submit.
    pub revision: u64,
}

async fn list_overlays(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Query(query): Query<ListOverlaysQuery>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant = ctx.subject_tenant_id();
    // A **read**, and the PDP derives the scope from the subject: reads pass no
    // owner tenant, so the returned scope is the SQL filter.
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PRICE_OVERLAY,
        crate::authz::actions::READ,
        None,
        None,
        true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    let class = match query.scope_class.as_deref() {
        None => None,
        Some(token) => Some(ScopeClass::parse(token).ok_or_else(|| {
            DomainError::InvalidRequest(format!("scope_class `{token}` is not a scope class"))
        })?),
    };

    let overlays = state
        .overlays
        .list(&scope, tenant, class)
        .await
        .map_err(|e| crate::infra::storage::repo_failure(&e))?;

    Ok(Json(OverlayListView {
        overlays: overlays.iter().map(view_of).collect(),
    })
    .into_response())
}

// ---------------------------------------------------------------------------
// The router.
// ---------------------------------------------------------------------------

/// Mount the overlay authoring routes.
pub fn router(state: Arc<AuthoringState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post(PRICE_OVERLAYS)
        .operation_id("bss_pricing.create_price_overlay")
        .summary("Author a PriceOverlay draft")
        .description(
            "Creates an overlay at revision 0 in `draft`, with its whole adjustment line set \
             (D-42: an overlay is a container of one or more lines, never a single adjustment). \
             A save **never publishes** - the submit route is what opens the always-material \
             approval unit. An absent `tax_basis` is refused `TAX_BASIS_UNDECLARED`: L5 says \
             the basis must be declared and silence fails. `disclosure` defaults to \
             `restricted`, which is L6's fail-closed default. Gates on `price_overlay` x \
             `write`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(create_overlay)
        .json_response_with_schema::<OverlayAcceptedView>(
            openapi,
            StatusCode::CREATED,
            "The overlay draft as created; the entity tag names the revision it stands at.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    let router = OperationBuilder::get(PRICE_OVERLAYS)
        .operation_id("bss_pricing.list_price_overlays")
        .summary("List PriceOverlays")
        .description(
            "The admin and Tariffs read. Returns **every** revision, draft included, ordered by \
             precedence then id then revision, optionally narrowed to one `scope_class`. It does \
             **not** filter on `disclosure`: L6 governs consumer-facing exposure and section 3 \
             step 7 is explicit that operator and service reads are unaffected, so a `restricted` \
             overlay is still its author's to read. Gates on `price_overlay` x `read`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(list_overlays)
        .json_response_with_schema::<OverlayListView>(
            openapi,
            StatusCode::OK,
            "The tenant's overlays.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::patch(PRICE_OVERLAY_BY_ID)
        .operation_id("bss_pricing.replace_price_overlay_lines")
        .summary("Replace an open draft revision's whole line set")
        .description(
            "Replaces the adjustment lines **wholesale** - every D-42 rule quantifies over the \
             set, so a partial update leaves nothing the validator can evaluate. `If-Match` \
             carries the **overlay revision's** entity tag, not a plan's: an overlay has no plan \
             and D-92 gives it a revision chain of its own. Only an open draft revision's lines \
             are mutable. Supplying a line's `line_id` is how its identity stays stable across \
             revisions. Gates on `price_overlay` x `write`.",
        )
        .tag(TAG)
        .path_param("overlayId", "The overlay whose line set is replaced.")
        .authenticated()
        .no_license_required()
        .handler(replace_lines)
        .json_response_with_schema::<OverlayAcceptedView>(
            openapi,
            StatusCode::OK,
            "The line set was replaced; the entity tag names the version it now stands at.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    OperationBuilder::post(PRICE_OVERLAY_SUBMIT)
        .operation_id("bss_pricing.submit_price_overlay")
        .summary("Submit a PriceOverlay draft revision")
        .description(
            "Runs the whole `PriceOverlayValidator` rule set - scope taxonomy, the line set, the \
             eligibility filter, magnitude ranges, per-currency coverage, precedence uniqueness, \
             interval non-overlap and referential integrity - and reports **every** failure in \
             one pass. A single blocking violation blocks the submit. `PRECEDENCE_DUPLICATE` and \
             `OVERLAY_INTERVAL_OVERLAP` answer **409**, which is what section 5 types them; the \
             other seven travel as an aggregate 400 whose per-violation codes are the \
             discriminators. Overlay mutation is **always material** (D-50) whatever a threshold \
             policy says. Gates on `price_overlay` x `write`.",
        )
        .tag(TAG)
        .path_param("overlayId", "The overlay to submit.")
        .authenticated()
        .no_license_required()
        .handler(submit_overlay)
        .json_response_with_schema::<SubmitAcceptedView>(
            openapi,
            StatusCode::ACCEPTED,
            "Accepted; the overlay is material and awaits an independent reviewer.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
        .layer(Extension(state))
        // D-178's edge, carried with the routes rather than at the merge, so a
        // surface reachable without it cannot build an `AuditStamp`.
        .layer(axum::middleware::from_fn(
            crate::api::rest::correlation::establish,
        ))
}
