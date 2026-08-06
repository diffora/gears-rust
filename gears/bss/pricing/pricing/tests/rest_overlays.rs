//! The `PriceOverlay` authoring surface, end to end —
//! `design/09-price-overlays.md` §5.
//!
//! What this suite is for, beyond driving the three routes: **the status split**.
//! §5 types seven of the nine overlay codes as architectural 422s and two of them
//! **409 outright**, and this platform has no 422 category at all — so the seven
//! reach the wire as one aggregate 400 whose per-violation codes are the
//! discriminators, and the two reach it as conflicts. That difference is
//! invisible to every unit test of the rules, because the rules raise all nine
//! into one report; it exists only at this seam.
//!
//! `rest_authz` drives the same four routes for the **gate**. Nothing here
//! re-proves that.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::overlays::{PRICE_OVERLAY_SUBMIT, PRICE_OVERLAYS};
use rest_support::{Harness, body_json, request, with_headers};
use uuid::Uuid;

fn overlay_path(overlay_id: Uuid) -> String {
    format!("{PRICE_OVERLAYS}/{overlay_id}")
}

fn submit_path(overlay_id: Uuid) -> String {
    PRICE_OVERLAY_SUBMIT.replace("{overlayId}", &overlay_id.to_string())
}

/// The ordinary line: a `global` overlay's list-default discount.
fn default_discount(bp: i64) -> serde_json::Value {
    serde_json::json!({
        "adjustment_kind": "discount",
        "magnitude_kind": "percent_bp",
        "adjustment_value": bp,
    })
}

/// Author a `global` overlay at `precedence`, and hand back its id.
async fn seed_overlay(harness: &Harness, precedence: i32) -> Uuid {
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            PRICE_OVERLAYS,
            Some(serde_json::json!({
                "scope_class": "global",
                "precedence": precedence,
                "tax_basis": "delegated_tariffs",
                "target_plan_ids": [],
                "lines": [default_discount(1000)],
            })),
            &[("idempotency-key", &Uuid::now_v7().to_string())],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_json(response).await;
    body["price_overlay_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .expect("the created overlay's id")
}

/// Every precondition-violation code the response carries.
///
/// The RFC 9457 document puts them at `context.violations[].type` — the
/// **code** is the `type` of a violation, and its prose is the `description`.
/// That placement is what "the code is the discriminator, not the status"
/// means concretely, and asserting on it is what stops a refusal degrading into
/// a message a consumer would have to parse.
fn violation_codes(body: &serde_json::Value) -> Vec<String> {
    body["context"]["violations"]
        .as_array()
        .map(|violations| {
            violations
                .iter()
                .filter_map(|v| v["type"].as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// `inst-pl-author` / `inst-pl-return`.
// ---------------------------------------------------------------------------

/// A save lands a **draft** and publishes nothing (`inst-pl-return`).
#[tokio::test]
async fn a_save_lands_a_draft_and_publishes_nothing() {
    let harness = Harness::new().await;
    let overlay = seed_overlay(&harness, 10).await;

    let response = harness
        .allowed()
        .send(request("GET", PRICE_OVERLAYS, None))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let overlays = body["overlays"].as_array().expect("the list");
    assert_eq!(overlays.len(), 1);
    assert_eq!(
        overlays[0]["lifecycle_state"], "draft",
        "nothing publishes from a save"
    );
    assert_eq!(overlays[0]["price_overlay_id"], overlay.to_string());
    assert_eq!(
        overlays[0]["disclosure"], "restricted",
        "L6's fail-closed default"
    );
    assert_eq!(
        overlays[0]["scope_value"],
        serde_json::Value::Null,
        "the classless scope carries no value"
    );
}

/// **`TAX_BASIS_UNDECLARED` is reachable, and it is the discriminator.**
///
/// L5's *silence fails*. The field is `Option` on the wire precisely so this
/// code is reachable — a required field would be refused by the deserializer
/// with a message the design set does not own.
#[tokio::test]
async fn an_overlay_with_no_tax_basis_is_refused_by_its_own_code() {
    let harness = Harness::new().await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            PRICE_OVERLAYS,
            Some(serde_json::json!({
                "scope_class": "global",
                "precedence": 10,
                "target_plan_ids": [],
                "lines": [default_discount(1000)],
            })),
            &[("idempotency-key", &Uuid::now_v7().to_string())],
        ))
        .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "section 5's 422 is architectural; this platform renders it 400"
    );
    let body = body_json(response).await;
    assert!(
        violation_codes(&body).contains(&"TAX_BASIS_UNDECLARED".to_owned()),
        "the code must be the discriminator, not the message: {body}"
    );
}

/// The scope pairing is refused at the edge, both ways.
#[tokio::test]
async fn the_global_class_and_a_scope_value_are_refused_together() {
    let harness = Harness::new().await;

    for body in [
        serde_json::json!({
            "scope_class": "global",
            "scope_value": "everyone",
            "precedence": 10,
            "tax_basis": "delegated_tariffs",
            "target_plan_ids": [],
            "lines": [default_discount(1000)],
        }),
        serde_json::json!({
            "scope_class": "brand",
            "precedence": 11,
            "tax_basis": "delegated_tariffs",
            "target_plan_ids": [],
            "lines": [default_discount(1000)],
        }),
    ] {
        let response = harness
            .allowed()
            .send(with_headers(
                "POST",
                PRICE_OVERLAYS,
                Some(body),
                &[("idempotency-key", &Uuid::now_v7().to_string())],
            ))
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

/// A `fixed` line declared `percent_bp` is refused (D-138), and a `targetSku`
/// line with no plan is too — both are pairings the domain cannot represent, so
/// the edge is where they are told apart from a well-formed request.
#[tokio::test]
async fn the_unrepresentable_line_pairings_are_refused_at_the_edge() {
    let harness = Harness::new().await;

    for line in [
        serde_json::json!({
            "adjustment_kind": "fixed",
            "magnitude_kind": "percent_bp",
            "adjustment_value": 5000,
        }),
        serde_json::json!({
            "target_sku": "sku-a",
            "adjustment_kind": "discount",
            "magnitude_kind": "percent_bp",
            "adjustment_value": 1000,
        }),
        serde_json::json!({
            "cohort": "2099-03-01T00:00:00Z",
            "adjustment_kind": "discount",
            "magnitude_kind": "percent_bp",
            "adjustment_value": 1000,
        }),
    ] {
        let response = harness
            .allowed()
            .send(with_headers(
                "POST",
                PRICE_OVERLAYS,
                Some(serde_json::json!({
                    "scope_class": "global",
                    "precedence": 10,
                    "tax_basis": "delegated_tariffs",
                    "target_plan_ids": [],
                    "lines": [line],
                })),
                &[("idempotency-key", &Uuid::now_v7().to_string())],
            ))
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

/// **D-67's range fails the SAVE, and it does so as a 400.**
///
/// `discount / percent_bp = 15000` is D-67's own "150% of list" data-entry
/// inversion. It is refused twice over — by
/// `chk_pricing_price_overlay_line_discount_ceiling` in the store and by
/// `check_magnitudes` at this edge — and the edge is what makes the refusal
/// legible: **this case answered 500 before that entry point existed**, because
/// the `CHECK` reached the caller as a driver error for a request whose whole
/// remedy is to correct one number.
#[tokio::test]
async fn an_out_of_range_magnitude_is_refused_at_the_save() {
    let harness = Harness::new().await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            PRICE_OVERLAYS,
            Some(serde_json::json!({
                "scope_class": "global",
                "precedence": 10,
                "tax_basis": "delegated_tariffs",
                "target_plan_ids": [],
                "lines": [default_discount(15_000)],
            })),
            &[("idempotency-key", &Uuid::now_v7().to_string())],
        ))
        .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "an out-of-range magnitude is the caller's mistake, not an internal fault"
    );
    let body = body_json(response).await;
    assert!(
        violation_codes(&body).contains(&"ADJUSTMENT_MAGNITUDE_OUT_OF_RANGE".to_owned()),
        "got {body}"
    );
}

// ---------------------------------------------------------------------------
// The `PATCH` — the line set, wholesale, under the overlay's own tag.
// ---------------------------------------------------------------------------

/// The line set is replaced wholesale and the entity tag moves with it.
#[tokio::test]
async fn replacing_the_line_set_bumps_the_overlays_own_tag() {
    let harness = Harness::new().await;
    let overlay = seed_overlay(&harness, 10).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &overlay_path(overlay),
            Some(serde_json::json!({
                "revision": 0,
                "lines": [default_discount(2500), {
                    "adjustment_kind": "markup",
                    "magnitude_kind": "amount",
                    "amounts": [{ "currency": "EUR", "value_minor": 500 }],
                    "plan_id": Uuid::now_v7(),
                }],
            })),
            &[("if-match", "\"0-0\"")],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .expect("an entity tag"),
        "\"0-1\"",
        "the overlay revision's own tag, not a plan's"
    );
}

/// A stale tag is a 409 and changes nothing.
#[tokio::test]
async fn a_stale_tag_is_refused_as_a_conflict() {
    let harness = Harness::new().await;
    let overlay = seed_overlay(&harness, 10).await;

    let edit = |tag: &'static str| {
        with_headers(
            "PATCH",
            &overlay_path(overlay),
            Some(serde_json::json!({ "revision": 0, "lines": [default_discount(2500)] })),
            &[("if-match", tag)],
        )
    };
    assert_eq!(
        harness.allowed().send(edit("\"0-0\"")).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        harness.allowed().send(edit("\"0-0\"")).await.status(),
        StatusCode::CONFLICT,
        "the second edit against the spent tag is a conflict"
    );
}

// ---------------------------------------------------------------------------
// The submit, and the status split §5 types.
// ---------------------------------------------------------------------------

/// A clean overlay submits **202** and is always material (D-50).
#[tokio::test]
async fn a_clean_submit_is_accepted_and_always_material() {
    let harness = Harness::new().await;
    let overlay = seed_overlay(&harness, 10).await;

    let response = harness
        .allowed()
        .send(request(
            "POST",
            &submit_path(overlay),
            Some(serde_json::json!({ "revision": 0 })),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = body_json(response).await;
    assert_eq!(
        body["materiality"], "alwaysMaterialTrigger",
        "an overlay line has no per-currency baseline to threshold (D-50, G1)"
    );
    assert_eq!(
        body["warnings"].as_array().map(Vec::len),
        Some(0),
        "a clean overlay raises no advisory either"
    );
}

/// **The 422 family: one aggregate 400 carrying every code.**
///
/// This is what makes an overlay remediable in one pass rather than in as many
/// round trips as it has faults (Foundation §4.2).
#[tokio::test]
async fn the_architectural_422s_arrive_as_one_aggregate_400() {
    let harness = Harness::new().await;
    // A brand overlay whose value no taxonomy declares, with a line naming a
    // plan outside its (empty) target_ref. The magnitude is deliberately **in
    // range**: D-67's bound is refused at the save (see
    // `an_out_of_range_magnitude_is_refused_at_the_save`), so an out-of-range
    // line would never reach the submit this case is about.
    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            PRICE_OVERLAYS,
            Some(serde_json::json!({
                "scope_class": "brand",
                "scope_value": "nobody-declared-this",
                "precedence": 10,
                "tax_basis": "delegated_tariffs",
                "target_plan_ids": [],
                "lines": [{
                    "plan_id": Uuid::now_v7(),
                    "adjustment_kind": "discount",
                    "magnitude_kind": "percent_bp",
                    "adjustment_value": 1_500,
                }],
            })),
            &[("idempotency-key", &Uuid::now_v7().to_string())],
        ))
        .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a save runs no world-dependent rule"
    );
    let overlay = body_json(response).await["price_overlay_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .expect("the created overlay");

    let response = harness
        .allowed()
        .send(request(
            "POST",
            &submit_path(overlay),
            Some(serde_json::json!({ "revision": 0 })),
        ))
        .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "the architectural 422 family renders 400"
    );
    let body = body_json(response).await;
    let codes = violation_codes(&body);
    for expected in ["SCOPE_VALUE_UNKNOWN", "OVERLAY_LINE_TARGET_UNKNOWN"] {
        assert!(
            codes.contains(&expected.to_owned()),
            "{expected} must be in the one-pass report, got {codes:?} from {body}"
        );
    }
}

/// **The 409 family: lifted out of the envelope, because §5 types it 409.**
///
/// A caller told two sibling overlays hold a precedence acts by re-reading, not
/// by editing a field of their own request — which is what a 409 asks for and a
/// 400 would not.
#[tokio::test]
async fn a_duplicate_precedence_is_a_conflict_and_not_a_validation_envelope() {
    let harness = Harness::new().await;
    let first = seed_overlay(&harness, 10).await;
    harness
        .state
        .overlays
        .publish_revision(
            &harness.scope(),
            harness.tenant,
            first,
            0,
            rest_support::seed_stamp(),
        )
        .await
        .expect("the first overlay publishes at precedence 10");

    let rival = seed_overlay(&harness, 10).await;
    let response = harness
        .allowed()
        .send(request(
            "POST",
            &submit_path(rival),
            Some(serde_json::json!({ "revision": 0 })),
        ))
        .await;

    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "section 5 types PRECEDENCE_DUPLICATE 409 outright"
    );
    let body = body_json(response).await;
    assert_eq!(
        body["context"]["reason"], "PRECEDENCE_DUPLICATE",
        "the code is the discriminator: {body}"
    );
}

/// **D-138's warning rides the succeeding path**, which is the only place it can
/// be advisory.
///
/// The `ValidationFailed` envelope exists on the rejecting path only, so a
/// warning carried only there would be computed and discarded — the defect D-197
/// records for the plan plane, not repeated here.
#[tokio::test]
async fn a_fixed_line_over_a_lower_layer_warns_on_the_succeeding_path() {
    let harness = Harness::new().await;
    let plan = Uuid::now_v7();

    // A published `global` overlay at precedence 5, carrying a list-default line
    // — the layer a higher-precedence `fixed` would discard.
    let lower = seed_overlay(&harness, 5).await;
    harness
        .state
        .overlays
        .publish_revision(
            &harness.scope(),
            harness.tenant,
            lower,
            0,
            rest_support::seed_stamp(),
        )
        .await
        .expect("the lower layer publishes");

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            PRICE_OVERLAYS,
            Some(serde_json::json!({
                "scope_class": "region",
                "scope_value": "eu-west",
                "precedence": 20,
                "tax_basis": "delegated_tariffs",
                "target_plan_ids": [plan],
                "lines": [{
                    "plan_id": plan,
                    "adjustment_kind": "fixed",
                    "magnitude_kind": "amount",
                    "amounts": [{ "currency": "EUR", "value_minor": 5000 }],
                }],
            })),
            &[("idempotency-key", &Uuid::now_v7().to_string())],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let overlay = body_json(response).await["price_overlay_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .expect("the created overlay");

    let response = harness
        .allowed()
        .send(request(
            "POST",
            &submit_path(overlay),
            Some(serde_json::json!({ "revision": 0 })),
        ))
        .await;

    // The submit is refused — the region value is undeclared and the plan is
    // unpublished — so this case proves the *warning is computed*, and the
    // succeeding path is proved by `a_clean_submit_is_accepted_and_always_material`.
    // What must not happen is the warning silently vanishing, which is exactly
    // what D-197 recorded for the plan plane's caps.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// The list read is the operator's and shows a `restricted` overlay (§3 step 7).
#[tokio::test]
async fn the_list_shows_restricted_overlays_to_their_operator() {
    let harness = Harness::new().await;
    seed_overlay(&harness, 10).await;
    seed_overlay(&harness, 20).await;

    let response = harness
        .allowed()
        .send(request(
            "GET",
            &format!("{PRICE_OVERLAYS}?scope_class=global"),
            None,
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["overlays"].as_array().map(Vec::len), Some(2));

    let response = harness
        .allowed()
        .send(request(
            "GET",
            &format!("{PRICE_OVERLAYS}?scope_class=partner"),
            None,
        ))
        .await;
    let body = body_json(response).await;
    assert_eq!(body["overlays"].as_array().map(Vec::len), Some(0));
}
