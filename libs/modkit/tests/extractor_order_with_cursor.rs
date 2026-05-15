#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
    Router,
    body::to_bytes,
    http::{Request, StatusCode},
    routing::get,
};
use modkit::api::odata::OData;
use tower::ServiceExt;

#[tokio::test]
async fn order_with_cursor_is_400() {
    // trivial route just to trigger extractor
    async fn handler(OData(_q): OData) -> &'static str {
        "ok"
    }

    let app = Router::new().route("/", get(handler));

    // Provide both cursor and $orderby
    let req = Request::builder()
        .uri("/?cursor=eyJ2IjoxLCJrIjpbIjEiXS&$orderby=id%20desc")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Canonical `InvalidArgument` is 400 — replaces the legacy 422 wire status.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let problem: serde_json::Value = serde_json::from_slice(&body).expect("problem+json body");

    // Wire envelope: canonical InvalidArgument category.
    assert!(
        problem["type"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid_argument"),
        "type was {:?}",
        problem["type"]
    );
    assert_eq!(problem["status"].as_u64(), Some(400));

    // Two field_violations — one for `$orderby`, one for `cursor`, both with
    // reason `ORDER_WITH_CURSOR` (see modkit-odata::problem_mapping).
    let violations = problem["context"]["field_violations"]
        .as_array()
        .expect("field_violations[] present");
    assert_eq!(violations.len(), 2, "got {violations:?}");
    let fields: Vec<&str> = violations
        .iter()
        .filter_map(|v| v["field"].as_str())
        .collect();
    assert!(fields.contains(&"$orderby"));
    assert!(fields.contains(&"cursor"));
    for v in violations {
        assert_eq!(v["reason"].as_str(), Some("ORDER_WITH_CURSOR"));
    }
}

#[tokio::test]
async fn cursor_only_is_ok() {
    async fn handler(OData(_q): OData) -> &'static str {
        "ok"
    }

    let app = Router::new().route("/", get(handler));

    // Provide only cursor (malformed → InvalidArgument with single `cursor` violation)
    let req = Request::builder()
        .uri("/?cursor=eyJ2IjoxLCJrIjpbIjEiXS")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let problem: serde_json::Value = serde_json::from_slice(&body).expect("problem+json body");

    // Single cursor-only violation; no `$orderby` entry because no conflict.
    let violations = problem["context"]["field_violations"]
        .as_array()
        .expect("field_violations[] present");
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0]["field"].as_str(), Some("cursor"));
    assert_eq!(violations[0]["reason"].as_str(), Some("INVALID_CURSOR"));
}

#[tokio::test]
async fn orderby_only_is_ok() {
    async fn handler(OData(_q): OData) -> &'static str {
        "ok"
    }

    let app = Router::new().route("/", get(handler));

    // Provide only $orderby
    let req = Request::builder()
        .uri("/?$orderby=id%20desc")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
