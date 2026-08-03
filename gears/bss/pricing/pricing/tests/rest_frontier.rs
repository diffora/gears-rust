//! Router-level tests for `GET /bss-pricing/v1/catalog-version/frontier`.
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
use bss_pricing::api::rest::frontier::{ApiState, router};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{PinFrontierRepo, pin_frontier_repo};
use bss_pricing_sdk::CatalogVersion;
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

const PATH: &str = "/bss-pricing/v1/catalog-version/frontier";

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

/// Put a tenant's frontier where a completed projection would have left it.
///
/// Through a plain connection, which is what a test has and the projector does
/// not: the advance takes a runner precisely so it can join the transaction
/// that completed the version (D-136).
async fn seed_frontier(
    db: &DBProvider<DbError>,
    tenant: Uuid,
    to: CatalogVersion,
    at: chrono::DateTime<Utc>,
) {
    let conn = db.conn().expect("conn");
    pin_frontier_repo::advance(&conn, &AccessScope::for_tenant(tenant), tenant, to, at)
        .await
        .expect("seed the frontier");
}

/// The real router over a real repository, plus the two layers `register_rest`
/// applies (here without the canonical-error middleware: these assertions are
/// about status codes, which `CanonicalError` already carries as an
/// `IntoResponse`).
fn frontier_router(
    db: DBProvider<DbError>,
    enforcer: PolicyEnforcer,
    ctx: Option<SecurityContext>,
) -> Router {
    let state = Arc::new(ApiState {
        pin_frontier: PinFrontierRepo::new(db),
    });
    let openapi = OpenApiRegistryImpl::new();
    let router = router(state, &openapi).layer(axum::Extension(enforcer));
    match ctx {
        Some(ctx) => router.layer(axum::Extension(ctx)),
        None => router,
    }
}

async fn get(router: Router) -> axum::http::Response<Body> {
    router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(PATH)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send")
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 1_000_000)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("the body is JSON")
}

#[tokio::test]
async fn an_unauthenticated_request_is_refused_with_401() {
    let tenant = Uuid::now_v7();
    let router = frontier_router(
        provider().await,
        PolicyEnforcer::new(Arc::new(FlatInResolver {
            allowed: vec![tenant],
        })),
        /* no SecurityContext extension */ None,
    );

    let response = get(router).await;

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "the frontier is tenant-scoped, so an anonymous caller is refused before the PEP"
    );
}

#[tokio::test]
async fn a_denied_request_is_refused_with_403() {
    let tenant = Uuid::now_v7();
    let router = frontier_router(
        provider().await,
        PolicyEnforcer::new(Arc::new(DenyingResolver)),
        Some(ctx_for(tenant)),
    );

    let response = get(router).await;

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a caller without plan x read must be denied, not answered with an empty frontier"
    );
}

#[tokio::test]
async fn an_authorized_read_of_an_advanced_frontier_is_200_with_the_version() {
    let tenant = Uuid::now_v7();
    let db = provider().await;
    let advanced_at = Utc.with_ymd_and_hms(2026, 8, 1, 9, 30, 0).unwrap();
    seed_frontier(&db, tenant, CatalogVersion::new(7), advanced_at).await;

    let router = frontier_router(
        db,
        PolicyEnforcer::new(Arc::new(FlatInResolver {
            allowed: vec![tenant],
        })),
        Some(ctx_for(tenant)),
    );

    let response = get(router).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["pin_eligible"], serde_json::json!(true));
    assert_eq!(body["catalog_version"], serde_json::json!(7));
    assert!(
        !body["advanced_at"].is_null(),
        "the advance instant is the referent of the pin-lag rule: {body}"
    );
}

#[tokio::test]
async fn a_tenant_with_no_completed_publish_is_200_and_says_so_explicitly() {
    // The 404-vs-200 decision, asserted on the wire. A consumer must be able to
    // tell "no publish has ever completed" from "the frontier is at version 0",
    // and this gear's 404 deliberately conflates absent with out-of-scope, so
    // the empty reading is a 200 carrying its own discriminator.
    let tenant = Uuid::now_v7();
    let router = frontier_router(
        provider().await,
        PolicyEnforcer::new(Arc::new(FlatInResolver {
            allowed: vec![tenant],
        })),
        Some(ctx_for(tenant)),
    );

    let response = get(router).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["pin_eligible"], serde_json::json!(false));
    assert!(body["catalog_version"].is_null(), "{body}");
    assert!(body["advanced_at"].is_null(), "{body}");
}

#[tokio::test]
async fn a_foreign_tenants_frontier_is_not_readable() {
    // SQL-level BOLA at the REST seam: the caller is authorized for its own
    // tenant only, so the other tenant's advanced frontier is invisible and the
    // answer is the empty reading, not someone else's version.
    let mine = Uuid::now_v7();
    let theirs = Uuid::now_v7();
    let db = provider().await;
    seed_frontier(
        &db,
        theirs,
        CatalogVersion::new(9),
        Utc.with_ymd_and_hms(2026, 8, 1, 9, 30, 0).unwrap(),
    )
    .await;

    let router = frontier_router(
        db,
        PolicyEnforcer::new(Arc::new(FlatInResolver {
            allowed: vec![mine],
        })),
        Some(ctx_for(mine)),
    );

    let response = get(router).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["pin_eligible"], serde_json::json!(false));
    assert!(body["catalog_version"].is_null(), "{body}");
}
