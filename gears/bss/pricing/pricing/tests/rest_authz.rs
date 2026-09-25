//! Authz census stays equal to the runtime and source route sets.
#![allow(clippy::expect_used, clippy::unwrap_used)]
#[path = "common/census.rs"]
pub mod census;
pub mod rest_support;

fn census() -> census::Routes {
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

#[tokio::test]
async fn the_census_covers_every_route_the_routers_register() {
    let harness = rest_support::Harness::new().await.unwrap();
    let (_, openapi) = harness.router(axum::Router::new()).unwrap();
    let registered: census::Routes = openapi
        .operation_specs
        .iter()
        .map(|e| {
            let (method, path) = e.key().split_once(':').unwrap();
            (method.to_owned(), path.to_owned())
        })
        .collect();
    assert_eq!(registered, census());
    assert_eq!(census::source_routes(), registered);
    assert_eq!(census::readers("require_authenticated("), registered);
    assert_eq!(census::readers("authz::access_scope("), registered);
    assert_eq!(registered.len(), 28);
    assert_eq!(bss_pricing::authz::labels::ALL.len(), 4);
    let permissions: Vec<_> = toolkit_gts::inventory::iter::<toolkit_gts::InventoryInstance>
        .into_iter()
        .filter(|i| i.instance_id.contains("~cf.bss.pricing."))
        .collect();
    assert!(permissions.is_empty());
}

#[test]
fn the_authentication_and_authz_parsers_have_positive_controls() {
    for needle in ["require_authenticated(", "authz::access_scope("] {
        assert_eq!(census::count_in_functions(census::CONTROL, needle), 2);
        assert_eq!(
            census::production_count(needle),
            if needle == "require_authenticated(" {
                29
            } else {
                28
            }
        );
    }
    let routes = census::registrations(census::CONTROL);
    assert_eq!(
        routes
            .iter()
            .filter(|r| r.declaration.contains(".authenticated()"))
            .count(),
        2
    );
    assert_eq!(census::source_routes().len(), 28);
}

#[test]
fn every_mounted_router_is_merged_into_both_censuses() {
    // Both census files and rest_support register through the runtime capability:
    // their inventories cannot omit a router by forgetting a manual merge.
    assert_eq!(
        census::functions(census::CONTROL)
            .keys()
            .filter(|name| name.ends_with("router"))
            .count(),
        1
    );
    let routers: Vec<_> = census::sources()
        .iter()
        .flat_map(|path| {
            census::functions(&std::fs::read_to_string(path).unwrap())
                .into_keys()
                .filter(|name| name.ends_with("router"))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        routers,
        vec!["router".to_owned()],
        "every authoring router is mounted"
    );
    assert_eq!(census::source_routes(), census());
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
