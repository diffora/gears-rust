//! Demolition contract, written before the legacy implementation is removed.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::module::BssPricingGear;
use toolkit::contracts::DatabaseCapability;

#[test]
fn runtime_chain_contains_coord_pricing_and_toolkit_delivery() {
    let chain = BssPricingGear::default().migrations();
    assert_eq!(
        chain.len(),
        15,
        "coordination, twelve pricing migrations and two toolkit migrations"
    );
    assert_eq!(chain[0].name(), "m0001_create_coord_leases");
}

#[test]
fn skeleton_config_tolerates_old_deployment_keys() {
    let config = serde_json::json!({"legacy_removed_setting": {"enabled": true}});
    serde_json::from_value::<bss_pricing::config::BssPricingConfig>(config)
        .expect("the skeleton tolerates retired deployment fields");
}

pub mod rest_support;

#[tokio::test]
async fn pricing_alone_initializes_serves_authoring_routes_and_stops() {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        routing::get,
    };
    use std::sync::{Arc, atomic::Ordering};
    use toolkit::lifecycle::Runnable;
    use tower::ServiceExt as _;

    let harness = rest_support::Harness::new().await.unwrap();
    assert_eq!(
        harness.registry.calls.load(Ordering::SeqCst),
        1,
        "real init must register schemas"
    );
    let (empty, openapi) = harness.router(Router::new()).unwrap();
    assert!(empty.has_routes());
    assert_eq!(openapi.operation_specs.len(), 36);
    let (router, _) = harness
        .router(Router::new().route("/host", get(|| async { "host" })))
        .unwrap();
    for (path, status) in [
        ("/bss-pricing/v1/anything", StatusCode::NOT_FOUND),
        // Promotions are deferred by the owner (D-409): nothing is mounted there.
        ("/bss-pricing/v1/promotions", StatusCode::NOT_FOUND),
        ("/host", StatusCode::OK),
    ] {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{path}");
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut lifecycle = Box::pin(Arc::new(harness.gear).run(cancel.clone()));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), &mut lifecycle)
            .await
            .is_err(),
        "serve must remain live until shutdown"
    );
    cancel.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(1), lifecycle)
        .await
        .unwrap()
        .unwrap();
}
