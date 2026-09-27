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

/// P-D-196: a SKU without a category never blocks a category's retirement, nor counts as the
/// category's use; a SKU that points at the category still does.
#[tokio::test]
async fn a_sku_without_a_category_never_blocks_a_retirement() {
    use crate::infra::storage::repo;
    use crate::test_support::repo_connection;
    use std::sync::Arc;
    fn doors(
        s: Arc<crate::api::rest::ApiState>,
        o: &dyn toolkit::api::OpenApiRegistry,
    ) -> axum::Router {
        router(Arc::clone(&s), o).merge(crate::api::rest::skus::router(s, o))
    }
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, doors).await;
    let category = |code: &'static str| {
        let app = app.clone();
        async move {
            let r = post(
                &app,
                tenant,
                "/bss-products/v1/categories",
                json!({"code":code,"name":code}),
            )
            .await;
            assert_eq!(r.status(), StatusCode::CREATED);
            serde_json::from_value::<Uuid>(body_json(r).await["id"].clone()).unwrap()
        }
    };
    let unused = category("unused").await;
    let r = post(
        &app,
        tenant,
        "/bss-products/v1/skus",
        json!({"code":"LOOSE","name":"Loose","type":"recurring"}),
    )
    .await;
    assert_eq!(r.status(), StatusCode::CREATED);
    let (db, scope) = repo_connection(&dsn, tenant).await;
    assert_eq!(
        repo::count_skus_in_category(&db.conn().unwrap(), &scope, tenant, unused)
            .await
            .unwrap(),
        0
    );
    let r = post(
        &app,
        tenant,
        &format!("/bss-products/v1/categories/{unused}/retire"),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await["status"], "retired");
    let used = category("used").await;
    let r = post(
        &app,
        tenant,
        "/bss-products/v1/skus",
        json!({"code":"HELD","name":"Held","type":"recurring","category_id":used}),
    )
    .await;
    assert_eq!(r.status(), StatusCode::CREATED);
    let r = post(
        &app,
        tenant,
        &format!("/bss-products/v1/categories/{used}/retire"),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(&body_json(r).await), "CATEGORY_IN_USE");
}

/// P-D-208 (D6): retiring a category that is already retired names its own cause,
/// `CATEGORY_RETIRED`, not `CATEGORY_IN_USE`.
#[tokio::test]
async fn retiring_a_retired_category_is_category_retired() {
    let tenant = Uuid::new_v4();
    let (app, _) = rest_app(tenant, router).await;
    let c = body_json(
        post(
            &app,
            tenant,
            "/bss-products/v1/categories",
            json!({"code":"gone","name":"Gone"}),
        )
        .await,
    )
    .await;
    let url = format!(
        "/bss-products/v1/categories/{}/retire",
        c["id"].as_str().unwrap()
    );
    assert_eq!(
        post(&app, tenant, &url, json!({})).await.status(),
        StatusCode::OK
    );
    let r = post(&app, tenant, &url, json!({})).await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(&body_json(r).await), "CATEGORY_RETIRED");
}

/// P-D-208 (#9, amends P-D-186): a category is in use only while a SKU in `draft`, `published`,
/// `deprecated` or `retiring` names it; SKUs that are all `retired` no longer hold it.
#[tokio::test]
async fn only_a_sku_that_is_not_retired_keeps_a_category_in_use() {
    use crate::test_support::{id_matches, repo_connection, seed_rest_sku};
    use sea_orm::{ConnectionTrait, Database};
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, router).await;
    let (db, scope) = repo_connection(&dsn, tenant).await;
    for (n, lifecycle, expected) in [
        (0, "draft", StatusCode::CONFLICT),
        (1, "published", StatusCode::CONFLICT),
        (2, "deprecated", StatusCode::CONFLICT),
        (3, "retiring", StatusCode::CONFLICT),
        (4, "retired", StatusCode::OK),
    ] {
        let c = body_json(
            post(
                &app,
                tenant,
                "/bss-products/v1/categories",
                json!({"code":format!("c{n}"),"name":format!("C{n}")}),
            )
            .await,
        )
        .await;
        let id: Uuid = serde_json::from_value(c["id"].clone()).unwrap();
        let retired =
            seed_rest_sku(&db.conn().unwrap(), &scope, tenant, id, &format!("R{n}")).await;
        let held = seed_rest_sku(&db.conn().unwrap(), &scope, tenant, id, &format!("H{n}")).await;
        let raw = Database::connect(&dsn).await.unwrap();
        raw.execute_unprepared(&format!(
            "UPDATE products_sku SET lifecycle = 'retired' WHERE {}",
            id_matches("id", retired.id)
        ))
        .await
        .unwrap();
        raw.execute_unprepared(&format!(
            "UPDATE products_sku SET lifecycle = '{lifecycle}' WHERE {}",
            id_matches("id", held.id)
        ))
        .await
        .unwrap();
        raw.close().await.unwrap();
        let r = post(
            &app,
            tenant,
            &format!("/bss-products/v1/categories/{id}/retire"),
            json!({}),
        )
        .await;
        assert_eq!(
            r.status(),
            expected,
            "a {lifecycle} SKU beside a retired one"
        );
        let b = body_json(r).await;
        if expected == StatusCode::OK {
            assert_eq!(b["status"], "retired");
        } else {
            assert_eq!(problem_code(&b), "CATEGORY_IN_USE", "{lifecycle}");
        }
    }
}
