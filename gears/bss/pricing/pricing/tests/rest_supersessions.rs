//! `POST /bss-pricing/v1/plans/{planId}/supersessions` over the real router
//! (`inst-su-api`, `inst-su-return`, D-88).
//!
//! What this suite owns that `sqlite_supersession_unit.rs` cannot: the **wire**. The
//! service-level suite drives `SupersessionService` directly and can say nothing about
//! the status code, the `outcome` token, which half of the view is null on which arm, or
//! whether the `{planId}` segment is checked against the row it names.
//!
//! The gate itself is not asserted here. `rest_authz.rs`'s census carries this route, so
//! `every_route_asks_the_catalogued_pair`, `a_pdp_outage_fails_closed_on_every_route` and
//! the deny properties all range over it — one set of gate properties for every route
//! rather than a per-suite copy free to be the weaker one.
//!
//! The commit arms here really do commit: `Harness` supplies a working
//! `RegistryDouble`. It is the **e2e stand** that has no `CatalogVersionRegistryV1`, so
//! there every commit arm is a 503 by design and what is asserted is that the arm was
//! reached — the asymmetry is the harness's, and it is why both suites exist.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::supersessions::PLAN_SUPERSESSIONS;
use bss_pricing::domain::approval::DecisionBy;
use bss_pricing::infra::approval::{DecideRequest, RegionGrant};
use chrono::{DateTime, TimeZone, Utc};
use rest_support::{Harness, Publishable, body_json, request, seed_publishable_plan};
use uuid::Uuid;

const SUBMITTER: Uuid = Uuid::from_u128(0x_5c_11);
const REVIEWER: Uuid = Uuid::from_u128(0x_5c_22);

/// Inside `common`'s `[2099-08-04, 2099-09-01)` coverage window, and clear of both of
/// `inst-su-instant`'s floors against a wall clock that is nowhere near 2099.
fn changeover() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 20, 0, 0, 0).unwrap()
}

fn path(plan_id: Uuid) -> String {
    PLAN_SUPERSESSIONS.replace("{planId}", &plan_id.to_string())
}

fn supersede_body(predecessor: Uuid, amount: i64) -> serde_json::Value {
    serde_json::json!({
        "predecessor_price_id": predecessor.to_string(),
        "changeover": changeover(),
        "successor": {
            "model_kind": "flat",
            "amount_minor": amount,
            "billing_timing": "advance",
            "rounding_policy_ref": "half_up"
        },
        "reason_code": "repricing"
    })
}

/// A published plan with one published row on the `eu` key.
async fn published(h: &Harness) -> (Uuid, Publishable) {
    let plan_id = Uuid::now_v7();
    let seeded = seed_publishable_plan(h, plan_id).await;
    h.publish(plan_id, seeded.revision).await;
    h.publish_price(plan_id, seeded.price_id).await;
    (plan_id, seeded)
}

async fn approve(h: &Harness, approval_id: Uuid) {
    h.governance
        .approvals
        .decide(
            &h.scope(),
            h.tenant,
            DecideRequest {
                approval_id,
                decision: DecisionBy::Approve(REVIEWER),
                reason: None,
                approver_regions: RegionGrant::Untransported,
                stamp: bss_pricing::domain::audit::AuditStamp {
                    actor_principal_id: REVIEWER,
                    recorded_at: Utc::now(),
                    correlation_id: Uuid::from_u128(0x_5c_c0),
                },
            },
        )
        .await
        .expect("the reviewer approves");
}

#[tokio::test]
async fn the_controlled_arm_answers_202_and_names_the_staged_successor() {
    let h = Harness::new().await;
    let (plan_id, seeded) = published(&h).await;

    let response = h
        .allowed_as(SUBMITTER)
        .send(request(
            "POST",
            &path(plan_id),
            Some(supersede_body(seeded.price_id, 12_000)),
        ))
        .await;
    // 202 on the arm that wrote no publish, for `inst-su-return`'s reason and the
    // window surface's: a publish unit is not consumer-visible until warm, so a 200
    // would claim the price changed for readers.
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let view = body_json(response).await;
    assert_eq!(view["outcome"], "submitted_for_approval");
    assert_eq!(view["predecessor_price_id"], seeded.price_id.to_string());
    // The staged draft is named on this arm too — it is the reviewer's subject, so
    // there is nothing to review until it exists.
    assert!(view["successor_price_id"].is_string());
    assert!(
        view["approval"]["approval_id"].is_string(),
        "the unit the call opened: {view}"
    );
    assert!(view["materiality"]["reason"].is_string());
    // And the halves that describe a commit are absent rather than zeroed.
    assert!(view["successor_window_id"].is_null());
    assert!(view["pending_version_ref"].is_null());
    assert!(view["shortened_mutation_seq"].is_null());
    // The window whose end *would* move is named, so a reviewer is shown the act
    // rather than the world it leaves.
    assert_eq!(
        view["shortened_window_id"],
        common::coverage_window_id(seeded.price_id).to_string()
    );
}

#[tokio::test]
async fn the_same_act_twice_over_the_wire_is_one_unit() {
    // `inst-su-api`'s idempotency, and this suite is where it is visible as a wire
    // property: there is **no `Idempotency-Key` header** — S5's column for this surface
    // is the act's own identity — so two identical requests must answer the same unit
    // rather than 409.
    let h = Harness::new().await;
    let (plan_id, seeded) = published(&h).await;

    let first = body_json(
        h.allowed_as(SUBMITTER)
            .send(request(
                "POST",
                &path(plan_id),
                Some(supersede_body(seeded.price_id, 12_000)),
            ))
            .await,
    )
    .await;
    let second_response = h
        .allowed_as(SUBMITTER)
        .send(request(
            "POST",
            &path(plan_id),
            Some(supersede_body(seeded.price_id, 12_000)),
        ))
        .await;
    assert_eq!(second_response.status(), StatusCode::ACCEPTED);
    let second = body_json(second_response).await;
    assert_eq!(
        second["approval"]["approval_id"], first["approval"]["approval_id"],
        "one act, one unit"
    );
    assert_eq!(second["successor_price_id"], first["successor_price_id"]);
}

#[tokio::test]
async fn the_same_act_with_different_content_is_409() {
    let h = Harness::new().await;
    let (plan_id, seeded) = published(&h).await;

    h.allowed_as(SUBMITTER)
        .send(request(
            "POST",
            &path(plan_id),
            Some(supersede_body(seeded.price_id, 12_000)),
        ))
        .await;
    let refused = h
        .allowed_as(SUBMITTER)
        .send(request(
            "POST",
            &path(plan_id),
            Some(supersede_body(seeded.price_id, 13_000)),
        ))
        .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        rest_support::problem_code(refused).await,
        "DUPLICATE_SCOPE_KEY"
    );
}

#[tokio::test]
async fn the_call_after_an_independent_approve_commits_and_answers_the_pending_handle() {
    let h = Harness::new().await;
    let (plan_id, seeded) = published(&h).await;

    let opened = body_json(
        h.allowed_as(SUBMITTER)
            .send(request(
                "POST",
                &path(plan_id),
                Some(supersede_body(seeded.price_id, 12_000)),
            ))
            .await,
    )
    .await;
    let unit: Uuid = opened["approval"]["approval_id"]
        .as_str()
        .expect("the unit id")
        .parse()
        .expect("a uuid");
    approve(&h, unit).await;

    let response = h
        .allowed_as(SUBMITTER)
        .send(request(
            "POST",
            &path(plan_id),
            Some(supersede_body(seeded.price_id, 12_000)),
        ))
        .await;
    // Still **202**, and that is the point of the token rather than the status: a
    // committed publish unit is not consumer-visible either.
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let view = body_json(response).await;
    assert_eq!(view["outcome"], "superseded");
    assert_eq!(
        view["successor_price_id"], opened["successor_price_id"],
        "the row the first call staged, never one this request minted"
    );
    assert!(view["pending_version_ref"].is_string());
    assert!(view["successor_window_id"].is_string());
    assert!(view["shortened_mutation_seq"].is_number());
    // And the review's halves are absent on this arm.
    assert!(view["approval"].is_null());
    assert!(view["materiality"].is_null());
}

#[tokio::test]
async fn a_row_of_another_plan_answers_404() {
    // The `{planId}` segment is checked against the row's own plan, which is
    // `patch_price`'s decision for the same shape: a 400 would confirm the row exists
    // somewhere else.
    let h = Harness::new().await;
    let (_, seeded) = published(&h).await;
    let (other_plan, _) = published(&h).await;

    let response = h
        .allowed_as(SUBMITTER)
        .send(request(
            "POST",
            &path(other_plan),
            Some(supersede_body(seeded.price_id, 12_000)),
        ))
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_unknown_predecessor_answers_404() {
    let h = Harness::new().await;
    let (plan_id, _) = published(&h).await;

    let response = h
        .allowed_as(SUBMITTER)
        .send(request(
            "POST",
            &path(plan_id),
            Some(supersede_body(Uuid::now_v7(), 12_000)),
        ))
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_dormant_changeover_is_400_carrying_its_code() {
    // An architectural refusal reaching the wire, which is the property this suite owns:
    // the design set's 422s have no canonical category, so they arrive as **400 carrying
    // the code** (Foundation section 3.3), and `module_test::no_operation_declares_a_422`
    // is that rule's other half.
    //
    // The refusal staged is **dormancy** and not the changeover floor, and that is a
    // limit of the wire rather than a preference. The route stamps `Utc::now()`, so an
    // instant inside `MAX_BATCHING_DELAY` of it is a 2026 instant, and the fixtures'
    // coverage window is in 2099 — `plan_supersession` answers the plane before the
    // commit floor is ever asked, so the floor is unreachable from here. It is pinned at
    // the service level, where the stamp is a constant
    // (`sqlite_supersession_unit::the_commit_floor_is_answered_before_any_catalog_version_is_requested`),
    // and the first version of this case asserted the floor and was answered by
    // dormancy — then, one fix earlier, by `TIMESTAMP_PRECISION_EXCEEDED`, because
    // `Utc::now()` carries microseconds and D-144's quantum is checked first.
    let h = Harness::new().await;
    rest_support::approve_threshold_policy(&h, &[("EUR", 1_000_000)]).await;
    let (plan_id, seeded) = published(&h).await;

    let mut body = supersede_body(seeded.price_id, 10_000);
    body["changeover"] = serde_json::json!(common::coverage_to() + chrono::Duration::days(30));
    let response = h
        .allowed_as(SUBMITTER)
        .send(request("POST", &path(plan_id), Some(body)))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        rest_support::problem_code(response).await,
        "LIFECYCLE_FORBIDDEN"
    );
}
