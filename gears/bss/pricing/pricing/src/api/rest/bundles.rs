//! The bundle authoring surface — `design/08-bundles.md` §5,
//! `inst-ba-author` / `inst-ba-validate` / `inst-ba-return`.
//!
//! Three routes: create the bundle on its plan, replace an open draft revision's
//! whole composition, and publish it.
//!
//! # D-11 is why the third route's gate is not the first two's
//!
//! Authoring is `bundle × write` (`ProductManager`, `CatalogAdmin`). **Publish
//! requires `plan × publish` only** — D-11, decided 2026-07-10: under the
//! conjunction the design set originally stated, only `CatalogAdmin` could publish
//! a bundle while §1.3 promises `FinanceManager` can, and one of the three
//! statements had to be wrong. Publish is a plan-level act; the authoring
//! authority was already exercised, and the composition is protected at publish
//! time by the approval content pin. Component checks are **validations, not
//! caller authz**, which is why nothing here gates on the components' own plans.
//!
//! # The composition is addressed by the bundle and versioned by the revision
//!
//! `If-Match` on the composition route carries the **plan revision's** entity tag
//! (`"<revision>-<version>"`, D-170), not a tag of the composition's own. The
//! composition has none, deliberately: a component-set edit advances the
//! revision's tag, so two authors editing one draft cannot both satisfy their
//! precondition. `BundleRepo`'s module doc carries the argument.
//!
//! **§5 spells the composition route `PATCH /bss-pricing/v1/bundles`** — a
//! collection PATCH with the subject in the body. This mounts
//! `PATCH /bss-pricing/v1/bundles/{bundleId}` instead, because a precondition
//! addresses a resource and a collection that answers `If-Match` is a collection
//! pretending to be one. The divergence is reported in the owed register (B-10)
//! rather than smoothed over.
//!
//! # 422 does not exist on this platform
//!
//! §5 types every composition refusal 422 and that notation is architectural:
//! `CanonicalError` renders `InvalidArgument`, `FailedPrecondition` and
//! `OutOfRange` all as **400**. The **code string** is the discriminator a
//! consumer matches on, and the ten of them travel inside the `ValidationFailed`
//! envelope, one violation per failing rule — which is what makes a composition
//! remediable in one pass. No route here declares a 422 response.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path};
use axum::http::header::{ETAG, LOCATION};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, http::HeaderMap, http::StatusCode};
use chrono::Utc;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::approvals::ApprovalView;
use crate::api::rest::auth_context::{audit_stamp, require_authenticated};
use crate::api::rest::correlation::{CorrelationId, require_correlation};
use crate::api::rest::error::authz_error_to_canonical;
use crate::api::rest::preconditions;
use crate::api::rest::state::AuthoringState;
use crate::domain::bundle::{
    Absorber, InvoiceItemization, Party, PartyShare, PriceBasis, RevShareGroup,
};
use crate::domain::bundle_rules::check_basis_declared;
use crate::domain::error::DomainError;
use crate::domain::materiality::{self, MaterialityVerdict};
use crate::domain::money::CurrencyCode;
use crate::domain::scope_key::{PlanId, Region};
use crate::infra::storage::repo::{BundleComponentDraft, CompositionDraft, NewBundle};

const TAG: &str = "BSS Pricing Bundles";

/// The wire tokens for the two arms — `api::rest::publish`'s, so a client's
/// `match` does not depend on which plane it called, and so `"published"` has one
/// home rather than a third spelling beside `publish.rs`'s and `overlays.rs`'s.
const OUTCOME_SUBMITTED: &str = crate::api::rest::publish::OUTCOME_SUBMITTED;
const OUTCOME_PUBLISHED: &str = crate::api::rest::publish::OUTCOME_PUBLISHED;

/// `POST` — create a bundle on its plan.
pub const BUNDLES: &str = "/bss-pricing/v1/bundles";
/// `PATCH` — replace an open draft revision's whole composition.
pub const BUNDLE_BY_ID: &str = "/bss-pricing/v1/bundles/{bundleId}";
/// `POST` — validate and publish.
pub const BUNDLE_PUBLISH: &str = "/bss-pricing/v1/bundles/{bundleId}/publish";

// ---------------------------------------------------------------------------
// Wire types.
// ---------------------------------------------------------------------------

/// `POST /bss-pricing/v1/bundles`.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct CreateBundleRequest {
    /// The `bundle`-type plan this bundle is the composition of.
    pub plan_id: Uuid,
    /// `sum_of_parts` or `own_price`. **Optional on the wire and required in
    /// substance**: `inst-bb-declared` says the basis MUST be declared, and
    /// `BASIS_MISSING` is what an absent one is told. Modelling it as
    /// `Option` is what makes that code reachable — a required field would be
    /// refused by the deserializer with a message the design set does not own.
    pub price_basis: Option<String>,
    /// `aggregate` or `itemize`. Defaults to `aggregate`, which is the layout
    /// that adds nothing to an invoice a bundle did not already have.
    pub invoice_itemization: Option<String>,
}

/// `PATCH /bss-pricing/v1/bundles/{bundleId}`.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct CompositionRequest {
    /// The plan revision this composition belongs to.
    pub plan_revision: u64,
    /// The referenced components, whole. **Replaced, never merged** — every
    /// Slice-8 rule quantifies over the set.
    pub components: Vec<ComponentRequest>,
    /// The rev-share groups, one per included vendor SKU. Empty is legal and is
    /// what a bundle with no revenue to allocate looks like.
    #[serde(default)]
    pub rev_share: Vec<RevShareGroupRequest>,
}

/// One component of a submitted composition.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct ComponentRequest {
    /// The component's **plan** (B1) — a bare SKU id is ambiguous per
    /// `(currency, region)`.
    pub component_plan_id: Uuid,
    /// The registry SKU it publishes under.
    pub included_sku_id: Uuid,
    /// Selection-time lower bound.
    pub min_qty: Option<i32>,
    /// Selection-time upper bound.
    pub max_qty: Option<i32>,
}

/// One rev-share group of a submitted composition.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct RevShareGroupRequest {
    /// The included vendor SKU whose revenue this group allocates.
    pub vendor_sku_id: Uuid,
    /// The group's explicit platform cut, in basis points.
    pub platform_cut_bp: i32,
    /// Who absorbs the publish-time residual — a party of this group, or the
    /// `platform` sentinel. Absent means `platform`, which is D-07's default and
    /// what makes an unnominated state unrepresentable.
    pub residual_absorber_party: Option<String>,
    /// The parties and their **typed** shares.
    pub parties: Vec<PartyShareRequest>,
}

/// One party's typed share.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct PartyShareRequest {
    /// The recipient. May not be blank and may not be `platform`.
    pub party: String,
    /// The typed share, in basis points.
    pub share_bp: i32,
}

/// `POST /bss-pricing/v1/bundles/{bundleId}/publish`.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct PublishBundleRequest {
    /// The plan revision to publish.
    pub plan_revision: u64,
    /// The `(currency, region)` markets the bundle sells in.
    ///
    /// Supplied rather than derived because a `sum_of_parts` bundle carries no
    /// price rows of its own (`inst-bb-rowless`), so there is nothing to read the
    /// market set off. It is the coverage walk's domain and the tax-basis walk's.
    pub markets: Vec<MarketRequest>,
}

/// One sold market.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct MarketRequest {
    /// ISO 4217.
    pub currency: String,
    /// The region axis value.
    pub region: String,
}

/// What a created bundle answers with.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct BundleView {
    /// The bundle's identity.
    pub bundle_id: Uuid,
    /// The plan it rides.
    pub plan_id: Uuid,
    /// Its declared basis.
    pub price_basis: String,
    /// Its invoice layout.
    pub invoice_itemization: String,
}

/// What a composition write answers with.
///
/// A body rather than a bare `204`, because the entity tag it carries is the one
/// the caller needs for its next edit and a response with a header and no
/// document invites a client to ignore the body it does not have.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request, response)]
pub struct CompositionAcceptedView {
    /// The bundle whose composition was replaced.
    pub bundle_id: Uuid,
    /// The revision it now stands at.
    pub plan_revision: u64,
}

/// What a publish answers with — which of the two acts it performed, and why.
///
/// `response` only, as `overlays::SubmitAcceptedView` is: it carries an
/// [`ApprovalView`], which is a projection of a stored record and has no
/// `Deserialize`. It was declared `request, response` while its fields were three
/// scalars, and nothing ever sent one.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct PublishAcceptedView {
    /// The bundle this call acted on.
    pub bundle_id: Uuid,
    /// The revision it acted at.
    pub plan_revision: u64,
    /// Which act this was: `submitted_for_approval` on the first call,
    /// `published` on the call a second principal's approval authorized.
    ///
    /// D-104 makes a composition change always material, so the first call over
    /// any content **stages** it. `POST …/price-overlays/{id}/submit` answers the
    /// same two-token shape for the same reason.
    pub outcome: String,
    /// Why this change is material — **evaluated, not asserted**.
    ///
    /// It reads `alwaysMaterialTrigger` for every composition change (D-104), and
    /// that constancy is a property of the rule rather than of this field: the
    /// token is produced by [`bundle_publish_materiality`] through the same
    /// evaluator every other unit's is. It was a hard-coded literal here until
    /// 2026-08-11, which told a client a property of the request that nothing had
    /// established — and a false token cannot be told from a real one downstream.
    pub materiality: String,
    /// The unit this call opened, on the submit arm; the unit that authorized it,
    /// on the publish arm.
    pub approval: Option<ApprovalView>,
}

// ---------------------------------------------------------------------------
// Conversions.
// ---------------------------------------------------------------------------

fn draft_of(request: &CompositionRequest) -> Result<CompositionDraft, DomainError> {
    let components = request
        .components
        .iter()
        .map(|c| BundleComponentDraft {
            component_plan_id: c.component_plan_id,
            included_sku_id: c.included_sku_id,
            min_qty: c.min_qty,
            max_qty: c.max_qty,
        })
        .collect();

    let mut rev_share_groups = Vec::with_capacity(request.rev_share.len());
    for group in &request.rev_share {
        // Absent means the platform, which is the column's default and D-07's
        // rule: an unnominated state cannot exist.
        let absorber = match group.residual_absorber_party.as_deref() {
            None => Absorber::Platform,
            Some(token) => Absorber::parse(token).ok_or_else(|| {
                DomainError::InvalidRequest(format!(
                    "residual_absorber_party `{token}` is neither a party nor the platform sentinel"
                ))
            })?,
        };
        let mut parties = Vec::with_capacity(group.parties.len());
        for party in &group.parties {
            let named = Party::new(&party.party).ok_or_else(|| {
                DomainError::InvalidRequest(format!(
                    "party `{}` is blank or spells the reserved `platform` sentinel",
                    party.party
                ))
            })?;
            parties.push(PartyShare {
                party: named,
                share_bp: party.share_bp,
            });
        }
        rev_share_groups.push(RevShareGroup {
            vendor_sku_id: group.vendor_sku_id,
            platform_cut_bp: group.platform_cut_bp,
            residual_absorber: absorber,
            parties,
        });
    }
    Ok(CompositionDraft {
        components,
        rev_share_groups,
    })
}

fn markets_of(request: &PublishBundleRequest) -> Result<Vec<(CurrencyCode, Region)>, DomainError> {
    request
        .markets
        .iter()
        .map(|m| Ok((CurrencyCode::new(&m.currency)?, Region::new(&m.region)?)))
        .collect()
}

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

async fn create_bundle(
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
    // Authoring is `bundle x write`; publish is the route below and is D-11's.
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::BUNDLE,
        crate::authz::actions::WRITE,
        Some(tenant),
        None,
        true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    // Parsed after the gate, as `plans.rs` and `prices.rs` order it.
    let body: CreateBundleRequest = preconditions::parse_body(&body)?;
    let _client_key = preconditions::idempotency_key(&headers)?;

    let basis = match body.price_basis.as_deref() {
        None => check_basis_declared(None).map_err(|code| {
            DomainError::InvalidRequest(format!("{code}: a bundle must declare its price basis"))
        })?,
        Some(token) => PriceBasis::parse(token).ok_or_else(|| {
            DomainError::InvalidRequest(format!(
                "price_basis `{token}` is neither sum_of_parts nor own_price"
            ))
        })?,
    };
    let itemization = match body.invoice_itemization.as_deref() {
        None => InvoiceItemization::Aggregate,
        Some(token) => InvoiceItemization::parse(token).ok_or_else(|| {
            DomainError::InvalidRequest(format!(
                "invoiceItemization `{token}` is neither aggregate nor itemize"
            ))
        })?,
    };

    let bundle_id = Uuid::new_v4();
    let plan_id = PlanId::new(body.plan_id);
    let stamp = audit_stamp(&ctx, Utc::now(), correlation);
    state
        .bundles
        .create(
            &scope,
            NewBundle {
                bundle_id,
                tenant_id: tenant,
                plan_id,
                price_basis: basis,
                invoice_itemization: itemization,
            },
            stamp,
        )
        .await
        .map_err(|e| crate::infra::storage::repo_failure(&e))?;

    let view = BundleView {
        bundle_id,
        plan_id: body.plan_id,
        price_basis: basis.as_str().to_owned(),
        invoice_itemization: itemization.as_str().to_owned(),
    };
    let mut response = (StatusCode::CREATED, Json(view)).into_response();
    response.headers_mut().insert(
        LOCATION,
        format!("/bss-pricing/v1/bundles/{bundle_id}")
            .parse()
            .map_err(|_| DomainError::InvalidRequest("unrenderable location".to_owned()))?,
    );
    Ok(response)
}

async fn replace_composition(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path(bundle_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::BUNDLE,
        crate::authz::actions::WRITE,
        Some(tenant),
        Some(bundle_id),
        true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    let body: CompositionRequest = preconditions::parse_body(&body)?;
    // The **plan revision's** tag, not one of the composition's own: see the
    // module doc.
    let tag = preconditions::if_match_revision(&headers)?;
    let draft = draft_of(&body)?;

    let plan_id = state
        .bundles
        .plan_of(&scope, tenant, bundle_id)
        .await
        .map_err(|e| crate::infra::storage::repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "bundle".to_owned(),
            id: bundle_id.to_string(),
        })?;

    let stamp = audit_stamp(&ctx, Utc::now(), correlation);
    let revision = state
        .bundles
        .replace_composition(
            &scope,
            tenant,
            plan_id,
            tag.revision,
            tag.version,
            draft,
            stamp,
        )
        .await
        .map_err(|e| crate::infra::storage::repo_failure(&e))?;

    let mut response = Json(CompositionAcceptedView {
        bundle_id,
        plan_revision: revision.revision,
    })
    .into_response();
    response.headers_mut().insert(
        ETAG,
        preconditions::revision_etag(revision.revision, revision.row_version)
            .parse()
            .map_err(|_| DomainError::InvalidRequest("unrenderable entity tag".to_owned()))?,
    );
    Ok(response)
}

async fn publish_bundle(
    Extension(state): Extension<Arc<AuthoringState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    extension_correlation: Option<Extension<CorrelationId>>,
    Path(bundle_id): Path<Uuid>,
    body: Bytes,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let correlation = require_correlation(extension_correlation)?;
    let tenant = ctx.subject_tenant_id();
    // **D-11**: `plan x publish` only. Not `bundle x write` as well — under the
    // conjunction the design set first stated, only `CatalogAdmin` could publish a
    // bundle while §1.3 promises `FinanceManager` can.
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PLAN,
        crate::authz::actions::PUBLISH,
        Some(tenant),
        Some(bundle_id),
        true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    let body: PublishBundleRequest = preconditions::parse_body(&body)?;
    let markets = markets_of(&body)?;

    let plan_id = state
        .bundles
        .plan_of(&scope, tenant, bundle_id)
        .await
        .map_err(|e| crate::infra::storage::repo_failure(&e))?
        .ok_or_else(|| DomainError::NotFound {
            subject: "bundle".to_owned(),
            id: bundle_id.to_string(),
        })?;

    // `inst-ba-validate`: the whole rule set, aggregate. A blocking violation
    // blocks the publish — there is no severity below blocking that still
    // publishes (§4.2).
    let report = state
        .bundle_service
        .validate_publish(&scope, tenant, plan_id, body.plan_revision, markets)
        .await
        .map_err(|e| crate::infra::storage::repo_failure(&e))?;
    if !report.is_publishable() {
        return Err(DomainError::ValidationFailed(report).into());
    }

    // **D-104, `inst-ba-material`.** A composition change is always material, so a
    // single principal's call stages it and a second principal's approval is what
    // publishes it. Until 2026-08-11 this route committed straight through: it
    // evaluated no verdict, opened no unit, pinned no content, and answered a
    // hard-coded `alwaysMaterialTrigger` — on the one surface in this gear where
    // the money being divided belongs to third parties. `composition_change_set`
    // and `rev_share_change_set` had existed since Slice 8 with **no caller
    // anywhere in the crate**, which is what made `Trigger::BundleComposition`
    // answer `subject_exists_in_this_crate` while nothing could ever evaluate it.
    //
    // `overlays::submit_overlay` is the precedent, one plane over and the same
    // shape: `priceOverlayMutation` was a mounted surface writing its materiality
    // token as a literal while nothing built its change set, and it was closed the
    // same way.
    let now = Utc::now();
    let conn = state.db.conn().map_err(|e| {
        CanonicalError::from(DomainError::Internal(format!(
            "bss-pricing: bundle publish subject: {e}"
        )))
    })?;

    // The content this call would freeze, assembled once and used for both arms —
    // it is what the lookup below matches an approval against and what the unit
    // pins. The composition normalizes onto its absorber inside the plan, so the
    // plan shape *is* the composition's content.
    let shape = crate::infra::publish::assemble(&conn, &scope, tenant, plan_id, now)
        .await
        .map_err(CanonicalError::from)?;
    let pin = crate::domain::approval::content_hash(&shape);
    let subject_ref =
        crate::infra::approval::bundle_composition_unit_ref(plan_id, body.plan_revision);

    // One verdict, rendered twice — never built twice. The wire's string and the
    // record's jsonb cannot come from two evaluations.
    let (reason, stored_materiality) =
        crate::api::rest::overlays::rendered_materiality(&bundle_publish_materiality())?;

    // Matched on the **content** and not merely on the subject: an approval whose
    // composition moved after the decision covers content that no longer exists,
    // so answering with it would authorize a component set nobody reviewed.
    let approved = state
        .approvals
        .approved_unit(&scope, tenant, &subject_ref, &pin)
        .await
        .map_err(CanonicalError::from)?;

    if let Some(record) = approved {
        // The publish arm: a second, independent person has seen exactly this
        // composition.
        state
            .bundle_service
            .publish_composition(
                &scope,
                tenant,
                plan_id,
                body.plan_revision,
                correlation,
                now,
            )
            .await
            .map_err(|e| crate::infra::storage::repo_failure(&e))?;

        // 202, per `inst-ba-return`: the composition is frozen into the read model
        // by the projector, which this response does not wait for.
        return Ok((
            StatusCode::ACCEPTED,
            Json(PublishAcceptedView {
                bundle_id,
                plan_revision: body.plan_revision,
                outcome: OUTCOME_PUBLISHED.to_owned(),
                materiality: reason,
                approval: Some(ApprovalView::from(&record)),
            }),
        )
            .into_response());
    }

    // The submit arm: open D-104's always-material unit over this composition.
    let opened = state
        .approvals
        .submit_bundle_publish(
            &scope,
            tenant,
            plan_id,
            body.plan_revision,
            Uuid::now_v7(),
            pin.to_vec(),
            stored_materiality,
            audit_stamp(&ctx, now, correlation),
        )
        .await
        .map_err(CanonicalError::from)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(PublishAcceptedView {
            bundle_id,
            plan_revision: body.plan_revision,
            outcome: OUTCOME_SUBMITTED.to_owned(),
            materiality: reason,
            approval: Some(ApprovalView::from(&opened)),
        }),
    )
        .into_response())
}

/// The materiality verdict a composition publish carries — **evaluated, not
/// asserted**.
///
/// `overlays::overlay_submit_materiality`'s shape and its reason: the token an
/// operator reads is produced by the same evaluator every other unit's is, so two
/// units compared by a reader are two answers from one function rather than one
/// answer and one literal.
///
/// It passes no policy and no baseline, and that is not a shortcut —
/// [`materiality::evaluate`] examines the **act** half before it consults either,
/// so a configured threshold cannot reach this act. That is the whole of what
/// D-104 decided: with a threshold configured, the evaluator saw no price-row
/// delta to trip on and a component swap reached consumers with no approver,
/// while a $1 price-row change above threshold took two people.
///
/// # This call is what makes `Trigger::BundleComposition` real
///
/// The act half is `ChangeSet::act()`, reachable through nothing but
/// [`ChangeSet::of_act`], so a trigger no surface constructs can never be answered
/// by the evaluator however many tables its subject has.
/// `infra::bundle::composition_change_set` was exactly such a constructor — a
/// `pub fn` building a declaration, with no caller — and this is its first one.
fn bundle_publish_materiality() -> MaterialityVerdict {
    materiality::evaluate(
        &crate::infra::bundle::composition_change_set(),
        /* policy */ None,
        /* baseline */ None,
    )
}

// ---------------------------------------------------------------------------
// The router.
// ---------------------------------------------------------------------------

/// Mount the three bundle routes.
pub fn router(state: Arc<AuthoringState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post(BUNDLES)
        .operation_id("bss_pricing.create_bundle")
        .summary("Create a bundle on its plan")
        .description(
            "Declares a `bundle`-type plan's price basis (`sum_of_parts` or `own_price`) and its \
             invoice itemization. The composition itself is authored through `PATCH \
             /bss-pricing/v1/bundles/{bundleId}`, which is where it becomes revision-scoped. A \
             plan carries at most one bundle. An absent `price_basis` is refused `BASIS_MISSING` \
             (`inst-bb-declared`). Gates on `bundle` x `write`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .handler(create_bundle)
        .json_response_with_schema::<BundleView>(
            openapi,
            StatusCode::CREATED,
            "The bundle as created.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    let router = OperationBuilder::patch(BUNDLE_BY_ID)
        .operation_id("bss_pricing.replace_bundle_composition")
        .summary("Replace an open draft revision's whole composition")
        .description(
            "Replaces the component set and the rev-share groups **wholesale** - every Slice-8 \
             rule quantifies over the set, so a partial update leaves nothing the validator can \
             evaluate. `If-Match` carries the **plan revision's** entity tag, because the \
             composition rides it: an edit that left the revision's tag alone would let two \
             authors edit one draft and both satisfy their precondition. Only an open draft \
             revision's composition is mutable (D-92). Gates on `bundle` x `write`.",
        )
        .tag(TAG)
        .path_param("bundleId", "The bundle whose composition is replaced.")
        .authenticated()
        .no_license_required()
        .handler(replace_composition)
        .json_response_with_schema::<CompositionAcceptedView>(
            openapi,
            StatusCode::OK,
            "The composition was replaced; the entity tag names the revision it now stands at.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    OperationBuilder::post(BUNDLE_PUBLISH)
        .operation_id("bss_pricing.publish_bundle")
        .summary("Validate and publish a bundle composition")
        .description(
            "Runs the whole composition rule set - basis, component publication, per-market \
             coverage, frequency match, one tax display basis per market (D-119) and the \
             rev-share reconciliation (D-07) - and reports **every** failure in one pass. A \
             single blocking violation blocks the publish. On success the group residuals are \
             normalized onto their absorbers so published effective shares sum to exactly 10000 \
             bp, and `BundleUpdated` is emitted in the same transaction. Composition changes are \
             **always material** (D-104) whatever a threshold policy says. Requires `plan` x \
             `publish` **only** (D-11): the composition was authored under `bundle` x `write` \
             and component checks are validations, not caller authz.",
        )
        .tag(TAG)
        .path_param("bundleId", "The bundle to publish.")
        .authenticated()
        .no_license_required()
        .handler(publish_bundle)
        .json_response_with_schema::<PublishAcceptedView>(
            openapi,
            StatusCode::ACCEPTED,
            "Accepted; the composition is frozen into the read model by the projector.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
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
