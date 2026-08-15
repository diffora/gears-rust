//! `GET/PUT /bss-pricing/v1/config/rounding-policy` — the tenant's default
//! rounding policy (PRD §17.4, D-320).
//!
//! # What is under test, and why the last case is the point of the surface
//!
//! Four of these pin the resource's own contract — unset reads as a state and
//! not an absence, a `PUT` under the read tag lands, a stale tag writes nothing,
//! blank is refused so that unset keeps one spelling. The fifth pins the reason
//! the route was built: with a default set, a plan whose rows carry **no**
//! `roundingPolicyRef` publishes, where before this surface existed the same
//! plan could not — `default_rounding_policy_ref` had no writer, so
//! `foundation.rounding_policy_resolved` always took its fail-closed arm and
//! every row of every plan had to carry its own.
//!
//! Without that last case the suite would prove a policy row can be written and
//! nothing about what writing it *does*, which is the shape of a green test over
//! an inert feature.

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::rounding_policy::ROUNDING_POLICY;
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::storage::repo::NewPriceDraft;
use rest_support::{
    Harness, body_json, etag_of, problem_code, publishable_row, publishable_scope_key,
    seed_publishable_shape, with_headers,
};
use uuid::Uuid;

/// The submitting principal.
const SUBMITTER: Uuid = Uuid::from_u128(0x5_7d);

// ---------------------------------------------------------------------------
// The surface itself.
// ---------------------------------------------------------------------------

async fn read_policy(harness: &Harness) -> (StatusCode, Option<String>, serde_json::Value) {
    let response = harness
        .allowed()
        .send(with_headers("GET", ROUNDING_POLICY, None, &[]))
        .await;
    let status = response.status();
    let tag = etag_of(&response);
    (status, tag, body_json(response).await)
}

async fn write_policy(
    harness: &Harness,
    value: serde_json::Value,
    tag: &str,
) -> axum::http::Response<axum::body::Body> {
    harness
        .allowed()
        .send(with_headers(
            "PUT",
            ROUNDING_POLICY,
            Some(serde_json::json!({ "default_rounding_policy_ref": value })),
            &[("if-match", tag)],
        ))
        .await
}

/// A tenant that has set nothing is answered **200 with `null`**, and it carries
/// a tag.
///
/// Not a 404, for the tax-display surface's reason: unset is a state — the one
/// every tenant is in — and answering an absence would make the bootstrap `PUT`
/// unaskable, there being no tag to assert.
#[tokio::test]
async fn a_tenant_that_set_nothing_reads_null_with_a_tag() {
    let harness = Harness::new().await;

    let (status, tag, body) = read_policy(&harness).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["default_rounding_policy_ref"], serde_json::Value::Null);
    assert!(tag.is_some(), "unset still has a representation to assert");
}

/// A `PUT` under the tag the `GET` handed back lands, and the `GET` agrees.
#[tokio::test]
async fn a_put_under_the_read_tag_sets_the_default_and_the_get_agrees() {
    let harness = Harness::new().await;
    let (_, tag, _) = read_policy(&harness).await;

    let response = write_policy(
        &harness,
        serde_json::json!("half_up_2dp"),
        &tag.expect("a tag"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await["default_rounding_policy_ref"],
        serde_json::json!("half_up_2dp")
    );

    let (_, _, body) = read_policy(&harness).await;
    assert_eq!(
        body["default_rounding_policy_ref"],
        serde_json::json!("half_up_2dp"),
        "the write is what the next reader sees, not just what the response said"
    );
}

/// The default can be cleared, and clearing is spelled `null`.
///
/// The state matters: a tenant who clears goes back to needing a reference on
/// every published row, so this is a real setting and not a one-way door.
#[tokio::test]
async fn the_default_can_be_cleared_back_to_null() {
    let harness = Harness::new().await;
    let (_, tag, _) = read_policy(&harness).await;
    write_policy(
        &harness,
        serde_json::json!("half_up_2dp"),
        &tag.expect("a tag"),
    )
    .await;

    let (_, set_tag, _) = read_policy(&harness).await;
    let response = write_policy(&harness, serde_json::Value::Null, &set_tag.expect("a tag")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let (_, _, body) = read_policy(&harness).await;
    assert_eq!(body["default_rounding_policy_ref"], serde_json::Value::Null);
}

/// A tag that no longer describes the stored default is refused, and nothing is
/// written.
///
/// The second half is the load-bearing one: a refusal that had already written
/// would be the lost update the precondition exists to prevent.
#[tokio::test]
async fn a_stale_tag_is_refused_and_writes_nothing() {
    let harness = Harness::new().await;
    let (_, first_tag, _) = read_policy(&harness).await;
    let first_tag = first_tag.expect("a tag");
    write_policy(&harness, serde_json::json!("half_up_2dp"), &first_tag).await;

    // The same tag again: it described the unset state, which has moved.
    let response = write_policy(&harness, serde_json::json!("bankers"), &first_tag).await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(response).await, "STALE_VERSION");
    let (_, _, body) = read_policy(&harness).await;
    assert_eq!(
        body["default_rounding_policy_ref"],
        serde_json::json!("half_up_2dp"),
        "a refused precondition leaves the stored default exactly where it was"
    );
}

/// A blank reference is refused rather than stored beside `null`.
///
/// D-318's rule on `planName`, applied to the field one surface over: two
/// spellings of unset is a state every reader has to special-case, and the first
/// that forgets shows a default that is there and means nothing.
#[tokio::test]
async fn a_blank_reference_is_refused() {
    let harness = Harness::new().await;
    let (_, tag, _) = read_policy(&harness).await;

    let response = write_policy(&harness, serde_json::json!("   "), &tag.expect("a tag")).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let (_, _, body) = read_policy(&harness).await;
    assert_eq!(
        body["default_rounding_policy_ref"],
        serde_json::Value::Null,
        "the refusal wrote nothing"
    );
}

/// **The reason the surface exists**: with a default set, a plan whose rows
/// carry no `roundingPolicyRef` publishes.
///
/// Before this route `default_rounding_policy_ref` had no writer at all, so
/// `foundation.rounding_policy_resolved` always took its fail-closed arm and
/// this same plan answered `ROUNDING_POLICY_UNRESOLVED`. The assertion is
/// therefore about the rule's **other** arm, which nothing in this crate could
/// reach — and the refusal is asserted first, so the case cannot pass by the
/// plan having been publishable all along.
#[tokio::test]
async fn with_a_default_set_a_plan_whose_rows_have_no_ref_publishes() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    let shape = seed_publishable_shape(&harness, plan_id).await;
    let plan = PlanId::new(plan_id);
    let scope = harness.scope();

    // One row, publishable in every respect but the rounding reference.
    let price_id = Uuid::now_v7();
    harness
        .state
        .prices
        .create_draft(
            &scope,
            harness.tenant,
            NewPriceDraft {
                price_id,
                scope_key: publishable_scope_key(plan, shape.phase, "eu"),
                content: PriceContent {
                    rounding_policy_ref: None,
                    ..publishable_row()
                },
                created_by: rest_support::SEED_ACTOR,
                created_at_utc: rest_support::at(10),
                correlation_id: Uuid::from_u128(0x_c0_11_a7_11),
            },
        )
        .await
        .expect("author the row");

    // `inst-wc-required`: no row publishes without a window on its canonical key.
    let conn = harness.state.db.conn().expect("conn");
    common::schedule_coverage_window(
        &conn,
        &scope,
        harness.tenant,
        price_id,
        rest_support::seed_stamp(),
    )
    .await;

    let refused = publish(&harness, plan_id, &shape.etag()).await;
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "with no tenant default and no per-row ref, publish must refuse"
    );
    let detail = body_json(refused).await.to_string();
    assert!(
        detail.contains("ROUNDING_POLICY_UNRESOLVED"),
        "the refusal names the rounding rule; got {detail}"
    );

    let (_, tag, _) = read_policy(&harness).await;
    let set = write_policy(
        &harness,
        serde_json::json!("half_up_2dp"),
        &tag.expect("a tag"),
    )
    .await;
    assert_eq!(set.status(), StatusCode::OK);

    let after = publish(&harness, plan_id, &shape.etag()).await;
    let status = after.status();
    let body = body_json(after).await;
    // **202 and an opened unit**, not merely "the code is absent". A publish that
    // failed for some other reason would satisfy an absence assertion just as
    // well, and the claim is that the plan now clears the whole rule set.
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the tenant default resolves every row that carries none; got {body}"
    );
    assert_eq!(body["outcome"], serde_json::json!("submitted_for_approval"));
}

async fn publish(
    harness: &Harness,
    plan_id: Uuid,
    tag: &str,
) -> axum::http::Response<axum::body::Body> {
    harness
        .allowed_as(SUBMITTER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/plans/{plan_id}/publish"),
            None,
            &[("if-match", tag)],
        ))
        .await
}
