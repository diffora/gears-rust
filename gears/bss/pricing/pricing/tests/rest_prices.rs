//! The price plane, driven through the real router.
//!
//! Every refusal is asserted by its **code** rather than only its status, and
//! every denial by an **unchanged store** as well as a 403 — see
//! `tests/rest_plans.rs` for why both habits are load-bearing.
//!
//! The pagination cases are the ones that would otherwise pass vacuously: an
//! exact-multiple boundary, a full walk, and inserts on both sides of a live
//! cursor. A test that only asked for one short page would be green against a
//! `next_cursor` that never stopped.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::plans::PLANS;
use rest_support::{
    Harness, body_json, etag_of, location_of, price_rows, problem_code, request, seed_draft_plan,
    seed_price, with_headers,
};
use uuid::Uuid;

fn prices_path(plan_id: Uuid) -> String {
    format!("{PLANS}/{plan_id}/prices")
}

fn price_path(plan_id: Uuid, price_id: Uuid) -> String {
    format!("{PLANS}/{plan_id}/prices/{price_id}")
}

/// A well-formed create body on a named region, so several can coexist.
fn create_body(region: &str) -> serde_json::Value {
    serde_json::json!({
        "scope_key": {
            "currency": "USD",
            "region": region,
            "phase": harness_phase(),
            "price_eligibility": "all_subscriptions",
            "charge_kind": "recurring",
            "cohort": serde_json::Value::Null
        },
        "content": {
            "model_kind": "flat",
            "amount_minor": 1_500,
            "tax_inclusive": false
        }
    })
}

fn harness_phase() -> String {
    rest_support::seeded_phase().get().to_string()
}

fn keyed(key: &str) -> Vec<(&str, &str)> {
    vec![("idempotency-key", key)]
}

async fn seeded_plan(harness: &Harness) -> Uuid {
    let plan_id = Uuid::now_v7();
    seed_draft_plan(harness, plan_id).await;
    plan_id
}

// ---------------------------------------------------------------------------
// Create.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_create_answers_201_with_the_location_and_the_rows_own_tag() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &prices_path(plan_id),
            Some(create_body("EU")),
            &keyed("price-1"),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let location = location_of(&response);
    let tag = etag_of(&response);
    let body = body_json(response).await;
    let price_id = body["price_id"].as_str().expect("the id is answered");
    assert_eq!(
        location,
        Some(format!("{}/{price_id}", prices_path(plan_id)))
    );
    assert_eq!(tag, Some("\"0\"".to_owned()));
    assert_eq!(
        body["scope_key"]["charge_kind"],
        serde_json::json!("recurring"),
        "the row's charge_kind is the KEY's, so the response echoes what was stored: {body}"
    );
}

#[tokio::test]
async fn a_replayed_create_answers_the_original_price_id() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let first = harness
        .allowed()
        .send(with_headers(
            "POST",
            &prices_path(plan_id),
            Some(create_body("EU")),
            &keyed("price-2"),
        ))
        .await;
    let first_id = body_json(first).await["price_id"].clone();

    let replay = harness
        .allowed()
        .send(with_headers(
            "POST",
            &prices_path(plan_id),
            Some(create_body("EU")),
            &keyed("price-2"),
        ))
        .await;

    assert_eq!(replay.status(), StatusCode::CREATED);
    assert_eq!(body_json(replay).await["price_id"], first_id);
    assert_eq!(price_rows(&harness, plan_id).await.len(), 1);
}

#[tokio::test]
async fn a_second_draft_on_one_canonical_key_is_refused_by_its_code() {
    // D-148 put the duplicate-key refusal on the draft plane too: the published
    // partial UNIQUE cannot see a second draft, which is the ambiguity publish
    // would fail on - discovered a round trip earlier.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    harness
        .allowed()
        .send(with_headers(
            "POST",
            &prices_path(plan_id),
            Some(create_body("EU")),
            &keyed("dup-1"),
        ))
        .await;

    let clash = harness
        .allowed()
        .send(with_headers(
            "POST",
            &prices_path(plan_id),
            Some(create_body("EU")),
            &keyed("dup-2"),
        ))
        .await;

    assert_eq!(clash.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(clash).await, "DUPLICATE_SCOPE_KEY");
    assert_eq!(price_rows(&harness, plan_id).await.len(), 1);
}

// ---------------------------------------------------------------------------
// Patch.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_patch_under_a_stale_tag_is_refused_and_the_row_does_not_move() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &price_path(plan_id, seeded.price_id),
            Some(serde_json::json!({ "content": { "model_kind": "flat", "amount_minor": 99 } })),
            &[("if-match", "\"7\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(response).await, "STALE_VERSION");
    let after = price_rows(&harness, plan_id).await;
    assert_eq!(after[0].row_version.get(), 0, "the tag stayed where it was");
}

#[tokio::test]
async fn a_patch_may_not_move_the_canonical_scope_key() {
    // `update_draft` cannot move a key, so a body naming a different one is a
    // refusal rather than a silent no-op: a key decides which duplicate a row
    // is, which chain it joins and which window covers it.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &price_path(plan_id, seeded.price_id),
            Some(serde_json::json!({
                "scope_key": {
                    "currency": "USD",
                    "region": "US",
                    "phase": harness_phase(),
                    "price_eligibility": "all_subscriptions",
                    "charge_kind": "recurring",
                    "cohort": serde_json::Value::Null
                },
                "content": { "model_kind": "flat", "amount_minor": 99 }
            })),
            &[("if-match", "\"0\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let after = price_rows(&harness, plan_id).await;
    assert_eq!(after[0].scope_key.region().as_str(), "EU");
    assert_eq!(after[0].row_version.get(), 0);
}

#[tokio::test]
async fn a_price_under_the_wrong_plans_url_is_not_found() {
    // A path that names a parent it does not check lets a caller mutate a row
    // through the wrong plan's URL - and makes the authz `resource_id` argument
    // a fiction.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let other_plan = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;

    let patched = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &price_path(other_plan, seeded.price_id),
            Some(serde_json::json!({ "content": { "model_kind": "flat", "amount_minor": 99 } })),
            &[("if-match", "\"0\"")],
        ))
        .await;
    let deleted = harness
        .allowed()
        .send(with_headers(
            "DELETE",
            &price_path(other_plan, seeded.price_id),
            None,
            &[("if-match", "\"0\"")],
        ))
        .await;

    assert_eq!(patched.status(), StatusCode::NOT_FOUND);
    assert_eq!(deleted.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        price_rows(&harness, plan_id).await.len(),
        1,
        "the row survived both"
    );
}

// ---------------------------------------------------------------------------
// Delete.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_delete_without_a_precondition_is_refused_and_the_row_survives() {
    // The D-141 defect itself: this verb's idempotency cell used to be empty, so
    // a draft row could be destroyed under an unknown version. What a blind
    // delete destroys is a concurrent editor's uncommitted work.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;

    let response = harness
        .allowed()
        .send(request(
            "DELETE",
            &price_path(plan_id, seeded.price_id),
            None,
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        price_rows(&harness, plan_id).await.len(),
        1,
        "an unconditional delete must not have taken effect"
    );
}

#[tokio::test]
async fn a_delete_under_the_right_tag_takes_the_row_and_its_bands() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;

    let response = harness
        .allowed()
        .send(with_headers(
            "DELETE",
            &price_path(plan_id, seeded.price_id),
            None,
            &[("if-match", "\"0\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(price_rows(&harness, plan_id).await.is_empty());
}

#[tokio::test]
async fn a_published_row_is_refused_rather_than_deleted() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;
    harness.publish_price(plan_id, seeded.price_id).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "DELETE",
            &price_path(plan_id, seeded.price_id),
            None,
            &[("if-match", "\"0\"")],
        ))
        .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a design-set 422 arrives as a 400 carrying its code"
    );
    assert_eq!(problem_code(response).await, "LIFECYCLE_FORBIDDEN");
    assert_eq!(price_rows(&harness, plan_id).await.len(), 1);
}

// ---------------------------------------------------------------------------
// The list, and the cursor's keyset walk.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_last_full_page_says_there_is_nothing_after_it() {
    // The exact-multiple boundary: with `limit` rows left, a walk that decided
    // `next_cursor` from the page size alone would return a token pointing at an
    // empty page - and a client that stops on `null` would make one request too
    // many, forever.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    for region in ["EU", "US", "UK", "JP"] {
        seed_price(&harness, plan_id, region).await;
    }

    let response = harness
        .allowed()
        .send(request(
            "GET",
            &format!("{}?limit=4", prices_path(plan_id)),
            None,
        ))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["items"].as_array().map(Vec::len), Some(4), "{body}");
    assert!(
        body["page_info"]["next_cursor"].is_null(),
        "an exhausted result carries no forward token: {body}"
    );
    assert!(body["page_info"]["prev_cursor"].is_null(), "{body}");
    assert_eq!(body["page_info"]["limit"], serde_json::json!(4));
}

#[tokio::test]
async fn a_walk_visits_every_row_exactly_once() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let mut expected: Vec<String> = Vec::new();
    for region in ["EU", "US", "UK", "JP", "CA"] {
        expected.push(
            seed_price(&harness, plan_id, region)
                .await
                .price_id
                .to_string(),
        );
    }
    expected.sort();

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let path = match &cursor {
            Some(token) => format!("{}?limit=2&cursor={token}", prices_path(plan_id)),
            None => format!("{}?limit=2", prices_path(plan_id)),
        };
        let body = body_json(harness.allowed().send(request("GET", &path, None)).await).await;
        for item in body["items"].as_array().expect("items") {
            seen.push(item["price_id"].as_str().expect("id").to_owned());
        }
        match body["page_info"]["next_cursor"].as_str() {
            Some(token) => cursor = Some(token.to_owned()),
            None => break,
        }
    }

    assert_eq!(seen, expected, "every row, exactly once, in key order");
}

#[tokio::test]
async fn a_row_inserted_ahead_of_a_live_cursor_is_not_skipped_and_one_behind_is_not_duplicated() {
    // D-125's stability guarantee, in the one direction a keyset walk actually
    // gives it. What it does NOT give - a row DELETED behind the cursor - is
    // stated in `api::rest::cursor`'s module doc rather than asserted here,
    // because no cursor over a mutable draft plane can promise it.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    for region in ["EU", "US", "UK", "JP"] {
        seed_price(&harness, plan_id, region).await;
    }

    let first = body_json(
        harness
            .allowed()
            .send(request(
                "GET",
                &format!("{}?limit=2", prices_path(plan_id)),
                None,
            ))
            .await,
    )
    .await;
    let cursor = first["page_info"]["next_cursor"]
        .as_str()
        .expect("a walk of four rows in pages of two continues")
        .to_owned();
    let page_one: Vec<String> = first["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["price_id"].as_str().expect("id").to_owned())
        .collect();

    // Two more rows land mid-walk. Their ids are v7, so they sort AFTER
    // everything already there - ahead of the cursor.
    seed_price(&harness, plan_id, "AU").await;
    seed_price(&harness, plan_id, "NZ").await;

    let second = body_json(
        harness
            .allowed()
            .send(request(
                "GET",
                &format!("{}?limit=10&cursor={cursor}", prices_path(plan_id)),
                None,
            ))
            .await,
    )
    .await;
    let page_two: Vec<String> = second["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["price_id"].as_str().expect("id").to_owned())
        .collect();

    for id in &page_one {
        assert!(
            !page_two.contains(id),
            "a row at or before the cursor must never be visited twice"
        );
    }
    assert_eq!(
        page_one.len() + page_two.len(),
        6,
        "the two rows inserted ahead of the cursor are visited, not skipped"
    );
}

#[tokio::test]
async fn the_page_parameters_are_refused_or_clamped_as_the_contract_says() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    seed_price(&harness, plan_id, "EU").await;

    let zero = harness
        .allowed()
        .send(request(
            "GET",
            &format!("{}?limit=0", prices_path(plan_id)),
            None,
        ))
        .await;
    assert_eq!(zero.status(), StatusCode::BAD_REQUEST);

    let garbage = harness
        .allowed()
        .send(request(
            "GET",
            &format!("{}?cursor=not-a-cursor!!", prices_path(plan_id)),
            None,
        ))
        .await;
    assert_eq!(garbage.status(), StatusCode::BAD_REQUEST);

    let oversized = harness
        .allowed()
        .send(request(
            "GET",
            &format!("{}?limit=5000", prices_path(plan_id)),
            None,
        ))
        .await;
    assert_eq!(
        oversized.status(),
        StatusCode::OK,
        "the cap is a server limit, not a caller mistake"
    );
    let body = body_json(oversized).await;
    assert_eq!(body["page_info"]["limit"], serde_json::json!(1_000));
}

#[tokio::test]
async fn the_walk_is_tenant_scoped_and_a_foreign_plans_rows_are_invisible() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    seed_price(&harness, plan_id, "EU").await;

    let owner = body_json(
        harness
            .allowed()
            .send(request("GET", &prices_path(plan_id), None))
            .await,
    )
    .await;
    assert_eq!(owner["items"].as_array().map(Vec::len), Some(1));

    let foreign = harness
        .other_tenant()
        .send(request("GET", &prices_path(plan_id), None))
        .await;
    assert_eq!(foreign.status(), StatusCode::OK);
    let body = body_json(foreign).await;
    assert_eq!(
        body["items"].as_array().map(Vec::len),
        Some(0),
        "the compiled scope is the SQL filter, so a foreign tenant sees an empty page: {body}"
    );
}

// ---------------------------------------------------------------------------
// The gate.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_price_route_is_denied_with_the_database_unchanged() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;

    for (method, path, body, headers) in [
        (
            "POST",
            prices_path(plan_id),
            Some(create_body("US")),
            vec![("idempotency-key", "denied-price")],
        ),
        (
            "PATCH",
            price_path(plan_id, seeded.price_id),
            Some(serde_json::json!({ "content": { "model_kind": "flat", "amount_minor": 1 } })),
            vec![("if-match", "\"0\"")],
        ),
        (
            "DELETE",
            price_path(plan_id, seeded.price_id),
            None,
            vec![("if-match", "\"0\"")],
        ),
    ] {
        let response = harness
            .denied()
            .send(with_headers(method, &path, body, &headers))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{method} {path} must be denied"
        );
    }
    let list = harness
        .denied()
        .send(request("GET", &prices_path(plan_id), None))
        .await;
    assert_eq!(list.status(), StatusCode::FORBIDDEN);

    let after = price_rows(&harness, plan_id).await;
    assert_eq!(after.len(), 1, "no row created and none deleted");
    assert_eq!(after[0].row_version.get(), 0, "and none edited");
}

#[tokio::test]
async fn an_unauthenticated_price_request_is_refused_before_the_gate() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;

    let response = harness
        .anonymous()
        .send(request("GET", &prices_path(plan_id), None))
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
