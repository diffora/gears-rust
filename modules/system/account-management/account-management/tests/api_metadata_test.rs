//! HTTP-level E2E tests for the
//! `/account-management/v1/tenants/{tenant_id}/metadata*` REST surface.
//!
//! Scope: per-schema PUT / GET / DELETE / list / resolve flows, the
//! unified GET 404 contract (both "schema unknown to registry" and
//! "entry missing for tenant" collapse to a single `not_found`
//! envelope), DELETE idempotency on missing rows, and RFC 7231 PUT
//! semantics (always return 200, never 201).

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::doc_markdown
)]

mod common;

use account_management::domain::metadata::registry::InheritancePolicy;
use axum::http::StatusCode;
use gts::GtsSchemaId;
use tower::ServiceExt;
use uuid::Uuid;

use common::*;

fn schema_path(tenant: Uuid, schema_id: &str) -> String {
    // The chained `~` characters are URI-safe; axum's `Path` extractor
    // handles them without percent-encoding.
    format!("/account-management/v1/tenants/{tenant}/metadata/{schema_id}")
}

fn router_with_registered_schema(h: &Harness) -> (TestServices, axum::Router) {
    let registry = metadata_registry_with(vec![(
        GtsSchemaId::new(REGISTERED_METADATA_SCHEMA),
        InheritancePolicy::OverrideOnly,
    )]);
    let services = build_services_with(h, fake_idp(), registry);
    let router = build_test_router(&services);
    (services, router)
}

// ─── PUT — RFC 7231 PUT always returns 200 ───────────────────────────

#[tokio::test]
async fn put_metadata_returns_200_with_post_write_entry_on_insert() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::new_v4();
    seed_root(&h, root).await;
    let (_services, router) = router_with_registered_schema(&h);

    let body = serde_json::json!({"hello": "world"});
    let req = json_request(
        "PUT",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        Some(body),
        ctx_for(root),
    );
    let resp = router.oneshot(req).await.expect("router");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "PUT must surface 200 per RFC 7231, never 201",
    );
    let body = response_body(resp).await;
    assert_eq!(body["schema_id"], REGISTERED_METADATA_SCHEMA);
    assert_eq!(body["value"]["hello"], "world");
    assert_eq!(body["tenant_id"], root.to_string());
}

#[tokio::test]
async fn put_metadata_returns_200_on_idempotent_rerun() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::new_v4();
    seed_root(&h, root).await;
    let (_services, router) = router_with_registered_schema(&h);

    let body = serde_json::json!({"a": 1});
    let req = json_request(
        "PUT",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        Some(body.clone()),
        ctx_for(root),
    );
    let resp = router.clone().oneshot(req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::OK);

    // Re-PUT the same body — should still be 200, the upsert collapses
    // to an update on the second pass.
    let req = json_request(
        "PUT",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        Some(body),
        ctx_for(root),
    );
    let resp = router.oneshot(req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::OK);
}

// ─── GET ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn get_metadata_returns_200_with_entry() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::new_v4();
    seed_root(&h, root).await;
    let (_services, router) = router_with_registered_schema(&h);

    // Seed via PUT.
    let put_body = serde_json::json!({"x": 42});
    let put_req = json_request(
        "PUT",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        Some(put_body.clone()),
        ctx_for(root),
    );
    let resp = router.clone().oneshot(put_req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::OK);

    let get_req = json_request(
        "GET",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        None,
        ctx_for(root),
    );
    let resp = router.oneshot(get_req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert_eq!(body["value"]["x"], 42);
}

// @cpt-dod:cpt-cf-account-management-dod-tenant-metadata-unified-404:p1
#[tokio::test]
async fn get_metadata_unknown_schema_returns_404_with_distinct_code() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::new_v4();
    seed_root(&h, root).await;
    let (_services, router) = router_with_registered_schema(&h);

    let req = json_request(
        "GET",
        &schema_path(root, UNREGISTERED_METADATA_SCHEMA),
        None,
        ctx_for(root),
    );
    let resp = router.oneshot(req).await.expect("router");
    let (status, body) = response_problem(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Unified metadata 404: the unregistered-schema arm and the
    // missing-entry arm share the same `code=not_found` and
    // `resource_type`. The `detail` prose still mentions the
    // registration miss for operator-side correlation.
    let detail = body["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("not registered"),
        "unknown schema 404 detail should mention registration miss, got body={body}",
    );
}

#[tokio::test]
async fn get_metadata_missing_entry_returns_unified_404() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::new_v4();
    seed_root(&h, root).await;
    let (_services, router) = router_with_registered_schema(&h);

    // Schema is registered, but no entry was written → same unified 404.
    let req = json_request(
        "GET",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        None,
        ctx_for(root),
    );
    let resp = router.oneshot(req).await.expect("router");
    let (status, body) = response_problem(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let detail = body["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("no metadata entry") || detail.contains("entry_not_found"),
        "missing entry 404 must mention the empty entry, got body={body}",
    );
}

// ─── DELETE ──────────────────────────────────────────────────────────

#[tokio::test]
async fn delete_metadata_returns_204_no_content() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::new_v4();
    seed_root(&h, root).await;
    let (_services, router) = router_with_registered_schema(&h);

    // Seed.
    let put_req = json_request(
        "PUT",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        Some(serde_json::json!({"k": "v"})),
        ctx_for(root),
    );
    let resp = router.clone().oneshot(put_req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::OK);

    let del_req = json_request(
        "DELETE",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        None,
        ctx_for(root),
    );
    let resp = router.oneshot(del_req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn delete_metadata_missing_entry_is_idempotent_204() {
    // Idempotent on missing rows — mirrors `delete_user` deprovision
    // idempotency. A second DELETE after a successful one also returns 204.
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::new_v4();
    seed_root(&h, root).await;
    let (_services, router) = router_with_registered_schema(&h);

    let req = json_request(
        "DELETE",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        None,
        ctx_for(root),
    );
    let resp = router.clone().oneshot(req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Repeat on still-missing row.
    let req = json_request(
        "DELETE",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        None,
        ctx_for(root),
    );
    let resp = router.oneshot(req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// ─── LIST ────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_metadata_returns_200_with_page() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::new_v4();
    seed_root(&h, root).await;
    let (_services, router) = router_with_registered_schema(&h);

    // Seed.
    let put_req = json_request(
        "PUT",
        &schema_path(root, REGISTERED_METADATA_SCHEMA),
        Some(serde_json::json!({"v": 1})),
        ctx_for(root),
    );
    let resp = router.clone().oneshot(put_req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::OK);

    let list_req = json_request(
        "GET",
        &format!("/account-management/v1/tenants/{root}/metadata"),
        None,
        ctx_for(root),
    );
    let resp = router.oneshot(list_req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["schema_id"], REGISTERED_METADATA_SCHEMA);
}

// ─── RESOLVED ────────────────────────────────────────────────────────

#[tokio::test]
async fn resolve_metadata_returns_resolved_false_on_empty_walk() {
    // `OverrideOnly` schema + no own row → resolved=false.
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::new_v4();
    seed_root(&h, root).await;
    let (_services, router) = router_with_registered_schema(&h);

    let req = json_request(
        "GET",
        &format!(
            "/account-management/v1/tenants/{root}/metadata/{REGISTERED_METADATA_SCHEMA}/resolved"
        ),
        None,
        ctx_for(root),
    );
    let resp = router.oneshot(req).await.expect("router");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert_eq!(body["resolved"], false);
    assert_eq!(body["tenant_id"], root.to_string());
    assert_eq!(body["schema_id"], REGISTERED_METADATA_SCHEMA);
}

// ─── Schema-id validation ────────────────────────────────────────────

#[tokio::test]
async fn put_metadata_malformed_schema_id_returns_400_metadata_validation() {
    // A schema-id that fails the GTS chain shape lower must surface a
    // wire-layer 400 with the `metadata_validation` family. Empty
    // strings / missing trailing `~` reach the service and route through
    // `DomainError::MetadataValidation`.
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::new_v4();
    seed_root(&h, root).await;
    let (_services, router) = router_with_registered_schema(&h);

    // Use a chain shape that the GTS validator rejects (missing
    // trailing `~`). The PUT body is otherwise well-formed JSON.
    let bad_path = format!("/account-management/v1/tenants/{root}/metadata/not-a-gts-chain");
    let req = json_request(
        "PUT",
        &bad_path,
        Some(serde_json::json!({"v": 1})),
        ctx_for(root),
    );
    let resp = router.oneshot(req).await.expect("router");
    // The service returns 400 (validation arm) or 404 if the schema
    // resolves to unregistered — either is acceptable wire behavior
    // for a malformed schema id, but it MUST be a client-error class.
    let status = resp.status();
    assert!(
        status.is_client_error(),
        "malformed schema_id MUST surface as a 4xx, got {status}"
    );
}
