//! Explicit draft-window authoring and Working reads (D-374).
//!
//! Live mutations stay in `rest_windows.rs`. This binary owns the draft door:
//! tagged `context`, plan ETags, Working reads, recovery, and replay after the
//! owner leaves `draft`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::windows::{
    DRAFT_WINDOW_BASELINE_REFRESH, PLAN_COVERAGE, PRICE_WINDOW, PRICE_WINDOWS_LIST,
};
use rest_support::{
    Harness, body_json, seed_draft_plan, seed_price, seed_publishable_plan, seed_window,
    with_headers,
};
use uuid::Uuid;

#[tokio::test]
async fn first_revision_accepts_a_symbolic_window() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let etag = h.plan_etag(plan).await;
    let response = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-first"),
            ],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_json(response).await;
    assert_eq!(body["state"], "draft");
    assert_eq!(body["start"]["kind"], "at_publish");
}

#[tokio::test]
async fn missing_context_is_a_malformed_request() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let response = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[("idempotency-key", "draft-window-missing-context")],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_draft_schedule_without_if_match_is_refused() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let response = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[("idempotency-key", "draft-window-missing-etag")],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let problem = body_json(response).await;
    assert!(
        problem["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("If-Match")),
        "the caller is told which header is missing: {problem}"
    );
}

#[tokio::test]
async fn a_stale_plan_etag_is_refused_as_stale_version() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let etag = h.plan_etag(plan).await;
    let first = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-stale-1"),
            ],
        ))
        .await;
    assert_eq!(first.status(), StatusCode::CREATED);

    let stale = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at", "at": "2099-06-01T00:00:00Z"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-stale-2"),
            ],
        ))
        .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let body = body_json(stale).await;
    assert_eq!(body["context"]["reason"], "STALE_VERSION");
}

#[tokio::test]
async fn a_foreign_tenant_cannot_author_a_draft_window() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let etag = h.plan_etag(plan).await;
    let posted = h
        .other_tenant()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-foreign"),
            ],
        ))
        .await;
    assert_eq!(posted.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn working_reads_on_a_published_plan_name_a_changed_context() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    let seeded = seed_publishable_plan(&h, plan).await;
    h.publish(plan, 0).await;
    h.publish_price(plan, seeded.price_id).await;

    let listed = h
        .allowed()
        .send(rest_support::request(
            "GET",
            &format!("{PRICE_WINDOWS_LIST}?view=working&plan_id={plan}&plan_revision=0"),
            None,
        ))
        .await;
    assert_eq!(listed.status(), StatusCode::CONFLICT);
    let body = body_json(listed).await;
    assert_eq!(body["context"]["reason"], "DRAFT_WINDOW_CONTEXT_CHANGED");
}

#[tokio::test]
async fn committed_collection_reads_do_not_carry_draft_rows() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let etag = h.plan_etag(plan).await;
    let created = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-committed-hide"),
            ],
        ))
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let window_id = body_json(created).await["window_id"]
        .as_str()
        .expect("draft create names the window")
        .to_owned();

    let committed = h
        .allowed()
        .send(rest_support::request("GET", PRICE_WINDOWS_LIST, None))
        .await;
    assert_eq!(committed.status(), StatusCode::OK);
    let page = body_json(committed).await;
    let items = page["items"].as_array().expect("a page carries items");
    assert!(
        items
            .iter()
            .all(|item| item["window_id"].as_str() != Some(&window_id)),
        "committed collection leaked a draft window: {page}"
    );
}

#[tokio::test]
async fn an_empty_draft_working_coverage_reports_the_key_uncovered() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let _row = seed_price(&h, plan, "eu").await;
    let report = h
        .allowed()
        .send(rest_support::request(
            "GET",
            &format!(
                "{}/{plan}/coverage?view=working&plan_revision=0",
                PLAN_COVERAGE.replace("/{planId}/coverage", "")
            ),
            None,
        ))
        .await;
    assert_eq!(report.status(), StatusCode::OK);
    let body = body_json(report).await;
    let keys = body["keys"].as_array().expect("coverage names keys");
    assert_eq!(keys.len(), 1, "{body}");
    assert_eq!(keys[0]["covered"], false);
    assert_eq!(keys[0]["coverage_end"]["kind"], "uncovered");
}

#[tokio::test]
async fn replay_after_the_owner_leaves_draft_returns_the_stored_answer() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let etag = h.plan_etag(plan).await;
    let first = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-replay"),
            ],
        ))
        .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    h.abandon_draft(plan, 0).await;

    let replay = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-replay"),
            ],
        ))
        .await;
    assert_eq!(replay.status(), StatusCode::CREATED);
    let body = body_json(replay).await;
    assert_eq!(body["state"], "draft");
    assert_eq!(body["start"]["kind"], "at_publish");
}

#[tokio::test]
async fn working_cursors_are_bound_to_the_named_parent() {
    let h = Harness::new().await;
    let plan_a = Uuid::now_v7();
    let plan_b = Uuid::now_v7();
    seed_draft_plan(&h, plan_a).await;
    seed_draft_plan(&h, plan_b).await;
    let first = seed_price(&h, plan_a, "eu").await;
    let second = seed_price(&h, plan_a, "us").await;
    let etag = h.plan_etag(plan_a).await;
    let created = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", first.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-page-1"),
            ],
        ))
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let etag = h.plan_etag(plan_a).await;
    let created = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", second.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-page-2"),
            ],
        ))
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);

    let page = h
        .allowed()
        .send(rest_support::request(
            "GET",
            &format!("{PRICE_WINDOWS_LIST}?view=working&plan_id={plan_a}&plan_revision=0&limit=1"),
            None,
        ))
        .await;
    assert_eq!(page.status(), StatusCode::OK);
    let body = body_json(page).await;
    let cursor = body["page_info"]["next_cursor"]
        .as_str()
        .expect("limit=1 over two windows yields a resume cursor")
        .to_owned();
    let crossed = h
        .allowed()
        .send(rest_support::request(
            "GET",
            &format!(
                "{PRICE_WINDOWS_LIST}?view=working&plan_id={plan_b}&plan_revision=0&cursor={cursor}"
            ),
            None,
        ))
        .await;
    assert_eq!(crossed.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn baseline_refresh_discards_operations_the_new_baseline_cannot_carry() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let etag = h.plan_etag(plan).await;
    let created = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-refresh-create"),
            ],
        ))
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_body = body_json(created).await;
    let operation_id = created_body["operation_id"]
        .as_str()
        .expect("create names its operation")
        .to_owned();
    seed_window(&h, row.price_id).await;

    let etag = h.plan_etag(plan).await;
    let refresh = h
        .allowed()
        .send(with_headers(
            "POST",
            &DRAFT_WINDOW_BASELINE_REFRESH.replace("{planId}", &plan.to_string()),
            Some(serde_json::json!({ "plan_revision": 0 })),
            &[
                ("if-match", etag.as_str()),
                ("idempotency-key", "draft-window-refresh"),
            ],
        ))
        .await;
    assert_eq!(refresh.status(), StatusCode::OK);
    let body = body_json(refresh).await;
    let discarded = body["discarded_operation_ids"]
        .as_array()
        .expect("refresh names the operations it dropped");
    assert!(
        discarded
            .iter()
            .any(|id| id.as_str() == Some(operation_id.as_str())),
        "the overlapping create must be discarded against the recaptured live window: {body}"
    );
}

#[tokio::test]
async fn a_draft_create_can_be_patched_and_keeps_its_price() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let etag = h.plan_etag(plan).await;
    let created = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-patch-create"),
            ],
        ))
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_body = body_json(created).await;
    let window_id = created_body["window_id"]
        .as_str()
        .expect("create names the window")
        .to_owned();
    let etag = h.plan_etag(plan).await;
    let patched = h
        .allowed()
        .send(with_headers(
            "PATCH",
            &PRICE_WINDOW.replace("{windowId}", &window_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "effective_to": "2099-06-01T00:00:00Z"
            })),
            &[
                ("if-match", etag.as_str()),
                ("idempotency-key", "draft-window-patch"),
            ],
        ))
        .await;
    assert_eq!(patched.status(), StatusCode::OK);
    let body = body_json(patched).await;
    assert_eq!(body["price_id"], row.price_id.to_string());
    assert_eq!(body["start"]["kind"], "at_publish");
    assert_eq!(body["effective_to"], "2099-06-01T00:00:00.000000Z");
}

#[tokio::test]
async fn deleting_a_draft_create_removes_the_staged_operation() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let etag = h.plan_etag(plan).await;
    let created = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", row.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", &etag),
                ("idempotency-key", "draft-window-cancel-create"),
            ],
        ))
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_body = body_json(created).await;
    let window_id = created_body["window_id"]
        .as_str()
        .expect("create names the window")
        .to_owned();
    let operation_id = created_body["operation_id"]
        .as_str()
        .expect("create names its operation")
        .to_owned();
    let etag = h.plan_etag(plan).await;
    let deleted = h
        .allowed()
        .send(with_headers(
            "DELETE",
            &format!(
                "{}?context=draft&plan_id={plan}&plan_revision=0",
                PRICE_WINDOW.replace("{windowId}", &window_id)
            ),
            None,
            &[
                ("if-match", etag.as_str()),
                ("idempotency-key", "draft-window-cancel"),
            ],
        ))
        .await;
    assert_eq!(deleted.status(), StatusCode::OK);
    let body = body_json(deleted).await;
    assert_eq!(body["operation_id"], operation_id);
    assert_eq!(body["price_id"], row.price_id.to_string());
    assert_eq!(body["start"]["kind"], "at_publish");

    let working = h
        .allowed()
        .send(rest_support::request(
            "GET",
            &format!("{PRICE_WINDOWS_LIST}?view=working&plan_id={plan}&plan_revision=0"),
            None,
        ))
        .await;
    assert_eq!(working.status(), StatusCode::OK);
    let page = body_json(working).await;
    let items = page["items"].as_array().expect("a page carries items");
    assert!(
        items
            .iter()
            .all(|item| item["window_id"].as_str() != Some(window_id.as_str())),
        "Create→Remove must drop the staged window from Working: {page}"
    );
}

#[tokio::test]
async fn a_captured_baseline_window_can_be_adjusted_on_a_draft() {
    let h = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_draft_plan(&h, plan).await;
    let row = seed_price(&h, plan, "eu").await;
    let window_id = seed_window(&h, row.price_id).await;
    let etag = h.plan_etag(plan).await;
    let refresh = h
        .allowed()
        .send(with_headers(
            "POST",
            &DRAFT_WINDOW_BASELINE_REFRESH.replace("{planId}", &plan.to_string()),
            Some(serde_json::json!({ "plan_revision": 0 })),
            &[
                ("if-match", etag.as_str()),
                ("idempotency-key", "draft-window-baseline-capture"),
            ],
        ))
        .await;
    assert_eq!(refresh.status(), StatusCode::OK);

    let etag = h.plan_etag(plan).await;
    let patched = h
        .allowed()
        .send(with_headers(
            "PATCH",
            &PRICE_WINDOW.replace("{windowId}", &window_id.to_string()),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "effective_to": "2099-06-01T00:00:00Z"
            })),
            &[
                ("if-match", etag.as_str()),
                ("idempotency-key", "draft-window-baseline-adjust"),
            ],
        ))
        .await;
    assert_eq!(patched.status(), StatusCode::OK);
    let body = body_json(patched).await;
    assert_eq!(body["price_id"], row.price_id.to_string());
    assert_eq!(body["start"]["kind"], "at");
    assert_eq!(body["start"]["at"], "2099-01-01T00:00:00.000000Z");
    assert_eq!(body["effective_to"], "2099-06-01T00:00:00.000000Z");
}

#[tokio::test]
async fn a_draft_window_cannot_be_rebound_onto_another_plans_price() {
    let h = Harness::new().await;
    let plan_a = Uuid::now_v7();
    let plan_b = Uuid::now_v7();
    seed_draft_plan(&h, plan_a).await;
    seed_draft_plan(&h, plan_b).await;
    let price_a = seed_price(&h, plan_a, "eu").await;
    let price_b = seed_price(&h, plan_b, "eu").await;
    let etag_a = h.plan_etag(plan_a).await;
    let bumped = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", price_a.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", etag_a.as_str()),
                ("idempotency-key", "draft-window-rebind-bump"),
            ],
        ))
        .await;
    assert_eq!(bumped.status(), StatusCode::CREATED);
    let etag_a = h.plan_etag(plan_a).await;
    let posted = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/prices/{}/windows", price_b.price_id),
            Some(serde_json::json!({
                "context": {"kind": "draft", "plan_revision": 0},
                "start": {"kind": "at_publish"},
                "reason_code": "launch"
            })),
            &[
                ("if-match", etag_a.as_str()),
                ("idempotency-key", "draft-window-rebind"),
            ],
        ))
        .await;
    assert!(
        posted.status() == StatusCode::CONFLICT || posted.status() == StatusCode::NOT_FOUND,
        "a draft create must not land a window of plan B under plan A's tag: {}",
        posted.status()
    );
}
