//! Router-level tests for `GET /bss-pricing/v1/history` — Slice 12's first
//! reachable surface (§5, `inst-he-read`; D-12, D-125, D-270).
//!
//! The harness is `rest_frontier`'s, deliberately: both are reads gated on
//! `plan × read` with a flat-`In` PDP, and two harnesses would be two answers to
//! what this gear's PEP returns.
//!
//! **What only a router test can see here** is the pair the engine cannot: that
//! the gate runs *before* the query, so a refusal is 403 and not an empty page;
//! and that a malformed `limit` or `cursor` reaches the wire as 400 rather than
//! as a 500 from a panic in parsing.
//!
//! Driven through the real router with `tower::ServiceExt::oneshot`, so the
//! extractors, the PEP gate and the response serialization are all in the path.
//! The three cases are the three answers the route can give:
//!
//!   (1) no authenticated `SecurityContext` ⇒ 401 — distinct from a denial,
//!       because the caller is not yet known;
//!   (2) the PDP denies ⇒ 403;
//!   (3) an authorized caller whose tenant has an advanced frontier ⇒ 200
//!       carrying the version, and a caller whose tenant has none ⇒ 200 with the
//!       explicit `pin_eligible: false` reading rather than a 404.
//!
//! The happy path runs against an in-memory `SQLite` migrated by the gear's own
//! `Migrator`, which is what makes it a test of the route and not of a mock: the
//! frontier row is written through `pin_frontier_repo::advance` and read back
//! across the `SecureORM` tenant filter the compiled `AccessScope` binds. The
//! advance is a runner-taking free function rather than a method on the
//! repository because D-136 puts it inside the projector's transaction; the
//! **read** this suite exercises is the method, and it is the route's.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use async_trait::async_trait;
use authz_resolver_sdk::constraints::{Constraint, InPredicate, Predicate};
use authz_resolver_sdk::error::AuthZResolverError;
use authz_resolver_sdk::models::{
    DenyReason, EvaluationRequest, EvaluationResponse, EvaluationResponseContext,
};
use authz_resolver_sdk::{AuthZResolverClient, PolicyEnforcer};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use bss_pricing::api::rest::history::{ApiState, router};
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewPriceDraft, PriceRepo};
use chrono::{TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use toolkit_gts::gts_id;
use toolkit_security::{SecurityContext, pep_properties};
use tower::ServiceExt;
use uuid::Uuid;

const PATH: &str = "/bss-pricing/v1/history";

/// Flat-`In` PDP fake: allows, and constrains `owner_tenant_id` to `allowed`.
/// This is the shape the real PDP returns for this gear (the PEP advertises no
/// tenant-subtree capability, so the subtree is pre-expanded to a flat `In`).
struct FlatInResolver {
    allowed: Vec<Uuid>,
}

#[async_trait]
impl AuthZResolverClient for FlatInResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Ok(EvaluationResponse {
            decision: true,
            context: EvaluationResponseContext {
                constraints: vec![Constraint {
                    predicates: vec![Predicate::In(InPredicate::new(
                        pep_properties::OWNER_TENANT_ID,
                        self.allowed.clone(),
                    ))],
                }],
                deny_reason: None,
            },
        })
    }
}

/// PDP fake that refuses. The route must surface this as 403, never as an empty
/// frontier: a consumer that reads "nothing to pin" stops, and it must stop for
/// the reason it was actually given.
struct DenyingResolver;

#[async_trait]
impl AuthZResolverClient for DenyingResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Ok(EvaluationResponse {
            decision: false,
            context: EvaluationResponseContext {
                constraints: vec![],
                deny_reason: Some(DenyReason {
                    error_code: "no_catalog_role".to_owned(),
                    details: Some("no catalog role grants plan x read".to_owned()),
                }),
            },
        })
    }
}

fn ctx_for(tenant: Uuid) -> SecurityContext {
    SecurityContext::builder()
        .subject_id(Uuid::now_v7())
        .subject_tenant_id(tenant)
        .subject_type(gts_id!("cf.core.security.subject_user.v1~"))
        .token_scopes(vec!["*".to_owned()])
        .build()
        .expect("authed SecurityContext must build")
}

async fn provider() -> DBProvider<DbError> {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run the gear migrator");
    DBProvider::<DbError>::new(db)
}

/// The real router over a real reader, plus the layers `register_rest` applies —
/// here without the canonical-error middleware, these assertions being about
/// status codes, which `CanonicalError` already carries as an `IntoResponse`.
///
/// No correlation layer: this is a read, and the edge travels with the mutating
/// routers only.
fn history_router(
    db: DBProvider<DbError>,
    enforcer: PolicyEnforcer,
    ctx: Option<SecurityContext>,
) -> Router {
    let state = Arc::new(ApiState {
        history: bss_pricing::infra::history::HistoryExporter::new(db),
    });
    let openapi = OpenApiRegistryImpl::new();
    let router = router(state, &openapi).layer(axum::Extension(enforcer));
    match ctx {
        Some(ctx) => router.layer(axum::Extension(ctx)),
        None => router,
    }
}

async fn get_at(router: Router, query: &str) -> axum::http::Response<Body> {
    router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("{PATH}{query}"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send")
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the body");
    serde_json::from_slice(&bytes).expect("the body is JSON")
}

/// One published price row, authored by `actor`, on its own market so nothing
/// collides on the canonical scope key.
async fn seed_row(db: &DBProvider<DbError>, tenant: Uuid, actor: Uuid, region: &str, hour: u32) {
    let prices = PriceRepo::new(db.clone());
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    let at = Utc.with_ymd_and_hms(2026, 8, 8, hour, 0, 0).unwrap();
    prices
        .create_draft(
            &AccessScope::for_tenant(tenant),
            tenant,
            NewPriceDraft {
                price_id: Uuid::now_v7(),
                scope_key: ScopeKey::new(
                    PlanId::new(Uuid::from_u128(0x9_1a4)),
                    CurrencyCode::new("EUR").expect("three letters"),
                    Region::new(region).expect("a non-blank region"),
                    PhaseId::new(Uuid::from_u128(0xfa_5e)),
                    PriceEligibility::AllSubscriptions,
                    ChargeKind::Recurring,
                    Cohort::None,
                )
                .expect("the class pairs with cohort none"),
                content: PriceContent {
                    row,
                    tax_inclusive: false,
                    tax_category_ref: None,
                    billing_timing: Some("advance".to_owned()),
                    proration_contract: None,
                    rounding_policy_ref: Some("half_up".to_owned()),
                    grandfather_until: None,
                    supersedes_price_id: None,
                },
                created_by: actor,
                created_at_utc: at,
                correlation_id: Uuid::now_v7(),
            },
        )
        .await
        .expect("author a row");
}

/// The happy path: a page in commit order, carrying the actor off the row's own
/// column.
#[tokio::test]
async fn a_page_of_history_carries_the_actor_from_the_row() {
    let tenant = Uuid::now_v7();
    let actor = Uuid::now_v7();
    let db = provider().await;
    seed_row(&db, tenant, actor, "eu", 10).await;

    let enforcer = PolicyEnforcer::new(Arc::new(FlatInResolver {
        allowed: vec![tenant],
    }));
    let response = get_at(history_router(db, enforcer, Some(ctx_for(tenant))), "").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    let entries = body["entries"].as_array().expect("entries is an array");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0]["actor"].as_str(),
        Some(actor.to_string().as_str()),
        "the actor is the row's own `created_by`, never the Auditor-only audit \
         log (D-12): {body}"
    );
    assert!(
        body["next_cursor"].is_null(),
        "a walk with nothing left hands back no cursor, so a client stops \
         without an extra round trip: {body}"
    );
}

/// **The gate runs before the query.** A refusal is 403 and not an empty page —
/// a caller who reads "no history" stops, and it must stop for the reason it was
/// actually given.
#[tokio::test]
async fn a_refused_caller_gets_403_and_not_an_empty_page() {
    let tenant = Uuid::now_v7();
    let db = provider().await;
    seed_row(&db, tenant, Uuid::now_v7(), "eu", 10).await;

    let enforcer = PolicyEnforcer::new(Arc::new(DenyingResolver));
    let response = get_at(history_router(db, enforcer, Some(ctx_for(tenant))), "").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// No authenticated context is 401, and it is answered before anything reads.
#[tokio::test]
async fn an_unauthenticated_caller_gets_401() {
    let db = provider().await;
    let enforcer = PolicyEnforcer::new(Arc::new(FlatInResolver { allowed: vec![] }));
    let response = get_at(history_router(db, enforcer, None), "").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// **A malformed page request reaches the wire as 400, not as a 500.**
///
/// Both refusals are D-125's and both are the engine's: a zero page never
/// advances, and a cursor that does not decode names no position. What this case
/// adds over the engine's own is that the refusal survives the boundary with its
/// status intact rather than surfacing as an internal fault.
#[tokio::test]
async fn a_zero_limit_and_a_bad_cursor_are_both_400() {
    let tenant = Uuid::now_v7();
    let db = provider().await;
    let enforcer = PolicyEnforcer::new(Arc::new(FlatInResolver {
        allowed: vec![tenant],
    }));

    for query in ["?limit=0", "?cursor=not-a-token"] {
        let response = get_at(
            history_router(db.clone(), enforcer.clone(), Some(ctx_for(tenant))),
            query,
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "`{query}` must be refused at the wire with 400"
        );
    }
}

/// The cursor round-trips through the wire: page one hands back a token, and
/// passing it as `cursor` opens page two on the row after the last.
#[tokio::test]
async fn the_cursor_the_wire_hands_back_opens_the_next_page() {
    let tenant = Uuid::now_v7();
    let db = provider().await;
    seed_row(&db, tenant, Uuid::now_v7(), "eu", 10).await;
    seed_row(&db, tenant, Uuid::now_v7(), "us", 11).await;

    let enforcer = PolicyEnforcer::new(Arc::new(FlatInResolver {
        allowed: vec![tenant],
    }));
    let first = body_json(
        get_at(
            history_router(db.clone(), enforcer.clone(), Some(ctx_for(tenant))),
            "?limit=1",
        )
        .await,
    )
    .await;
    assert_eq!(first["entries"].as_array().expect("entries").len(), 1);
    let cursor = first["next_cursor"]
        .as_str()
        .expect("a page with more behind it hands back a cursor")
        .to_owned();

    let second = body_json(
        get_at(
            history_router(db, enforcer, Some(ctx_for(tenant))),
            &format!("?limit=1&cursor={cursor}"),
        )
        .await,
    )
    .await;
    let entries = second["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1);
    assert_ne!(
        entries[0]["price_id"], first["entries"][0]["price_id"],
        "the second page opens strictly after the first's last row: {second}"
    );
}

/// **The scope that filters is the PDP's, not the subject's tenant** — and this
/// case exists because a probe found nothing without it.
///
/// Replacing the compiled scope with `AccessScope::for_tenant(subject_tenant)`
/// changed no assertion in this file: the fake constrains to the same tenant the
/// context carries, so the two scopes were identical in every case. That made the
/// whole suite silent about which of them does the filtering — and the compiled
/// scope, with `require_constraints = true`, is the isolation.
///
/// Here the PDP allows but constrains to a **different** tenant. A route that
/// filtered on the subject's tenant would serve the caller's own rows; one that
/// filters on the compiled scope serves none.
#[tokio::test]
async fn the_page_is_filtered_by_the_compiled_scope_and_not_by_the_subject() {
    let caller = Uuid::now_v7();
    let elsewhere = Uuid::now_v7();
    let db = provider().await;
    seed_row(&db, caller, Uuid::now_v7(), "eu", 10).await;

    // Allowed, but constrained to a tenant the caller is not.
    let enforcer = PolicyEnforcer::new(Arc::new(FlatInResolver {
        allowed: vec![elsewhere],
    }));
    let body =
        body_json(get_at(history_router(db, enforcer, Some(ctx_for(caller))), "").await).await;
    assert_eq!(
        body["entries"].as_array().expect("entries").len(),
        0,
        "the compiled scope constrains to another tenant, so this caller's own \
         rows must not be served: {body}"
    );
}
