//! Gear-declaration smoke tests: the capability wiring is real, not decorative.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use bss_pricing::api::rest::state::AuthoringState;
use bss_pricing::infra::storage::repo::{
    IdempotencyGate, PinFrontierRepo, PlanRepo, PlanShapeRepo, PriceRepo,
};
use bss_pricing::module::BssPricingGear;
use toolkit::GearCtx;
use toolkit::api::OpenApiRegistryImpl;
use toolkit::api::operation_builder::ParamLocation;
use toolkit::contracts::{DatabaseCapability, RestApiCapability};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};

#[test]
fn the_gear_declares_the_database_capability() {
    // The `db` capability must resolve to the Foundation chain, not to an empty
    // vec: a gear that declares `db` and hands the platform nothing has tables
    // no migration ever creates.
    let gear = BssPricingGear::default();

    assert!(
        !gear.migrations().is_empty(),
        "the Foundation migration chain must be wired into the db capability"
    );
}

#[test]
fn every_migration_name_is_unique() {
    // The toolkit runner applies migrations in NAME order and rejects a
    // duplicate name outright — so a copy-pasted `DeriveMigrationName` does not
    // merely sort oddly, it aborts the whole chain at boot. Asserting it here
    // means the mistake is caught by `cargo test` rather than by a crash loop
    // in a cluster.
    let gear = BssPricingGear::default();
    let migrations = gear.migrations();
    let names: Vec<String> = migrations.iter().map(|m| m.name().to_owned()).collect();
    let unique: HashSet<&String> = names.iter().collect();

    assert_eq!(
        unique.len(),
        names.len(),
        "duplicate migration name in the chain: {names:?}"
    );
}

/// The nine routes this gear serves, spelled through the modules' own consts.
///
/// A tenth path registered without a line here fails the census below, which is
/// what stops a route landing without anybody deciding it should. The list is
/// also the only thing pinning those `const`s against the **literals** the
/// `OperationBuilder` calls take: DE0801 validates a literal argument and
/// silently passes a `const` one, so the route-shape rule binds where the
/// literal is, and this binds the two spellings together.
fn declared_paths() -> Vec<(&'static str, &'static str)> {
    use bss_pricing::api::rest::plans::{PLAN, PLAN_ABANDON, PLANS};
    use bss_pricing::api::rest::prices::{PLAN_PRICE, PLAN_PRICES};
    vec![
        ("GET", "/bss-pricing/v1/catalog-version/frontier"),
        ("GET", PLAN),
        ("POST", PLANS),
        ("PATCH", PLAN),
        ("POST", PLAN_ABANDON),
        ("POST", PLAN_PRICES),
        ("GET", PLAN_PRICES),
        ("PATCH", PLAN_PRICE),
        ("DELETE", PLAN_PRICE),
    ]
}

/// Build the three routers over a connected-but-empty database and hand back
/// what they registered.
///
/// No migrations: nothing here sends a request, and the registration happens
/// while the router is built. What this needs a provider for is that the states
/// hold one.
async fn registered_operations() -> OpenApiRegistryImpl {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    let db = DBProvider::<DbError>::new(db);
    let openapi = OpenApiRegistryImpl::new();

    let frontier_state = Arc::new(bss_pricing::api::rest::frontier::ApiState {
        pin_frontier: PinFrontierRepo::new(db.clone()),
    });
    let authoring = Arc::new(AuthoringState {
        db: db.clone(),
        plans: PlanRepo::new(db.clone()),
        shapes: PlanShapeRepo::new(db.clone()),
        prices: PriceRepo::new(db),
        idempotency: IdempotencyGate::new(Duration::from_hours(1)),
    });

    drop(
        bss_pricing::api::rest::frontier::router(frontier_state, &openapi)
            .merge(bss_pricing::api::rest::plans::router(
                Arc::clone(&authoring),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::prices::router(authoring, &openapi)),
    );
    openapi
}

#[tokio::test]
async fn the_registered_route_set_is_exactly_the_nine_paths() {
    // Six groups built repositories, guards, a validator, a commit and a
    // projector; this is the whole of what an operator can reach. A route added
    // later without a line in `declared_paths` fails here rather than shipping
    // ungated and undocumented.
    let openapi = registered_operations().await;

    let mut found: Vec<String> = openapi
        .operation_specs
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    found.sort();
    let mut expected: Vec<String> = declared_paths()
        .into_iter()
        .map(|(method, path)| format!("{method}:{path}"))
        .collect();
    expected.sort();

    assert_eq!(found, expected);
}

#[tokio::test]
async fn every_registered_operation_carries_a_tag_and_a_summary() {
    // DE0205 in a test as well as in a lint: an operation without them is one an
    // operator meets in a generated client with no name and no group.
    let openapi = registered_operations().await;

    for entry in &openapi.operation_specs {
        let spec = entry.value();
        assert!(!spec.tags.is_empty(), "{} carries no tag", entry.key());
        assert!(
            spec.summary.as_ref().is_some_and(|s| !s.trim().is_empty()),
            "{} carries no summary",
            entry.key()
        );
    }
}

#[tokio::test]
async fn no_operation_declares_a_422() {
    // §3.3 states it once for the whole design set: the platform has no 422
    // category, every architectural 422 reaches the wire as a 400 carrying its
    // code, and an endpoint MUST NOT declare a response no path can produce.
    let openapi = registered_operations().await;

    for entry in &openapi.operation_specs {
        for response in &entry.value().responses {
            assert_ne!(
                response.status,
                422,
                "{} declares a 422; no path in this gear can produce one",
                entry.key()
            );
        }
    }
}

/// A config provider that knows about no gear at all.
struct NoConfig;

impl toolkit::config::ConfigProvider for NoConfig {
    fn get_gear_config(&self, _gear: &str) -> Option<&serde_json::Value> {
        None
    }
}

#[tokio::test]
async fn an_unconfigured_gear_reserves_its_prefix_and_answers_404_under_it() {
    // A gear compiled in but absent from `gears:` has no runtime. It must still
    // claim `/bss-pricing/v1` - so another gear cannot take the namespace - and
    // a request under it must be a clean 404 rather than a 500 from a handler
    // reaching for state that was never built.
    let gear = BssPricingGear::default();
    let openapi = OpenApiRegistryImpl::new();
    let ctx = GearCtx::new(
        "bss-pricing",
        uuid::Uuid::new_v4(),
        Arc::new(NoConfig),
        Arc::new(toolkit::client_hub::ClientHub::new()),
        tokio_util::sync::CancellationToken::new(),
    );

    let router = gear
        .register_rest(&ctx, Router::new(), &openapi)
        .expect("an unconfigured gear still registers a router");

    assert_eq!(
        openapi.operation_specs.len(),
        0,
        "an unconfigured gear registers no operation at all"
    );

    let response = tower::ServiceExt::oneshot(
        router,
        axum::http::Request::builder()
            .method("GET")
            .uri("/bss-pricing/v1/plans/00000000-0000-0000-0000-000000000000")
            .body(axum::body::Body::empty())
            .expect("build request"),
    )
    .await
    .expect("the router answers");

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

/// The six mutating routes, each of which D-171 requires to declare `If-Match`.
///
/// Transcribed rather than derived from the router, for `declared_paths`' reason:
/// a declaration dropped from a registration is invisible if the expectation is
/// read off the same registration.
fn if_match_routes() -> Vec<(&'static str, &'static str)> {
    use bss_pricing::api::rest::plans::{PLAN, PLAN_ABANDON, PLANS};
    use bss_pricing::api::rest::prices::{PLAN_PRICE, PLAN_PRICES};
    vec![
        ("PATCH", PLAN),
        ("POST", PLAN_ABANDON),
        ("PATCH", PLAN_PRICE),
        ("DELETE", PLAN_PRICE),
        // The two creates assert their precondition through the idempotency gate
        // rather than through a version, and are listed under
        // `idempotency_key_routes` instead.
        ("POST", PLANS),
        ("POST", PLAN_PRICES),
    ]
}

/// The two creates, each of which requires an `Idempotency-Key` (D-141/D-142).
fn idempotency_key_routes() -> Vec<(&'static str, &'static str)> {
    use bss_pricing::api::rest::plans::PLANS;
    use bss_pricing::api::rest::prices::PLAN_PRICES;
    vec![("POST", PLANS), ("POST", PLAN_PRICES)]
}

/// The header parameters one registered operation declares.
fn declared_headers(openapi: &OpenApiRegistryImpl, method: &str, path: &str) -> Vec<String> {
    let key = format!("{method}:{path}");
    let entry = openapi
        .operation_specs
        .get(&key)
        .unwrap_or_else(|| panic!("{key} is not a registered operation"));
    entry
        .value()
        .params
        .iter()
        .filter(|param| matches!(param.location, ParamLocation::Header))
        .map(|param| param.name.to_ascii_lowercase())
        .collect()
}

#[tokio::test]
async fn every_mutating_route_declares_its_precondition_header() {
    // D-171's owed clause: the declarations exist on all six routes and **no test
    // reads the registration's params**, so deleting one failed nothing. A
    // declaration is what a generated client sends; a route whose `If-Match` is
    // undeclared is one whose clients omit the header and are then refused 400 by
    // a server that never told them to send it.
    //
    // Delete one `.param(if_match_param(...))` and exactly this assertion fails.
    let openapi = registered_operations().await;

    for (method, path) in if_match_routes() {
        let headers = declared_headers(&openapi, method, path);
        let expected = if idempotency_key_routes().contains(&(method, path)) {
            "idempotency-key"
        } else {
            "if-match"
        };
        assert!(
            headers.iter().any(|name| name == expected),
            "{method} {path} declares no {expected}: {headers:?}"
        );
    }
}

#[tokio::test]
async fn every_create_declares_its_idempotency_key() {
    // The other half, and it is a different header on a different pair of routes:
    // a create has no version to assert, so its precondition is the gate's key.
    let openapi = registered_operations().await;

    for (method, path) in idempotency_key_routes() {
        let headers = declared_headers(&openapi, method, path);
        assert!(
            headers.iter().any(|name| name == "idempotency-key"),
            "{method} {path} declares no Idempotency-Key: {headers:?}"
        );
    }
}

#[tokio::test]
async fn a_read_route_declares_no_precondition_header() {
    // The negative side, which is what stops the two assertions above from being
    // satisfiable by declaring both headers everywhere. A GET has no precondition
    // to assert, and a declared one would tell a client to send a header the
    // server ignores.
    let openapi = registered_operations().await;

    for (method, path) in [
        ("GET", bss_pricing::api::rest::plans::PLAN),
        ("GET", bss_pricing::api::rest::prices::PLAN_PRICES),
    ] {
        let headers = declared_headers(&openapi, method, path);
        assert!(
            !headers.iter().any(|name| name == "if-match"),
            "{method} {path} declares an If-Match it cannot honour: {headers:?}"
        );
    }
}
