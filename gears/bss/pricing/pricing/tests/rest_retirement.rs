//! `POST /bss-pricing/v1/plans/{planId}/retire` over the wire
//! (`inst-rt-api`, `inst-rt-return`, `inst-re-warn`, D-109, D-128, D-182).
//!
//! `rest_cutovers`' shape. What this file adds that no other route test can is
//! the **default**: this is the one surface in the gear where omitting a body
//! field decides between reading and acting, and the two failure modes are not
//! symmetric — a caller who meant to confirm and got a preview reads it and calls
//! again, while a caller who meant to preview and got a confirm has opened an
//! approval unit over an irreversible act.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::retirement::PLAN_RETIRE;
use rest_support::{Harness, body_json, plan_state, problem_code, request, seed_publishable_plan};
use uuid::Uuid;

const SUBMITTER: Uuid = Uuid::from_u128(0x_5e_11);

fn path(plan_id: Uuid) -> String {
    PLAN_RETIRE.replace("{planId}", &plan_id.to_string())
}

fn confirm_body() -> serde_json::Value {
    serde_json::json!({ "dry_run": false })
}

fn preview_body() -> serde_json::Value {
    serde_json::json!({ "dry_run": true })
}

/// A published plan with one published price row and its coverage window.
async fn published(h: &Harness) -> Uuid {
    let plan_id = Uuid::now_v7();
    let seeded = seed_publishable_plan(h, plan_id).await;
    h.publish(plan_id, seeded.revision).await;
    h.publish_price(plan_id, seeded.price_id).await;
    plan_id
}

#[tokio::test]
async fn the_dry_run_answers_200_with_the_windows_labelled() {
    let h = Harness::new().await;
    let plan_id = published(&h).await;

    let response = h
        .allowed_as(SUBMITTER)
        .send(request("POST", &path(plan_id), Some(preview_body())))
        .await;

    // **200, not 202.** The dry-run is a read: it opens no unit, requests no
    // version and writes nothing, which is what makes it legible to the approver
    // before the decision (D-61).
    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert_eq!(view["plan_id"], plan_id.to_string());
    assert_eq!(view["revision"], 0);

    // `inst-re-warn` + `inst-re-cancelflow`: every window carries what would
    // happen to it **and why**, not a bare count.
    let windows = view["windows"].as_array().expect("windows array");
    assert!(!windows.is_empty(), "the seed must carry a window: {view}");
    for window in windows {
        assert_eq!(window["disposition"], "kept");
        assert_eq!(window["kept_reason"], "presence_unresolved");
    }
    // D-182: the lane has no client, so the preview says the keeping is for want
    // of an answer rather than because a subscriber was found.
    assert_eq!(view["presence_unresolved"], true);
    assert!(
        view["blocking_references"]
            .as_array()
            .expect("array")
            .is_empty()
    );
}

#[tokio::test]
async fn a_body_that_omits_the_flag_is_read_as_a_dry_run() {
    // The safe default, and the asymmetry above is the argument. A regression
    // here does not fail loudly — it silently turns a reader into an actor.
    let h = Harness::new().await;
    let plan_id = published(&h).await;

    let response = h
        .allowed_as(SUBMITTER)
        .send(request("POST", &path(plan_id), Some(serde_json::json!({}))))
        .await;

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "an omitted dry_run must not act"
    );
    let view = body_json(response).await;
    assert!(view.get("outcome").is_none(), "a preview has no outcome");
}

#[tokio::test]
async fn the_confirm_answers_202_and_opens_a_unit_rather_than_committing() {
    // D-109: retirement is a registered always-material trigger, so this arm can
    // only ever be `submitted_for_approval` however the tenant's thresholds are
    // configured.
    let h = Harness::new().await;
    let plan_id = published(&h).await;

    let response = h
        .allowed_as(SUBMITTER)
        .send(request("POST", &path(plan_id), Some(confirm_body())))
        .await;

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let view = body_json(response).await;
    assert_eq!(view["outcome"], "submitted_for_approval");
    assert!(view["approval"].is_object(), "{view}");
    // No version is requested for an act that did not commit (D-156).
    assert!(view["pending_version_ref"].is_null(), "{view}");
    // The approver reads the same preview the submitter saw — on this arm it is
    // the response body rather than a second call.
    assert_eq!(view["preview"]["presence_unresolved"], true);
    assert_eq!(
        view["cancelled_window_ids"]
            .as_array()
            .expect("array")
            .len(),
        0
    );

    // **Both halves of this test's name, read off the store rather than off the
    // response.** Every assertion above reads the document the handler just wrote,
    // and until 2026-08-11 they were the whole test — so a handler that rendered an
    // `approval` object and opened nothing passed "opens a unit", and a handler that
    // retired the plan *and* rendered the same body passed "rather than committing".
    //
    // Retirement is an irreversible act over a published plan (D-128), which makes
    // this the highest-stakes place for that pattern. The tools were in the file's
    // reach the whole time: `rest_support/mod.rs:490` documents `read_approval` as
    // existing for exactly this — "the response can be made to say anything, and a
    // test that only asserted the response would pass against a handler that minted
    // an id and opened nothing. That is the exact defect D-225 records for the
    // overlay submit's 202."
    let approval_id = view["approval"]["approval_id"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .unwrap_or_else(|| panic!("the 202 names the unit it opened: {view}"));
    let unit = h
        .read_approval(approval_id)
        .await
        .unwrap_or_else(|| panic!("the id the body names must be a stored unit: {view}"));
    assert_eq!(
        unit.state,
        bss_pricing::domain::approval::ApprovalState::Submitted,
        "the unit is open for a second principal to decide"
    );

    assert_eq!(
        plan_state(&h, plan_id, 0).await.as_deref(),
        Some("published"),
        "the plan is still published: a single principal's confirm stages the \
         retirement, it does not perform it"
    );
}

#[tokio::test]
async fn a_plan_that_is_not_published_is_refused() {
    let h = Harness::new().await;
    let plan_id = published(&h).await;
    h.retire(plan_id, 0).await;

    let response = h
        .allowed_as(SUBMITTER)
        .send(request("POST", &path(plan_id), Some(preview_body())))
        .await;

    // The lifecycle refusal is architecturally a 422 and reaches the wire as a
    // 400 carrying its code (Foundation §3.3) — the code is the discriminator,
    // not the status. This case stated exactly that and then tested the half the
    // sentence says is not the discriminator; several distinct refusals share this
    // status, so the status alone passes with the wrong one.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(response).await, "LIFECYCLE_FORBIDDEN");
}

#[tokio::test]
async fn an_unknown_plan_is_a_404_rather_than_an_empty_preview() {
    let h = Harness::new().await;

    let response = h
        .allowed_as(SUBMITTER)
        .send(request("POST", &path(Uuid::now_v7()), Some(preview_body())))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // The generic `NotFound` mints **no** wire code — `error_mapping.rs` puts the
    // id in `context.resource_name` and the subject in the detail — so the subject
    // is what discriminates this 404 from the four other things on this route that
    // can be absent. Asserted rather than the status alone, which every one of them
    // shares.
    let problem = body_json(response).await;
    assert!(
        problem["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("plan") && detail.contains("not found")),
        "the refusal names what was absent: {problem}"
    );
}
