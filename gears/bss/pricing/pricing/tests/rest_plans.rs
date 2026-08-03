//! The plan plane, driven through the real router.
//!
//! `tower::ServiceExt::oneshot` over the router `register_rest` mounts, so the
//! extractors, the PEP gate, the repositories and the response serialization are
//! all in the path — the same harness `tests/rest_frontier.rs` established, over
//! a migrated in-memory `SQLite`.
//!
//! Two habits run through every case here and both are deliberate. A denial
//! asserts **403 and an unchanged database**, because a 403 alone would also be
//! produced by a handler that wrote first and checked second — "the authz gate
//! before the repository" is only a claim about source order until somebody
//! observes the store. And a refusal asserts its **code**, never only its
//! status: §3.3 makes the code the discriminator a consumer matches on, and
//! several distinct refusals share one status.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::plans::PLANS;
use rest_support::{
    Harness, body_json, etag_of, location_of, plan_count, plan_row_version, plan_state,
    problem_code, request, seed_current_plan, seed_draft_plan, with_headers,
};
use uuid::Uuid;

fn plan_path(plan_id: Uuid) -> String {
    format!("{PLANS}/{plan_id}")
}

#[tokio::test]
async fn an_unauthenticated_read_is_refused_before_anything_else() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();

    let response = harness
        .anonymous()
        .send(request("GET", &plan_path(plan_id), None))
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_denied_read_is_403_and_the_row_is_untouched() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;
    let before = plan_row_version(&harness, plan_id, 0).await;

    let response = harness
        .denied()
        .send(request("GET", &plan_path(plan_id), None))
        .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        plan_row_version(&harness, plan_id, 0).await,
        before,
        "a denied read must not have reached the repository at all"
    );
}

#[tokio::test]
async fn the_open_draft_is_what_an_author_is_answered() {
    // A plan with both a current revision and an open draft: the author is
    // editing the draft, and a read that answered the published revision would
    // hand them a body their next PATCH could not match.
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_current_plan(&harness, plan_id).await;
    let response = harness
        .allowed()
        .send(request("GET", &plan_path(plan_id), None))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let published = body_json(response).await;
    assert_eq!(published["lifecycle_state"], serde_json::json!("published"));
    assert_eq!(published["revision"], serde_json::json!(0));

    harness.open_successor(plan_id).await;

    let response = harness
        .allowed()
        .send(request("GET", &plan_path(plan_id), None))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let draft = body_json(response).await;
    assert_eq!(draft["lifecycle_state"], serde_json::json!("draft"));
    assert_eq!(draft["revision"], serde_json::json!(1));
}

#[tokio::test]
async fn the_read_carries_the_revisions_child_sets_and_its_etag() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;
    harness.attach_shape(plan_id, 0).await;

    let response = harness
        .allowed()
        .send(request("GET", &plan_path(plan_id), None))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let tag = etag_of(&response);
    let body = body_json(response).await;
    assert_eq!(
        tag,
        Some(format!(
            "\"{}-{}\"",
            body["revision"].as_u64().unwrap(),
            body["row_version"].as_u64().unwrap()
        )),
        "a caller that cannot obtain a tag cannot satisfy the next verb's precondition"
    );
    assert_eq!(
        body["phases"].as_array().map(Vec::len),
        Some(1),
        "S2 §5's PATCH names four facets, so a read that omits them cannot round-trip one: {body}"
    );
    assert_eq!(
        body["addon_rules"].as_array().map(Vec::len),
        Some(1),
        "{body}"
    );
    assert!(body["descriptor_set"].is_object(), "{body}");
}

#[tokio::test]
async fn an_absent_plan_and_a_foreign_tenants_plan_answer_identically() {
    // The owner's 200 is the baseline: without it the 404 below would be
    // consistent with the plan simply not existing, and the test would prove
    // nothing about isolation.
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;

    let owner = harness
        .allowed()
        .send(request("GET", &plan_path(plan_id), None))
        .await;
    assert_eq!(
        owner.status(),
        StatusCode::OK,
        "the baseline: it is readable"
    );

    let foreign = harness
        .other_tenant()
        .send(request("GET", &plan_path(plan_id), None))
        .await;
    let absent = harness
        .allowed()
        .send(request("GET", &plan_path(Uuid::now_v7()), None))
        .await;

    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        absent.status(),
        StatusCode::NOT_FOUND,
        "absent and out-of-scope are one answer, or the surface leaks whose catalog exists"
    );
}

#[tokio::test]
async fn a_plan_whose_only_revision_is_abandoned_is_not_readable() {
    // A read is not one of the four authoring calls S2 §5 owes
    // PLAN_ABANDONED_NO_SUCCESSOR: the plan holds no revision an author can act
    // on, which is the 404 this gear already gives for "nothing here for you".
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;
    harness.abandon_draft(plan_id, 0).await;

    let response = harness
        .allowed()
        .send(request("GET", &plan_path(plan_id), None))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// The writes.
// ---------------------------------------------------------------------------

/// A minimal well-formed create body.
fn create_body(tier: &str) -> serde_json::Value {
    serde_json::json!({ "plan_tier": tier, "billing_cycle": "recurring" })
}

fn keyed(key: &str) -> Vec<(&str, &str)> {
    vec![("idempotency-key", key)]
}

#[tokio::test]
async fn a_create_answers_201_with_the_location_and_the_tag_a_caller_needs_next() {
    let harness = Harness::new().await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            PLANS,
            Some(create_body("gold")),
            &keyed("create-1"),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let location = location_of(&response);
    let tag = etag_of(&response);
    let body = body_json(response).await;
    let plan_id = body["plan_id"].as_str().expect("the id is answered");
    assert_eq!(
        location,
        Some(format!("{PLANS}/{plan_id}")),
        "the surface mints the id and has to return it before the row is durable"
    );
    assert_eq!(
        tag,
        Some("\"0-0\"".to_owned()),
        "revision 0 at version 0: the tag names its subject, so the caller can edit exactly \
         the revision it was just handed"
    );
    assert_eq!(body["revision"], serde_json::json!(0));
    assert_eq!(body["lifecycle_state"], serde_json::json!("draft"));
}

#[tokio::test]
async fn a_replayed_create_answers_the_original_id_and_creates_nothing() {
    // The whole reason the response body is recorded: a retry must be told the
    // plan id the FIRST call minted, or the caller ends up with two plans and a
    // reference to neither.
    let harness = Harness::new().await;
    let first = harness
        .allowed()
        .send(with_headers(
            "POST",
            PLANS,
            Some(create_body("gold")),
            &keyed("create-2"),
        ))
        .await;
    let first_id = body_json(first).await["plan_id"].clone();

    let replay = harness
        .allowed()
        .send(with_headers(
            "POST",
            PLANS,
            Some(create_body("gold")),
            &keyed("create-2"),
        ))
        .await;

    assert_eq!(replay.status(), StatusCode::CREATED);
    let replayed = body_json(replay).await;
    assert_eq!(
        replayed["plan_id"], first_id,
        "a replay answers the recorded body, id and all"
    );
    assert_eq!(
        plan_count(&harness).await,
        1,
        "an answered key must not have run its mutation again"
    );
}

#[tokio::test]
async fn one_key_with_two_different_bodies_is_refused_by_its_code() {
    let harness = Harness::new().await;
    harness
        .allowed()
        .send(with_headers(
            "POST",
            PLANS,
            Some(create_body("gold")),
            &keyed("create-3"),
        ))
        .await;

    let clash = harness
        .allowed()
        .send(with_headers(
            "POST",
            PLANS,
            Some(create_body("platinum")),
            &keyed("create-3"),
        ))
        .await;

    assert_eq!(clash.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(clash).await, "IDEMPOTENCY_PAYLOAD_MISMATCH");
    assert_eq!(plan_count(&harness).await, 1);
}

#[tokio::test]
async fn a_create_without_an_idempotency_key_is_refused_and_writes_nothing() {
    // An unguarded create on a governed authoring plane is the retry hazard the
    // gate exists for; S2 §5 gives this verb the client-key cell, so absent is a
    // malformed request rather than an unguarded execution.
    let harness = Harness::new().await;

    let response = harness
        .allowed()
        .send(request("POST", PLANS, Some(create_body("gold"))))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(plan_count(&harness).await, 0);
}

#[tokio::test]
async fn a_patch_under_a_stale_tag_is_refused_by_its_code_and_the_row_does_not_move() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;
    let before = plan_row_version(&harness, plan_id, 0).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            &[("if-match", "\"0-9\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(response).await, "STALE_VERSION");
    assert_eq!(
        plan_row_version(&harness, plan_id, 0).await,
        before,
        "a refused precondition leaves the row's tag exactly where it was"
    );
}

#[tokio::test]
async fn a_patch_without_a_precondition_is_a_malformed_request() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;

    let response = harness
        .allowed()
        .send(request(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
        ))
        .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "D-141: an absent precondition mints no code of its own"
    );
    assert_eq!(plan_row_version(&harness, plan_id, 0).await, Some(0));
}

#[tokio::test]
async fn a_patch_naming_two_facets_is_refused_rather_than_half_applied() {
    // Each facet compare-and-swaps on the revision's own row version and bumps
    // it, so the second could not match the tag the first advanced - and the two
    // are separate transactions with a visible half-applied state in between.
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({
                "shape": { "plan_tier": "platinum" },
                "addon_rules": []
            })),
            &[("if-match", "\"0-0\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        plan_row_version(&harness, plan_id, 0).await,
        Some(0),
        "neither facet may have landed"
    );
}

#[tokio::test]
async fn a_patch_on_a_plan_with_no_open_draft_opens_the_successor() {
    // D-146's own sentence requires this arm to exist. The successor's number
    // comes from the whole chain, and the patch lands on the newly opened
    // revision rather than on the frozen one the caller read.
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_current_plan(&harness, plan_id).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            &[("if-match", "\"0-1\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["revision"], serde_json::json!(1), "{body}");
    assert_eq!(
        body["lifecycle_state"],
        serde_json::json!("draft"),
        "{body}"
    );
    assert_eq!(body["plan_tier"], serde_json::json!("platinum"), "{body}");
    assert_eq!(
        plan_state(&harness, plan_id, 0).await.as_deref(),
        Some("published"),
        "the predecessor is untouched until the successor publishes"
    );
}

#[tokio::test]
async fn a_patch_on_the_successor_arm_asserts_the_current_revisions_tag() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_current_plan(&harness, plan_id).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            &[("if-match", "\"0-0\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(response).await, "STALE_VERSION");
    assert_eq!(
        plan_state(&harness, plan_id, 1).await,
        None,
        "no successor may be opened under a precondition that did not hold"
    );
}

#[tokio::test]
async fn a_patch_on_a_retired_plan_is_a_stop_and_says_so_in_its_own_words() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_current_plan(&harness, plan_id).await;
    harness.retire(plan_id, 0).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            &[("if-match", "\"0-1\"")],
        ))
        .await;

    // The design set calls this a 422 and it arrives as a 400 carrying its code:
    // §3.3's status-rendering rule, asserted end to end for the first time.
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "the platform has no 422 category; the code is the discriminator"
    );
    assert_eq!(problem_code(response).await, "PLAN_RETIRED_NO_SUCCESSOR");
}

/// The world in which the two version counters **coincide**, which is the only
/// world where a revision binding is the thing doing the refusing.
///
/// Seed a draft at revision 0, version 0; publish it, so revision 0 moves to
/// version 1; open the successor, so revision 1 stands at version 0. The plan
/// now holds a published revision 0 at version 1 and an open draft revision 1 at
/// version **0** — exactly where revision 0 stood when the caller read it.
///
/// A tag of `"0-0"` is therefore **version-correct for the draft and
/// revision-wrong**: the compare-and-swap it would produce (`expected = 0`
/// against revision 1, which stands at 0) *succeeds*. Nothing but the revision
/// binding can refuse it. A staging where the versions differ — say `"0-1"`
/// against a draft at 0 — is answered 409 by the plain staleness test whether
/// the binding exists or not, which is a test that passes for a reason other
/// than the property it names.
async fn plan_with_a_successor_at_the_predecessors_version(harness: &Harness) -> Uuid {
    let plan_id = Uuid::now_v7();
    seed_current_plan(harness, plan_id).await;
    harness.open_successor(plan_id).await;
    assert_eq!(
        (
            plan_row_version(harness, plan_id, 0).await,
            plan_row_version(harness, plan_id, 1).await
        ),
        (Some(1), Some(0)),
        "the staging is the test: revision 1 must stand exactly where revision 0 was read"
    );
    plan_id
}

#[tokio::test]
async fn a_tag_read_from_one_revision_cannot_edit_the_open_draft() {
    // The lost update D-145 spends two paragraphs eliminating, arriving through
    // the **revision** instead of the version - and reachable with no race at
    // all, because `/plans/{planId}` names no revision and the two counters are
    // unrelated. This is `target_revision`'s open-draft arm.
    let harness = Harness::new().await;
    let plan_id = plan_with_a_successor_at_the_predecessors_version(&harness).await;

    let wrong_subject = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            &[("if-match", "\"0-0\"")],
        ))
        .await;

    assert_eq!(
        wrong_subject.status(),
        StatusCode::CONFLICT,
        "the version is right for the draft and the revision is not; only the binding refuses"
    );
    assert_eq!(problem_code(wrong_subject).await, "STALE_VERSION");
    assert_eq!(
        plan_row_version(&harness, plan_id, 1).await,
        Some(0),
        "the successor must not have moved under a tag read from its predecessor"
    );

    // The tag that names the successor is the one that edits it.
    let correct = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            &[("if-match", "\"1-0\"")],
        ))
        .await;

    assert_eq!(correct.status(), StatusCode::OK);
    let body = body_json(correct).await;
    assert_eq!(body["revision"], serde_json::json!(1), "{body}");
    assert_eq!(body["plan_tier"], serde_json::json!("platinum"), "{body}");
}

#[tokio::test]
async fn a_tag_read_from_one_revision_cannot_abandon_another() {
    // The same staging against `abandon_plan_draft`'s own binding. It needs its
    // own case: the abandon path resolves the draft itself and never goes
    // through `target_revision`, so the patch arm's guard says nothing about it.
    // What an unbound tag destroys here is a whole revision.
    let harness = Harness::new().await;
    let plan_id = plan_with_a_successor_at_the_predecessors_version(&harness).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &format!("{PLANS}/{plan_id}/abandon"),
            None,
            &[("if-match", "\"0-0\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(response).await, "STALE_VERSION");
    assert_eq!(
        plan_state(&harness, plan_id, 1).await.as_deref(),
        Some("draft"),
        "a tag read from revision 0 must not tombstone revision 1"
    );

    // The tag that names the draft is the one that discards it.
    let correct = harness
        .allowed()
        .send(with_headers(
            "POST",
            &format!("{PLANS}/{plan_id}/abandon"),
            None,
            &[("if-match", "\"1-0\"")],
        ))
        .await;

    assert_eq!(correct.status(), StatusCode::OK);
    assert_eq!(
        plan_state(&harness, plan_id, 1).await.as_deref(),
        Some("abandoned")
    );
}

#[tokio::test]
async fn a_tag_read_from_a_tombstone_cannot_open_a_successor() {
    // `target_revision`'s successor arm, staged so the versions coincide there
    // too. Seed, publish (revision 0 -> version 1), open the successor, then
    // abandon it (revision 1 -> version 1). The plan now holds **no** open draft,
    // a current revision 0 at version 1, and a tombstone revision 1 at version 1.
    //
    // A tag of `"1-1"` is the tombstone's own, and its version is exactly the
    // current revision's - so the staleness test passes and only the revision
    // binding can refuse. Without it, a successor opens under a tag read from a
    // revision that was discarded.
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_current_plan(&harness, plan_id).await;
    harness.open_successor(plan_id).await;
    harness.abandon_draft(plan_id, 1).await;
    assert_eq!(
        (
            plan_row_version(&harness, plan_id, 0).await,
            plan_row_version(&harness, plan_id, 1).await
        ),
        (Some(1), Some(1)),
        "the staging is the test: the tombstone must stand exactly where the current revision does"
    );

    let response = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            &[("if-match", "\"1-1\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(response).await, "STALE_VERSION");
    assert_eq!(
        plan_state(&harness, plan_id, 2).await,
        None,
        "no successor may be opened under a tag read from a revision that is not the current one"
    );

    // The tag that names the current revision opens the successor.
    let correct = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            &[("if-match", "\"0-1\"")],
        ))
        .await;

    assert_eq!(correct.status(), StatusCode::OK);
    assert_eq!(
        body_json(correct).await["revision"],
        serde_json::json!(2),
        "revision 2, because 1 stays consumed (D-145)"
    );
}

#[tokio::test]
async fn a_read_hands_back_a_tag_that_names_the_revision_it_answered() {
    // The other half: a caller can only obtain a revision-qualified tag if the
    // read emits one, so a `GET` that answered a bare version would make every
    // mutating verb unsatisfiable.
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_current_plan(&harness, plan_id).await;
    harness.open_successor(plan_id).await;

    let response = harness
        .allowed()
        .send(request("GET", &plan_path(plan_id), None))
        .await;

    assert_eq!(
        etag_of(&response),
        Some("\"1-0\"".to_owned()),
        "the tag names the revision the read answered, not just its version"
    );
}

#[tokio::test]
async fn an_unqualified_tag_is_a_malformed_request_on_a_plan_route() {
    // A bare version is what a caller of the price plane sends, and what this
    // surface used to accept. It has to be refused rather than read as revision
    // 0, or the whole distinction is advisory.
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;

    for raw in ["\"0\"", "\"0-\"", "\"-0\"", "\"0-0-0\"", "*", "W/\"0-0\""] {
        let response = harness
            .allowed()
            .send(with_headers(
                "PATCH",
                &plan_path(plan_id),
                Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
                &[("if-match", raw)],
            ))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{raw} must not be readable as a plan precondition"
        );
    }
    assert_eq!(
        plan_row_version(&harness, plan_id, 0).await,
        Some(0),
        "none of them may have edited anything"
    );
}

#[tokio::test]
async fn an_abandon_tombstones_the_draft_and_keeps_its_number() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &format!("{PLANS}/{plan_id}/abandon"),
            None,
            &[("if-match", "\"0-0\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["lifecycle_state"], serde_json::json!("abandoned"));
    assert_eq!(
        plan_state(&harness, plan_id, 0).await.as_deref(),
        Some("abandoned"),
        "the row is flipped, never deleted: the number it consumed stays consumed (D-145)"
    );
}

#[tokio::test]
async fn abandoning_a_plan_with_no_open_draft_describes_no_alternative_action() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_current_plan(&harness, plan_id).await;

    let response = harness
        .allowed()
        .send(with_headers(
            "POST",
            &format!("{PLANS}/{plan_id}/abandon"),
            None,
            &[("if-match", "\"0-1\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(response).await, "LIFECYCLE_FORBIDDEN");
}

#[tokio::test]
async fn every_authoring_verb_on_a_spent_plan_says_the_id_is_spent() {
    // The refusal S2 §5 explicitly owed this group. Before it, a plan whose only
    // revision is abandoned answered PATCH and abandon with a not-found - which
    // tells an operator to look for a plan that is right there.
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;
    harness.abandon_draft(plan_id, 0).await;

    let patched = harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            &[("if-match", "\"0-1\"")],
        ))
        .await;
    assert_eq!(patched.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(patched).await, "PLAN_ABANDONED_NO_SUCCESSOR");

    let abandoned = harness
        .allowed()
        .send(with_headers(
            "POST",
            &format!("{PLANS}/{plan_id}/abandon"),
            None,
            &[("if-match", "\"0-1\"")],
        ))
        .await;
    assert_eq!(abandoned.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(abandoned).await, "PLAN_ABANDONED_NO_SUCCESSOR");
}

#[tokio::test]
async fn every_write_is_denied_with_the_database_unchanged() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&harness, plan_id).await;
    let before = plan_row_version(&harness, plan_id, 0).await;

    for (method, path, body, headers) in [
        (
            "POST",
            PLANS.to_owned(),
            Some(create_body("gold")),
            vec![("idempotency-key", "denied-1")],
        ),
        (
            "PATCH",
            plan_path(plan_id),
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            vec![("if-match", "\"0-0\"")],
        ),
        (
            "POST",
            format!("{PLANS}/{plan_id}/abandon"),
            None,
            vec![("if-match", "\"0-0\"")],
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

    assert_eq!(
        plan_row_version(&harness, plan_id, 0).await,
        before,
        "the gate runs before the repository, which is only observable as an unchanged store"
    );
    assert_eq!(
        plan_count(&harness).await,
        1,
        "a denied create must leave no row behind"
    );
}

#[tokio::test]
async fn a_write_aimed_at_a_tenant_outside_the_compiled_scope_is_denied() {
    // `access_scope`'s membership assertion: the degraded flat-`In` decision does
    // not re-check `owner_tenant_id`, so a write anchored to a tenant the caller
    // is not authorized for has to be refused there or nowhere. The caller here
    // is authenticated in this tenant and authorized only for the other one.
    let harness = Harness::new().await;

    let response = harness
        .scope_mismatch()
        .send(with_headers(
            "POST",
            PLANS,
            Some(create_body("gold")),
            &keyed("cross-1"),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        plan_count(&harness).await,
        0,
        "a cross-tenant write must leave no row behind"
    );
}
