//! Crash windows drop the actual door futures at deterministic registry awaits.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod price_support;
use bss_pricing::{
    domain::price::OpState,
    infra::{
        reference_ticker::Ticker,
        reference_work::Clock,
        storage::repo::{idempotency_repo as idem, price_repo, reference_op_repo as ops, row_repo},
    },
};
use price_support::{Fixture, Script};
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
    let path = format!("/price-books/{}/prices", book["id"].as_str().unwrap());
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
    let price = price_repo::find(&conn, &scope, f.ctx.subject_tenant_id(), before[0].price_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(price.reference_state, "confirmed");
    let replay = f.call("POST", &path, input, None, Some("crash")).await;
    assert_eq!(replay.0, 201, "{replay:?}");
    assert_eq!(replay.1["id"], price.id.to_string());
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
#[tokio::test]
async fn crash_after_tx_a_recovers_and_answers_key() {
    crash_create(1, "reserving").await;
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
    let path = format!("/prices/{}", created.1["id"].as_str().unwrap());
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
async fn ticker_recovers_lost_reserve_response_and_confirm_timeout() {
    for mode in [5, 6] {
        let (f, script, path, input) = setup().await;
        script.set(mode);
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
}
#[tokio::test]
async fn forced_release_reconciles_unfenced_and_fenced_prices() {
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
            .call("GET", &format!("/prices/{id}"), json!({}), None, None)
            .await;
        assert_eq!(
            read.1["reference_state"],
            if fenced { "lost" } else { "confirmed" },
            "{read:?}"
        );
        if fenced {
            let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
            let conn = f.db.conn().unwrap();
            let price = price_repo::find(&conn, &scope, f.ctx.subject_tenant_id(), id)
                .await
                .unwrap()
                .unwrap();
            let error = row_repo::insert(&conn, &scope, price_support::row(&price))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("PRICE_REFERENCE_LOST"));
        } else {
            assert_ne!(first.1["reservation_id"], read.1["reservation_id"]);
        }
        let export = f
            .call(
                "GET",
                &path.replace("/prices", "/export"),
                json!({}),
                None,
                None,
            )
            .await;
        assert_eq!(
            export.1["prices"][0]["price"]["reference_state"],
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

#[tokio::test]
async fn lost_price_emits_one_durable_event() {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let (f, script, path, input) = setup().await;
    script.set(7);
    let created = f
        .call("POST", &path, input.clone(), None, Some("one"))
        .await;
    assert_eq!(created.0, 201);
    f.call("POST", &path, input, None, Some("one")).await;
    let raw = Database::connect(&f.dsn).await.unwrap();
    let rows = raw
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT CAST(payload AS TEXT) AS payload FROM bss_pricing_outbox_body",
        ))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let envelope: serde_json::Value =
        serde_json::from_str(&rows[0].try_get::<String>("", "payload").unwrap()).unwrap();
    assert_eq!(
        envelope["type"],
        "gts.cf.core.events.event.v1~cf.bss.pricing.price_reference_lost.v1~"
    );
    assert_eq!(envelope["data"]["priceId"], created.1["id"]);
    assert_eq!(envelope["tenant_id"], f.ctx.subject_tenant_id().to_string());
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
            assert_eq!(
                f.call(
                    "DELETE",
                    &format!("/prices/{}", first.1["id"].as_str().unwrap()),
                    json!({}),
                    None,
                    None
                )
                .await
                .0,
                503
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
    let mut prices = Vec::new();
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
        prices.push(result.1);
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
        // Reconciliation runs every second tick and repairs one price per run.
        assert_eq!(repaired, tick.div_euclid(2));
    }
    let actors = script.actors.lock().await;
    assert_eq!(&actors[3..], &[bss_products_sdk::PRICING_SYSTEM_ACTOR; 3]);
    assert_eq!(prices.len(), 3);
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
