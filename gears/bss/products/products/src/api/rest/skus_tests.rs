#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::router;
use crate::api::rest::{ApiState, categories};
use crate::domain::{recognized::UsageTypeAnswer, references::RefKind};
use crate::infra::storage::repo;
use crate::test_support::{
    StubUsageTypes, at, body_json, get, patch, post, problem_code, raw_i64, repo_connection,
    rest_app, rest_app_with_catalog, violation_for,
};
use axum::{Router, http::StatusCode};
use bss_products_sdk::models::{Lifecycle, SkuContent};
use serde_json::{Value, json};
use std::sync::Arc;
use toolkit::api::OpenApiRegistry;
use uuid::Uuid;

fn doors(s: Arc<ApiState>, o: &dyn OpenApiRegistry) -> Router {
    categories::router(Arc::clone(&s), o).merge(router(s, o))
}
async fn category(app: &Router, tenant: Uuid) -> Uuid {
    let r = post(
        app,
        tenant,
        "/bss-products/v1/categories",
        json!({"code":"hosting","name":"Hosting"}),
    )
    .await;
    assert_eq!(r.status(), StatusCode::CREATED);
    serde_json::from_value(body_json(r).await["id"].clone()).unwrap()
}
fn new(cat: Uuid, code: &str, name: &str) -> Value {
    json!({"code":code,"name":name,"type":"usage","category_id":cat})
}

#[tokio::test]
async fn create_read_patch_and_duplicates_keep_etags_and_codes() {
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, doors).await;
    let cat = category(&app, tenant).await;
    let r = post(
        &app,
        tenant,
        "/bss-products/v1/skus",
        new(cat, "STOR", "Storage"),
    )
    .await;
    assert_eq!(r.status(), StatusCode::CREATED);
    let tag = r.headers()["etag"].to_str().unwrap().to_owned();
    let s = body_json(r).await;
    assert_eq!(s["lifecycle"], "draft");
    assert_eq!(s["type"], "usage");
    assert!(s["created_at"].as_str().unwrap().contains('T'));
    let url = format!("/bss-products/v1/skus/{}", s["id"].as_str().unwrap());
    let r = get(&app, tenant, &url).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["etag"], tag);
    assert_eq!(body_json(r).await["sku"], s);
    for (code, name, reason) in [
        ("STOR", "Other", "SKU_CODE_TAKEN"),
        ("OTHER", "Storage", "SKU_NAME_TAKEN"),
    ] {
        let r = post(&app, tenant, "/bss-products/v1/skus", new(cat, code, name)).await;
        assert_eq!(r.status(), StatusCode::CONFLICT);
        assert_eq!(problem_code(&body_json(r).await), reason);
    }
    let r = patch(&app, tenant, &url, json!({"name":"Renamed"}), None).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(violation_for(&body_json(r).await, "If-Match").is_some());
    let r = patch(
        &app,
        tenant,
        &url,
        json!({"name":"Renamed"}),
        Some("\"99\""),
    )
    .await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(&body_json(r).await), "STALE_REVISION");
    let r = patch(
        &app,
        tenant,
        &url,
        json!({"type":"recurring","gl_code":"4010","billing_timing":"advance"}),
        Some(&tag),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let tag2 = r.headers()["etag"].to_str().unwrap().to_owned();
    let s = body_json(r).await;
    assert_eq!(s["type"], "recurring");
    assert_eq!(s["type_change_pending"], false);
    assert_eq!(s["billing_timing"], "advance");
    let r = patch(
        &app,
        tenant,
        &url,
        json!({"gl_code":null,"billing_timing":null}),
        Some(&tag2),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let s = body_json(r).await;
    assert_eq!(s["gl_code"], Value::Null);
    assert_eq!(s["billing_timing"], Value::Null);
    assert_eq!(
        raw_i64(
            &dsn,
            "SELECT COUNT(*) AS v FROM products_audit_log WHERE action LIKE 'sku.%'"
        )
        .await,
        3
    );
    assert_eq!(
        get(&app, Uuid::new_v4(), &url).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn retired_missing_and_foreign_categories_refuse_assignment() {
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, doors).await;
    let cat = category(&app, tenant).await;
    assert_eq!(
        post(
            &app,
            tenant,
            &format!("/bss-products/v1/categories/{cat}/retire"),
            json!({})
        )
        .await
        .status(),
        StatusCode::OK
    );
    let r = post(&app, tenant, "/bss-products/v1/skus", new(cat, "A", "A")).await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(&body_json(r).await), "CATEGORY_RETIRED");
    assert_eq!(
        post(
            &app,
            tenant,
            "/bss-products/v1/skus",
            new(Uuid::new_v4(), "A", "A")
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let foreign = Uuid::new_v4();
    let (db, scope) = repo_connection(&dsn, foreign).await;
    let foreign_cat = repo::insert_category(
        &db.conn().unwrap(),
        &scope,
        foreign,
        crate::domain::category::NewCategory {
            code: "foreign".into(),
            name: "Foreign".into(),
            is_default: false,
            sort_order: 0,
        },
        at(9),
    )
    .await
    .unwrap();
    assert_eq!(
        post(
            &app,
            tenant,
            "/bss-products/v1/skus",
            new(foreign_cat.id, "A", "A")
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn published_and_locked_drafts_refuse_direct_edits() {
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, doors).await;
    let cat = category(&app, tenant).await;
    let (db, scope) = repo_connection(&dsn, tenant).await;
    let conn = db.conn().unwrap();
    let s = crate::test_support::seed_rest_sku(&conn, &scope, tenant, cat, "A").await;
    repo::set_lifecycle(
        &conn,
        &scope,
        tenant,
        s.id,
        &[Lifecycle::Draft],
        Lifecycle::Published,
        at(9),
    )
    .await
    .unwrap();
    let r = patch(
        &app,
        tenant,
        &format!("/bss-products/v1/skus/{}", s.id),
        json!({"name":"changed"}),
        Some("\"2\""),
    )
    .await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let b = body_json(r).await;
    assert_eq!(problem_code(&b), "NOT_A_DRAFT");
    assert!(b.to_string().contains("/changes"));
    let s = crate::test_support::seed_rest_sku(&conn, &scope, tenant, cat, "B").await;
    assert!(
        repo::try_lock_sku(&conn, &scope, tenant, s.id, Uuid::new_v4(), s.revision)
            .await
            .unwrap()
    );
    let r = patch(
        &app,
        tenant,
        &format!("/bss-products/v1/skus/{}", s.id),
        json!({"name":"changed"}),
        Some("\"1\""),
    )
    .await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(&body_json(r).await), "ROW_LOCKED_PENDING");
}

#[tokio::test]
async fn card_and_reference_details_read_the_live_registry() {
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, doors).await;
    let cat = category(&app, tenant).await;
    let (db, scope) = repo_connection(&dsn, tenant).await;
    let conn = db.conn().unwrap();
    let s = crate::test_support::seed_rest_sku(&conn, &scope, tenant, cat, "A").await;
    repo::set_lifecycle(
        &conn,
        &scope,
        tenant,
        s.id,
        &[Lifecycle::Draft],
        Lifecycle::Published,
        at(9),
    )
    .await
    .unwrap();
    for (i, kind) in [RefKind::Price, RefKind::Price, RefKind::PlanItem]
        .into_iter()
        .enumerate()
    {
        let r = repo::reserve_reference(
            &conn,
            &scope,
            tenant,
            s.id,
            "pricing",
            kind,
            Uuid::new_v4(),
            tenant,
            at(9),
        )
        .await
        .unwrap();
        if i < 2 {
            repo::confirm_reference(&conn, &scope, tenant, r.id, at(10))
                .await
                .unwrap();
        }
    }
    let released = repo::reserve_reference(
        &conn,
        &scope,
        tenant,
        s.id,
        "pricing",
        RefKind::SoldAs,
        Uuid::new_v4(),
        tenant,
        at(9),
    )
    .await
    .unwrap();
    repo::release_reference(
        &conn,
        &scope,
        tenant,
        released.id,
        tenant,
        Some("abandoned attempt"),
        true,
        at(11),
    )
    .await
    .unwrap();
    let url = format!("/bss-products/v1/skus/{}", s.id);
    let card = body_json(get(&app, tenant, &url).await).await;
    assert_eq!(
        card["references"],
        json!({"prices":2,"plans":1,"reserved":1,"by_owner":{"pricing":{"price":2,"plan_item":1,"reserved":1}}})
    );
    let refs = body_json(get(&app, tenant, &format!("{url}/references")).await).await;
    assert_eq!(refs["summary"], card["references"]);
    assert_eq!(refs["items"].as_array().unwrap().len(), 3);
    for item in refs["items"].as_array().unwrap() {
        assert_eq!(item["owner"], "pricing");
        assert!(item["reserved_at"].as_str().unwrap().contains('T'));
    }
    assert_eq!(
        get(&app, Uuid::new_v4(), &format!("{url}/references"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let history_url = format!("{url}/references?include_released=true");
    let history = body_json(get(&app, tenant, &history_url).await).await;
    assert_eq!(history["summary"], card["references"]);
    assert_eq!(history["items"].as_array().unwrap().len(), 4);
    let row = history["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == released.id.to_string())
        .unwrap();
    assert_eq!(row["state"], "released");
    assert_eq!(row["released_by"], tenant.to_string());
    assert_eq!(row["release_reason"], "abandoned attempt");
    assert_eq!(row["forced"], true);
    assert!(row["released_at"].as_str().unwrap().contains('T'));
    assert_eq!(
        get(&app, Uuid::new_v4(), &history_url).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(
            &app,
            tenant,
            &format!("{url}/references?include_released=invalid")
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let live = body_json(
        get(
            &app,
            tenant,
            &format!("{url}/references?include_released=false"),
        )
        .await,
    )
    .await;
    assert_eq!(live["items"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn version_dates_resolve_as_of_and_before_the_first_is_named_404() {
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, doors).await;
    let cat = category(&app, tenant).await;
    let (db, scope) = repo_connection(&dsn, tenant).await;
    let conn = db.conn().unwrap();
    let s = crate::test_support::seed_rest_sku(&conn, &scope, tenant, cat, "A").await;
    let mut content = SkuContent::from(&s);
    for (v, day) in [(1, 2), (2, 20), (3, 20)] {
        content.gl_code = Some(format!("40{v}"));
        repo::append_version(
            &conn,
            &scope,
            tenant,
            s.id,
            v,
            crate::test_support::utc(2026, 9, day, 0, 0, 0).date(),
            &content,
            at(9),
        )
        .await
        .unwrap();
    }
    let url = format!("/bss-products/v1/skus/{}/versions", s.id);
    let r = get(&app, tenant, &format!("{url}?as_of=2026-09-15")).await;
    assert_eq!(r.status(), StatusCode::OK);
    let v = body_json(r).await;
    assert_eq!(v["published_version"], 1);
    assert_eq!(v["effective_from"], "2026-09-02");
    assert_eq!(v["content"]["gl_code"], "401");
    assert_eq!(
        body_json(get(&app, tenant, &format!("{url}?as_of=2026-09-20")).await).await["published_version"],
        3
    );
    assert_eq!(
        body_json(get(&app, tenant, &url).await)
            .await
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let r = get(&app, tenant, &format!("{url}?as_of=2026-09-01")).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert_eq!(problem_code(&body_json(r).await), "NO_VERSION_IN_FORCE");
    assert_eq!(
        get(&app, tenant, &format!("{url}?as_of=bad"))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&app, Uuid::new_v4(), &url).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn filtered_code_cursor_never_skips_the_first_row_of_the_next_page() {
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, doors).await;
    let cat = category(&app, tenant).await;
    let (db, scope) = repo_connection(&dsn, tenant).await;
    let conn = db.conn().unwrap();
    for code in ["stor-c", "stor-a", "stor-b", "other"] {
        let s = crate::test_support::seed_rest_sku(&conn, &scope, tenant, cat, code).await;
        repo::set_lifecycle(
            &conn,
            &scope,
            tenant,
            s.id,
            &[Lifecycle::Draft],
            Lifecycle::Published,
            at(9),
        )
        .await
        .unwrap();
    }
    let query = format!(
        "/bss-products/v1/skus?q=stor&type=usage&lifecycle=published&category={cat}&limit=2"
    );
    let page = body_json(get(&app, tenant, &query).await).await;
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    assert_eq!(page["items"][0]["code"], "stor-a");
    assert_eq!(page["items"][1]["code"], "stor-b");
    let next = page["next"].as_str().unwrap();
    let page = body_json(get(&app, tenant, &format!("{query}&after={next}")).await).await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["code"], "stor-c");
    assert_eq!(page["next"], Value::Null);
}

#[tokio::test]
async fn draft_usage_resolution_allows_silence_but_refuses_definite_unknowns() {
    for (answer, source, expected) in [
        (UsageTypeAnswer::Unavailable, "test", StatusCode::CREATED),
        (UsageTypeAnswer::Unresolved, "test", StatusCode::BAD_REQUEST),
        (
            UsageTypeAnswer::Unresolved,
            "unconfigured",
            StatusCode::CREATED,
        ),
    ] {
        let tenant = Uuid::new_v4();
        let catalog = Arc::new(StubUsageTypes::always(answer));
        let (app, dsn) = rest_app_with_catalog(tenant, doors, catalog.clone(), source).await;
        let cat = category(&app, tenant).await;
        let mut body = new(cat, "A", "A");
        body["usage_type_ref"] = json!("usage:new");
        let r = post(&app, tenant, "/bss-products/v1/skus", body).await;
        assert_eq!(r.status(), expected);
        if expected == StatusCode::BAD_REQUEST {
            let body = body_json(r).await;
            assert_eq!(problem_code(&body), "USAGE_TYPE_UNRESOLVED");
            assert!(violation_for(&body, "usage_type_ref").is_some());
        }
        let s =
            body_json(post(&app, tenant, "/bss-products/v1/skus", new(cat, "B", "B")).await).await;
        let url = format!("/bss-products/v1/skus/{}", s["id"].as_str().unwrap());
        let r = patch(
            &app,
            tenant,
            &url,
            json!({"usage_type_ref":"usage:new"}),
            Some("\"1\""),
        )
        .await;
        assert_eq!(
            r.status(),
            if expected == StatusCode::BAD_REQUEST {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::OK
            }
        );
        assert_eq!(
            raw_i64(&dsn, "SELECT COUNT(*) AS v FROM products_sku").await,
            if expected == StatusCode::BAD_REQUEST {
                1
            } else {
                2
            }
        );
        if source == "unconfigured" {
            assert_eq!(catalog.asked.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn invalid_enum_fields_and_lifecycle_edits_are_400_and_audit_failure_rolls_back() {
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, doors).await;
    let cat = category(&app, tenant).await;
    let s = body_json(post(&app, tenant, "/bss-products/v1/skus", new(cat, "A", "A")).await).await;
    let url = format!("/bss-products/v1/skus/{}", s["id"].as_str().unwrap());
    for field in ["type", "lifecycle", "billing_timing"] {
        let r = patch(&app, tenant, &url, json!({field:"bad"}), Some("\"1\"")).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        assert!(violation_for(&body_json(r).await, field).is_some());
    }
    let r = patch(
        &app,
        tenant,
        &url,
        json!({"lifecycle":"published"}),
        Some("\"1\""),
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(violation_for(&body_json(r).await, "lifecycle").is_some());
    assert_eq!(
        get(&app, tenant, "/bss-products/v1/skus?type=bad")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&app, tenant, "/bss-products/v1/skus?limit=0")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let r = post(&app, tenant, "/bss-products/v1/skus", json!({"type":42})).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    crate::test_support::drop_table(&dsn, "products_audit_log").await;
    let r = patch(
        &app,
        tenant,
        &url,
        json!({"name":"rollback"}),
        Some("\"1\""),
    )
    .await;
    assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body_json(get(&app, tenant, &url).await).await["sku"]["name"],
        "A"
    );
}

#[tokio::test]
async fn unchanged_meter_is_not_resolved_again_and_clearing_it_does_not_resolve() {
    let tenant = Uuid::new_v4();
    let catalog = Arc::new(StubUsageTypes::scripted([
        UsageTypeAnswer::Resolved(crate::test_support::probe_binding()),
        UsageTypeAnswer::Unresolved,
    ]));
    let (app, _) = rest_app_with_catalog(tenant, doors, catalog.clone(), "test").await;
    let cat = category(&app, tenant).await;
    let mut body = new(cat, "A", "A");
    body["usage_type_ref"] = json!("usage:storage");
    let r = post(&app, tenant, "/bss-products/v1/skus", body).await;
    assert_eq!(r.status(), StatusCode::CREATED);
    let s = body_json(r).await;
    let url = format!("/bss-products/v1/skus/{}", s["id"].as_str().unwrap());
    for (version, body) in [
        (1, json!({"usage_type_ref":"usage:storage"})),
        (2, json!({"description":"changed"})),
        (3, json!({"usage_type_ref":null})),
    ] {
        let r = patch(&app, tenant, &url, body, Some(&format!("\"{version}\""))).await;
        assert_eq!(r.status(), StatusCode::OK);
    }
    assert_eq!(catalog.asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    let r = patch(
        &app,
        tenant,
        &url,
        json!({"usage_type_ref":"usage:new"}),
        Some("\"4\""),
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(&body_json(r).await), "USAGE_TYPE_UNRESOLVED");
    assert_eq!(catalog.asked.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test]
async fn reassignment_and_rename_apply_the_same_category_and_uniqueness_guards() {
    let tenant = Uuid::new_v4();
    let (app, dsn) = rest_app(tenant, doors).await;
    let cat = category(&app, tenant).await;
    let first =
        body_json(post(&app, tenant, "/bss-products/v1/skus", new(cat, "A", "A")).await).await;
    post(&app, tenant, "/bss-products/v1/skus", new(cat, "B", "B")).await;
    let other = body_json(
        post(
            &app,
            tenant,
            "/bss-products/v1/categories",
            json!({"code":"other", "name":"Other"}),
        )
        .await,
    )
    .await;
    let other_id = other["id"].as_str().unwrap();
    assert_eq!(
        post(
            &app,
            tenant,
            &format!("/bss-products/v1/categories/{other_id}/retire"),
            json!({})
        )
        .await
        .status(),
        StatusCode::OK
    );
    let url = format!("/bss-products/v1/skus/{}", first["id"].as_str().unwrap());
    for (body, status, code) in [
        (
            json!({"category_id":other_id}),
            StatusCode::CONFLICT,
            Some("CATEGORY_RETIRED"),
        ),
        (
            json!({"category_id":Uuid::new_v4()}),
            StatusCode::NOT_FOUND,
            None,
        ),
        (
            json!({"name":"B"}),
            StatusCode::CONFLICT,
            Some("SKU_NAME_TAKEN"),
        ),
    ] {
        let r = patch(&app, tenant, &url, body, Some("\"1\"")).await;
        assert_eq!(r.status(), status);
        if let Some(code) = code {
            assert_eq!(problem_code(&body_json(r).await), code);
        }
    }
    let current = body_json(get(&app, tenant, &url).await).await;
    assert_eq!(current["sku"]["revision"], 1);
    assert_eq!(current["sku"]["category_id"], json!(cat));
    assert_eq!(
        raw_i64(
            &dsn,
            "SELECT COUNT(*) AS v FROM products_audit_log WHERE action = 'sku.draft_update'"
        )
        .await,
        0
    );
}
