//! `/bss-pricing/v1/migrations` over the wire (`inst-ms-api`, `inst-ms-return`,
//! `inst-mg-target`, `inst-mg-idem`, `inst-mg-cancel`, D-34, D-49).
//!
//! `rest_retirement`'s shape. What this file adds that no other route test can is
//! the **status discrimination of an idempotent create**: M2 makes the schedule
//! idempotent on a client-supplied key, and the only externally visible part of
//! that rule is `202` on a fresh schedule against `200` on a replay. A suite that
//! asserted "both succeed" would pass with the idempotency removed.
//!
//! The other thing proved only here is that **`DELETE` cancels**: the row is read
//! back afterwards and is still there, in `cancelled`. A route test that only
//! checked the status would pass against a handler that deleted the row, and an
//! executor re-reading a deleted schedule cannot tell it from one that never
//! existed — which is the distinction `inst-mg-cancel`'s handshake rests on.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::migrations::{MIGRATION_BY_ID, MIGRATIONS};
use chrono::{DateTime, Duration, SubsecRound, Utc};
use rest_support::{Harness, body_json, request, seed_publishable_plan};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::SecureEntityExt;
use uuid::Uuid;

const OPERATOR: Uuid = Uuid::from_u128(0x_09_e2);

fn item_path(migration_id: Uuid) -> String {
    MIGRATION_BY_ID.replace("{migrationId}", &migration_id.to_string())
}

/// A published plan with one published price row and its coverage window.
async fn published(h: &Harness) -> Uuid {
    let plan_id = Uuid::now_v7();
    let seeded = seed_publishable_plan(h, plan_id).await;
    h.publish(plan_id, seeded.revision).await;
    h.publish_price(plan_id, seeded.price_id).await;
    plan_id
}

/// **An authored instant at the millisecond quantum** (D-144), relative to now.
///
/// `Utc::now()` carries microseconds and `effective_at` is authored, carried in a
/// contract field and compared, so the store refuses a finer one —
/// `an_effective_instant_finer_than_the_quantum_is_refused_and_the_announcement_is_not`
/// is that gate's own case. It is offset from *now* rather than fixed in 2099
/// because D-49's notice floor is measured from the scheduling commit, so a fixture
/// instant that did not move with the clock would be testing the wrong distance.
fn authored(days: i64) -> DateTime<Utc> {
    Utc::now().trunc_subsecs(3) + Duration::days(days)
}

/// A schedule body clearing D-49's 60-day floor by a wide margin.
fn schedule_body(migration_id: Uuid, source: Uuid, target: Uuid) -> serde_json::Value {
    serde_json::json!({
        "migration_id": migration_id,
        "source_plan_id": source,
        "target_plan_id": target,
        "effective_at": authored(120),
    })
}

#[tokio::test]
async fn a_fresh_schedule_answers_202_and_names_what_it_scheduled() {
    let h = Harness::new().await;
    let source = published(&h).await;
    let target = published(&h).await;
    let migration_id = Uuid::now_v7();

    let response = h
        .allowed_as(OPERATOR)
        .send(request(
            "POST",
            MIGRATIONS,
            Some(schedule_body(migration_id, source, target)),
        ))
        .await;

    // **202, not 201.** The catalog has scheduled; nothing has migrated. A 201
    // would claim a resource whose whole content is a promise about the future.
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let view = body_json(response).await;
    assert_eq!(view["migration_id"], migration_id.to_string());
    assert_eq!(view["state"], "scheduled");
    assert_eq!(view["source_plan_id"], source.to_string());
    assert_eq!(view["target_plan_id"], target.to_string());
    // The two honesty flags: nobody could be enumerated, and the lock registry
    // could not be asked. An empty delta report without these reads as an
    // all-clear it has no basis for.
    assert_eq!(view["subjects_unresolved"], true);
    assert_eq!(view["exclusions_unresolved"], true);
}

#[tokio::test]
async fn a_replay_of_one_migration_id_answers_200_with_the_original_schedule() {
    // M2, and the only externally visible part of it. A route answering 202 to a
    // retry would tell a client it had just scheduled something it scheduled an
    // hour ago.
    let h = Harness::new().await;
    let source = published(&h).await;
    let target = published(&h).await;
    let migration_id = Uuid::now_v7();

    let first = h
        .allowed_as(OPERATOR)
        .send(request(
            "POST",
            MIGRATIONS,
            Some(schedule_body(migration_id, source, target)),
        ))
        .await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let original = body_json(first).await;

    // The retry carries a **different** effective date. The stored schedule is
    // what answers: the key is the identity, and the second body never lands.
    let mut retry = schedule_body(migration_id, source, target);
    retry["effective_at"] = serde_json::json!(authored(300));
    let second = h
        .allowed_as(OPERATOR)
        .send(request("POST", MIGRATIONS, Some(retry)))
        .await;

    assert_eq!(
        second.status(),
        StatusCode::OK,
        "a replay scheduled nothing"
    );
    let replayed = body_json(second).await;
    assert_eq!(replayed["effective_at"], original["effective_at"]);
}

/// Every `PlanMigrationScheduled` event name the caller's tenant holds.
async fn scheduled_events(h: &Harness) -> Vec<serde_json::Value> {
    use bss_pricing::infra::storage::entity::outbox;
    let conn = h.db.conn().expect("conn");
    outbox::Entity::find()
        .secure()
        .scope_with(&h.scope())
        .filter(
            Condition::all()
                .add(outbox::Column::TenantId.eq(h.tenant))
                .add(outbox::Column::EventName.eq("PlanMigrationScheduled")),
        )
        .all(&conn)
        .await
        .expect("read the outbox")
        .into_iter()
        .map(|row| row.payload)
        .collect()
}

#[tokio::test]
async fn scheduling_enqueues_plan_migration_scheduled_with_its_dedup_contract() {
    // `inst-ms-emit`. **This case exists because a probe found nothing.**
    // Deleting the `outbox_repo::enqueue` call reddened not one test in this
    // file: the surface answers 202 whether or not Subscriptions ever hears
    // about the schedule, and a migration nobody is told about is a migration
    // that does not happen.
    let h = Harness::new().await;
    let source = published(&h).await;
    let target = published(&h).await;
    let migration_id = Uuid::now_v7();

    h.allowed_as(OPERATOR)
        .send(request(
            "POST",
            MIGRATIONS,
            Some(schedule_body(migration_id, source, target)),
        ))
        .await;

    let events = scheduled_events(&h).await;
    assert_eq!(events.len(), 1, "exactly one event per schedule");
    let payload = &events[0];
    assert_eq!(payload["migrationId"], migration_id.to_string());
    assert_eq!(payload["sourcePlanId"], source.to_string());
    assert_eq!(payload["targetPlanId"], target.to_string());
    // M2's contract rides the event: the consumer dedups per
    // `(migrationId, subscription)`, and it cannot do that from a payload that
    // does not carry the id.
    assert_eq!(payload["dedupContract"], "(migrationId, subscription)");
    // D-39, on the contract rather than in the consumer's head.
    assert_eq!(payload["entryPhaseIsFirstNonTrial"], true);
    assert_eq!(payload["exclusionsUnresolved"], true);
}

#[tokio::test]
async fn a_replay_enqueues_no_second_event() {
    // M2 again, on the half that matters to the consumer: a retried create must
    // not produce a second `PlanLink` fan-out.
    let h = Harness::new().await;
    let source = published(&h).await;
    let target = published(&h).await;
    let migration_id = Uuid::now_v7();
    let body = schedule_body(migration_id, source, target);

    for _ in 0..2 {
        h.allowed_as(OPERATOR)
            .send(request("POST", MIGRATIONS, Some(body.clone())))
            .await;
    }

    assert_eq!(scheduled_events(&h).await.len(), 1);
}

#[tokio::test]
async fn a_retired_target_is_refused_on_its_own_code() {
    // `inst-mg-target`, and **this is the case that reaches the predicate**.
    //
    // A *draft* target does not: a draft-only plan has no **current** revision at
    // all (`uq_pricing_plan_current` spans `published | retired`), so the refusal
    // arrives from the missing-current-revision branch one line earlier and
    // `ensure_target_publishable` is never called. A probe deleting that call
    // reddened nothing, which is how this case came to exist. A retired plan
    // *does* have a current revision, so it is the one target shape that walks
    // into the predicate and is turned away by it.
    let h = Harness::new().await;
    let source = published(&h).await;
    let retired_target = published(&h).await;
    h.retire(retired_target, 0).await;

    let response = h
        .allowed_as(OPERATOR)
        .send(request(
            "POST",
            MIGRATIONS,
            Some(schedule_body(Uuid::now_v7(), source, retired_target)),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await.to_string();
    assert!(body.contains("MIGRATION_TARGET_INVALID"), "{body}");
    // The refusal names the state the target is actually in.
    assert!(body.contains("retired"), "{body}");
}

#[tokio::test]
async fn a_draft_target_is_refused_because_it_has_no_current_revision() {
    // Kept, and **renamed to what it actually proves**. It is a different branch
    // from the case above and both are worth holding: a plan nobody ever
    // published is not a migration target, and the message an operator gets says
    // so rather than naming a lifecycle state the plan does not have.
    let h = Harness::new().await;
    let source = published(&h).await;
    let draft_target = Uuid::now_v7();
    seed_publishable_plan(&h, draft_target).await;

    let response = h
        .allowed_as(OPERATOR)
        .send(request(
            "POST",
            MIGRATIONS,
            Some(schedule_body(Uuid::now_v7(), source, draft_target)),
        ))
        .await;

    // An architectural 422 rendered 400, the code the discriminator.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert!(
        body.to_string().contains("MIGRATION_TARGET_INVALID"),
        "{body}"
    );
}

/// **An `effective_at` finer than the millisecond quantum is refused on the wire**
/// (D-144), and the refusal names the field rather than the notice period.
///
/// The instant clears D-49's floor by sixty days, so the only thing wrong with it is
/// its precision — a body that also fell inside the notice period would be refused
/// by the earlier rule and would prove nothing about this one. `+137µs`, because an
/// instant already at the quantum is accepted with the gate and without it.
#[tokio::test]
async fn an_effective_instant_below_the_quantum_is_refused_on_the_wire() {
    let h = Harness::new().await;
    let source = published(&h).await;
    let target = published(&h).await;

    let mut body = schedule_body(Uuid::now_v7(), source, target);
    body["effective_at"] = serde_json::json!(authored(120) + Duration::microseconds(137));

    let response = h
        .allowed_as(OPERATOR)
        .send(request("POST", MIGRATIONS, Some(body)))
        .await;

    // The architectural 422 reaches the wire as a 400 carrying its code.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let rendered = body_json(response).await.to_string();
    assert!(
        rendered.contains("TIMESTAMP_PRECISION_EXCEEDED"),
        "{rendered}"
    );
    assert!(
        rendered.contains("effectiveAt"),
        "the author is told which of the request's instants to correct: {rendered}"
    );
}

#[tokio::test]
async fn a_migration_inside_the_notice_period_is_refused_naming_the_earliest_instant() {
    // D-49. There is no override on this request: a shorter migration needs an
    // audited change to the tenant's notice policy first.
    let h = Harness::new().await;
    let source = published(&h).await;
    let target = published(&h).await;

    let mut body = schedule_body(Uuid::now_v7(), source, target);
    body["effective_at"] = serde_json::json!(authored(30));

    let response = h
        .allowed_as(OPERATOR)
        .send(request("POST", MIGRATIONS, Some(body)))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    let rendered = body.to_string();
    assert!(
        rendered.contains("MIGRATION_NOTICE_TOO_SHORT"),
        "{rendered}"
    );
    assert!(rendered.contains("earliest admissible"), "{rendered}");
}

#[tokio::test]
async fn a_migration_onto_the_source_plan_itself_is_refused() {
    let h = Harness::new().await;
    let plan = published(&h).await;

    let response = h
        .allowed_as(OPERATOR)
        .send(request(
            "POST",
            MIGRATIONS,
            Some(schedule_body(Uuid::now_v7(), plan, plan)),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        body_json(response)
            .await
            .to_string()
            .contains("MIGRATION_TARGET_INVALID")
    );
}

#[tokio::test]
async fn the_schedule_reads_back_with_its_frozen_delta_report() {
    let h = Harness::new().await;
    let source = published(&h).await;
    let target = published(&h).await;
    let migration_id = Uuid::now_v7();

    h.allowed_as(OPERATOR)
        .send(request(
            "POST",
            MIGRATIONS,
            Some(schedule_body(migration_id, source, target)),
        ))
        .await;

    let response = h
        .allowed_as(OPERATOR)
        .send(request("GET", &item_path(migration_id), None))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert_eq!(view["state"], "scheduled");
    // The schedule-time evidence, verbatim.
    assert_eq!(view["delta_report"]["subjectsUnresolved"], true);
    assert_eq!(view["delta_report"]["locksUnresolved"], true);
}

#[tokio::test]
async fn an_unknown_migration_answers_404() {
    let h = Harness::new().await;
    let response = h
        .allowed_as(OPERATOR)
        .send(request("GET", &item_path(Uuid::now_v7()), None))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_cancels_the_run_and_leaves_the_record_standing() {
    // **The property this file exists for.** A handler that deleted the row would
    // pass a status-only assertion, and an executor re-reading a deleted schedule
    // cannot tell it from one that never existed - which is the distinction
    // `inst-mg-cancel`'s state handshake rests on.
    let h = Harness::new().await;
    let source = published(&h).await;
    let target = published(&h).await;
    let migration_id = Uuid::now_v7();

    h.allowed_as(OPERATOR)
        .send(request(
            "POST",
            MIGRATIONS,
            Some(schedule_body(migration_id, source, target)),
        ))
        .await;

    let cancelled = h
        .allowed_as(OPERATOR)
        .send(request("DELETE", &item_path(migration_id), None))
        .await;

    // 200 with the record, not 204: D-34 puts the partial sets on it, and a
    // body-less answer would need a second call to read them.
    assert_eq!(cancelled.status(), StatusCode::OK);
    assert_eq!(body_json(cancelled).await["state"], "cancelled");

    // The row is still there.
    let read_back = h
        .allowed_as(OPERATOR)
        .send(request("GET", &item_path(migration_id), None))
        .await;
    assert_eq!(read_back.status(), StatusCode::OK);
    assert_eq!(body_json(read_back).await["state"], "cancelled");
}

#[tokio::test]
async fn a_second_cancel_is_refused_and_is_not_a_completion_refusal() {
    // The two say different things to an operator: one finished doing work, the
    // other was already stopped.
    let h = Harness::new().await;
    let source = published(&h).await;
    let target = published(&h).await;
    let migration_id = Uuid::now_v7();

    h.allowed_as(OPERATOR)
        .send(request(
            "POST",
            MIGRATIONS,
            Some(schedule_body(migration_id, source, target)),
        ))
        .await;
    h.allowed_as(OPERATOR)
        .send(request("DELETE", &item_path(migration_id), None))
        .await;

    let again = h
        .allowed_as(OPERATOR)
        .send(request("DELETE", &item_path(migration_id), None))
        .await;

    let status = again.status();
    let body = body_json(again).await.to_string();
    assert!(
        !body.contains("MIGRATION_COMPLETED"),
        "an already-cancelled run has not completed: {status} {body}"
    );
}

#[tokio::test]
async fn cancelling_an_unknown_migration_answers_404() {
    let h = Harness::new().await;
    let response = h
        .allowed_as(OPERATOR)
        .send(request("DELETE", &item_path(Uuid::now_v7()), None))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
