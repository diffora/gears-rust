//! Runtime registration, declared routes and header readers must remain equal.
#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "common/census.rs"]
pub mod census;
pub mod rest_support;
use census::Routes;

fn declared_paths() -> Routes {
    [
        ("POST", "/bss-pricing/v1/price-books"),
        ("POST", "/bss-pricing/v1/price-books/{id}/prices"),
        ("GET", "/bss-pricing/v1/prices/{id}"),
        ("PATCH", "/bss-pricing/v1/prices/{id}"),
        ("DELETE", "/bss-pricing/v1/prices/{id}"),
        ("GET", "/bss-pricing/v1/reference-ops"),
        ("GET", "/bss-pricing/v1/price-books"),
        ("GET", "/bss-pricing/v1/price-books/{id}"),
        ("PATCH", "/bss-pricing/v1/price-books/{id}"),
        ("GET", "/bss-pricing/v1/price-books/{id}/prices"),
        ("GET", "/bss-pricing/v1/price-books/{id}/export"),
        ("GET", "/bss-pricing/v1/settings"),
        ("PUT", "/bss-pricing/v1/settings"),
        ("GET", "/bss-pricing/v1/dimension-keys"),
        ("PUT", "/bss-pricing/v1/dimension-keys"),
        ("POST", "/bss-pricing/v1/prices/{id}/rows"),
        ("PATCH", "/bss-pricing/v1/rows/{id}"),
        ("DELETE", "/bss-pricing/v1/rows/{id}"),
        ("POST", "/bss-pricing/v1/rows/{id}/submit"),
        ("GET", "/bss-pricing/v1/price-books/{id}/publish-changes"),
        ("POST", "/bss-pricing/v1/price-books/{id}/publish-changes"),
        ("GET", "/bss-pricing/v1/approval-units"),
        ("GET", "/bss-pricing/v1/approval-units/{id}"),
        ("POST", "/bss-pricing/v1/approval-units/{id}/approve"),
        ("POST", "/bss-pricing/v1/approval-units/{id}/reject"),
        ("POST", "/bss-pricing/v1/approval-units/{id}/withdraw"),
        ("GET", "/bss-pricing/v1/approval-policy"),
        ("PUT", "/bss-pricing/v1/approval-policy"),
    ]
    .into_iter()
    .map(|(m, p)| (m.to_owned(), p.to_owned()))
    .collect()
}
fn if_match_routes() -> Routes {
    [
        ("PATCH", "/bss-pricing/v1/prices/{id}"),
        ("PATCH", "/bss-pricing/v1/price-books/{id}"),
        ("PUT", "/bss-pricing/v1/settings"),
        ("PUT", "/bss-pricing/v1/dimension-keys"),
        ("PATCH", "/bss-pricing/v1/rows/{id}"),
        ("DELETE", "/bss-pricing/v1/rows/{id}"),
        ("PUT", "/bss-pricing/v1/approval-policy"),
    ]
    .into_iter()
    .map(|(m, p)| (m.to_owned(), p.to_owned()))
    .collect()
}
fn idempotency_key_routes() -> Routes {
    [
        ("POST", "/bss-pricing/v1/price-books"),
        ("POST", "/bss-pricing/v1/price-books/{id}/prices"),
        ("POST", "/bss-pricing/v1/prices/{id}/rows"),
        ("POST", "/bss-pricing/v1/rows/{id}/submit"),
        ("POST", "/bss-pricing/v1/price-books/{id}/publish-changes"),
        ("POST", "/bss-pricing/v1/approval-units/{id}/approve"),
        ("POST", "/bss-pricing/v1/approval-units/{id}/reject"),
        ("POST", "/bss-pricing/v1/approval-units/{id}/withdraw"),
    ]
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
    assert_eq!(registered.len(), 28);
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
        ("preconditions::if_match(", 1, 7),
        ("preconditions::idempotency_key(", 1, 8),
        ("Query<", 1, 0),
        ("StatusCode::", 2, 57),
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

// Run-3 route contract: method | path | resource:action | If-Match | Idempotency-Key
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

// POST /price-books/{id}/prices price:author false true
// GET /prices/{id} price:read false false
// PATCH /prices/{id} price:author true false
// DELETE /prices/{id} price:author false false

// GET /reference-ops config:settings false false

// Run-4 rows: method | path | resource:action | If-Match | Idempotency-Key
// POST /prices/{id}/rows price:author false true
// PATCH /rows/{id} price:author true false
// DELETE /rows/{id} price:author true false

// Run-4 approvals: method | path | resource:action | If-Match | Idempotency-Key
// POST /rows/{id}/submit price:submit false true
// GET /price-books/{id}/publish-changes price_book:read false false
// POST /price-books/{id}/publish-changes price_book:submit false true
// GET /approval-units approval_unit:read false false
// GET /approval-units/{id} approval_unit:read false false
// POST /approval-units/{id}/approve approval_unit:approve false true
// POST /approval-units/{id}/reject approval_unit:approve false true
// POST /approval-units/{id}/withdraw approval_unit:submit false true
// GET /approval-policy config:read false false
// PUT /approval-policy config:settings true false
