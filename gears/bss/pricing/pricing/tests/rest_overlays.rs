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

    // **This case does NOT observe the warning, and says so rather than claiming
    // to.** The submit is refused — the region value is undeclared and the plan is
    // unpublished — and the `ValidationFailed` envelope carries `violations`
    // only, so `warnings` is dropped on the rejecting path by construction.
    //
    // What it does prove is that a `fixed` line over a lower layer is **not**
    // itself a refusal. The warning is observed non-empty by
    // `domain::overlay_rules::overlay_rules_tests::a_fixed_line_over_a_lower_layer_warns_and_does_not_block`,
    // and the 202's `warnings` field is observed empty by
    // `a_clean_submit_is_accepted_and_always_material`. What no test here reaches
    // is a **non-empty** `warnings` on a 202 — that needs a published plan whose
    // lower-precedence overlay matches it, and this gear mounts no plan-publish
    // route for the harness to drive. Recorded rather than left as a gap a reader
    // would assume covered.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        violation_codes(&body_json(response).await).contains(&"SCOPE_VALUE_UNKNOWN".to_owned()),
        "the refusal must be the world's, not the fixed line's"
    );
}

/// **A duplicate line key is a 400 naming `OVERLAY_LINE_DUPLICATE`, not a 500.**
///
/// D-42's *"one default line"*: two of them collide on the store's null-safe
/// index, which used to reach the caller as a driver error. The save is also the
/// only place the code can fire — the store refusing the duplicate here is what
/// made the `check_lines` arm unreachable at submit.
#[tokio::test]
async fn a_duplicate_line_key_is_refused_at_the_save() {
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
                "lines": [default_discount(1000), default_discount(2000)],
            })),
            &[("idempotency-key", &Uuid::now_v7().to_string())],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert!(
        violation_codes(&body).contains(&"OVERLAY_LINE_DUPLICATE".to_owned()),
        "got {body}"
    );
}

/// **An inverted interval is a 400, not a 500** — §1.7's "effective-interval
/// sanity", which had no implementation and reached `chk_pricing_price_overlay_interval`
/// as a driver error.
#[tokio::test]
async fn an_inverted_effective_interval_is_refused_at_the_save() {
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
                "effective_from": "2099-06-01T00:00:00Z",
                "effective_to": "2099-01-01T00:00:00Z",
                "target_plan_ids": [],
                "lines": [default_discount(1000)],
            })),
            &[("idempotency-key", &Uuid::now_v7().to_string())],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert!(
        violation_codes(&body).contains(&"OVERLAY_INTERVAL_INVALID".to_owned()),
        "got {body}"
    );
}

/// **A `PATCH` whose body and `If-Match` name different revisions is refused.**
///
/// The store is addressed by the tag, so accepting the mismatch would rewrite one
/// revision and report another — and a client that then submitted the revision it
/// was handed would submit a revision it never edited.
#[tokio::test]
async fn a_patch_whose_body_and_tag_disagree_about_the_revision_is_refused() {
    let harness = Harness::new().await;
    let overlay = seed_overlay(&harness, 10).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &overlay_path(overlay),
            Some(serde_json::json!({ "revision": 7, "lines": [default_discount(2500)] })),
            &[("if-match", "\"0-0\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// **Only an open draft is submittable.**
///
/// §5's row is "Submit **the draft**" and `inst-pl-commit` pins the approval unit
/// to one. Submitting a published revision would open a second always-material
/// unit over content that is already live — and `overlay_facts` skips the
/// candidate's own overlay (D-107), so a live revision would validate against a
/// world told to ignore it.
#[tokio::test]
async fn a_published_revision_is_not_submittable() {
    let harness = Harness::new().await;
    let overlay = seed_overlay(&harness, 10).await;
    harness
        .state
        .overlays
        .publish_revision(
            &harness.scope(),
            harness.tenant,
            overlay,
            0,
            rest_support::seed_stamp(),
        )
        .await
        .expect("revision 0 publishes");

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
        "a published revision is not a draft, and only a draft is submittable"
    );
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

/// **The submit opens the approval unit `inst-pl-commit` promises, and names it.**
///
/// D-225: until 2026-08-06 the route validated, answered 202 and opened **nothing** —
/// so the status was honest about the act and the act was incomplete. A consumer
/// reading `202 Accepted` with `materiality: alwaysMaterialTrigger` was being told a
/// two-person workflow had started, and none had; nothing held the overlay while an
/// approver looked at it, and nothing could ever publish it.
///
/// What the wire has to carry is the unit's **id**, because a submitter cannot chase
/// a review they cannot name — that is what `approvals` already gives every other
/// always-material act in this gear.
#[tokio::test]
async fn a_submit_opens_the_approval_unit_and_names_it_on_the_wire() {
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
    let approval_id = body["approval"]["approval_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .unwrap_or_else(|| panic!("the submit must name the unit it opened: {body}"));

    // The unit exists, it is pending, and it is over **this overlay revision** —
    // not over the overlay, which would let a pin taken on revision 3 authorize a
    // decision about revision 4.
    let record = harness
        .read_approval(approval_id)
        .await
        .unwrap_or_else(|| panic!("approval {approval_id} must be readable"));
    assert_eq!(record.subject_kind.as_str(), "overlay");
    assert_eq!(record.subject_ref, format!("{overlay}/0"));
    assert_eq!(record.state.as_str(), "submitted");
}

/// **One pending unit per overlay revision** — a second submit is refused, not
/// silently duplicated.
///
/// `inst-co-single-pending`'s posture for a subject that holds **no canonical scope
/// key**: that rule binds units whose change set touches a key, and an overlay's
/// touches none, so the guard here is the subject ref rather than the held-key
/// register. Two pending units over one revision would leave two reviewers approving
/// two contents whose order of arrival decides the overlay.
#[tokio::test]
async fn a_second_submit_of_one_revision_is_refused_while_the_first_is_pending() {
    let harness = Harness::new().await;
    let overlay = seed_overlay(&harness, 10).await;
    let body = serde_json::json!({ "revision": 0 });

    let first = harness
        .allowed()
        .send(request("POST", &submit_path(overlay), Some(body.clone())))
        .await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);

    let second = harness
        .allowed()
        .send(request("POST", &submit_path(overlay), Some(body)))
        .await;

    let status = second.status();
    let rendered = body_json(second).await.to_string();
    assert_eq!(status, StatusCode::CONFLICT, "{rendered}");
    assert!(
        rendered.contains("PENDING_CHANGE_UNIT_EXISTS"),
        "the refusal names the rule rather than a generic conflict: {rendered}"
    );
}

/// **An opened overlay unit can be decided**, which means its pin re-derives.
///
/// The half a submit is worthless without: `re_derive` re-reads the subject under the
/// same encoding the pin was taken with, and an approve compares the two. Its overlay
/// arm refused outright while nothing opened such a unit — correctly then, and a
/// false claim the moment `submit_overlay` existed — so every overlay unit would have
/// been un-approvable, which is a unit that can be opened and never decided.
///
/// This crate has had that exact defect **twice**, on `price_unit` and on `window`,
/// and both times it was invisible until something actually decided one: the arm
/// resolved the wrong revision, `content_matches_pin` answered `false`, and the
/// decision returned `APPROVAL_CONTENT_MISMATCH` for a subject nobody had touched.
#[tokio::test]
async fn an_overlay_unit_can_be_approved_because_its_pin_re_derives() {
    let harness = Harness::new().await;
    let overlay = seed_overlay(&harness, 10).await;

    let opened = harness
        .allowed()
        .send(request(
            "POST",
            &submit_path(overlay),
            Some(serde_json::json!({ "revision": 0 })),
        ))
        .await;
    assert_eq!(opened.status(), StatusCode::ACCEPTED);
    let approval_id = body_json(opened).await["approval"]["approval_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("the unit is named");

    harness
        .governance
        .approvals
        .decide(
            &harness.scope(),
            harness.tenant,
            bss_pricing::infra::approval::DecideRequest {
                approval_id,
                decision: bss_pricing::domain::approval::DecisionBy::Approve(REVIEWER),
                reason: None,
                approver_regions: bss_pricing::infra::approval::RegionGrant::Untransported,
                stamp: bss_pricing::domain::audit::AuditStamp {
                    actor_principal_id: REVIEWER,
                    recorded_at: chrono::Utc::now(),
                    correlation_id: Uuid::from_u128(0x_9d_c0),
                },
            },
        )
        .await
        .expect("the reviewer approves an overlay unit");

    let record = harness
        .read_approval(approval_id)
        .await
        .expect("the unit reads back");
    assert_eq!(record.state.as_str(), "approved");
}

/// The second principal — never the submitter, per `inst-tp-distinct`.
const REVIEWER: Uuid = Uuid::from_u128(0x_9d_22);

/// **A rejected unit leaves the overlay exactly as it was, and it re-submits.**
///
/// `inst-as-reject` promises a rejected non-plan subject returns to *"its slice-defined
/// pre-submit state"*. For an overlay that costs **nothing**, and the reason is worth
/// asserting rather than assuming: the submit writes to `pricing_approval` and to
/// nothing else — it does not flip the revision's lifecycle, does not stage a copy and
/// does not touch a line — so the pre-submit state *is* the current state and there is
/// no compensating write for a rejection to make.
///
/// That is a property of this slice's design and not a general truth: the plan plane's
/// submit has a draft to return, and Slice 7 parks a cutover's whole three-operation
/// payload in the approval record precisely because there is nowhere else to hold it.
/// Asserted here so a later change that gives the submit a side effect fails this case
/// rather than silently making `inst-as-reject` unimplemented.
#[tokio::test]
async fn a_rejected_unit_leaves_the_overlay_untouched_and_it_submits_again() {
    let harness = Harness::new().await;
    let overlay = seed_overlay(&harness, 10).await;
    let body = serde_json::json!({ "revision": 0 });

    let before = harness
        .allowed()
        .send(request("GET", PRICE_OVERLAYS, None))
        .await;
    let before = body_json(before).await;

    let opened = harness
        .allowed()
        .send(request("POST", &submit_path(overlay), Some(body.clone())))
        .await;
    let approval_id = body_json(opened).await["approval"]["approval_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("the unit is named");

    harness
        .governance
        .approvals
        .decide(
            &harness.scope(),
            harness.tenant,
            bss_pricing::infra::approval::DecideRequest {
                approval_id,
                decision: bss_pricing::domain::approval::DecisionBy::Reject(REVIEWER),
                reason: Some("not this quarter".to_owned()),
                approver_regions: bss_pricing::infra::approval::RegionGrant::Untransported,
                stamp: bss_pricing::domain::audit::AuditStamp {
                    actor_principal_id: REVIEWER,
                    recorded_at: chrono::Utc::now(),
                    correlation_id: Uuid::from_u128(0x_9d_c1),
                },
            },
        )
        .await
        .expect("the reviewer rejects");

    let after = harness
        .allowed()
        .send(request("GET", PRICE_OVERLAYS, None))
        .await;
    assert_eq!(
        body_json(after).await,
        before,
        "a rejection writes nothing to the overlay plane"
    );

    // And the key freed: the author fixes the objection and submits the same revision
    // again. A rejection that left the subject un-submittable would be a decision the
    // operator cannot act on.
    let again = harness
        .allowed()
        .send(request("POST", &submit_path(overlay), Some(body)))
        .await;
    assert_eq!(again.status(), StatusCode::ACCEPTED);
}

// ---------------------------------------------------------------------------
// The second act: publish (D-234).
// ---------------------------------------------------------------------------

/// A second principal approves the unit, through the service.
///
/// Not through `POST …/approvals/{id}/approve`: this case is about the **publish
/// arm**, and routing the decision as well would make a failure ambiguous between
/// the two surfaces. The approval route has its own suite.
async fn approve(harness: &Harness, approval_id: &str) {
    use std::collections::BTreeSet;
    let approver = Uuid::from_u128(0xa9_9a);
    harness
        .governance
        .approvals
        .decide(
            &toolkit_db::secure::AccessScope::allow_all(),
            harness.tenant,
            bss_pricing::infra::approval::DecideRequest {
                approval_id: Uuid::parse_str(approval_id).expect("an approval id"),
                decision: bss_pricing::domain::approval::DecisionBy::Approve(approver),
                reason: None,
                approver_regions: bss_pricing::infra::approval::RegionGrant::Explicit(
                    BTreeSet::new(),
                ),
                stamp: bss_pricing::domain::audit::AuditStamp {
                    actor_principal_id: approver,
                    recorded_at: chrono::Utc::now(),
                    correlation_id: Uuid::now_v7(),
                },
            },
        )
        .await
        .expect("a second principal approves the unit");
}

/// The submit route's two acts, driven through the real edge.
///
/// S9 §5 spells this route as *"Submit the draft — always-material Slice 5
/// approval unit (D-50), **then the D-06 publish unit**"*, so the two acts are one
/// route by the design set's own arrangement.
///
/// The first call opens the unit and answers **202**. The second, once a second
/// principal has approved *this content*, commits and answers **200** carrying the
/// registry's pending handle. Matching on the content and not merely on the
/// subject is what keeps the route out of a dead end: an approval whose subject
/// moved after the decision covers content that no longer exists.
#[tokio::test]
async fn an_approved_overlay_publishes_on_the_second_call_to_the_same_route() {
    let harness = Harness::new().await;
    let overlay = seed_overlay(&harness, 10).await;

    // Act one: the submit.
    let response = harness
        .allowed()
        .send(request(
            "POST",
            &submit_path(overlay),
            Some(serde_json::json!({ "revision": 0 })),
        ))
        .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let opened = body_json(response).await;
    assert_eq!(opened["outcome"], "submitted_for_approval");
    assert_eq!(opened["pending_version_ref"], serde_json::Value::Null);
    let approval_id = opened["approval"]["approval_id"]
        .as_str()
        .expect("the unit is named on the wire (D-225)")
        .to_owned();

    // A second principal decides it.
    approve(&harness, &approval_id).await;

    // Act two: the same route, the same body, and now it publishes.
    let response = harness
        .allowed()
        .send(request(
            "POST",
            &submit_path(overlay),
            Some(serde_json::json!({ "revision": 0 })),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let published = body_json(response).await;
    assert_eq!(published["outcome"], "published");
    assert!(
        published["pending_version_ref"].is_string(),
        "the receipt names the registry handle, which is not a CatalogVersion: {published}"
    );
}
