//! Tenant descriptor defaults: conditional reads, validation and atomic replacement.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod common;
mod rest_support;
use axum::http::StatusCode;
use bss_pricing::api::rest::billing_descriptors::BILLING_DESCRIPTORS;
use rest_support::{Harness, body_json, etag_of, with_headers};

async fn read(h: &Harness) -> (String, serde_json::Value) {
    let response = h
        .allowed()
        .send(with_headers("GET", BILLING_DESCRIPTORS, None, &[]))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tag = etag_of(&response).expect("policy ETag");
    (tag, body_json(response).await)
}
#[tokio::test]
async fn defaults_replace_conditionally_and_clear_gl_without_changing_other_tenants() {
    let h = Harness::new().await;
    let (initial_tag, mut body) = read(&h).await;
    assert!(body["default_gl_code_ref"].is_null());
    assert_eq!(
        body["default_line_templates"],
        serde_json::json!({"recurring":"{sku} - {period}","usage":"{sku}, {unit}","one_time":"{sku}"})
    );
    body["default_gl_code_ref"] = serde_json::json!("4000");
    body["default_line_templates"]["usage"] = serde_json::json!("{sku_code}: {dimension}");
    let saved = h
        .allowed()
        .send(with_headers(
            "PUT",
            BILLING_DESCRIPTORS,
            Some(body.clone()),
            &[("if-match", &initial_tag)],
        ))
        .await;
    assert_eq!(saved.status(), StatusCode::OK);
    let (saved_tag, saved_body) = read(&h).await;
    assert_eq!(saved_body, body);
    assert_ne!(saved_tag, initial_tag);
    let conditional = h
        .allowed()
        .send(with_headers(
            "GET",
            BILLING_DESCRIPTORS,
            None,
            &[("if-none-match", &saved_tag)],
        ))
        .await;
    assert_eq!(conditional.status(), StatusCode::NOT_MODIFIED);
    let stale = h
        .allowed()
        .send(with_headers(
            "PUT",
            BILLING_DESCRIPTORS,
            Some(body.clone()),
            &[("if-match", &initial_tag)],
        ))
        .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    body["default_gl_code_ref"] = serde_json::Value::Null;
    let cleared = h
        .allowed()
        .send(with_headers(
            "PUT",
            BILLING_DESCRIPTORS,
            Some(body.clone()),
            &[("if-match", &saved_tag)],
        ))
        .await;
    assert_eq!(cleared.status(), StatusCode::OK);
    assert_eq!(read(&h).await.1, body);
    let other = h
        .other_tenant()
        .send(with_headers("GET", BILLING_DESCRIPTORS, None, &[]))
        .await;
    assert_eq!(
        body_json(other).await["default_line_templates"]["usage"],
        "{sku}, {unit}"
    );
}
#[tokio::test]
async fn invalid_templates_and_incomplete_key_maps_leave_the_policy_unchanged() {
    let h = Harness::new().await;
    let (tag, original) = read(&h).await;
    let mut invalid = original.clone();
    invalid["default_line_templates"]["recurring"] = serde_json::json!("{sku_typo}");
    let refused = h
        .allowed()
        .send(with_headers(
            "PUT",
            BILLING_DESCRIPTORS,
            Some(invalid),
            &[("if-match", &tag)],
        ))
        .await;
    assert!(refused.status().is_client_error());
    assert!(
        body_json(refused)
            .await
            .to_string()
            .contains("LINE_TEMPLATE_INVALID")
    );
    let mut missing = original.clone();
    missing["default_line_templates"]
        .as_object_mut()
        .expect("map")
        .remove("usage");
    let refused = h
        .allowed()
        .send(with_headers(
            "PUT",
            BILLING_DESCRIPTORS,
            Some(missing),
            &[("if-match", &tag)],
        ))
        .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(read(&h).await, (tag, original));
}
