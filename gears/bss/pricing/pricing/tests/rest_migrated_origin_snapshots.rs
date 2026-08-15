//! Synthesis end to end, and `GET
//! /bss-pricing/v1/migrated-origin-snapshots/{subscriptionRef}` over the wire
//! (`inst-sy-freeze`, `inst-sy-select`, `inst-sy-payload`, `inst-sy-provenance`,
//! `inst-sy-surface`, `inst-sy-firstrating`, D-76, D-81, D-87, D-102).
//!
//! The domain suite proves the **rule** — tier order, the fail-closed clause, the
//! trigger tokens. What only this file can prove is the **reader**: that tier 1
//! actually finds the `pricing_price` row whose window covered `t`, and that it
//! stops finding it on the other side of the interval. Those are properties of a
//! query over real windows, and no unit test over hand-built candidates can see
//! them — a reader that ignored `effective_from` entirely would pass every case
//! in `domain::synthesis`.
//!
//! # The covering window is the publish path's own, and that was a finding
//!
//! This file first seeded one with `rest_support::seed_window`. Every case
//! reddened with `WindowOverlap` — a **driver** refusal rather than an assertion,
//! which is the signal that a guard exists that the fixture did not know about.
//! It does: `Harness::publish_price` already schedules `[2099-08-04,
//! 2099-09-01)` on the published row, so the extra open-ended window collided
//! with it on the same canonical scope key. The fixture uses the real one
//! instead. Both instants below are therefore facts about a window the
//! production path created, not about one the test invented, and neither ages -
//! `rest_support`'s standing rule is that fixture windows are dated 2099 so no
//! wall clock reaches them and no activation sweep moves them.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::migrated_origin_snapshots::MIGRATED_ORIGIN_SNAPSHOT;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::plan_shape::PeriodFloorCap;
use bss_pricing::domain::scope_key::{PlanId, Region};
use bss_pricing::domain::synthesis::SynthesisTrigger;
use bss_pricing::infra::synthesis::{FrozenKey, SynthesisRequest};
use chrono::{DateTime, TimeZone, Utc};
use rest_support::{Harness, body_json, request, seed_publishable_plan, seed_stamp};
use uuid::Uuid;

const RATING_SERVICE: Uuid = Uuid::from_u128(0x_4a_71_46);

fn path(subscription_ref: Uuid) -> String {
    MIGRATED_ORIGIN_SNAPSHOT.replace("{subscriptionRef}", &subscription_ref.to_string())
}

/// Inside `[2099-08-04, 2099-09-01)` — the window the publish path schedules.
fn covered_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 15, 0, 0, 0).unwrap()
}

/// A second instant inside the same window, for the idempotency case.
fn also_covered_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 20, 0, 0, 0).unwrap()
}

/// **Before** the window opens, by seven weeks. Deliberately just outside rather
/// than a century away: what is under test is the interval bound, and a distant
/// instant would pass against a reader that compared nothing at all.
fn before_the_window() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 6, 15, 0, 0, 0).unwrap()
}

/// The key `rest_support`'s publishable seed files its row under.
fn seeded_key() -> FrozenKey {
    FrozenKey {
        currency: "EUR".to_owned(),
        region: "eu".to_owned(),
    }
}

/// A published plan whose one price row carries the publish path's own window.
async fn covered_plan(h: &Harness) -> Uuid {
    let plan_id = Uuid::now_v7();
    let seeded = seed_publishable_plan(h, plan_id).await;
    h.publish(plan_id, seeded.revision).await;
    // This schedules `[2099-08-04, 2099-09-01)` on the row. See the module doc.
    h.publish_price(plan_id, seeded.price_id).await;
    plan_id
}

fn synthesis_request(subscription: Uuid, plan_id: Uuid, at: DateTime<Utc>) -> SynthesisRequest {
    SynthesisRequest {
        subscription_ref: subscription,
        source_plan_id: PlanId::new(plan_id),
        keys: vec![seeded_key()],
        at,
        trigger: SynthesisTrigger::Migration,
        acting_principal: RATING_SERVICE,
    }
}

// ---------------------------------------------------------------------------
// The reader — tier 1 over real windows.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_instant_the_window_covers_resolves_from_live_history() {
    // D-76 tier 1: the `pricing_price` row whose `PriceWindow` covered `t`. This
    // reproduces what rating would have resolved and needs no import at all.
    let h = Harness::new().await;
    let plan_id = covered_plan(&h).await;
    let subscription = Uuid::now_v7();

    let frozen = h
        .governance
        .synthesis
        .synthesize(
            &h.scope(),
            h.tenant,
            synthesis_request(subscription, plan_id, covered_at()),
        )
        .await
        .expect("the instant is covered");

    assert!(frozen.created);
    assert_eq!(frozen.record.subscription_ref, subscription);
    assert_eq!(frozen.record.snapshot_instant, covered_at());
    assert_eq!(frozen.record.trigger, SynthesisTrigger::Migration);

    // `inst-sy-provenance`: the tier rides each resolved id, so an auditor can
    // tell a real published price from a governed reconstruction.
    let resolved = frozen.record.resolved.as_array().expect("an array");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0]["source"], "live_history");
}

#[tokio::test]
async fn an_instant_before_the_window_fails_closed_and_never_takes_the_current_row() {
    // Clause (3), and the property the whole rule exists for: the current row is
    // precisely the price the subscriber was **not** paying. A reader that
    // ignored `effective_from` would resolve it here and this case is what says
    // so.
    let h = Harness::new().await;
    let plan_id = covered_plan(&h).await;

    let err = h
        .governance
        .synthesis
        .synthesize(
            &h.scope(),
            h.tenant,
            synthesis_request(Uuid::now_v7(), plan_id, before_the_window()),
        )
        .await
        .expect_err("the instant is before the window opens");

    let rendered = format!("{err:?}");
    assert!(rendered.contains("EUR"), "{rendered}");
    assert!(rendered.contains("were not paying"), "{rendered}");
}

/// **D-144's quantum on `snapshotInstant`** — the third authored-instant plane the
/// gate `repo.rs` calls "one rule" did not reach.
///
/// D-81's `t` is supplied by the trigger, carried back in a contract field
/// (`MigratedOriginSnapshotView.snapshot_instant`) and compared against window
/// bounds to select the rows, which is the whole of the rule's scope. Refused
/// rather than truncated, for D-144's reason: the caller can correct one value,
/// and a truncating freeze would resolve a different `t` than the one it recorded.
///
/// The instant is 137 microseconds inside the covering window, so the resolution
/// itself succeeds and the refusal can only be the gate — a value outside the
/// window would have been refused by the reader and proved nothing.
#[tokio::test]
async fn a_snapshot_instant_finer_than_the_quantum_is_refused() {
    let h = Harness::new().await;
    let plan_id = covered_plan(&h).await;
    let subscription = Uuid::now_v7();
    let unquantized = covered_at() + chrono::TimeDelta::microseconds(137);

    let err = h
        .governance
        .synthesis
        .synthesize(
            &h.scope(),
            h.tenant,
            synthesis_request(subscription, plan_id, unquantized),
        )
        .await
        .expect_err("a sub-millisecond snapshot instant must be refused");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("snapshotInstant"),
        "the refusal names the field the caller corrects: {rendered}"
    );
    assert!(
        rendered.contains("TimestampPrecisionExceeded"),
        "and it is the precision refusal, not a resolution failure: {rendered}"
    );

    // Nothing was frozen, so the subscription is still free to be synthesized at
    // the quantum — the same instant with its microseconds cleared.
    assert!(
        h.governance
            .synthesis
            .load(&h.scope(), h.tenant, subscription)
            .await
            .expect("load")
            .is_none()
    );
    h.governance
        .synthesis
        .synthesize(
            &h.scope(),
            h.tenant,
            synthesis_request(subscription, plan_id, covered_at()),
        )
        .await
        .expect("clearing the sub-millisecond digits is the whole remedy");
}

#[tokio::test]
async fn a_key_the_plan_does_not_publish_fails_closed() {
    // The key axis, not the time axis: the plan sells on `(EUR, eu)` and nothing
    // else, so a subscription frozen onto another market resolves nothing.
    let h = Harness::new().await;
    let plan_id = covered_plan(&h).await;

    let mut request = synthesis_request(Uuid::now_v7(), plan_id, covered_at());
    request.keys = vec![FrozenKey {
        currency: "JPY".to_owned(),
        region: "apac".to_owned(),
    }];

    assert!(
        h.governance
            .synthesis
            .synthesize(&h.scope(), h.tenant, request)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn one_uncovered_key_refuses_the_whole_snapshot() {
    // Partial synthesis is the one outcome that must not exist: a snapshot
    // missing a key is a subscription that will fail to rate on it later, with a
    // frozen record asserting its economics were captured.
    let h = Harness::new().await;
    let plan_id = covered_plan(&h).await;
    let subscription = Uuid::now_v7();

    let mut request = synthesis_request(subscription, plan_id, covered_at());
    request.keys = vec![
        seeded_key(),
        FrozenKey {
            currency: "JPY".to_owned(),
            region: "apac".to_owned(),
        },
    ];

    assert!(
        h.governance
            .synthesis
            .synthesize(&h.scope(), h.tenant, request)
            .await
            .is_err(),
        "one covered key must not carry an uncovered one"
    );

    // ...and nothing was frozen.
    assert!(
        h.governance
            .synthesis
            .load(&h.scope(), h.tenant, subscription)
            .await
            .expect("load")
            .is_none()
    );
}

#[tokio::test]
async fn a_cancelled_window_is_not_evidence_and_the_key_fails_closed() {
    // **This case exists because a probe found nothing.** Deleting the
    // `state != Cancelled` conjunct reddened not one test in this file: every
    // fixture window was `scheduled`, so the exclusion was never exercised.
    //
    // The rule matters more than its size suggests. A cancelled window never took
    // effect, so the row it scheduled was never what rating resolved — freezing
    // from it would put a price in the snapshot that the subscriber demonstrably
    // never paid, which is the same failure clause (3) refuses in the other
    // direction.
    let h = Harness::new().await;
    let plan_id = covered_plan(&h).await;

    // Cancel the window the publish path scheduled. `scheduled -> cancelled` is
    // one of §4's sanctioned edges, so this is a real transition rather than a
    // hand-written row.
    let conn = h.db.conn().expect("conn");
    let windows = bss_pricing::infra::storage::repo::window_repo::list_for_plan(
        &conn,
        &h.scope(),
        h.tenant,
        PlanId::new(plan_id),
    )
    .await
    .expect("list the plan's windows");
    assert_eq!(windows.len(), 1, "the publish path schedules exactly one");

    let (_, cancelled) =
        h.db.db()
            .in_transaction::<(), bss_pricing::infra::storage::RepoError, _>({
                let scope = h.scope();
                let tenant = h.tenant;
                let window_id = windows[0].window_id;
                move |txn| {
                    Box::pin(async move {
                        bss_pricing::infra::storage::repo::window_repo::transition(
                            txn,
                            &scope,
                            tenant,
                            window_id,
                            bss_pricing::domain::window::WindowState::Cancelled,
                            covered_at(),
                            rest_support::stamp_of(RATING_SERVICE, covered_at()),
                        )
                        .await
                        .map(|_| ())
                    })
                }
            })
            .await;
    cancelled.expect("the scheduled window cancels");

    // The instant is still inside the cancelled window's interval. Only the
    // exclusion stands between that and a frozen price nobody ever paid.
    assert!(
        h.governance
            .synthesis
            .synthesize(
                &h.scope(),
                h.tenant,
                synthesis_request(Uuid::now_v7(), plan_id, covered_at())
            )
            .await
            .is_err(),
        "a cancelled window must not be evidence"
    );
}

// ---------------------------------------------------------------------------
// The freeze, and Section 9's idempotency.
// ---------------------------------------------------------------------------

/// **A draft row's window is not evidence** — D-76 clause 1 is "the `pricing_price`
/// row, **current or superseded**", and a `draft` row is neither.
///
/// `window_repo::list_for_plan` is taken whole: every window state, over every
/// price row of the plan *whatever its lifecycle state*. The filter tested
/// cancelled-ness, currency, region and the interval, and said nothing about the
/// row behind the window — so synthesis could freeze, as "what the subscriber was
/// paying", a row that never published, never passed the publish rules and was
/// never approved.
///
/// The module doc's "**Cancelled windows are excluded and nothing else is**" was
/// the claim that made it read as complete. It was about *windows*; the omission
/// is on the *row*.
///
/// `read_model::project_windows` — the other reader of this same
/// `list_for_plan`, and the one that asserts fact to a consumer — restricts on
/// exactly this axis (`PROJECTED_ROW_STATES`).
#[tokio::test]
async fn a_draft_rows_window_is_not_evidence_and_the_key_fails_closed() {
    let h = Harness::new().await;
    let plan_id = Uuid::now_v7();
    let seeded = seed_publishable_plan(&h, plan_id).await;
    // The plan revision publishes; **the row does not**. `seed_publishable_plan`
    // already schedules the coverage window `[2099-08-04, 2099-09-01)` on the row,
    // and `publish_price` is what would move the row itself to `published` — so
    // skipping it leaves exactly the state under test: a covering window over a
    // `draft` row, which the store admits and the reader has to refuse on its own.
    h.publish(plan_id, seeded.revision).await;

    let outcome = h
        .governance
        .synthesis
        .synthesize(
            &h.scope(),
            h.tenant,
            synthesis_request(Uuid::now_v7(), plan_id, covered_at()),
        )
        .await;

    match outcome {
        Err(err) => {
            let rendered = err.to_string();
            assert!(
                rendered.contains("EUR") || rendered.contains("eu"),
                "the refusal must name the key it could not resolve: {rendered}"
            );
        }
        Ok(frozen) => panic!(
            "a draft row is not 'current or superseded', so this key has no evidence and the \
             snapshot must fail closed; it froze: {:?}",
            frozen.record.resolved
        ),
    }
}

#[tokio::test]
async fn a_second_synthesis_returns_the_same_frozen_ref_at_the_same_instant() {
    // §9: "a second synthesis attempt is idempotent (same frozen ref)". The
    // second call names a **different** instant, and it must not take: D-81 gives
    // the two triggers different `t`, so a re-freeze would leave the subscription
    // with two different frozen prices and no rule saying which rating reads.
    let h = Harness::new().await;
    let plan_id = covered_plan(&h).await;
    let subscription = Uuid::now_v7();

    let first = h
        .governance
        .synthesis
        .synthesize(
            &h.scope(),
            h.tenant,
            synthesis_request(subscription, plan_id, covered_at()),
        )
        .await
        .expect("first");

    let mut second_request = synthesis_request(subscription, plan_id, also_covered_at());
    second_request.trigger = SynthesisTrigger::FirstRating;
    let second = h
        .governance
        .synthesis
        .synthesize(&h.scope(), h.tenant, second_request)
        .await
        .expect("second");

    assert!(first.created);
    assert!(!second.created, "the second attempt froze nothing");
    assert_eq!(second.record.provenance_id, first.record.provenance_id);
    assert_eq!(second.record.snapshot_instant, covered_at());
    assert_eq!(second.record.trigger, SynthesisTrigger::Migration);
}

// ---------------------------------------------------------------------------
// D-87's payload.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_frozen_payload_is_self_contained_and_says_it_has_no_catalog_version() {
    // D-87: a `migrated-origin` ref resolves through **no** `CatalogVersion`, so
    // rating and Billing can look nothing up and everything they need is here.
    let h = Harness::new().await;
    let plan_id = covered_plan(&h).await;
    let subscription = Uuid::now_v7();

    let frozen = h
        .governance
        .synthesis
        .synthesize(
            &h.scope(),
            h.tenant,
            synthesis_request(subscription, plan_id, covered_at()),
        )
        .await
        .expect("synthesize");

    let payload = &frozen.record.payload;
    // Stated rather than merely absent, so a consumer looking for a version
    // learns why there is none instead of failing.
    assert!(payload["catalogVersion"].is_null());
    assert_eq!(payload["catalogVersionDeliberatelyAbsent"], true);

    // The row half: evaluable content, not just ids.
    let rows = payload["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["currency"], "EUR");
    assert_eq!(rows[0]["region"], "eu");
    assert_eq!(rows[0]["source"], "live_history");
    assert!(rows[0].get("modelKind").is_some());
    assert!(rows[0].get("taxInclusive").is_some());
    assert!(rows[0].get("roundingPolicyRef").is_some());

    // C-5's plan-level half, and the absence that is reported rather than hidden:
    // there is no entitlement grant store in this gear, so a consumer must not
    // read the empty set as "this plan grants nothing".
    assert_eq!(payload["planLevel"]["grantSetUnavailable"], true);
    assert!(payload["planLevel"]["grantSet"].is_null());
    assert!(payload["planLevel"].get("invoiceLineTemplate").is_some());
    // D-319, and the marker beside it is the point: this plan authored no
    // minimum, so the empty list **means** "no minimum" — which is only readable
    // because the payload also says the set was available to be read. Without
    // the marker an unread set and an unauthored one render identically, and
    // this payload resolves through no `CatalogVersion`, so a consumer has
    // nowhere to go and check.
    assert_eq!(
        payload["planLevel"]["periodFloorCaps"],
        serde_json::json!([])
    );
    assert_eq!(payload["planLevel"]["periodFloorCapsUnavailable"], false);
}

/// **A period bound reaches the frozen `migrated-origin` payload** (D-319).
///
/// The case exists because this is the gear's **second** plan-level payload and
/// it does not go through `publish::assemble_from`: `plan_level` reads the
/// descriptor set off the current revision by hand, and a field added to the
/// read model alone would be invisible here. That matters more here than
/// anywhere else — a `migrated-origin` ref resolves through **no**
/// `CatalogVersion` by construction, so a bound outside this payload is one
/// Billing cannot apply and cannot look up, on a record that is frozen and
/// therefore permanently wrong.
#[tokio::test]
async fn a_period_bound_is_materialized_into_the_frozen_payload() {
    let h = Harness::new().await;
    let plan_id = Uuid::now_v7();
    let seeded = seed_publishable_plan(&h, plan_id).await;

    // On the market the seeded row prices — anything else is
    // `PERIOD_FLOOR_CAP_MARKET_UNSOLD` and would never publish.
    h.state
        .shapes
        .replace_period_floor_caps(
            &h.scope(),
            h.tenant,
            PlanId::new(plan_id),
            seeded.revision,
            seeded.version,
            vec![PeriodFloorCap {
                currency: CurrencyCode::new("EUR").expect("three letters"),
                region: Region::new("eu").expect("non-blank"),
                floor_minor: Some(MinorAmount::new(50_000).expect("non-negative")),
                cap_minor: None,
            }],
            seed_stamp(),
        )
        .await
        .expect("author the period floor on the open draft");

    h.publish(plan_id, seeded.revision).await;
    h.publish_price(plan_id, seeded.price_id).await;

    let frozen = h
        .governance
        .synthesis
        .synthesize(
            &h.scope(),
            h.tenant,
            synthesis_request(Uuid::now_v7(), plan_id, covered_at()),
        )
        .await
        .expect("synthesize");

    assert_eq!(
        frozen.record.payload["planLevel"]["periodFloorCaps"],
        serde_json::json!([{
            "currency": "EUR",
            "region": "eu",
            "floorMinor": 50_000,
            "capMinor": null,
        }]),
        "the bound is materialized whole: {}",
        frozen.record.payload
    );
    assert_eq!(
        frozen.record.payload["planLevel"]["periodFloorCapsUnavailable"],
        false
    );
}

// ---------------------------------------------------------------------------
// D-102's read surface.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_surface_returns_the_frozen_payload_with_its_provenance() {
    let h = Harness::new().await;
    let plan_id = covered_plan(&h).await;
    let subscription = Uuid::now_v7();
    h.governance
        .synthesis
        .synthesize(
            &h.scope(),
            h.tenant,
            synthesis_request(subscription, plan_id, covered_at()),
        )
        .await
        .expect("synthesize");

    let response = h
        .allowed_as(RATING_SERVICE)
        .send(request("GET", &path(subscription), None))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert_eq!(view["subscription_ref"], subscription.to_string());
    assert_eq!(view["trigger"], "migration");
    assert_eq!(view["payload"]["catalogVersionDeliberatelyAbsent"], true);
    assert_eq!(view["resolved"][0]["source"], "live_history");
}

#[tokio::test]
async fn the_surface_answers_404_before_synthesis_and_that_is_the_contract() {
    // `inst-sy-firstrating`: rating a legacy subscription before synthesis fails
    // closed into the exception path, synthesis runs as a separate audited step,
    // and rating retries. A 200 carrying a partial payload would make those two
    // states indistinguishable.
    let h = Harness::new().await;
    let response = h
        .allowed_as(RATING_SERVICE)
        .send(request("GET", &path(Uuid::now_v7()), None))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_caller_without_plan_read_is_denied() {
    let h = Harness::new().await;
    let plan_id = covered_plan(&h).await;
    let subscription = Uuid::now_v7();
    h.governance
        .synthesis
        .synthesize(
            &h.scope(),
            h.tenant,
            synthesis_request(subscription, plan_id, covered_at()),
        )
        .await
        .expect("synthesize");

    let response = h
        .denied()
        .send(request("GET", &path(subscription), None))
        .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
