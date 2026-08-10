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

    assert_eq!(
        aborted.status(),
        StatusCode::BAD_REQUEST,
        "a completed run is not abortable, and the refusal is a lifecycle conflict rather \
         than a 500 saying the store is broken"
    );
    assert_eq!(problem_code(aborted).await, "LIFECYCLE_FORBIDDEN");
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
async fn aborting_a_run_that_finished_with_conflicts_is_refused_too() {
    // **The hole the first abort case could not see** (D-294). It used a
    // `completed` run, where the state machine genuinely refuses. But a move to
    // the state a run is already IN returns early on both engines, so
    // `completed_with_conflicts` — the ordinary terminal state of any partially
    // conflicted import — would have been rewritten: a fresh `completed_at` and an
    // abort note stamped over a report where every row was attempted.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;

    // A batch with one row that cannot commit: it asserts a version over a key
    // nothing holds, so Phase 2 conflicts it and the run ends with conflicts.
    let mut conflicting = row(plan, "eu", 1_500);
    conflicting["if_match"] = serde_json::json!(7);
    let submitted = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[conflicting])),
            &keyed("bulk-5"),
        ))
        .await;
    let body = body_json(submitted).await;
    assert_eq!(
        body["state"],
        serde_json::json!("completed_with_conflicts"),
        "the fixture has to reach that state or this case tests nothing: {body}"
    );
    let operation_id = body["operation_id"].as_str().expect("the ref").to_owned();

    let aborted = harness
        .allowed()
        .send(with_headers(
            "POST",
            &abort_path(&operation_id),
            None,
            &keyed("abort-2"),
        ))
        .await;
    assert_eq!(aborted.status(), StatusCode::BAD_REQUEST);

    let still = body_json(
        harness
            .allowed()
            .send(with_headers("GET", &import_path(&operation_id), None, &[]))
            .await,
    )
    .await;
    assert!(
        still["report"].get("aborted").is_none(),
        "and no abort note was stamped over a report whose rows were all attempted: {still}"
    );
}

#[tokio::test]
async fn the_refusal_names_the_run_and_the_run_serves_the_per_row_report() {
    // **The ref is a field, not a phrase** (D-294). Phase 1's entire value is the
    // per-row report, and the only thing pointing at it is the operation ref — so
    // a ref readable only by parsing an English sentence leaves that report
    // unreachable by every client that is not a person. The **shape** the GET
    // then serves is pinned here for the first time: `inst-bk-idem` makes the
    // stored report a wire contract, and until now no case named a field in it
    // beyond `committed`, so a rename would have been invisible.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;

    let refused = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "eu", 1_500), row(plan, "eu", 2_500)])),
            &keyed("bulk-6"),
        ))
        .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let problem = body_json(refused).await;
    assert_eq!(rest_support::code_in(&problem), "BULK_VALIDATION_FAILED");
    let operation_id = rest_support::violation_for(&problem, "operation_id")
        .unwrap_or_else(|| panic!("the refusal has to name the run it opened: {problem}"));

    let run = body_json(
        harness
            .allowed()
            .send(with_headers("GET", &import_path(&operation_id), None, &[]))
            .await,
    )
    .await;
    assert_eq!(
        run["state"],
        serde_json::json!("validation_failed"),
        "the ref the refusal handed back has to be the run that was refused: {run}"
    );

    let rows = run["report"]["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("Phase 1's report is a list of rows: {run}"));
    assert!(!rows.is_empty(), "and it names the rows: {run}");
    for outcome in rows {
        assert!(
            outcome["row"].is_u64(),
            "each outcome carries its position: {outcome}"
        );
        let violations = outcome["violations"]
            .as_array()
            .unwrap_or_else(|| panic!("and every violation found against it: {outcome}"));
        assert!(!violations.is_empty(), "never an empty list: {outcome}");
        for violation in violations {
            assert!(violation["code"].is_string(), "{violation}");
            assert!(violation["detail"].is_string(), "{violation}");
        }
    }
    assert!(
        rows.iter().any(|outcome| {
            outcome["violations"].as_array().is_some_and(|found| {
                found
                    .iter()
                    .any(|v| v["code"] == serde_json::json!("DUPLICATE_SCOPE_KEY"))
            })
        }),
        "and the duplicate is what this batch was refused for: {run}"
    );
}

#[tokio::test]
async fn a_replay_of_a_refused_key_is_refused_again_and_imports_nothing() {
    // **A replay answers what the first call answered** (D-294). The refused
    // batch was a 400; a replay under the same key used to answer 202 with the
    // failed run, so a client that retried on a timeout — the exact client
    // idempotency is for — read the retry as having succeeded where the original
    // failed. The key is also **spent**, which the refusal now says: a corrected
    // batch under it imports nothing, and the third leg proves nothing landed.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;

    let refused = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "eu", 1_500), row(plan, "eu", 2_500)])),
            &keyed("bulk-7"),
        ))
        .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);

    // The same key, a **corrected** body — one row, no duplicate.
    let replayed = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "eu", 1_500)])),
            &keyed("bulk-7"),
        ))
        .await;
    assert_eq!(
        replayed.status(),
        StatusCode::BAD_REQUEST,
        "a replay may not answer 202 where the first call answered 400"
    );
    assert_eq!(problem_code(replayed).await, "BULK_VALIDATION_FAILED");

    // And it imported nothing: the same row under a *fresh* key claims the key
    // outright, which it could not do had the replay written it.
    let fresh = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "eu", 1_500)])),
            &keyed("bulk-7-corrected"),
        ))
        .await;
    assert_eq!(fresh.status(), StatusCode::ACCEPTED);
    let landed = body_json(fresh).await;
    assert_eq!(
        landed["report"]["committed"]
            .as_array()
            .expect("an array")
            .len(),
        1,
        "the corrected batch lands under its own key, so the replay wrote nothing: {landed}"
    );
}

#[tokio::test]
async fn the_commit_report_names_its_rows_by_field() {
    // The other half of the wire contract `inst-bk-idem` pins (D-294): a run that
    // committed one row and conflicted another. Both arms are asserted by field
    // name, because a report whose readers are all inside this crate today
    // becomes the operator's only record of what an import did.
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();
    seed_current_plan(&harness, plan).await;

    let mut conflicting = row(plan, "us", 2_500);
    conflicting["if_match"] = serde_json::json!(7);
    let submitted = harness
        .allowed()
        .send(with_headers(
            "POST",
            BULK_IMPORTS,
            Some(batch(&[row(plan, "eu", 1_500), conflicting])),
            &keyed("bulk-8"),
        ))
        .await;
    assert_eq!(submitted.status(), StatusCode::ACCEPTED);
    let run = body_json(submitted).await;
    assert_eq!(
        run["state"],
        serde_json::json!("completed_with_conflicts"),
        "the fixture has to reach both arms or this case pins one: {run}"
    );

    let committed = run["report"]["committed"]
        .as_array()
        .unwrap_or_else(|| panic!("the committed arm: {run}"));
    assert_eq!(committed.len(), 1, "{run}");
    assert_eq!(committed[0]["row"], serde_json::json!(0));
    assert!(
        committed[0]["price_id"].is_string(),
        "a committed row names the draft it wrote: {run}"
    );

    let conflicted = run["report"]["conflicted"]
        .as_array()
        .unwrap_or_else(|| panic!("the conflicted arm: {run}"));
    assert_eq!(conflicted.len(), 1, "{run}");
    assert_eq!(
        conflicted[0]["row"],
        serde_json::json!(1),
        "and it is the row that conflicted, not the one that landed: {run}"
    );
    let violations = conflicted[0]["violations"]
        .as_array()
        .unwrap_or_else(|| panic!("carrying its violation: {run}"));
    assert_eq!(violations.len(), 1, "{run}");
    assert!(violations[0]["code"].is_string(), "{run}");
    assert!(violations[0]["detail"].is_string(), "{run}");
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
