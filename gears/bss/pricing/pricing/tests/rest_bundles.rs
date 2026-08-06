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
use rest_support::{Harness, body_json, etag_of, problem_code, seed_draft_plan, with_headers};
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

/// A composition with nothing to refuse publishes, answers **202**, and says the
/// verdict was `alwaysMaterialTrigger` (D-104) — stated in the body because an
/// operator who expected auto-publish under a configured threshold needs the
/// reason and not the outcome alone.
#[tokio::test]
async fn a_clean_publish_is_accepted_and_is_always_material() {
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

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = body_json(response).await;
    assert_eq!(
        body["materiality"].as_str(),
        Some("alwaysMaterialTrigger"),
        "D-104: a composition publish is material whatever a threshold says"
    );
}
