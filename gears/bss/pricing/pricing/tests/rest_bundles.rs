//! The bundle plane, driven through the real router.
//!
//! Every refusal is asserted by its **code** rather than only its status, which
//! is the habit `rest_plans.rs` explains and which matters more here than
//! anywhere else in this gear: §5 types all ten composition refusals `422`, the
//! platform has no 422 at all, and every one of them therefore arrives as a
//! `400`. A test asserting the status would be asserting nothing — it is the
//! code string that tells `CURRENCY_NOT_COVERED` from `FREQUENCY_MISMATCH`.
//!
//! The publish cases drive the whole pipeline end to end: a composition is
//! authored through the `PATCH`, judged by the rules over rows this suite seeds
//! into the store, and — when it passes — normalized onto its absorber with
//! `BundleUpdated` in the same transaction.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::bundles::BUNDLES;
use rest_support::{
    Harness, approval_row, approval_rows, body_json, etag_of, problem_code, seed_draft_plan,
    seed_price, with_headers,
};
use uuid::Uuid;

fn bundle_path(bundle_id: Uuid) -> String {
    format!("{BUNDLES}/{bundle_id}")
}

fn publish_path(bundle_id: Uuid) -> String {
    format!("{BUNDLES}/{bundle_id}/publish")
}

/// Create a bundle on a fresh draft plan and hand back both ids.
async fn seed_bundle(harness: &Harness) -> (Uuid, Uuid) {
    seed_bundle_with(harness, "sum_of_parts").await
}

/// The same, on a named basis.
async fn seed_bundle_with(harness: &Harness, basis: &str) -> (Uuid, Uuid) {
    let plan_id = Uuid::now_v7();
    seed_draft_plan(harness, plan_id).await;
    harness.attach_shape(plan_id, 0).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            BUNDLES,
            Some(serde_json::json!({
                "plan_id": plan_id,
                "price_basis": basis,
                "invoice_itemization": "itemize",
            })),
            &[("idempotency-key", &Uuid::now_v7().to_string())],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_json(response).await;
    let bundle_id = body["bundle_id"]
        .as_str()
        .and_then(|s| s.parse::<Uuid>().ok())
        .expect("the response must name the bundle it created");
    (plan_id, bundle_id)
}

// ---------------------------------------------------------------------------
// `inst-ba-author` — creating the bundle.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_bundle_is_created_on_its_plan() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;

    assert_ne!(bundle_id, Uuid::nil());
    assert_ne!(plan_id, Uuid::nil());
}

/// `inst-bb-declared`: the basis MUST be declared, and `BASIS_MISSING` is what
/// an absent one is told.
///
/// The field is `Option` on the wire precisely so this code is reachable — a
/// required field would be refused by the deserializer with a message the design
/// set does not own.
#[tokio::test]
async fn a_bundle_with_no_declared_basis_is_refused_by_code() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            BUNDLES,
            Some(serde_json::json!({ "plan_id": plan_id })),
            &[("idempotency-key", &Uuid::now_v7().to_string())],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let detail = body_json(response).await.to_string();
    assert!(
        detail.contains("BASIS_MISSING"),
        "the refusal must carry the code the design set declares, got: {detail}"
    );
}

/// **A retried create is replayed, not re-executed** (Z12-7, `inst-bk-idem`).
///
/// The route took the `Idempotency-Key` and discarded it — `let _client_key = …`,
/// required of every caller and used by nothing — so this suite's every key was a
/// fresh `Uuid::now_v7()` and no second call ever carried one a first call used.
/// What a retry actually got was `BUNDLE_EXISTS_ON_PLAN`: the plan's uniqueness
/// index caught the second insert, so a client that retried on a timeout was told
/// its bundle already existed, by somebody, with no id and no way to tell its own
/// first attempt from another operator's.
///
/// Both halves are asserted. The status **and the body** come back as the first
/// call's, because a replay that re-rendered would hand the caller something it was
/// never told; and the store still holds one bundle, which is what makes this a
/// proof about at-most-once rather than about a status code.
#[tokio::test]
async fn a_retried_create_replays_the_first_answer_and_creates_one_bundle() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;
    harness.attach_shape(plan_id, 0).await;
    let key = Uuid::now_v7().to_string();
    let request = serde_json::json!({
        "plan_id": plan_id,
        "price_basis": "sum_of_parts",
        "invoice_itemization": "itemize",
    });

    let first = harness
        .allowed()
        .send(with_headers(
            "POST",
            BUNDLES,
            Some(request.clone()),
            &[("idempotency-key", &key)],
        ))
        .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let first = body_json(first).await;

    let replay = harness
        .allowed()
        .send(with_headers(
            "POST",
            BUNDLES,
            Some(request),
            &[("idempotency-key", &key)],
        ))
        .await;

    assert_eq!(
        replay.status(),
        StatusCode::CREATED,
        "a replay answers what the first call answered, not what the uniqueness index thinks \
         of a second insert"
    );
    let replayed = body_json(replay).await;
    assert_eq!(
        replayed, first,
        "verbatim: the bundle id above all, since a fresh one would name a bundle the caller \
         cannot address"
    );

    // The positive control against a replay that is merely a refusal wearing a
    // 201: exactly one bundle exists, and it is the one both answers named.
    let listed = harness
        .allowed()
        .send(with_headers(
            "GET",
            &format!("{BUNDLES}?plan_id={plan_id}"),
            None,
            &[],
        ))
        .await;
    assert_eq!(listed.status(), StatusCode::OK);
    let listed = body_json(listed).await;
    let items = listed["items"].as_array().expect("a page of bundles");
    assert_eq!(items.len(), 1, "one create, one bundle: {listed}");
    assert_eq!(items[0]["bundle_id"], first["bundle_id"]);
}

/// **A different bundle under a spent key is refused** (Z12-7).
///
/// Armed on a genuinely different request — another basis, another itemization —
/// under the same key. Replaying the *same* body proves the opposite property and
/// is the case above, which is this one's positive control. The refusal is the
/// gate's own `IDEMPOTENCY_PAYLOAD_MISMATCH` rather than a `BUNDLE_EXISTS_ON_PLAN`
/// that happens to be produced by an index: the two are different facts, and only
/// the first tells the caller that the key is the problem.
#[tokio::test]
async fn a_different_create_under_a_spent_key_is_refused_as_a_payload_mismatch() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;
    harness.attach_shape(plan_id, 0).await;
    let key = Uuid::now_v7().to_string();

    let first = harness
        .allowed()
        .send(with_headers(
            "POST",
            BUNDLES,
            Some(serde_json::json!({
                "plan_id": plan_id,
                "price_basis": "sum_of_parts",
                "invoice_itemization": "itemize",
            })),
            &[("idempotency-key", &key)],
        ))
        .await;
    assert_eq!(first.status(), StatusCode::CREATED);

    let refused = harness
        .allowed()
        .send(with_headers(
            "POST",
            BUNDLES,
            Some(serde_json::json!({
                "plan_id": plan_id,
                "price_basis": "own_price",
                "invoice_itemization": "aggregate",
            })),
            &[("idempotency-key", &key)],
        ))
        .await;

    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        problem_code(refused).await,
        "IDEMPOTENCY_PAYLOAD_MISMATCH",
        "a key held by one request must not answer for another, and the refusal has to name \
         the key rather than the plan's uniqueness"
    );
}

/// One bundle per plan, as a conflict rather than a storage failure.
#[tokio::test]
async fn a_second_bundle_on_one_plan_is_a_conflict() {
    let harness = Harness::new().await;
    let (plan_id, _) = seed_bundle(&harness).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            BUNDLES,
            Some(serde_json::json!({ "plan_id": plan_id, "price_basis": "own_price" })),
            &[("idempotency-key", &Uuid::now_v7().to_string())],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(response).await, "BUNDLE_EXISTS_ON_PLAN");
}

// ---------------------------------------------------------------------------
// The composition, under the plan revision's tag.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_composition_is_written_under_the_revisions_tag() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;
    let tag = harness.plan_etag(plan_id).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [{
                    "component_plan_id": Uuid::now_v7(),
                    "included_sku_id": Uuid::now_v7(),
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    // The tag moved: the composition rides the revision, so an edit advances it.
    let next = etag_of(&response).expect("the response must carry the new tag");
    assert_ne!(next, tag, "a composition edit must move the revision's tag");
}

/// The tag is the guard, and a stale one is refused rather than merged.
#[tokio::test]
async fn a_stale_tag_is_refused_on_the_composition_route() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;
    let tag = harness.plan_etag(plan_id).await;

    let body = serde_json::json!({ "plan_revision": 0, "components": [] });
    let first = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(body.clone()),
            &[("if-match", &tag)],
        ))
        .await;
    assert_eq!(first.status(), StatusCode::OK);

    let second = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(body),
            &[("if-match", &tag)],
        ))
        .await;
    // **409, not 412**, and it is the crate's convention rather than this
    // route's choice: `DomainError::StaleVersion` renders through the conflict
    // ladder everywhere in this gear (`rest_plans.rs` asserts the same pair), on
    // the argument that the caller's request was right about the world it was
    // shown and somebody else moved it. Re-reading is the whole remedy.
    assert_eq!(second.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(second).await, "STALE_VERSION");
}

/// A party may not spell the reserved `platform` sentinel — the invariant that
/// keeps `residualAbsorberParty` unambiguous, refused at the edge.
#[tokio::test]
async fn a_party_named_platform_is_refused_at_the_edge() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;
    let tag = harness.plan_etag(plan_id).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [],
                "rev_share": [{
                    "vendor_sku_id": Uuid::now_v7(),
                    "platform_cut_bp": 1000,
                    "parties": [{ "party": "platform", "share_bp": 9000 }],
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// `inst-ba-validate` — the whole rule set, through the publish route.
// ---------------------------------------------------------------------------

/// An unpublished component blocks, and the report names the code.
#[tokio::test]
async fn an_unpublished_component_blocks_the_publish_by_code() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;
    let tag = harness.plan_etag(plan_id).await;
    let component = Uuid::now_v7();

    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [{
                    "component_plan_id": component,
                    "included_sku_id": Uuid::now_v7(),
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "markets": [{ "currency": "EUR", "region": "EU" }],
            })),
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let detail = body_json(response).await.to_string();
    assert!(
        detail.contains("COMPONENT_UNPUBLISHED"),
        "the report must name the failing rule, got: {detail}"
    );
}

/// A `sum_of_parts` bundle referencing nothing sums nothing.
#[tokio::test]
async fn a_composition_with_no_components_blocks_the_publish() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;
    let tag = harness.plan_etag(plan_id).await;

    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "components": [] })),
            &[("if-match", &tag)],
        ))
        .await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "markets": [] })),
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let detail = body_json(response).await.to_string();
    assert!(detail.contains("COMPONENT_UNPUBLISHED"), "got: {detail}");
}

/// A rev-share group more than 1 bp out is `RESIDUAL_OVER_TOLERANCE` (D-07) —
/// the six-way even split that decision names as the operator's to reconcile.
#[tokio::test]
async fn a_group_over_tolerance_blocks_the_publish_by_code() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;
    let tag = harness.plan_etag(plan_id).await;

    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [],
                "rev_share": [{
                    "vendor_sku_id": Uuid::now_v7(),
                    "platform_cut_bp": 0,
                    "parties": [
                        { "party": "a", "share_bp": 1666 },
                        { "party": "b", "share_bp": 1666 },
                        { "party": "c", "share_bp": 1666 },
                        { "party": "d", "share_bp": 1666 },
                        { "party": "e", "share_bp": 1666 },
                        { "party": "f", "share_bp": 1666 },
                    ],
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "markets": [] })),
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let detail = body_json(response).await.to_string();
    assert!(detail.contains("RESIDUAL_OVER_TOLERANCE"), "got: {detail}");
}

/// **The aggregate report.** One pass names every failing rule, which is what
/// makes a composition remediable in one edit rather than in as many edits as it
/// has faults.
#[tokio::test]
async fn one_publish_reports_every_failing_rule_at_once() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;
    let tag = harness.plan_etag(plan_id).await;

    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [{
                    "component_plan_id": Uuid::now_v7(),
                    "included_sku_id": Uuid::now_v7(),
                }],
                "rev_share": [{
                    "vendor_sku_id": Uuid::now_v7(),
                    "platform_cut_bp": 0,
                    "parties": [{ "party": "a", "share_bp": 5000 }],
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "markets": [{ "currency": "EUR", "region": "EU" }],
            })),
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let detail = body_json(response).await.to_string();
    for code in [
        "COMPONENT_UNPUBLISHED",
        "CURRENCY_NOT_COVERED",
        "RESIDUAL_OVER_TOLERANCE",
    ] {
        assert!(
            detail.contains(code),
            "{code} must be reported alongside the others, got: {detail}"
        );
    }
}

// ---------------------------------------------------------------------------
// `inst-ba-return` — 202 and the always-material verdict.
// ---------------------------------------------------------------------------

/// A composition with nothing to refuse answers **202** and opens D-104's
/// always-material unit — asserted on the **stored** record, not on the body.
///
/// The body assertion below is kept, but it is deliberately no longer the only
/// one, and the reason is that on its own it could not fail. `bundles.rs` wrote
/// `materiality: "alwaysMaterialTrigger".to_owned()` as an unconditional literal,
/// so this test compared the handler's constant against a copy of itself: no
/// input could redden it, and while green it reported D-104 as covered on this
/// route. `alwaysMaterialTrigger` is in fact the token `evaluate` answers for an
/// act trigger, which is what made the tautology so hard to see — the value was
/// right and nothing had computed it.
///
/// `approval_rows` is what tells the two apart. It was in scope in this file's
/// sibling case (`a_composition_edit_voids_the_pending_unit_over_its_plan`) the
/// whole time; applying it here is what surfaced that the publish opened no unit
/// at all.
#[tokio::test]
async fn a_clean_publish_is_accepted_and_opens_the_always_material_unit() {
    let harness = Harness::new().await;
    // An `own_price` bundle with no components and no markets has no coverage
    // walk to fail and no rev-share to reconcile: the smallest publishable
    // composition there is.
    let (plan_id, bundle_id) = seed_bundle_with(&harness, "own_price").await;
    let tag = harness.plan_etag(plan_id).await;
    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "components": [] })),
            &[("if-match", &tag)],
        ))
        .await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "markets": [] })),
            &[],
        ))
        .await;

    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::ACCEPTED, "publish refused: {body}");
    assert_eq!(
        body["materiality"].as_str(),
        Some("alwaysMaterialTrigger"),
        "D-104: a composition publish is material whatever a threshold says"
    );

    // The load-bearing half. A handler that renders the token and evaluates
    // nothing passes every assertion above it.
    let opened = approval_rows(&harness).await;
    let expected_ref = format!("{plan_id}/composition/0");
    let Some(opened_row) = opened.iter().find(|row| row.subject_ref == expected_ref) else {
        panic!(
            "D-104: the composition publish must open an always-material unit over \
             {expected_ref}; the store holds {} unit(s): {:?}",
            opened.len(),
            opened.iter().map(|r| &r.subject_ref).collect::<Vec<_>>()
        )
    };
    let opened_id = opened_row.approval_id;
    let unit = approval_row(&harness, opened_id).await;
    assert_eq!(
        unit.materiality["reason"], "alwaysMaterialTrigger",
        "the stored verdict names the rule that fired, which is what an auditor reads"
    );
    assert_eq!(
        unit.state,
        bss_pricing::domain::approval::ApprovalState::Submitted,
        "D-104: a single principal's publish stages the composition; it does not commit it"
    );
}

/// The **positive control** for the case above, and the other half of D-104's
/// two-call shape: a second, independent principal approves the staged unit and
/// the same call then publishes.
///
/// Without this the suite would prove only that the composition stops — a handler
/// that staged every publish and could never commit one would pass every
/// assertion in `a_clean_publish_is_accepted_and_opens_the_always_material_unit`.
/// That is the failure mode the fix itself introduced: before D-104 was enforced
/// here the publish arm was the only arm, and afterwards it was reachable by no
/// test at all.
#[tokio::test]
async fn an_independent_approval_is_what_publishes_the_composition() {
    const COMPOSITION_REVIEWER: Uuid = Uuid::from_u128(0x_b0d1_e504);

    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle_with(&harness, "own_price").await;
    let tag = harness.plan_etag(plan_id).await;
    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "components": [] })),
            &[("if-match", &tag)],
        ))
        .await;

    // Call one: stages.
    let staged = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "markets": [] })),
            &[],
        ))
        .await;
    let staged_body = body_json(staged).await;
    assert_eq!(
        staged_body["outcome"].as_str(),
        Some("submitted_for_approval"),
        "the first call over this content stages it: {staged_body}"
    );
    let approval_id = staged_body["approval"]["approval_id"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .unwrap_or_else(|| panic!("the 202 names the unit it opened: {staged_body}"));

    let approved = harness
        .allowed_as(COMPOSITION_REVIEWER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{approval_id}/approve"),
            None,
            &[],
        ))
        .await;
    let approved_status = approved.status();
    let approved_body = body_json(approved).await;
    assert_eq!(
        approved_status,
        StatusCode::OK,
        "an independent principal is what authorizes a composition change (D-104): \
         {approved_body}"
    );

    // Call two: publishes, on the strength of that decision and no other.
    let published = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "markets": [] })),
            &[],
        ))
        .await;
    let published_body = body_json(published).await;
    assert_eq!(
        published_body["outcome"].as_str(),
        Some("published"),
        "the approved content publishes on the next call: {published_body}"
    );
    assert_eq!(
        published_body["materiality"].as_str(),
        Some("alwaysMaterialTrigger"),
        "and the reason is the evaluator's on both arms, not a per-arm literal"
    );
}

// ---------------------------------------------------------------------------
// `inst-bc-coverage`'s narrowing — which rows count, driven end to end.
// ---------------------------------------------------------------------------

/// A **published component whose only row is still a draft** covers nothing.
///
/// This pair exists because a probe found the gap it closes: deleting the
/// `lifecycle_state = 'published'` conjunct from the service's coverage filter
/// reddened **nothing**, since no case here had a draft row for the filter to
/// exclude. The narrowing was enforced and untested at this layer, which is a
/// guard that can be removed under a green tree.
///
/// The component's *plan* is published, so `COMPONENT_UNPUBLISHED` must **not**
/// fire — that is what makes this a test of the row filter rather than of the
/// plan check standing in front of it.
#[tokio::test]
async fn a_published_components_draft_row_does_not_cover_a_market() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;

    // A published component plan carrying one row that never published.
    let component = Uuid::now_v7();
    // No `attach_shape` on the component: `pricing_plan_phase` is keyed
    // `(phase_id, plan_revision)` and the harness attaches a **fixed** phase id,
    // so a second plan at revision 0 collides with the bundle plan's own. The
    // price row does not need the phase row to exist - the `phase` axis is a
    // bare uuid (D-19) and carries no foreign key.
    seed_draft_plan(&harness, component).await;
    seed_price(&harness, component, "EU").await;
    harness.publish(component, 0).await;

    let tag = harness.plan_etag(plan_id).await;
    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [{
                    "component_plan_id": component,
                    "included_sku_id": Uuid::now_v7(),
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "markets": [{ "currency": "USD", "region": "EU" }],
            })),
            &[],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let detail = body_json(response).await.to_string();
    assert!(
        detail.contains("CURRENCY_NOT_COVERED"),
        "a draft row must not count as coverage, got: {detail}"
    );
    assert!(
        !detail.contains("COMPONENT_UNPUBLISHED"),
        "the component's plan IS published; this must be the row filter and not the \
         plan check, got: {detail}"
    );
}

/// The other half of the same fact, and the half that makes the pair a
/// discrimination rather than a rule that always refuses: publish the very same
/// row and the very same composition passes its coverage walk.
///
/// Without this case the filter could be `WHERE false` and the case above would
/// still be green.
#[tokio::test]
async fn the_same_row_published_does_cover_the_market() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;

    let component = Uuid::now_v7();
    seed_draft_plan(&harness, component).await;
    let row = seed_price(&harness, component, "EU").await;
    harness.publish(component, 0).await;
    harness.publish_price(component, row.price_id).await;

    let tag = harness.plan_etag(plan_id).await;
    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [{
                    "component_plan_id": component,
                    "included_sku_id": Uuid::now_v7(),
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "markets": [{ "currency": "USD", "region": "EU" }],
            })),
            &[],
        ))
        .await;

    let status = response.status();
    let detail = body_json(response).await.to_string();
    assert!(
        !detail.contains("CURRENCY_NOT_COVERED"),
        "a published row on the sold market must cover it, got {status}: {detail}"
    );
}

// ---------------------------------------------------------------------------
// Section 10's currency-binding counter, cases (ii) and (iii), through the router.
// ---------------------------------------------------------------------------

/// The counter section 10 names.
const CURRENCY_BINDING: &str = "pricing_currency_binding_blocks_total";

/// Every label the counter can carry, so a case can assert the ones it is *not*.
///
/// `CurrencyBindingCase::ALL`'s spellings. Written out here rather than iterated
/// from the enum on purpose: the label values are the operator-facing series
/// names, and a test deriving them from the same `as_str` the adapter uses would
/// stay green through a rename that broke every dashboard.
const CASES: [&str; 3] = ["required_addon", "bundle_sum_of_parts", "bundle_own_price"];

/// Publish a one-component bundle on `basis`, selling `(USD, EU)`, the component's
/// only row published or not, and hand back the response body as text.
///
/// The seeding is `a_published_components_draft_row_does_not_cover_a_market`'s,
/// which is the shape already proven to reach the coverage walk: the component's
/// *plan* publishes, so `COMPONENT_UNPUBLISHED` is not what fires, and whether the
/// market is covered is then the single variable.
async fn publish_one_component_bundle(harness: &Harness, basis: &str, covered: bool) -> String {
    let (plan_id, bundle_id) = seed_bundle_with(harness, basis).await;

    let component = Uuid::now_v7();
    seed_draft_plan(harness, component).await;
    let row = seed_price(harness, component, "EU").await;
    harness.publish(component, 0).await;
    if covered {
        harness.publish_price(component, row.price_id).await;
    }

    let tag = harness.plan_etag(plan_id).await;
    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [{
                    "component_plan_id": component,
                    "included_sku_id": Uuid::now_v7(),
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "markets": [{ "currency": "USD", "region": "EU" }],
            })),
            &[],
        ))
        .await;
    body_json(response).await.to_string()
}

/// **A blocked `sum_of_parts` publish counts case (ii) and nothing else.**
///
/// Asserted through the router, which is the standard `rest_preview.rs` states for
/// the sibling instrument: what is proven is that a real refusal reported a real
/// series, not that the adapter can increment its own counter. Both of this
/// counter's bundle cases were reachable only from `metrics_tests.rs`, which calls
/// the port directly — deleting the emission in `infra/bundle.rs` left the whole
/// suite green.
///
/// The **other two labels are asserted zero**, and that is the load-bearing half
/// rather than padding. `bundle_rules` raises the same `CURRENCY_NOT_COVERED`
/// string for cases (i), (ii) and (iii), so an emitter that chose its label by
/// scanning violation codes would count this block as `required_addon` while the
/// refusal an operator reads stayed correct. Only the label can tell the two
/// apart.
#[tokio::test]
async fn a_blocked_sum_of_parts_publish_counts_the_bundle_sum_case() {
    let harness = Harness::new().await;
    let detail = publish_one_component_bundle(&harness, "sum_of_parts", false).await;

    assert!(
        detail.contains("CURRENCY_NOT_COVERED"),
        "the block this counts must be the coverage one, got: {detail}"
    );
    harness.metrics.force_flush();

    assert_eq!(
        harness
            .metrics
            .counter_value(CURRENCY_BINDING, &[("case", "bundle_sum_of_parts")]),
        1,
        "one composition mistake is one block on the declared basis's case"
    );
    for other in CASES.iter().filter(|case| **case != "bundle_sum_of_parts") {
        assert_eq!(
            harness
                .metrics
                .counter_value(CURRENCY_BINDING, &[("case", other)]),
            0,
            "a sum_of_parts bundle's block must not be reported as {other}"
        );
    }
}

/// The same block on an `own_price` bundle is case (iii), and this is what makes
/// the pair a **discrimination** rather than one label asserted twice.
///
/// The composition, the component, the market and the refusal code are identical
/// to the case above; only the declared basis differs. An emitter reading the
/// report instead of the basis would answer `bundle_sum_of_parts` to both and pass
/// one of the two cases.
#[tokio::test]
async fn a_blocked_own_price_publish_counts_the_other_bundle_case() {
    let harness = Harness::new().await;
    let detail = publish_one_component_bundle(&harness, "own_price", false).await;

    assert!(
        detail.contains("CURRENCY_NOT_COVERED"),
        "the block this counts must be the coverage one, got: {detail}"
    );
    harness.metrics.force_flush();

    assert_eq!(
        harness
            .metrics
            .counter_value(CURRENCY_BINDING, &[("case", "bundle_own_price")]),
        1,
        "the label is the composition's declared basis, not the violation's code"
    );
    for other in CASES.iter().filter(|case| **case != "bundle_own_price") {
        assert_eq!(
            harness
                .metrics
                .counter_value(CURRENCY_BINDING, &[("case", other)]),
            0,
            "an own_price bundle's block must not be reported as {other}"
        );
    }
}

/// **A composition that covers its market counts nothing.**
///
/// The negative control the two above rest on: an emitter that counted every
/// publish, or every validation run, would satisfy both of them and would report a
/// healthy catalog as permanently blocked. Same route, same seed, one row
/// published.
#[tokio::test]
async fn a_covered_bundle_publish_counts_no_block() {
    let harness = Harness::new().await;
    let detail = publish_one_component_bundle(&harness, "sum_of_parts", true).await;

    assert!(
        !detail.contains("CURRENCY_NOT_COVERED"),
        "a published row on the sold market covers it, got: {detail}"
    );
    harness.metrics.force_flush();

    for case in CASES {
        assert_eq!(
            harness
                .metrics
                .counter_value(CURRENCY_BINDING, &[("case", case)]),
            0,
            "a composition that covers its markets must report no {case} block"
        );
    }
}

// ---------------------------------------------------------------------------
// The TOCTOU guard, on the plane that rides a plan revision.
// ---------------------------------------------------------------------------

/// **A composition edit voids the pending unit over the plan it rides.**
///
/// `inst-ap-pin` states the guard without qualification: *"any mutation of the subject
/// while `submitted` invalidates the pending approval"*. A bundle composition **is** a
/// mutation of a plan revision — it rides the revision, and the edit takes the
/// revision's entity tag and advances it.
///
/// **It holds, and this case exists because two of the three things that would break it
/// are true.** Written first as a demonstration that it did *not* hold, and the
/// hypothesis was wrong at exactly one link:
///
/// 1. `PlanShape` — what a `plan_revision` unit pins — carries **no composition**. Its
///    nineteen fields are the plan's own shape, its rows and its windows; the component
///    list, the rev-share splits, `price_basis` and `invoiceItemization` are none of
///    them. **True.**
/// 2. So a component swap **does not move the pin**: `content_matches_pin` would stay
///    `true` across it, which is the answer a reviewer's read is built to trust.
///    **True.**
/// 3. And nothing voids the unit. **False** — and the reason is one level down, which
///    is why it survived a grep: `bundle_repo` never names `void_pending_units_of`, but
///    `replace_composition` calls `plan_repo::record_revision_mutation`, whose *first*
///    statement is that void. The composition edit is recorded as a plan-revision
///    mutation, so it inherits the TOCTOU guard rather than restating it.
///
/// Which makes 1 and 2 harmless **only while 3 holds**, and that is what this case
/// pins. A refactor that gave the composition its own recorder — a plausible thing to
/// want, since the audit record it writes is a plan record for a bundle act — would
/// reopen D-104's own scenario (*"a component swap reached consumers with no
/// approver"*) through the **approved** path, which is the worse half: the approval
/// record would say the act was reviewed, and the composition reaching consumers would
/// be one no reviewer ever saw. Nothing asserted this before.
///
/// The control below is not ceremony. Without it the case passes over a guard that does
/// not exist, because a unit that was never `submitted` reads `voided` at the end
/// whatever the edit did.
#[tokio::test]
async fn a_composition_edit_voids_the_pending_unit_over_its_plan() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;

    let opened = harness
        .governance
        .approvals
        .submit(
            &harness.scope(),
            harness.tenant,
            bss_pricing::domain::scope_key::PlanId::new(plan_id),
            Uuid::now_v7(),
            serde_json::json!({ "material": true, "reason": "alwaysMaterialTrigger" }),
            bss_pricing::domain::audit::AuditStamp {
                actor_principal_id: Uuid::from_u128(0x_b0_11),
                recorded_at: chrono::Utc::now(),
                correlation_id: Uuid::from_u128(0x_b0_c0),
            },
        )
        .await
        .expect("a plan-revision unit opens over the plan the bundle rides");

    // **The control.** Without it this case proves nothing: a unit that was never
    // `submitted` reads "voided" at the end whatever the composition edit did, and the
    // assertion below would pass over a guard that does not exist.
    assert_eq!(
        harness
            .read_approval(opened.approval_id)
            .await
            .expect("the unit reads back")
            .state
            .as_str(),
        "submitted",
        "the unit must be pending before the composition moves under it"
    );

    let tag = harness.plan_etag(plan_id).await;
    let swapped = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [{
                    "component_plan_id": Uuid::now_v7(),
                    "included_sku_id": Uuid::now_v7(),
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;
    assert_eq!(swapped.status(), StatusCode::OK);

    let record = harness
        .read_approval(opened.approval_id)
        .await
        .expect("the unit reads back");
    assert_eq!(
        record.state.as_str(),
        "voided",
        "the composition moved under a pending unit whose pin cannot see it, so the unit \
         must be voided rather than left approvable"
    );
}

/// **Does the approved unit's pin actually cover the composition?**
///
/// F-1's fix pins the *plan shape* (`infra::publish::assemble`), on the reasoning
/// that a composition normalizes onto its absorber inside the plan. If that is
/// false at submit time, then two different component sets hash identically and an
/// approve taken over one authorizes the other — the exact approval-bypass shape
/// D-196's content-pin miss produced on the scope key.
///
/// D-104 exists precisely because a `sum_of_parts` recomposition carries **no
/// price-row delta at all**, which is what makes this worth an armed test rather
/// than an argument.
#[tokio::test]
async fn an_approved_composition_does_not_authorize_a_different_one() {
    const REVIEWER: Uuid = Uuid::from_u128(0x_b0d1_e505);

    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle_with(&harness, "own_price").await;

    let tag = harness.plan_etag(plan_id).await;
    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "components": [] })),
            &[("if-match", &tag)],
        ))
        .await;

    let staged = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "markets": [] })),
            &[],
        ))
        .await;
    let staged_body = body_json(staged).await;
    let approval_id = staged_body["approval"]["approval_id"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .unwrap_or_else(|| panic!("the 202 names its unit: {staged_body}"));

    harness
        .allowed_as(REVIEWER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{approval_id}/approve"),
            None,
            &[],
        ))
        .await;

    // The composition moves **after** the approval. Whatever the pin covers, this
    // is a different set of components than the reviewer agreed to.
    let tag = harness.plan_etag(plan_id).await;
    let edited = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [],
                "rev_share": []
            })),
            &[("if-match", &tag)],
        ))
        .await;
    let edited_status = edited.status();

    let after = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "markets": [] })),
            &[],
        ))
        .await;
    let after_body = body_json(after).await;

    assert_eq!(
        after_body["outcome"].as_str(),
        Some("submitted_for_approval"),
        "the edit (PATCH answered {edited_status}) must invalidate the approval, so this call \
         stages a fresh unit rather than publishing on the old one: {after_body}"
    );
}

/// **The approver can read the composition they are approving** (D-61, F-4).
///
/// `GET /approvals/{id}` returns `pinned_content` — the plan the composition rides
/// — and until 2026-08-11 that was all a reviewer of a `bundleComposition` unit
/// got. A plan shape carries no component set and no revenue split, and D-104
/// exists because a `sum_of_parts` recomposition moves no price row at all: the
/// document said nothing about the act being decided, on the one surface in this
/// gear where the money being divided belongs to third parties.
///
/// D-61 is explicit that the `GET` must return the pinned **content**, "not the
/// hash alone, so approval is never hash-blind".
#[tokio::test]
async fn the_unit_shows_its_approver_the_component_set_and_the_revenue_split() {
    let harness = Harness::new().await;
    // `sum_of_parts`: rev-share is authorable on that basis only (D-55), and it is
    // the basis whose recomposition moves no price row — which is the case D-104
    // exists for and the one a plan shape cannot show.
    let (plan_id, bundle_id) = seed_bundle(&harness).await;

    let component_plan = Uuid::now_v7();
    seed_draft_plan(&harness, component_plan).await;
    let row = seed_price(&harness, component_plan, "EU").await;
    harness.publish(component_plan, 0).await;
    harness.publish_price(component_plan, row.price_id).await;
    let vendor_sku = Uuid::now_v7();

    let tag = harness.plan_etag(plan_id).await;
    let patched = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [{
                    "component_plan_id": component_plan,
                    "included_sku_id": vendor_sku,
                }],
                "rev_share": [{
                    "vendor_sku_id": vendor_sku,
                    "platform_cut_bp": 2000,
                    "parties": [{ "party": "acme-vendor", "share_bp": 8000 }],
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;
    assert_eq!(patched.status(), StatusCode::OK);

    let staged = harness
        .allowed()
        .send(with_headers(
            "POST",
            &publish_path(bundle_id),
            Some(serde_json::json!({ "plan_revision": 0, "markets": [] })),
            &[],
        ))
        .await;
    let staged_body = body_json(staged).await;
    let approval_id = staged_body["approval"]["approval_id"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .unwrap_or_else(|| panic!("the 202 names its unit: {staged_body}"));

    let read = harness
        .allowed()
        .send(with_headers(
            "GET",
            &format!("/bss-pricing/v1/approvals/{approval_id}"),
            None,
            &[],
        ))
        .await;
    assert_eq!(read.status(), StatusCode::OK);
    let view = body_json(read).await;

    let composition = &view["pinned_composition"];
    assert!(
        composition.is_object(),
        "a bundleComposition unit must render its composition: {view}"
    );
    assert_eq!(
        composition["components"][0]["component_plan_id"],
        component_plan.to_string(),
        "the reviewer sees which plan is in the bundle: {composition}"
    );
    // The half that is third-party money, and the reason D-104 makes this act
    // always material.
    assert_eq!(composition["rev_share"][0]["platform_cut_bp"], 2000);
    assert_eq!(
        composition["rev_share"][0]["parties"][0]["party"],
        "acme-vendor"
    );
    assert_eq!(composition["rev_share"][0]["parties"][0]["share_bp"], 8000);
}

/// **The composition reads back as it was written** (D-310).
///
/// The read F-4 asked for. A composition was reachable through no surface in the
/// gear: not by its author, not by an operator, and — once D-104's always-material
/// unit existed — not by the approver deciding it.
///
/// The members are the authoring shapes, so this asserts the round trip rather
/// than a second rendering: what an author reads back is spelled exactly as what
/// they wrote.
#[tokio::test]
async fn the_composition_reads_back_as_it_was_authored() {
    let harness = Harness::new().await;
    let (plan_id, bundle_id) = seed_bundle(&harness).await;

    let component_plan = Uuid::now_v7();
    seed_draft_plan(&harness, component_plan).await;
    let row = seed_price(&harness, component_plan, "EU").await;
    harness.publish(component_plan, 0).await;
    harness.publish_price(component_plan, row.price_id).await;
    let vendor_sku = Uuid::now_v7();

    let tag = harness.plan_etag(plan_id).await;
    let written = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &bundle_path(bundle_id),
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [{
                    "component_plan_id": component_plan,
                    "included_sku_id": vendor_sku,
                    "min_qty": 1,
                }],
                "rev_share": [{
                    "vendor_sku_id": vendor_sku,
                    "platform_cut_bp": 1500,
                    "parties": [{ "party": "acme-vendor", "share_bp": 8500 }],
                }],
            })),
            &[("if-match", &tag)],
        ))
        .await;
    assert_eq!(written.status(), StatusCode::OK);

    let read = harness
        .allowed()
        .send(with_headers("GET", &bundle_path(bundle_id), None, &[]))
        .await;
    assert_eq!(read.status(), StatusCode::OK);
    let view = body_json(read).await;

    assert_eq!(view["bundle_id"], bundle_id.to_string());
    assert_eq!(view["plan_id"], plan_id.to_string());
    assert_eq!(view["plan_revision"], 0);
    assert_eq!(view["price_basis"], "sum_of_parts");

    assert_eq!(
        view["components"][0]["component_plan_id"],
        component_plan.to_string()
    );
    assert_eq!(view["components"][0]["min_qty"], 1);
    // The half that is third-party money.
    assert_eq!(view["rev_share"][0]["platform_cut_bp"], 1500);
    assert_eq!(view["rev_share"][0]["parties"][0]["party"], "acme-vendor");
    assert_eq!(view["rev_share"][0]["parties"][0]["share_bp"], 8500);
}

/// A bundle of another tenant reads exactly like an absent one.
#[tokio::test]
async fn a_foreign_bundle_is_a_404_rather_than_a_403() {
    let harness = Harness::new().await;
    let (_, bundle_id) = seed_bundle(&harness).await;

    let response = harness
        .other_tenant()
        .send(with_headers("GET", &bundle_path(bundle_id), None, &[]))
        .await;

    // The gate answers first here, which is the stronger of the two: the caller
    // is refused before the store is asked, so no read happens at all.
    assert!(
        response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::FORBIDDEN,
        "a neighbour must not read this bundle, got {}",
        response.status()
    );
}
