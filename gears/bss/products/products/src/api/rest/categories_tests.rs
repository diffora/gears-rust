#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::router;
use crate::test_support::{
    body_json, get, patch, post, problem_code, raw_i64, rest_app, violation_for,
};
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn create_list_patch_and_the_duplicate_and_stale_paths() {
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, router).await;
    let r = post(
        &app,
        tenant,
        "/bss-products/v1/categories",
        json!({"code":" hosting ","name":" Hosting ","is_default":true}),
    )
    .await;
    assert_eq!(r.status(), StatusCode::CREATED);
    let etag = r.headers()["etag"].to_str().unwrap().to_owned();
    let c = body_json(r).await;
    assert_eq!(c["code"], "hosting");
    assert_eq!(c["is_default"], true);
    let url = format!("/bss-products/v1/categories/{}", c["id"].as_str().unwrap());
    let r = post(
        &app,
        tenant,
        "/bss-products/v1/categories",
        json!({"code":"hosting","name":"Again"}),
    )
    .await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(&body_json(r).await), "CATEGORY_CODE_TAKEN");
    let r = patch(&app, tenant, &url, json!({"name":"Cloud"}), None).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(violation_for(&body_json(r).await, "If-Match").is_some());
    let r = patch(&app, tenant, &url, json!({"name":"Cloud"}), Some("\"99\"")).await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(&body_json(r).await), "STALE_REVISION");
    let r = patch(
        &app,
        tenant,
        &url,
        json!({"name":"Cloud", "sort_order":2}),
        Some(&etag),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_ne!(r.headers()["etag"], etag);
    assert_eq!(body_json(r).await["name"], "Cloud");
    let list = body_json(get(&app, tenant, "/bss-products/v1/categories").await).await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        raw_i64(&dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await,
        2
    );
    let other = body_json(get(&app, Uuid::new_v4(), "/bss-products/v1/categories").await).await;
    assert_eq!(other["items"], json!([]));
}

#[tokio::test]
async fn retire_refuses_used_categories_and_audit_failure_rolls_back() {
    use crate::test_support::{drop_table, repo_connection, seed_rest_sku};
    use sea_orm::{ConnectionTrait, Database};
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, router).await;
    let c = body_json(
        post(
            &app,
            tenant,
            "/bss-products/v1/categories",
            json!({"code":"hosting","name":"Hosting"}),
        )
        .await,
    )
    .await;
    let id: Uuid = serde_json::from_value(c["id"].clone()).unwrap();
    let (db, scope) = repo_connection(&dsn, tenant).await;
    seed_rest_sku(&db.conn().unwrap(), &scope, tenant, id, "A").await;
    let url = format!("/bss-products/v1/categories/{id}/retire");
    let r = post(&app, tenant, &url, json!({})).await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(&body_json(r).await), "CATEGORY_IN_USE");
    let raw = Database::connect(&dsn).await.unwrap();
    raw.execute_unprepared("DELETE FROM products_sku")
        .await
        .unwrap();
    raw.close().await.unwrap();
    let r = post(&app, tenant, &url, json!({})).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["status"], "retired");
    assert_eq!(
        post(
            &app,
            tenant,
            &format!("/bss-products/v1/categories/{}/retire", Uuid::new_v4()),
            json!({})
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    drop_table(&dsn, "products_audit_log").await;
    let r = post(
        &app,
        tenant,
        "/bss-products/v1/categories",
        json!({"code":"rollback","name":"Rollback"}),
    )
    .await;
    assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        raw_i64(
            &dsn,
            "SELECT COUNT(*) AS v FROM products_category WHERE code = 'rollback'"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn malformed_fields_are_400_and_authentication_precedes_body_validation() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let tenant = Uuid::new_v4();
    let (app, _) = rest_app(tenant, router).await;
    let r = post(
        &app,
        tenant,
        "/bss-products/v1/categories",
        json!({"code":42,"name":"Bad"}),
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bss-products/v1/categories")
                .header("Content-Type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}
