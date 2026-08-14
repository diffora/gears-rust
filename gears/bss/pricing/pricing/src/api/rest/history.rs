//! `GET /bss-pricing/v1/history` — the price-history read
//! (`design/12-operator-efficiency.md` §3 `inst-he-read`, §5; D-12, D-125,
//! D-270).
//!
//! **The first Slice 12 surface an operator can reach.** §5 declares eight and
//! this is the one whose engine was built and whose shape needs nothing that does
//! not exist yet: it is a read, so it opens no transaction, claims no idempotency
//! key and writes no audit record. Its sibling, the clone, is blocked on
//! something this route is not — see D-272.
//!
//! # No correlation edge, and that is the same reason `frontier` has none
//!
//! D-178's edge is applied inside each **mutating** router's own `router()`, and
//! `rest_authz::every_mutating_router_applies_the_correlation_edge` scans this
//! directory's sources for a mutating builder to decide who owes it. Nothing here
//! writes an audit record or an outbox row, so nothing here needs an
//! `AuditStamp`, so the edge would be a layer with no reader.
//!
//! # `audit × read`, and the actor is the row's own column
//!
//! This section and the route's own description both said **`plan × read`** until
//! 2026-08-14, and both were false: the handler asks `audit × read` and
//! `rest_authz`'s census catalogues it that way. The correction is recorded rather
//! than quietly applied because the withdrawn sentence was the *original* reading
//! of D-12 — price history is plan and price data, Finance reads it by construction
//! — and what overrode it is stated at the gate itself: `/history` is the catalog
//! audit trail, so filing it under catalog read handed "who changed what, when"
//! to every holder of `plan × read` while `audit_read` granted nothing.
//!
//! What that leaves standing is the **source** rule, which is a different claim and
//! still exactly true: [`HistoryEntry::actor`](crate::infra::history::HistoryEntry::actor)
//! reads `pricing_price.created_by` and never `pricing_audit_log`. The two surfaces
//! now share a permission and not a store — see [`crate::api::rest::audit`], which
//! is the Slice 5 trail's own read — and a surface that took its actor from the
//! audit log would still be reading a store it is not the reader of.
//!
//! # The cursor is opaque on the wire because it is opaque in the engine
//!
//! `limit` and `cursor` are the two D-125 spellings. The token this surface hands
//! back is [`crate::infra::history::encode`]'s, not
//! [`crate::api::rest::cursor`]'s: the engine's position is an instant **and** a
//! row, which the id-only REST token cannot spell (D-270). The direction of the
//! dependency is what matters — a route may name an engine type, and `DE0202`
//! refuses the reverse, which is how the first draft of D-270 was caught.

use std::sync::Arc;

use axum::extract::{Extension, Query};
use axum::{Json, Router, http::StatusCode};
use serde::Deserialize;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit::api::operation_builder::{ParamLocation, ParamSpec};
use toolkit::api::{OpenApiRegistry, operation_builder::OperationBuilder};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::rest::auth_context::require_authenticated;
use crate::api::rest::error::authz_error_to_canonical;
use crate::infra::history::{HistoryExporter, HistoryPage, HistoryPageRequest};

const TAG: &str = "BSS Pricing History";

/// The path this surface answers on.
///
/// A `const` for its siblings' reason: `OperationBuilder` takes the literal, so
/// the route-shape rule binds where the literal is, and a route census that
/// spelled its paths as string literals is a census one rename walks away from.
pub const HISTORY: &str = "/bss-pricing/v1/history";

/// What this surface needs, and nothing else.
pub struct ApiState {
    /// §1.7's chronological read over the append-only price rows.
    pub history: HistoryExporter,
}

/// The two D-125 query parameters, both optional.
#[derive(Debug, Default, Deserialize)]
pub struct HistoryQuery {
    /// Page size. Absent means D-125's server default; above the cap it clamps.
    pub limit: Option<u64>,
    /// The opaque token the previous page handed back.
    pub cursor: Option<String>,
}

/// One history record on the wire.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct HistoryEntryView {
    /// The append-only row this record is of.
    pub price_id: Uuid,
    /// The plan it prices.
    pub plan_id: Uuid,
    /// Its lifecycle state at read time — `draft`, `published` or `superseded`.
    pub lifecycle_state: String,
    /// The pseudonymous principal who authored the row, from its **own**
    /// column (D-12).
    pub actor: String,
    /// When the row was authored, UTC.
    pub authored_at: String,
    /// The row's scheduled intervals in `from` order, each with its own state.
    /// Empty when the row has never been scheduled.
    pub effective: Vec<EffectiveIntervalView>,
}

/// One scheduled interval of a history record.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct EffectiveIntervalView {
    /// Start of the interval, UTC.
    pub from: String,
    /// End of the interval, UTC; absent when the interval is open-ended.
    pub to: Option<String>,
    /// The window's state — a cancelled or expired interval is **included** and
    /// distinguishable, history being a record of what was and not of what is.
    pub state: String,
}

/// A page of history.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct HistoryPageView {
    /// The page's records, oldest first.
    pub entries: Vec<HistoryEntryView>,
    /// The token to pass as `cursor` for the next page, or absent when the walk
    /// is exhausted. Absent on the last page rather than on the page after it,
    /// so a client stops without an extra round trip.
    pub next_cursor: Option<String>,
}

impl From<HistoryPage> for HistoryPageView {
    fn from(page: HistoryPage) -> Self {
        Self {
            entries: page
                .entries
                .iter()
                .map(|entry| HistoryEntryView {
                    price_id: entry.record.price_id,
                    plan_id: entry.record.scope_key.plan_id().get(),
                    lifecycle_state: entry.record.lifecycle_state.as_str().to_owned(),
                    actor: entry.actor().to_string(),
                    authored_at: entry.record.created_at_utc.to_rfc3339(),
                    effective: entry
                        .effective
                        .iter()
                        .map(|interval| EffectiveIntervalView {
                            from: interval.from.to_rfc3339(),
                            to: interval.to.map(|to| to.to_rfc3339()),
                            state: interval.state.as_str().to_owned(),
                        })
                        .collect(),
                })
                .collect(),
            next_cursor: page.next.map(crate::infra::history::encode),
        }
    }
}

/// D-125's page size, declared once for every cursor walk in the gear.
///
/// `pub(crate)` for `plans::idempotency_key_param`'s reason and on its precedent:
/// the contract is the gear's rather than this route's, and two spellings of one
/// parameter's description are two answers to what a generated client's docs say
/// about it. This surface was the first D-125 walk to declare its parameters at
/// all, which is why the spelling lives here.
pub(crate) fn limit_param() -> ParamSpec {
    ParamSpec {
        name: "limit".to_owned(),
        location: ParamLocation::Query,
        required: false,
        description: Some(
            "Page size. Absent takes the server default; a value above the cap is clamped \
             rather than refused, the cap being a server limit and the page size the export \
             SLO is stated per (D-125). Zero is refused: a page of zero rows never advances."
                .to_owned(),
        ),
        param_type: "integer".to_owned(),
    }
}

/// D-125's opaque page token, declared once — [`limit_param`]'s note.
pub(crate) fn cursor_param() -> ParamSpec {
    ParamSpec {
        name: "cursor".to_owned(),
        location: ParamLocation::Query,
        required: false,
        description: Some(
            "The opaque token the previous page returned as `next_cursor`. Opaque by contract: \
             a token a caller can read is a token a caller will construct, and then the walk's \
             ordering guarantee is whatever that caller assumed."
                .to_owned(),
        ),
        param_type: "string".to_owned(),
    }
}

/// Build the Axum router for the price-history surface.
///
/// The declared errors are the ones this path can produce: 400 (a `limit` of zero
/// or a cursor that does not decode), 401, 403, 503 and 500. No 404 — an empty
/// history is a page with no entries, not a missing resource, exactly as the
/// frontier's empty reading is a 200.
pub fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get(HISTORY)
        .operation_id("bss_pricing.read_price_history")
        .summary("Read immutable price history")
        .description(
            "Returns the tenant's price history in commit order, oldest first: every \
             append-only `pricing_price` row with the principal who authored it, the instant \
             it was authored, and the scheduled intervals it has held. The read adds no store \
             - the Foundation's immutability IS the history (`inst-he-nostore`) - and it \
             paginates on an opaque cursor per D-125, with a server default page size and a \
             hard cap. Superseded and cancelled records are included and distinguishable: \
             history records what was, not what is. Gates on `audit` x `read` (D-12): this is \
             the catalog audit trail, so it is Auditor-only rather than Finance-readable, and \
             `plan` x `read` grants no access to it. The separate Slice 5 trail over \
             `pricing_audit_log` has its own read (`GET /bss-pricing/v1/audit`) and is never \
             this surface's source for the actor.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .param(limit_param())
        .param(cursor_param())
        .handler(read_history)
        .json_response_with_schema::<HistoryPageView>(
            openapi,
            StatusCode::OK,
            "A page of history in commit order, with the token for the next page when one \
             remains.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    router.layer(Extension(state))
}

/// `GET /history`: the caller tenant's price history, one page.
///
/// `owner_tenant_id` is `None` — a read, so the PDP derives the scope from the
/// subject and its roles rather than trusting a caller-supplied tenant, and the
/// compiled scope becomes the SQL filter. `require_constraints = true` so an
/// unconstrained allow fails closed instead of exposing every tenant's history.
async fn read_history(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryPageView>, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        // The audit trail is its own disclosure, so it is its own permission:
        // `audit_read` ("Read the catalog audit trail") is declared for exactly
        // this surface. Gating on `plan x read` handed the trail -- who changed
        // what, when, and to what -- to every holder of catalog read, and left
        // the declared permission granting nothing.
        &crate::authz::resource_types::AUDIT,
        crate::authz::actions::READ,
        /* owner_tenant_id */ None,
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    .map_err(authz_error_to_canonical)?;

    // Parsed after the gate, for `plans.rs`'s stated reason: a module doc
    // asserting "the gate before the request" reads as two disciplines if two
    // modules order it differently.
    let request = HistoryPageRequest::parse(query.limit, query.cursor.as_deref())
        .map_err(CanonicalError::from)?;

    let page = state
        .history
        .read_page(&scope, ctx.subject_tenant_id(), request)
        .await
        // Through the gear's single authoritative ladder rather than a mapping
        // invented here: `From<RepoError> for DomainError` already decides what a
        // storage failure means, and forking that per handler is how two surfaces
        // start disagreeing about the same failure.
        .map_err(|e| CanonicalError::from(crate::infra::storage::repo_failure(&e)))?;

    Ok(Json(HistoryPageView::from(page)))
}
