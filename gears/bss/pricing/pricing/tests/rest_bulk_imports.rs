//! The bulk import's three surfaces, over HTTP
//! (`design/12-operator-efficiency.md` §5, `inst-bi-api`, `inst-bi-return`,
//! `inst-bk-idem`, `inst-bs-abort`; D-293).
//!
//! Every case asserts what the **run** holds as well as what the response said:
//! `inst-bi-return` makes the response an operation ref and the `GET` the place
//! the answer lives, so a suite that only read responses would be testing the
//! smaller half of the contract.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::bulk_imports::BULK_IMPORTS;
use rest_support::{Harness, body_json, problem_code, seed_current_plan, with_headers};
use uuid::Uuid;

fn keyed(key: &str) -> Vec<(&str, &str)> {
    vec![("idempotency-key", key)]
}

fn import_path(operation_id: &str) -> String {
    format!("{BULK_IMPORTS}/{operation_id}")
}

fn abort_path(operation_id: &str) -> String {
    format!("{BULK_IMPORTS}/{operation_id}/abort")
}

/// One row on a named region of a named plan.
fn row(plan_id: Uuid, region: &str, amount: i64) -> serde_json::Value {
    serde_json::json!({
        "plan_id": plan_id,
        "scope_key": {
            "currency": "USD",
            "region": region,
            "phase": rest_support::seeded_phase().get().to_string(),
            "price_eligibility": "all_subscriptions",
            "charge_kind": "recurring",
            "cohort": serde_json::Value::Null
        },
        "content": {
            "model_kind": "flat",
            "amount_minor": amount,
            "tax_inclusive": false
        }
    })
}

fn batch(rows: &[serde_json::Value]) -> serde_json::Value {
    serde_json::json!({ "rows": rows })
}

#[tokio::test]
async fn a_batch_of_new_keys_is_accepted_and_the_run_reports_what_landed() {
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "eu", 1_500), row(plan, "us", 2_500)])),
            &keyed("bulk-1"),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::ACCEPTED, "inst-bi-return");
    let body = body_json(response).await;
    assert_eq!(body["state"], serde_json::json!("completed"));
    let operation_id = body["operation_id"].as_str().expect("the ref").to_owned();

    // The answer lives at the GET, for the first caller and every replay alike.
    let read = harness
        .allowed()
        .send(with_headers("GET", &import_path(&operation_id), None, &[]))
        .await;
    assert_eq!(read.status(), StatusCode::OK);
    let run = body_json(read).await;
    assert_eq!(
        run["report"]["committed"]
            .as_array()
            .expect("an array")
            .len(),
        2,
        "both rows landed and the run says so: {run}"
    );
}

#[tokio::test]
async fn a_duplicate_inside_the_batch_blocks_it_and_commits_nothing() {
    // Phase 1's all-or-nothing posture, over HTTP. The refusal is a 400 carrying
    // its code, not a 422: Foundation section 3.3 gives the platform no 422
    // category, and the per-row report is at the GET.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "eu", 1_500), row(plan, "eu", 2_500)])),
            &keyed("bulk-2"),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        problem_code(response)
            .await
            .contains("BULK_VALIDATION_FAILED"),
        "the batch-level code names what happened"
    );
}

#[tokio::test]
async fn a_replayed_key_answers_the_run_it_opened_and_imports_nothing_twice() {
    // `inst-bk-idem`. The run's own unique `(tenant, client_key)` is the record,
    // not a dedup row with a TTL — which is what lets the replay work "during and
    // after the lock window".
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;

    let first = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "eu", 1_500)])),
            &keyed("bulk-3"),
        ))
        .await;
    let first_id = body_json(first).await["operation_id"].clone();

    let replay = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "us", 9_900)])),
            &keyed("bulk-3"),
        ))
        .await;

    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    let replayed = body_json(replay).await;
    assert_eq!(
        replayed["operation_id"], first_id,
        "a replay answers the run the key opened"
    );
    assert_eq!(
        replayed["report"]["committed"]
            .as_array()
            .expect("an array")
            .len(),
        1,
        "and the second body imported nothing: {replayed}"
    );
}

#[tokio::test]
async fn a_submit_without_an_idempotency_key_is_refused() {
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "eu", 1_500)])),
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_absent_run_reads_as_absent() {
    let harness = Harness::new().await;
    let response = harness
        .allowed()
        .send(with_headers(
            "GET",
            &import_path(&Uuid::now_v7().to_string()),
            None,
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn aborting_a_finished_run_is_refused_by_the_state_machine() {
    // Abort is an edge out of `committing` and nothing else. A run that already
    // completed has no lock to clear and no work to stop, so the refusal comes
    // from the trigger rather than from a check written here.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;

    let submitted = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "eu", 1_500)])),
            &keyed("bulk-4"),
        ))
        .await;
    let operation_id = body_json(submitted).await["operation_id"]
        .as_str()
        .expect("the ref")
        .to_owned();

    let aborted = harness
        .allowed()
        .send(with_headers(
            "POST",
            &abort_path(&operation_id),
            None,
            &keyed("abort-1"),
        ))
        .await;

    assert_ne!(
        aborted.status(),
        StatusCode::OK,
        "a completed run is not abortable"
    );
    let still = harness
        .allowed()
        .send(with_headers("GET", &import_path(&operation_id), None, &[]))
        .await;
    assert_eq!(
        body_json(still).await["state"],
        serde_json::json!("completed"),
        "and the refusal left it where it was"
    );
}

#[tokio::test]
async fn an_abort_without_an_idempotency_key_is_refused() {
    let harness = Harness::new().await;
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &abort_path(&Uuid::now_v7().to_string()),
            None,
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
