//! Crash windows drop the actual door futures at deterministic registry awaits.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod entry_support;
use bss_pricing::{
    domain::price_book_entry::OpState,
    infra::{
        reference_ticker::Ticker,
        reference_work::Clock,
        storage::repo::{
            idempotency_repo as idem, price_book_entry_repo, price_repo, reference_op_repo as ops,
        },
    },
};
use entry_support::{Fixture, Script};
use serde_json::json;
use std::sync::Arc;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;
struct FixedClock(time::OffsetDateTime);
impl Clock for FixedClock {
    fn now(&self) -> time::OffsetDateTime {
        self.0
    }
}
fn clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock(
        time::OffsetDateTime::now_utc() + time::Duration::days(2),
    ))
}
async fn setup() -> (Fixture, Arc<Script>, String, serde_json::Value) {
    let script = Arc::new(Script::default());
    let f = Fixture::new(script.clone()).await;
    let (book, _) = f.book().await;
    let path = format!("/price-books/{}/entries", book["id"].as_str().unwrap());
    (f, script, path, json!({"sku_id":Uuid::new_v4()}))
}
async fn crash_create(mode: usize, expected_state: &str) {
    let (f, script, path, input) = setup().await;
    script.set(mode);
    let mut door = Box::pin(f.call("POST", &path, input.clone(), None, Some("crash")));
    tokio::select! { result = &mut door => panic!("door did not park: {result:?}"), () = script.parked.notified() => {} }
    drop(door);
    let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
    let before = ops::due(&f.db.conn().unwrap(), &scope, clock().now(), 10)
        .await
        .unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].state, expected_state);
    script.set(0);
    let mut ticker = Ticker::new(f.state.clone(), clock(), 10, 100);
    ticker.tick().await.unwrap();
    let conn = f.db.conn().unwrap();
    let after = ops::find(&conn, &scope, f.ctx.subject_tenant_id(), before[0].op_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.state, "done");
    let entry = price_book_entry_repo::find(
        &conn,
        &scope,
        f.ctx.subject_tenant_id(),
        before[0].price_book_entry_id,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(entry.reference_state, "confirmed");
    let replay = f.call("POST", &path, input, None, Some("crash")).await;
    assert_eq!(replay.0, 201, "{replay:?}");
    assert_eq!(replay.1["id"], entry.id.to_string());
    assert!(matches!(
        idem::lookup_idempotency_key(
            &conn,
            &scope,
            f.ctx.subject_tenant_id(),
            &format!("/bss-pricing/v1{path}"),
            "crash",
            time::OffsetDateTime::now_utc()
        )
        .await
        .unwrap(),
        Some(idem::IdempotencyClaim::Answered { .. })
    ));
}
/// The key and its op as the store holds them after the given op finished.
async fn key_claim(f: &Fixture, path: &str, key: &str) -> Option<idem::IdempotencyClaim> {
    idem::lookup_idempotency_key(
        &f.db.conn().unwrap(),
        &AccessScope::for_tenant(f.ctx.subject_tenant_id()),
        f.ctx.subject_tenant_id(),
        &format!("/bss-pricing/v1{path}"),
        key,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap()
}
/// A create cancelled before its reservation outcome was known: done, recorded `cancelled`,
/// no entry, and its key free for a fresh attempt.
async fn assert_cancelled_without_entry(f: &Fixture, op_id: Uuid, path: &str, key: &str) {
    let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
    let conn = f.db.conn().unwrap();
    let op = ops::find(&conn, &scope, f.ctx.subject_tenant_id(), op_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(op.state, "done");
    let work = bss_pricing::infra::reference_work::Work::read(&op).unwrap();
    assert_eq!(work.outcome.as_deref(), Some("cancelled"), "{op:?}");
    assert!(
        price_book_entry_repo::find(
            &conn,
            &scope,
            f.ctx.subject_tenant_id(),
            op.price_book_entry_id
        )
        .await
        .unwrap()
        .is_none(),
        "a cancelled create writes no entry"
    );
    assert_eq!(
        key_claim(f, path, key).await,
        None,
        "the claim was released"
    );
}
#[tokio::test]
async fn crash_after_tx_a_cancels_the_create_and_frees_the_key() {
    // The door died before it learned whether Products reserved. The ticker never reserves on
    // a user's behalf: it cancels, releases the key, and releases whatever reservation exists.
    let (f, script, path, input) = setup().await;
    script.set(1);
    let mut door = Box::pin(f.call("POST", &path, input.clone(), None, Some("crash")));
    tokio::select! { result = &mut door => panic!("door did not park: {result:?}"), () = script.parked.notified() => {} }
    drop(door);
    let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
    let before = ops::due(&f.db.conn().unwrap(), &scope, clock().now(), 10)
        .await
        .unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].state, "reserving");
    assert_eq!(before[0].reservation_id, None);
    script.set(0);
    Ticker::new(f.state.clone(), clock(), 10, 100)
        .tick()
        .await
        .unwrap();
    assert_cancelled_without_entry(&f, before[0].op_id, &path, "crash").await;
    assert_eq!(
        Script::count(&script.releases),
        1,
        "the cancellation's own reserve found the reservation and released it"
    );
    assert!(
        script
            .refs
            .lock()
            .await
            .get(&before[0].price_book_entry_id)
            .is_none_or(|r| r.1 == bss_products_sdk::models::ReferenceState::Released)
    );
    let retry = f.call("POST", &path, input, None, Some("crash")).await;
    assert_eq!(retry.0, 201, "{retry:?}");
    assert_ne!(
        retry.1["id"],
        before[0].price_book_entry_id.to_string(),
        "a fresh entry"
    );
    assert!(matches!(
        key_claim(&f, &path, "crash").await,
        Some(idem::IdempotencyClaim::Answered { .. })
    ));
}
#[tokio::test]
async fn crash_after_tx_b_recovers_and_answers_key() {
    crash_create(2, "written").await;
}
#[tokio::test]
async fn crash_after_delete_tx_recovers_release() {
    let (f, script, path, input) = setup().await;
    let created = f.call("POST", &path, input, None, Some("one")).await;
    assert_eq!(created.0, 201);
    let path = format!("/price-book-entries/{}", created.1["id"].as_str().unwrap());
    script.set(3);
    let mut door = Box::pin(f.call("DELETE", &path, json!({}), None, None));
    tokio::select! { result = &mut door => panic!("door did not park: {result:?}"), () = script.parked.notified() => {} }
    drop(door);
    assert_eq!(f.call("GET", &path, json!({}), None, None).await.0, 404);
    script.set(0);
    let mut ticker = Ticker::new(f.state.clone(), clock(), 10, 100);
    ticker.tick().await.unwrap();
    assert_eq!(Script::count(&script.releases), 1);
    let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
    assert!(
        ops::due(&f.db.conn().unwrap(), &scope, clock().now(), 10)
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn a_lost_reserve_response_writes_no_entry_and_its_reservation_is_released() {
    // Products reserved but the answer never arrived: the door answers 503, writes no entry and
    // frees the key; the ticker's cancellation finds that reservation and releases it.
    let (f, script, path, input) = setup().await;
    script.set(5);
    let first = f
        .call("POST", &path, input.clone(), None, Some("one"))
        .await;
    assert_eq!(first.0, 503, "{first:?}");
    assert!(
        first.1.to_string().contains("REGISTRY_UNAVAILABLE"),
        "{first:?}"
    );
    let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
    let cancelling = ops::page(
        &f.db.conn().unwrap(),
        &scope,
        f.ctx.subject_tenant_id(),
        Some(OpState::Cancelling),
        None,
        10,
    )
    .await
    .unwrap();
    assert_eq!(cancelling.len(), 1, "the door cancelled its own op");
    assert_eq!(
        key_claim(&f, &path, "one").await,
        None,
        "and released the key"
    );
    let (lost_ref, lost_receipt) = {
        let refs = script.refs.lock().await;
        let (id, (receipt, _)) = refs.iter().next().unwrap();
        (*id, *receipt)
    };
    assert_eq!(lost_ref, cancelling[0].price_book_entry_id);
    Ticker::new(f.state.clone(), clock(), 10, 100)
        .tick()
        .await
        .unwrap();
    assert_cancelled_without_entry(&f, cancelling[0].op_id, &path, "one").await;
    assert_eq!(Script::count(&script.releases), 1);
    assert_eq!(
        script.refs.lock().await[&lost_ref],
        (
            lost_receipt,
            bss_products_sdk::models::ReferenceState::Released
        )
    );
    let retry = f.call("POST", &path, input, None, Some("one")).await;
    assert_eq!(retry.0, 201, "{retry:?}");
    assert_ne!(retry.1["id"], lost_ref.to_string());
    assert_ne!(retry.1["reservation_id"], lost_receipt.to_string());
}
#[tokio::test]
async fn ticker_recovers_a_confirm_timeout_and_answers_the_key() {
    let (f, script, path, input) = setup().await;
    script.set(6);
    assert_eq!(
        f.call("POST", &path, input.clone(), None, Some("one"))
            .await
            .0,
        503
    );
    let receipt_before = script.refs.lock().await.values().next().unwrap().0;
    script.set(0);
    Ticker::new(f.state.clone(), clock(), 10, 100)
        .tick()
        .await
        .unwrap();
    let replay = f.call("POST", &path, input, None, Some("one")).await;
    assert_eq!(replay.0, 201);
    assert_eq!(replay.1["reservation_id"], receipt_before.to_string());
    assert_eq!(script.refs.lock().await.len(), 1);
    assert_eq!(Script::count(&script.releases), 0);
}
#[tokio::test]
async fn forced_release_reconciles_unfenced_and_fenced_entries() {
    use bss_products_sdk::models::ReferenceState;
    for fenced in [false, true] {
        let (f, script, path, input) = setup().await;
        let first = f.call("POST", &path, input, None, Some("one")).await;
        assert_eq!(first.0, 201);
        for value in script.refs.lock().await.values_mut() {
            value.1 = ReferenceState::Released;
        }
        if fenced {
            script.set(4);
        }
        Ticker::new(f.state.clone(), clock(), 1, 1)
            .tick()
            .await
            .unwrap();
        let id: Uuid = first.1["id"].as_str().unwrap().parse().unwrap();
        let read = f
            .call(
                "GET",
                &format!("/price-book-entries/{id}"),
                json!({}),
                None,
                None,
            )
            .await;
        assert_eq!(
            read.1["reference_state"],
            if fenced { "lost" } else { "confirmed" },
            "{read:?}"
        );
        if fenced {
            let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
            let conn = f.db.conn().unwrap();
            let entry = price_book_entry_repo::find(&conn, &scope, f.ctx.subject_tenant_id(), id)
                .await
                .unwrap()
                .unwrap();
            let error = price_repo::insert(&conn, &scope, entry_support::price(&entry))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("ENTRY_REFERENCE_LOST"));
        } else {
            assert_ne!(first.1["reservation_id"], read.1["reservation_id"]);
        }
        let export = f
            .call(
                "GET",
                &path.replace("/entries", "/export"),
                json!({}),
                None,
                None,
            )
            .await;
        assert_eq!(
            export.1["entries"][0]["entry"]["reference_state"],
            read.1["reference_state"]
        );
    }
}
#[tokio::test]
async fn operator_list_filters_paginates_and_validates_query() {
    let (f, script, path, input) = setup().await;
    script.set(6);
    f.call("POST", &path, input, None, Some("one")).await;
    let result = f
        .call(
            "GET",
            "/reference-ops?state=written&limit=1",
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(result.0, 200, "{result:?}");
    assert_eq!(result.1["items"].as_array().unwrap().len(), 1);
    let id = result.1["items"][0]["op_id"].as_str().unwrap();
    assert_eq!(
        f.call(
            "GET",
            &format!("/reference-ops?cursor={id}&limit=1"),
            json!({}),
            None,
            None
        )
        .await
        .1["items"],
        json!([])
    );
    assert_eq!(
        f.call("GET", "/reference-ops?state=wat", json!({}), None, None)
            .await
            .0,
        400
    );
    assert_eq!(
        f.call("GET", "/reference-ops?limit=0", json!({}), None, None)
            .await
            .0,
        400
    );
}
#[test]
fn backoff_is_exponential_and_bounded() {
    use bss_pricing::infra::reference_work::backoff;
    for (attempts, seconds) in [
        (0, 1),
        (1, 2),
        (8, 256),
        (9, 300),
        (10, 300),
        (i32::MAX, 300),
    ] {
        assert_eq!(backoff(attempts).whole_seconds(), seconds);
    }
    assert_eq!(OpState::Done.as_str(), "done");
}

/// Every `PriceBookEntryReferenceLost` envelope in the fixture's outbox.
async fn lost_events(f: &Fixture) -> Vec<serde_json::Value> {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let raw = Database::connect(&f.dsn).await.unwrap();
    raw.query_all_raw(Statement::from_string(
        DbBackend::Sqlite,
        "SELECT CAST(payload AS TEXT) AS payload FROM bss_pricing_outbox_body",
    ))
    .await
    .unwrap()
    .iter()
    .map(|r| serde_json::from_str(&r.try_get::<String>("", "payload").unwrap()).unwrap())
    .filter(|e: &serde_json::Value| {
        e["type"]
            == "gts.cf.core.events.event.v1~cf.bss.pricing.price_book_entry_reference_lost.v1~"
    })
    .collect()
}
async fn read_entry(f: &Fixture, id: &serde_json::Value) -> serde_json::Value {
    let read = f
        .call(
            "GET",
            &format!("/price-book-entries/{}", id.as_str().unwrap()),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(read.0, 200, "{read:?}");
    read.1
}
#[tokio::test]
async fn a_reservation_released_before_confirm_is_rereserved_not_lost() {
    // An operator force-released the reservation between Tx B and the confirm. The SKU is not
    // fenced, so the entry is re-reserved (D-401); the create is answered, never as `lost`.
    let (f, script, path, input) = setup().await;
    script.set(7);
    let created = f
        .call("POST", &path, input.clone(), None, Some("one"))
        .await;
    assert_eq!(created.0, 201, "{created:?}");
    assert_eq!(created.1["reference_state"], "confirmation_pending");
    assert_eq!(
        f.call("POST", &path, input, None, Some("one")).await,
        created
    );
    let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
    let open = ops::page(
        &f.db.conn().unwrap(),
        &scope,
        f.ctx.subject_tenant_id(),
        Some(OpState::Reserving),
        None,
        10,
    )
    .await
    .unwrap();
    assert_eq!(open.len(), 1, "one rereserve_entry op is open");
    assert_eq!(open[0].kind, "rereserve_entry");
    assert_eq!(
        open[0].price_book_entry_id.to_string(),
        created.1["id"].as_str().unwrap()
    );
    script.set(0);
    Ticker::new(f.state.clone(), clock(), 10, 100)
        .tick()
        .await
        .unwrap();
    let read = read_entry(&f, &created.1["id"]).await;
    assert_eq!(read["reference_state"], "confirmed", "{read}");
    assert_ne!(read["reservation_id"], created.1["reservation_id"]);
    assert!(lost_events(&f).await.is_empty());
    assert_eq!(Script::count(&script.releases), 0);
}
#[tokio::test]
async fn a_released_entry_is_lost_only_behind_a_fence_and_found_again_when_it_lifts() {
    let (f, script, path, input) = setup().await;
    script.set(7);
    let created = f.call("POST", &path, input, None, Some("one")).await;
    assert_eq!(created.1["reference_state"], "confirmation_pending");
    // The SKU is now fenced: the re-reservation is refused SKU_FENCED, so the entry is lost.
    script.set(4);
    let mut ticker = Ticker::new(f.state.clone(), clock(), 10, 1);
    ticker.tick().await.unwrap();
    let read = read_entry(&f, &created.1["id"]).await;
    assert_eq!(read["reference_state"], "lost", "{read}");
    let events = lost_events(&f).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["data"]["priceBookEntryId"], created.1["id"]);
    assert_eq!(
        events[0]["tenant_id"],
        f.ctx.subject_tenant_id().to_string()
    );
    // Still fenced: reconciliation leaves the lost entry alone and announces nothing twice.
    let reserves = Script::count(&script.reserve_calls);
    ticker.tick().await.unwrap();
    assert_eq!(Script::count(&script.reserve_calls), reserves);
    assert_eq!(lost_events(&f).await.len(), 1);
    // The fence lifts: reconciliation re-reserves the lost entry.
    script.set(0);
    ticker.tick().await.unwrap();
    let read = read_entry(&f, &created.1["id"]).await;
    assert_eq!(read["reference_state"], "confirmed", "{read}");
    assert_ne!(read["reservation_id"], created.1["reservation_id"]);
    assert_eq!(lost_events(&f).await.len(), 1);
}
#[tokio::test]
async fn a_rereserve_refused_for_another_reason_is_retried_never_lost() {
    let (f, script, path, input) = setup().await;
    script.set(7);
    let created = f.call("POST", &path, input, None, Some("one")).await;
    assert_eq!(created.1["reference_state"], "confirmation_pending");
    script.set(16);
    Ticker::new(f.state.clone(), clock(), 10, 100)
        .tick()
        .await
        .unwrap();
    let read = read_entry(&f, &created.1["id"]).await;
    assert_eq!(read["reference_state"], "confirmation_pending", "{read}");
    assert!(lost_events(&f).await.is_empty());
    let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
    let open = ops::page(
        &f.db.conn().unwrap(),
        &scope,
        f.ctx.subject_tenant_id(),
        Some(OpState::Reserving),
        None,
        10,
    )
    .await
    .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].attempts, 1, "retried with backoff, not cancelled");
}

#[tokio::test]
async fn cancelling_releasing_backoff_and_threshold_never_drop_work() {
    for cancelling in [false, true] {
        let (f, script, path, input) = setup().await;
        if cancelling {
            script.set(14);
        }
        let first = f
            .call("POST", &path, input.clone(), None, Some("one"))
            .await;
        if cancelling {
            assert_eq!(first.0, 503);
        } else {
            assert_eq!(first.0, 201);
            script.set(12);
            // The delete committed; its release proceeds through the op and the ticker.
            assert_eq!(
                f.call(
                    "DELETE",
                    &format!("/price-book-entries/{}", first.1["id"].as_str().unwrap()),
                    json!({}),
                    None,
                    None
                )
                .await
                .0,
                204
            );
        }
        let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
        let mut now = clock().now();
        for attempt in 2..=11 {
            Ticker::new(f.state.clone(), Arc::new(FixedClock(now)), 1, 100)
                .tick()
                .await
                .unwrap();
            let ops = ops::page(
                &f.db.conn().unwrap(),
                &scope,
                f.ctx.subject_tenant_id(),
                Some(if cancelling {
                    OpState::Cancelling
                } else {
                    OpState::Releasing
                }),
                None,
                10,
            )
            .await
            .unwrap();
            assert_eq!(ops.len(), 1);
            assert_eq!(ops[0].attempts, attempt);
            assert_eq!(
                ops[0].next_attempt_at,
                now + bss_pricing::infra::reference_work::backoff(attempt)
            );
            now += time::Duration::seconds(301);
        }
        script.set(0);
        Ticker::new(f.state.clone(), Arc::new(FixedClock(now)), 1, 100)
            .tick()
            .await
            .unwrap();
        assert!(
            ops::due(&f.db.conn().unwrap(), &scope, now, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(Script::count(&script.releases), 1);
        if cancelling {
            let response = f.call("POST", &path, input, None, Some("one")).await;
            assert_eq!(response.0, 409);
            assert!(response.1.to_string().contains("BUNDLE_SKU_NOT_PRICEABLE"));
        }
    }
}
#[tokio::test]
async fn door_losing_completion_race_rereads_the_tickers_receipt() {
    let (f, script, path, input) = setup().await;
    script.set(2);
    let mut door = Box::pin(f.call("POST", &path, input.clone(), None, Some("one")));
    tokio::select! { result = &mut door => panic!("did not park: {result:?}"), () = script.parked.notified() => {} }
    script.set(0);
    Ticker::new(f.state.clone(), clock(), 1, 100)
        .tick()
        .await
        .unwrap();
    script.resume.notify_one();
    let result = door.await;
    assert_eq!(result.0, 201, "{result:?}");
    assert_eq!(
        result,
        f.call("POST", &path, input, None, Some("one")).await
    );
}
#[tokio::test]
async fn reconciliation_is_periodic_bounded_and_uses_the_system_actor() {
    let (f, script, path, _) = setup().await;
    let mut entries = Vec::new();
    for n in 0..3 {
        let result = f
            .call(
                "POST",
                &path,
                json!({"sku_id":Uuid::new_v4()}),
                None,
                Some(&format!("{n}")),
            )
            .await;
        assert_eq!(result.0, 201);
        entries.push(result.1);
    }
    for value in script.refs.lock().await.values_mut() {
        value.1 = bss_products_sdk::models::ReferenceState::Released;
    }
    let mut ticker = Ticker::new(f.state.clone(), clock(), 1, 2);
    for tick in 1_usize..=6 {
        ticker.tick().await.unwrap();
        let repaired = script
            .refs
            .lock()
            .await
            .values()
            .filter(|(_, state)| *state == bss_products_sdk::models::ReferenceState::Confirmed)
            .count();
        // Reconciliation runs every second tick and repairs one entry per run.
        assert_eq!(repaired, tick.div_euclid(2));
    }
    let actors = script.actors.lock().await;
    assert_eq!(&actors[3..], &[bss_products_sdk::PRICING_SYSTEM_ACTOR; 3]);
    assert_eq!(entries.len(), 3);
}
#[tokio::test]
async fn a_live_door_owns_its_op_for_the_grace_period() {
    // The ticker runs every second. An op a door is still driving must not be due
    // to it, or every create races a second registry caller under another actor.
    let (f, script, path, input) = setup().await;
    script.set(1);
    let mut door = Box::pin(f.call("POST", &path, input, None, Some("live")));
    tokio::select! { result = &mut door => panic!("door did not park: {result:?}"), () = script.parked.notified() => {} }
    let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
    let now = time::OffsetDateTime::now_utc();
    let conn = f.db.conn().unwrap();
    assert!(
        ops::due(&conn, &scope, now, 10).await.unwrap().is_empty(),
        "a fresh op is not due while its door may still be driving it"
    );
    let later =
        now + bss_pricing::infra::reference_work::IN_FLIGHT_GRACE + time::Duration::seconds(1);
    assert_eq!(
        ops::due(&conn, &scope, later, 10).await.unwrap().len(),
        1,
        "an abandoned op becomes due"
    );
    drop(door);
}

#[tokio::test]
async fn one_tenants_states_failure_skips_only_that_tenant_and_the_cursor_moves_on() {
    use entry_support::{app_for, request, user_of};
    let (f, script, path, input) = setup().await;
    let first = f.call("POST", &path, input, None, Some("a")).await;
    assert_eq!(first.0, 201, "{first:?}");
    // A second tenant on the same pricing database, created after the first.
    let other = Uuid::new_v4();
    let (app, ctx) = (app_for(f.state.clone(), other), user_of(other));
    let (s, book, _) = request(
        &app,
        &ctx,
        "POST",
        "/price-books",
        json!({"code":"standard","name":"Standard","currency":"EUR"}),
        None,
        Some("book"),
    )
    .await;
    assert_eq!(s, 201, "{book}");
    let second = request(
        &app,
        &ctx,
        "POST",
        &format!("/price-books/{}/entries", book["id"].as_str().unwrap()),
        json!({"sku_id":Uuid::new_v4()}),
        None,
        Some("b"),
    )
    .await;
    assert_eq!(second.0, 201, "{second:?}");
    for value in script.refs.lock().await.values_mut() {
        value.1 = bss_products_sdk::models::ReferenceState::Released;
    }
    *script.states_down_for.lock().unwrap() = Some(f.ctx.subject_tenant_id());
    // One entry per reconciliation pass, every pass.
    let mut ticker = Ticker::new(f.state.clone(), clock(), 1, 1);
    ticker.tick().await.unwrap();
    ticker.tick().await.unwrap();
    let read = |id: &serde_json::Value| {
        let id: Uuid = id.as_str().unwrap().parse().unwrap();
        let state = f.state.clone();
        async move {
            let conn = state.db.conn().unwrap();
            price_book_entry_repo::find(
                &conn,
                &AccessScope::allow_all(),
                tenant_of(&conn, id).await,
                id,
            )
            .await
            .unwrap()
            .unwrap()
        }
    };
    let a = read(&first.1["id"]).await;
    let b = read(&second.1["id"]).await;
    assert_eq!(a.reference_state, "confirmed");
    assert_eq!(
        a.reservation_id.to_string(),
        first.1["reservation_id"].as_str().unwrap(),
        "the failing tenant was skipped"
    );
    assert_eq!(b.reference_state, "confirmed");
    assert_ne!(
        b.reservation_id.to_string(),
        second.1["reservation_id"].as_str().unwrap(),
        "the next tenant was still reconciled"
    );
}
/// The tenant of an entry, read by id alone (the ticker's own view).
async fn tenant_of(conn: &toolkit_db::DbConn<'_>, id: Uuid) -> Uuid {
    use sea_orm::EntityTrait;
    use toolkit_db::secure::SecureEntityExt;
    bss_pricing::infra::storage::entity::price_book_entry::Entity::find_by_id(id)
        .secure()
        .scope_with(&AccessScope::allow_all())
        .one(conn)
        .await
        .unwrap()
        .unwrap()
        .tenant_id
}
#[tokio::test]
async fn a_reservation_products_does_not_know_is_treated_as_released() {
    let (f, script, path, input) = setup().await;
    let created = f.call("POST", &path, input, None, Some("one")).await;
    assert_eq!(created.0, 201, "{created:?}");
    // Products' database was restored from a backup that predates this reservation.
    script.refs.lock().await.clear();
    Ticker::new(f.state.clone(), clock(), 10, 1)
        .tick()
        .await
        .unwrap();
    let read = read_entry(&f, &created.1["id"]).await;
    assert_eq!(read["reference_state"], "confirmed", "{read}");
    assert_ne!(read["reservation_id"], created.1["reservation_id"]);
}
