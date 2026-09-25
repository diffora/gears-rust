//! Authz census stays equal to the runtime and source route sets.
#![allow(clippy::expect_used, clippy::unwrap_used)]
#[path = "common/census.rs"]
pub mod census;
pub mod rest_support;

fn census() -> census::Routes {
    census::Routes::new()
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
    assert_eq!(registered.len(), 0);
    assert!(bss_pricing::authz::labels::ALL.is_empty());
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
        assert_eq!(census::production_count(needle), 0);
    }
    let routes = census::registrations(census::CONTROL);
    assert_eq!(
        routes
            .iter()
            .filter(|r| r.declaration.contains(".authenticated()"))
            .count(),
        2
    );
    assert_eq!(census::source_routes().len(), 0);
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
        Vec::<String>::new(),
        "the skeleton has no door routers"
    );
    assert_eq!(census::source_routes(), census());
}
