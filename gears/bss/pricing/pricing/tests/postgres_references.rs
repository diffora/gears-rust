//! The reference state machine end to end on native Postgres: the entries door and the ticker
//! over pricing's real chain, against the scripted Products registry double of run 3 (see
//! `entry_support::Script`). Products' own Postgres half of the barrier (reserve against the
//! fences) is proven by its tier, `postgres_sku_chain.rs`.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod entry_support;
mod pg_support;
use bss_pricing::{
    api::rest::authoring::AuthoringState,
    infra::{
        reference_ticker::Ticker,
        reference_work::Clock,
        storage::repo::{price_book_entry_repo, price_repo, reference_op_repo as ops},
    },
};
use bss_products_sdk::models::ReferenceState;
use entry_support::{Script, app_for, request, state_on, user_of};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde_json::{Value, json};
use std::sync::Arc;
use toolkit_db::{DBProvider, DbError, secure::AccessScope};
use toolkit_security::SecurityContext;
use uuid::Uuid;

struct FixedClock(time::OffsetDateTime);
impl Clock for FixedClock {
    fn now(&self) -> time::OffsetDateTime {
        self.0
    }
}
/// Past the in-flight grace, so every op a door left behind is due.
fn clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock(
        time::OffsetDateTime::now_utc() + time::Duration::days(2),
    ))
}

/// One pricing process on its own Postgres database with one book.
struct Door {
    pg: pg_support::Pg,
    db: DBProvider<DbError>,
    state: Arc<AuthoringState>,
    app: axum::Router,
    script: Arc<Script>,
    ctx: SecurityContext,
    /// `/price-books/{book}/entries`
    path: String,
    sku: Uuid,
}
async fn setup() -> Door {
    let pg = pg_support::Pg::applied().await;
    let script = Arc::new(Script::default());
    let db = DBProvider::<DbError>::new(pg.db().await);
    let state = state_on(db.clone(), script.clone()).await;
    let tenant = Uuid::new_v4();
    let app = app_for(state.clone(), tenant);
    let ctx = user_of(tenant);
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
    let path = format!("/price-books/{}/entries", book["id"].as_str().unwrap());
    Door {
        pg,
        db,
        state,
        app,
        script,
        ctx,
        path,
        sku: Uuid::new_v4(),
    }
}
impl Door {
    fn tenant(&self) -> Uuid {
        self.ctx.subject_tenant_id()
    }
    fn scope(&self) -> AccessScope {
        AccessScope::for_tenant(self.tenant())
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Value,
        key: Option<&str>,
    ) -> (u16, Value, String) {
        request(&self.app, &self.ctx, method, path, body, None, key).await
    }
    async fn create(&self, key: &str) -> (u16, Value, String) {
        self.call("POST", &self.path, json!({"sku_id":self.sku}), Some(key))
            .await
    }
    async fn due(&self) -> Vec<bss_pricing::infra::storage::entity::reference_op::Model> {
        ops::due(&self.db.conn().unwrap(), &self.scope(), clock().now(), 10)
            .await
            .unwrap()
    }
    async fn tick(&self) {
        Ticker::new(self.state.clone(), clock(), 10, 1)
            .tick()
            .await
            .unwrap();
    }
    async fn reference_state(&self, entry: &Value) -> String {
        price_book_entry_repo::find(
            &self.db.conn().unwrap(),
            &self.scope(),
            self.tenant(),
            entry.as_str().unwrap().parse().unwrap(),
        )
        .await
        .unwrap()
        .map(|p| p.reference_state)
        .unwrap_or_default()
    }
    /// Every envelope in the Postgres outbox, as JSON.
    async fn envelopes(&self) -> Vec<Value> {
        self.pg
            .raw()
            .await
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT convert_from(payload, 'UTF8') AS p FROM public.bss_pricing_outbox_body \
                 ORDER BY id"
                    .to_owned(),
            ))
            .await
            .unwrap()
            .iter()
            .map(|r| serde_json::from_str(&r.try_get::<String>("", "p").unwrap()).unwrap())
            .collect()
    }
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_create_reserves_writes_and_confirms_and_a_delete_releases() {
    let p = setup().await;
    let created = p.create("one").await;
    assert_eq!(created.0, 201, "{created:?}");
    assert_eq!(created.1["reference_state"], "confirmed");
    let reservation = created.1["reservation_id"].clone();
    assert_eq!(
        p.script.refs.lock().await.values().next().copied(),
        Some((
            reservation.as_str().unwrap().parse().unwrap(),
            ReferenceState::Confirmed
        ))
    );
    assert!(p.due().await.is_empty(), "the door finished its op");
    assert_eq!(p.create("one").await, created, "the key replays its answer");
    assert_eq!(Script::count(&p.script.reserve_calls), 1);
    let one = format!("/price-book-entries/{}", created.1["id"].as_str().unwrap());
    assert_eq!(p.call("DELETE", &one, json!({}), None).await.0, 204);
    assert_eq!(Script::count(&p.script.releases), 1);
    assert!(p.due().await.is_empty(), "the delete op finished too");
    assert_eq!(p.call("GET", &one, json!({}), None).await.0, 404);
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_every_crash_window_is_resumed_by_the_ticker() {
    for (mode, state) in [(1, "reserving"), (2, "written")] {
        let p = setup().await;
        p.script.set(mode);
        let mut door = Box::pin(p.create("crash"));
        tokio::select! {
            result = &mut door => panic!("door did not park: {result:?}"),
            () = p.script.parked.notified() => {}
        }
        drop(door);
        let due = p.due().await;
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].state, state);
        p.script.set(0);
        p.tick().await;
        assert!(
            p.due().await.is_empty(),
            "{state}: the ticker finished the op"
        );
        let replay = p.create("crash").await;
        assert_eq!(replay.0, 201, "{state}: {replay:?}");
        if mode == 1 {
            // Before Tx B the reserve outcome was unknown: the ticker cancelled the create,
            // released the key and the reservation, and the same key ran afresh.
            assert_ne!(replay.1["id"], due[0].price_book_entry_id.to_string());
            assert_eq!(Script::count(&p.script.releases), 1);
            let op = ops::find(&p.db.conn().unwrap(), &p.scope(), p.tenant(), due[0].op_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(op.state, "done");
            assert_eq!(
                bss_pricing::infra::reference_work::Work::read(&op)
                    .unwrap()
                    .outcome
                    .as_deref(),
                Some("cancelled")
            );
            assert_eq!(
                p.reference_state(&json!(due[0].price_book_entry_id)).await,
                ""
            );
        } else {
            assert_eq!(replay.1["id"], due[0].price_book_entry_id.to_string());
        }
        assert_eq!(p.reference_state(&replay.1["id"]).await, "confirmed");
    }
    let p = setup().await;
    let created = p.create("one").await;
    assert_eq!(created.0, 201);
    let one = format!("/price-book-entries/{}", created.1["id"].as_str().unwrap());
    p.script.set(3);
    let mut door = Box::pin(p.call("DELETE", &one, json!({}), None));
    tokio::select! {
        result = &mut door => panic!("door did not park: {result:?}"),
        () = p.script.parked.notified() => {}
    }
    drop(door);
    assert_eq!(p.due().await[0].state, "releasing");
    p.script.set(0);
    p.tick().await;
    assert_eq!(Script::count(&p.script.releases), 1);
    assert!(p.due().await.is_empty());
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_forced_release_reconciles_both_ways_and_announces_a_lost_entry() {
    for fenced in [false, true] {
        let p = setup().await;
        let created = p.create("one").await;
        assert_eq!(created.0, 201);
        for value in p.script.refs.lock().await.values_mut() {
            value.1 = ReferenceState::Released;
        }
        if fenced {
            p.script.set(4);
        }
        p.tick().await;
        let (_, read, _) = p
            .call(
                "GET",
                &format!("/price-book-entries/{}", created.1["id"].as_str().unwrap()),
                json!({}),
                None,
            )
            .await;
        let lost: Vec<Value> = p
            .envelopes()
            .await
            .into_iter()
            .filter(|e| {
                e["type"] == "gts.cf.core.events.event.v1~cf.bss.pricing.price_book_entry_reference_lost.v1~"
            })
            .collect();
        if fenced {
            assert_eq!(read["reference_state"], "lost", "{read}");
            assert_eq!(lost.len(), 1, "{lost:?}");
            assert_eq!(lost[0]["data"]["priceBookEntryId"], created.1["id"]);
            assert_eq!(lost[0]["tenant_id"], p.tenant().to_string());
            let conn = p.db.conn().unwrap();
            let entry = price_book_entry_repo::find(
                &conn,
                &p.scope(),
                p.tenant(),
                created.1["id"].as_str().unwrap().parse().unwrap(),
            )
            .await
            .unwrap()
            .unwrap();
            let refused = price_repo::insert(&conn, &p.scope(), entry_support::price(&entry))
                .await
                .unwrap_err();
            assert!(refused.to_string().contains("ENTRY_REFERENCE_LOST"));
        } else {
            assert_eq!(read["reference_state"], "confirmed", "{read}");
            assert_ne!(read["reservation_id"], created.1["reservation_id"]);
            assert!(lost.is_empty());
        }
        assert!(p.due().await.is_empty());
    }
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_two_tickers_on_two_pools_finish_one_op_once() {
    let p = setup().await;
    p.script.set(6);
    let first = p.create("one").await;
    assert_eq!(first.0, 503, "the confirm timed out: {first:?}");
    let due = p.due().await;
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].state, "written");
    p.script.set(0);
    let other = state_on(
        DBProvider::<DbError>::new(p.pg.db().await),
        p.script.clone(),
    )
    .await;
    let (mut a, mut b) = (
        Ticker::new(p.state.clone(), clock(), 10, 100),
        Ticker::new(other, clock(), 10, 100),
    );
    let (left, right) = tokio::join!(a.tick(), b.tick());
    left.unwrap();
    right.unwrap();
    assert!(p.due().await.is_empty(), "exactly one of them finished it");
    let op = ops::find(&p.db.conn().unwrap(), &p.scope(), p.tenant(), due[0].op_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(op.state, "done");
    let replay = p.create("one").await;
    assert_eq!(replay.0, 201, "{replay:?}");
    assert_eq!(replay.1["id"], due[0].price_book_entry_id.to_string());
    assert_eq!(replay.1["reference_state"], "confirmed");
    assert_eq!(
        Script::count(&p.script.releases),
        0,
        "a timeout never releases"
    );
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_removing_a_dimension_key_an_entry_names_is_refused_409() {
    // Postgres raises 23503 on pricing_price_book_entry's key FK; the door must refuse before it.
    let p = setup().await;
    let (_, _, tag) = p.call("GET", "/dimension-keys", json!({}), None).await;
    let declared = request(
        &p.app,
        &p.ctx,
        "PUT",
        "/dimension-keys",
        json!({"items":[{"key":"region","values":["eu","us"]}]}),
        Some(&tag),
        None,
    )
    .await;
    assert_eq!(declared.0, 200, "{declared:?}");
    let created = p
        .call(
            "POST",
            &p.path,
            json!({"sku_id":p.sku,"dimension_key":"region"}),
            Some("one"),
        )
        .await;
    assert_eq!(created.0, 201, "{created:?}");
    let refused = request(
        &p.app,
        &p.ctx,
        "PUT",
        "/dimension-keys",
        json!({"items":[]}),
        Some(&declared.2),
        None,
    )
    .await;
    assert_eq!(refused.0, 409, "{refused:?}");
    assert!(
        refused.1.to_string().contains("DIMENSION_KEY_IN_USE"),
        "{refused:?}"
    );
}
