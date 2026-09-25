//! Runtime registration, declared routes and header readers must remain equal.
#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "common/census.rs"]
pub mod census;
pub mod rest_support;
use census::Routes;

fn declared_paths() -> Routes {
    [
        ("POST", "/bss-pricing/v1/price-books"),
        ("GET", "/bss-pricing/v1/price-books"),
        ("GET", "/bss-pricing/v1/price-books/{id}"),
        ("PATCH", "/bss-pricing/v1/price-books/{id}"),
        ("GET", "/bss-pricing/v1/price-books/{id}/prices"),
        ("GET", "/bss-pricing/v1/price-books/{id}/export"),
        ("GET", "/bss-pricing/v1/settings"),
        ("PUT", "/bss-pricing/v1/settings"),
        ("GET", "/bss-pricing/v1/dimension-keys"),
        ("PUT", "/bss-pricing/v1/dimension-keys"),
    ]
    .into_iter()
    .map(|(m, p)| (m.to_owned(), p.to_owned()))
    .collect()
}
fn if_match_routes() -> Routes {
    [
        ("PATCH", "/bss-pricing/v1/price-books/{id}"),
        ("PUT", "/bss-pricing/v1/settings"),
        ("PUT", "/bss-pricing/v1/dimension-keys"),
    ]
    .into_iter()
    .map(|(m, p)| (m.to_owned(), p.to_owned()))
    .collect()
}
fn idempotency_key_routes() -> Routes {
    [("POST", "/bss-pricing/v1/price-books")]
        .into_iter()
        .map(|(m, p)| (m.to_owned(), p.to_owned()))
        .collect()
}

#[tokio::test]
async fn the_registered_route_set_is_exactly_the_declared_paths() {
    let harness = rest_support::Harness::new().await.unwrap();
    let (router, openapi) = harness.router(axum::Router::new()).unwrap();
    let registered: Routes = openapi
        .operation_specs
        .iter()
        .map(|e| {
            let (method, path) = e.key().split_once(':').unwrap();
            (method.to_owned(), path.to_owned())
        })
        .collect();
    assert_eq!(registered, declared_paths());
    assert_eq!(census::source_routes(), registered);
    assert_eq!(registered.len(), 10);
    assert!(router.has_routes());
}

#[test]
fn the_registration_parser_has_a_positive_control() {
    let routes = census::registrations(census::CONTROL);
    assert_eq!(routes.len(), 2);
    assert_eq!(
        (&*routes[0].method, &*routes[0].path, &*routes[0].handler),
        ("POST", "/bss-pricing/v1/control", "create")
    );
    assert_eq!(
        (&*routes[1].method, &*routes[1].path, &*routes[1].handler),
        ("GET", "/bss-pricing/v1/control", "read")
    );
}

#[test]
fn every_precondition_reading_route_is_in_the_precondition_census() {
    assert_eq!(
        census::readers("preconditions::if_match("),
        if_match_routes()
    );
    assert_eq!(
        census::readers("preconditions::idempotency_key("),
        idempotency_key_routes()
    );
    for (needle, control, production) in [
        ("preconditions::if_match(", 1, 3),
        ("preconditions::idempotency_key(", 1, 1),
        ("Query<", 1, 0),
        ("StatusCode::", 2, 21),
    ] {
        assert_eq!(census::count_in_functions(census::CONTROL, needle), control);
        assert_eq!(census::production_count(needle), production, "{needle}");
    }
}

#[tokio::test]
async fn every_precondition_reading_route_declares_the_header_it_reads() {
    use toolkit::api::operation_builder::ParamLocation;
    let harness = rest_support::Harness::new().await.unwrap();
    let (_, openapi) = harness.router(axum::Router::new()).unwrap();
    for (header, expected) in [
        ("if-match", if_match_routes()),
        ("idempotency-key", idempotency_key_routes()),
    ] {
        let declared: Routes = openapi
            .operation_specs
            .iter()
            .filter(|e| {
                e.value().params.iter().any(|p| {
                    p.location == ParamLocation::Header && p.name.eq_ignore_ascii_case(header)
                })
            })
            .map(|e| {
                let (method, path) = e.key().split_once(':').unwrap();
                (method.to_owned(), path.to_owned())
            })
            .collect();
        assert_eq!(declared, expected, "{header}");
    }
}

#[test]
fn no_handler_takes_axums_json_extractor() {
    assert_eq!(
        census::count_in_functions("async fn create(Json(body): Json<Input>) {}", "Json<"),
        1
    );
    assert_eq!(census::production_count("Json<"), 0);
}

#[tokio::test]
async fn no_operation_declares_a_422() {
    let harness = rest_support::Harness::new().await.unwrap();
    let (_, openapi) = harness.router(axum::Router::new()).unwrap();
    for entry in &openapi.operation_specs {
        for response in &entry.value().responses {
            assert_ne!(response.status, 422);
        }
    }
}

// Run-2 route contract: method | path | resource:action | If-Match | Idempotency-Key
// POST /price-books price_book:author false true
// GET /price-books price_book:read false false
// GET /price-books/{id} price_book:read false false
// PATCH /price-books/{id} price_book:author true false
// GET /price-books/{id}/prices price:read false false
// GET /price-books/{id}/export price_book:read false false
// GET /settings config:read false false
// PUT /settings config:settings true false
// GET /dimension-keys config:read false false
// PUT /dimension-keys config:settings true false
