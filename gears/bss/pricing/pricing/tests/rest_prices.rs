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

mod common;
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
// The two Slice-10 primitives, refused rather than stored.
// ---------------------------------------------------------------------------

/// A create body whose content carries one extra member.
fn create_body_with(region: &str, key: &str, value: serde_json::Value) -> serde_json::Value {
    let mut body = create_body(region);
    body["content"][key] = value;
    body
}

/// Assert a refusal is the 400 the Foundation validation envelope renders, that
/// it names the field, and that no row was stored.
///
/// The status matters twice over: a 422 would contradict §3.3 (no path in this
/// gear produces one) and a 500 would say the gear tried and broke rather than
/// that it does not support the field.
async fn assert_refused_naming(response: axum::response::Response<axum::body::Body>, field: &str) {
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "an unsupported primitive is a malformed request, not a 422 and not a fault"
    );
    let body = body_json(response).await;
    let rendered = body.to_string();
    assert!(
        rendered.contains(field),
        "the refusal must name the field the caller has to remove: {rendered}"
    );
    assert!(
        !rendered.contains("422"),
        "no path in this gear produces a 422: {rendered}"
    );
}

#[tokio::test]
async fn a_create_carrying_a_tier_qualification_window_is_refused_at_any_value() {
    // `inst-tt-forbidden` refuses an EXPLICIT window of any value - `current`
    // included - because an accepted-but-ignored value masks authoring errors.
    // A check that only caught `trailing_period` would store the default spelled
    // out, and nothing in the crate judges either.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;

    for (index, window) in ["trailing_period", "current"].into_iter().enumerate() {
        let response = harness
            .allowed()
            .send(with_headers(
                "POST",
                &prices_path(plan_id),
                Some(create_body_with(
                    "EU",
                    "tier_qualification_window",
                    serde_json::json!(window),
                )),
                &keyed(&format!("tqw-{index}")),
            ))
            .await;

        assert_refused_naming(response, "tier_qualification_window").await;
    }
    assert!(
        price_rows(&harness, plan_id).await.is_empty(),
        "neither refusal may have stored a row"
    );
}

#[tokio::test]
async fn a_create_carrying_an_included_allowance_is_refused_under_either_rollover_policy() {
    // `none` needs the band compile (the $0 band, the offset set, the marker);
    // `carry` needs a `pricing_plan_grant` row. Neither exists, so a stored
    // declaration is an allowance billed from the first unit.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;

    for (index, policy) in ["none", "carry"].into_iter().enumerate() {
        let response = harness
            .allowed()
            .send(with_headers(
                "POST",
                &prices_path(plan_id),
                Some(create_body_with(
                    "EU",
                    "included_allowance",
                    serde_json::json!({ "quantity": 100, "rollover_policy": policy }),
                )),
                &keyed(&format!("allow-{index}")),
            ))
            .await;

        assert_refused_naming(response, "included_allowance").await;
    }
    assert!(
        price_rows(&harness, plan_id).await.is_empty(),
        "neither refusal may have stored a row"
    );
}

#[tokio::test]
async fn a_patch_cannot_slip_either_primitive_past_the_create_check() {
    // The `PATCH` is a WHOLE-content submission, so a caller who creates a clean
    // row and then patches it would otherwise reach exactly the state the create
    // refuses. Both refusals run in `content_of`, which both verbs call.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;

    for (field, value) in [
        (
            "tier_qualification_window",
            serde_json::json!("trailing_period"),
        ),
        (
            "included_allowance",
            serde_json::json!({ "quantity": 100, "rollover_policy": "none" }),
        ),
    ] {
        let response = harness
            .allowed()
            .send(with_headers(
                "PATCH",
                &price_path(plan_id, seeded.price_id),
                Some(serde_json::json!({
                    "content": {
                        "model_kind": "flat",
                        "amount_minor": 99,
                        field: value
                    }
                })),
                &[("if-match", "\"0\"")],
            ))
            .await;

        assert_refused_naming(response, field).await;
    }
    let after = price_rows(&harness, plan_id).await;
    assert_eq!(after[0].row_version.get(), 0, "no refusal moved the row");
}

#[tokio::test]
async fn a_row_carrying_neither_primitive_creates_and_patches_exactly_as_before() {
    // The refusal is conditioned on a NON-NULL value, so an explicit `null` is
    // the same request as an absent member. A refusal that fired on presence
    // rather than on a value would break every client that serializes its whole
    // content type.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;

    let created = harness
        .allowed()
        .send(with_headers(
            "POST",
            &prices_path(plan_id),
            Some(create_body_with(
                "EU",
                "tier_qualification_window",
                serde_json::Value::Null,
            )),
            &keyed("null-window"),
        ))
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let price_id = body_json(created).await["price_id"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .expect("the id is answered");

    let patched = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &price_path(plan_id, price_id),
            Some(serde_json::json!({
                "content": {
                    "model_kind": "flat",
                    "amount_minor": 42,
                    "included_allowance": serde_json::Value::Null
                }
            })),
            &[("if-match", "\"0\"")],
        ))
        .await;
    assert_eq!(patched.status(), StatusCode::OK);
    let after = price_rows(&harness, plan_id).await;
    assert_eq!(after[0].row_version.get(), 1, "the edit landed");
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

// ---------------------------------------------------------------------------
// The audit trail the three price routes write (D-135, D-158).
// ---------------------------------------------------------------------------

/// The records on one plan's segment, in seq order.
///
/// A price row's chain is the **plan's** (D-135 keys the segment on the audited
/// subject's aggregate, and a price row's aggregate is the plan it prices), so a
/// plan's authoring history and its rows' edits are one walk rather than a join.
async fn plan_records(harness: &Harness, plan_id: Uuid) -> Vec<(String, String, String)> {
    rest_support::audit_rows(harness)
        .await
        .into_iter()
        .filter(|row| row.chain_id == plan_id)
        .map(|row| (row.action, row.subject_kind, row.subject_ref))
        .collect()
}

#[tokio::test]
async fn a_price_create_writes_exactly_one_record_naming_the_row() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let before = plan_records(&harness, plan_id).await.len();

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &prices_path(plan_id),
            Some(create_body("EU")),
            &keyed("audit-create"),
        ))
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let price_id = body_json(response).await["price_id"]
        .as_str()
        .expect("the id is answered")
        .to_owned();

    let records = plan_records(&harness, plan_id).await;
    assert_eq!(records.len(), before + 1, "{records:?}");
    assert_eq!(
        records.last(),
        Some(&("create".to_owned(), "price_unit".to_owned(), price_id)),
        "{records:?}"
    );
}

#[tokio::test]
async fn a_price_patch_writes_exactly_one_record_naming_the_row() {
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;
    let before = plan_records(&harness, plan_id).await.len();

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &price_path(plan_id, seeded.price_id),
            Some(serde_json::json!({ "content": { "model_kind": "flat", "amount_minor": 99 } })),
            &[("if-match", "\"0\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let records = plan_records(&harness, plan_id).await;
    assert_eq!(records.len(), before + 1, "{records:?}");
    assert_eq!(
        records.last(),
        Some(&(
            "update".to_owned(),
            "price_unit".to_owned(),
            seeded.price_id.to_string()
        )),
        "{records:?}"
    );
}

#[tokio::test]
async fn a_price_delete_writes_a_record_that_outlives_the_row_it_names() {
    // The row is gone and the record is not, which is the whole point of an
    // append-only trail on a >= 7-year horizon: "who deleted this" is answerable
    // precisely because the answer does not live on the deleted row.
    //
    // The action is `delete` and not `abandon`: a never-published price row is
    // really removed (S3 sec 4.3), where a discarded plan revision is flipped and
    // keeps its number (D-145).
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;
    let before = plan_records(&harness, plan_id).await.len();

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

    let records = plan_records(&harness, plan_id).await;
    assert_eq!(records.len(), before + 1, "{records:?}");
    assert_eq!(
        records.last(),
        Some(&(
            "delete".to_owned(),
            "price_unit".to_owned(),
            seeded.price_id.to_string()
        )),
        "{records:?}"
    );
}

#[tokio::test]
async fn a_refused_price_write_leaves_no_record_of_having_happened() {
    // The record is inside the mutation's own transaction (D-135). Two refusals,
    // one per verb, because each opens its own transaction.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;
    let before = plan_records(&harness, plan_id).await.len();

    for (method, body) in [
        (
            "PATCH",
            Some(serde_json::json!({ "content": { "model_kind": "flat", "amount_minor": 99 } })),
        ),
        ("DELETE", None),
    ] {
        let response = harness
            .allowed()
            .send(with_headers(
                method,
                &price_path(plan_id, seeded.price_id),
                body,
                &[("if-match", "\"7\"")],
            ))
            .await;
        assert_eq!(response.status(), StatusCode::CONFLICT, "{method}");
    }

    assert_eq!(
        plan_records(&harness, plan_id).await.len(),
        before,
        "neither refusal recorded anything"
    );
}

#[tokio::test]
async fn every_price_record_extends_the_plans_own_segment() {
    // D-135's keying, asserted rather than assumed: a price row's records must
    // not open a segment of their own, or "who changed this plan" becomes a join
    // across chains and the roll-up has one more head to carry.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;

    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &price_path(plan_id, seeded.price_id),
            Some(serde_json::json!({ "content": { "model_kind": "flat", "amount_minor": 99 } })),
            &[("if-match", "\"0\"")],
        ))
        .await;

    let chains: std::collections::BTreeSet<Uuid> = rest_support::audit_rows(&harness)
        .await
        .into_iter()
        .map(|row| row.chain_id)
        .collect();
    assert_eq!(
        chains,
        std::collections::BTreeSet::from([plan_id]),
        "one segment for the plan and everything under it"
    );
}

/// One record's action and the pair `inst-au-complete` names first.
type StatePair = (String, Option<serde_json::Value>, Option<serde_json::Value>);

#[tokio::test]
async fn every_price_record_carries_the_before_and_after_state_its_action_implies() {
    // `inst-au-complete`'s first field, on every price route. Blanking both to
    // `None` in `record_price_mutation` used to leave the whole suite green.
    //
    // A delete has no after-state, and that is the assertion rather than an
    // omission: an empty object would be a claim that the row still stands in
    // some shape, and the row is gone.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;
    let seeded = seed_price(&harness, plan_id, "EU").await;
    for (method, body, tag) in [
        (
            "PATCH",
            Some(serde_json::json!({ "content": { "model_kind": "flat", "amount_minor": 99 } })),
            "\"0\"",
        ),
        ("DELETE", None, "\"1\""),
    ] {
        let response = harness
            .allowed()
            .send(with_headers(
                method,
                &price_path(plan_id, seeded.price_id),
                body,
                &[("if-match", tag)],
            ))
            .await;
        assert!(
            response.status().is_success(),
            "{method}: {:?}",
            response.status()
        );
    }

    let pairs: Vec<StatePair> = rest_support::audit_rows(&harness)
        .await
        .into_iter()
        .filter(|row| row.subject_kind == "price_unit")
        .map(|row| (row.action, row.before_state, row.after_state))
        .collect();
    assert_eq!(pairs.len(), 3, "create, update, delete: {pairs:?}");

    let (action, before, after) = &pairs[0];
    assert_eq!(action, "create");
    assert!(before.is_none(), "a create has no before-state: {before:?}");
    assert_eq!(
        after.as_ref().and_then(|state| state.get("rowVersion")),
        Some(&serde_json::json!(0))
    );

    let (action, before, after) = &pairs[1];
    assert_eq!(action, "update");
    assert_eq!(
        before.as_ref().and_then(|state| state.get("rowVersion")),
        Some(&serde_json::json!(0)),
        "the version the caller's precondition matched"
    );
    assert_eq!(
        after.as_ref().and_then(|state| state.get("rowVersion")),
        Some(&serde_json::json!(1))
    );

    let (action, before, after) = &pairs[2];
    assert_eq!(action, "delete");
    assert_eq!(
        before.as_ref().and_then(|state| state.get("rowVersion")),
        Some(&serde_json::json!(1)),
        "the row as it stood when it was taken"
    );
    assert!(
        after.is_none(),
        "and no after-state: an empty object would claim the row still stands: {after:?}"
    );
}

#[tokio::test]
async fn two_lines_of_one_market_render_two_distinct_keys() {
    // **The read side of D-196 clause (3), on the wire.** `ScopeKeyRequest` has no
    // usage-line member by decision — the line is authored on the content — but the
    // *response* view is the store's rendering of what a row is filed under, and it
    // showed eight axes. So two rows on two meters of one market answered with byte-
    // identical `scope_key` objects, and the surface a reviewer approves from could
    // not tell them apart. Nothing here is about what a caller may say; it is about
    // what they are told.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;

    let author = async |meter: &str, key: &str| {
        let response = harness
            .allowed()
            .send(with_headers(
                "POST",
                &prices_path(plan_id),
                Some(serde_json::json!({
                    "scope_key": {
                        "currency": "USD",
                        "region": "EU",
                        "phase": harness_phase(),
                        "price_eligibility": "all_subscriptions",
                        "charge_kind": "usage",
                        "cohort": serde_json::Value::Null
                    },
                    "content": {
                        "model_kind": "per_unit",
                        "amount_minor": 700,
                        "tax_inclusive": false,
                        "meter": meter,
                        "billing_granularity": "per_hour"
                    }
                })),
                &keyed(key),
            ))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "{:?}",
            response.body()
        );
        body_json(response).await
    };

    let cloudlets = author("cloudlets", "d196-view-1").await;
    let egress = author("egress_gb", "d196-view-2").await;

    assert_ne!(
        cloudlets["scope_key"], egress["scope_key"],
        "two meters of one market are two keys and the view has to say so: {} vs {}",
        cloudlets["scope_key"], egress["scope_key"]
    );
    assert_eq!(
        cloudlets["scope_key"]["meter"],
        serde_json::json!("cloudlets")
    );
    assert_eq!(
        cloudlets["scope_key"]["dimension_key"],
        serde_json::Value::Null,
        "the undimensioned line renders as absent rather than as the store's empty sentinel"
    );
}

#[tokio::test]
async fn a_metered_row_may_patch_while_echoing_the_key_it_cannot_fully_name() {
    // **The hole D-196 clause (3) opened on this path, pinned.** The usage line
    // is authored on the *content* view — `ScopeKeyRequest` has no `meter`
    // member — so the door derives the ninth and tenth axes from the content and
    // a stored usage row's key carries a line no request body could have stated.
    // The key-immutability check above compares the named key against the stored
    // one, and comparing those two raw would refuse **every** `PATCH` that echoes
    // its own key on a metered row: the caller would be told their key moved by
    // naming exactly the key they are on.
    //
    // The comparison is therefore over the axes the wire can express, with the
    // stored line carried onto the named key first. The neighbouring case proves
    // the check still refuses a key that really did move.
    let harness = Harness::new().await;
    let plan_id = seeded_plan(&harness).await;

    let create = harness
        .allowed()
        .send(with_headers(
            "POST",
            &prices_path(plan_id),
            Some(serde_json::json!({
                "scope_key": {
                    "currency": "USD",
                    "region": "EU",
                    "phase": harness_phase(),
                    "price_eligibility": "all_subscriptions",
                    "charge_kind": "usage",
                    "cohort": serde_json::Value::Null
                },
                "content": {
                    "model_kind": "per_unit",
                    "amount_minor": 700,
                    "tax_inclusive": false,
                    "meter": "cloudlets",
                    "billing_granularity": "per_hour"
                }
            })),
            &keyed("d196-clause3-create"),
        ))
        .await;
    assert_eq!(create.status(), StatusCode::CREATED, "{:?}", create.body());
    let price_id = price_rows(&harness, plan_id).await[0].price_id;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &price_path(plan_id, price_id),
            Some(serde_json::json!({
                "scope_key": {
                    "currency": "USD",
                    "region": "EU",
                    "phase": harness_phase(),
                    "price_eligibility": "all_subscriptions",
                    "charge_kind": "usage",
                    "cohort": serde_json::Value::Null
                },
                "content": {
                    "model_kind": "per_unit",
                    "amount_minor": 900,
                    "tax_inclusive": false,
                    "meter": "cloudlets",
                    "billing_granularity": "per_hour"
                }
            })),
            &[("if-match", "\"0\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::OK, "{:?}", response.body());
    let after = price_rows(&harness, plan_id).await;
    assert_eq!(
        after[0]
            .row
            .amount_minor
            .map(bss_pricing::domain::money::MinorAmount::get),
        Some(900)
    );
    assert_eq!(
        after[0]
            .scope_key
            .meter()
            .map(bss_pricing::domain::scope_key::Meter::as_str),
        Some("cloudlets"),
        "the line the row is filed under is untouched by an ordinary content edit"
    );
}
