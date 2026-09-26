//! Price book entry authoring protocol branches through the production router.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod entry_support;
use entry_support::{Fixture, KINDS, Kind, Script, Target};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;
async fn setup(mode: usize) -> (Fixture, Arc<Script>, String, Value) {
    let script = Arc::new(Script::default());
    script.set(mode);
    let f = Fixture::new(script.clone()).await;
    let (book, _) = f.book().await;
    let path = format!("/price-books/{}/entries", book["id"].as_str().unwrap());
    (
        f,
        script,
        path,
        if mode == 11 {
            json!({"sku_id":Uuid::new_v4(),"period":"month"})
        } else {
            json!({"sku_id":Uuid::new_v4()})
        },
    )
}
#[tokio::test]
async fn create_replay_current_kind_unique_key_and_delete() {
    let (f, script, path, input) = setup(11).await;
    assert_eq!(
        f.call("POST", &path, input.clone(), None, None).await.0,
        400
    );
    let first = f
        .call("POST", &path, input.clone(), None, Some("one"))
        .await;
    assert_eq!(first.0, 201, "{first:?}");
    assert_eq!(first.1["reference_state"], "confirmed");
    assert_eq!(first.1["charge_kind"], "recurring");
    assert_eq!(
        f.call("POST", &path, input.clone(), None, Some("one"))
            .await,
        first
    );
    assert_eq!(Script::count(&script.reserve_calls), 1);
    let dup = f.call("POST", &path, input, None, Some("two")).await;
    assert_eq!(dup.0, 409, "{dup:?}");
    assert!(dup.1.to_string().contains("ENTRY_KEY_TAKEN"));
    let id = first.1["id"].as_str().unwrap();
    assert_eq!(
        f.call(
            "GET",
            &format!("/price-book-entries/{id}"),
            json!({}),
            None,
            None
        )
        .await
        .1,
        first.1
    );
    assert_eq!(
        f.call(
            "DELETE",
            &format!("/price-book-entries/{id}"),
            json!({}),
            None,
            None
        )
        .await
        .0,
        204
    );
    assert_eq!(Script::count(&script.releases), 2);
}
#[tokio::test]
async fn refusal_branches_answer_key_and_release_only_after_reservation() {
    for (mode, code, releases) in [
        (4, "SKU_FENCED", 0),
        (8, "BUNDLE_SKU_NOT_PRICEABLE", 1),
        (9, "SKU_DEPRECATED", 1),
        (10, "SKU_DRAFT", 1),
    ] {
        let (f, script, path, input) = setup(mode).await;
        let refused = f
            .call("POST", &path, input.clone(), None, Some("one"))
            .await;
        assert_eq!(refused.0, 409, "{refused:?}");
        assert!(refused.1.to_string().contains(code), "{refused:?}");
        script.set(0);
        assert_eq!(
            f.call("POST", &path, input, None, Some("one")).await,
            refused
        );
        assert_eq!(Script::count(&script.releases), releases);
    }
}
/// A fresh fixture for one reference kind, its target and a create body.
async fn setup_kind(mode: usize, kind: Kind) -> (Fixture, Arc<Script>, Target, Value) {
    let script = Arc::new(Script::default());
    script.set(mode);
    let f = Fixture::new(script.clone()).await;
    let t = f.target(kind).await;
    let input = t.input();
    (f, script, t, input)
}
#[tokio::test]
async fn a_503_before_the_write_frees_the_key_and_a_confirm_timeout_keeps_it() {
    for kind in KINDS {
        // Mode 5: the reserve answer is lost. Nothing was written, so the same key runs afresh.
        let (f, script, t, input) = setup_kind(5, kind).await;
        let c = f.caller();
        let result = t.create(&c, input.clone(), "one").await;
        assert_eq!(result.0, 503, "{kind:?}: {result:?}");
        let retry = t.create(&c, input, "one").await;
        assert_eq!(retry.0, 201, "{kind:?}: {retry:?}");
        assert_eq!(Script::count(&script.reserve_calls), 2);
        // Mode 6: the reference is written and its confirm timed out; the key stays in flight.
        let (f, script, t, input) = setup_kind(6, kind).await;
        let c = f.caller();
        let result = t.create(&c, input.clone(), "one").await;
        assert_eq!(result.0, 503, "{kind:?}: {result:?}");
        let retry = t.create(&c, input, "one").await;
        assert_eq!(retry.0, 409);
        assert!(
            retry.1.to_string().contains("IDEMPOTENCY_KEY_IN_FLIGHT"),
            "{kind:?}: {retry:?}"
        );
        assert_eq!(Script::count(&script.releases), 0);
    }
}
#[tokio::test]
async fn released_on_confirm_is_answered_pending_never_lost() {
    for kind in KINDS {
        let (f, script, t, input) = setup_kind(7, kind).await;
        let c = f.caller();
        let first = t.create(&c, input.clone(), "one").await;
        assert_eq!(first.0, 201, "{kind:?}: {first:?}");
        assert_eq!(first.1["reference_state"], "confirmation_pending");
        assert_eq!(t.create(&c, input, "one").await, first);
        assert_eq!(Script::count(&script.releases), 0);
    }
}
#[tokio::test]
async fn patch_template_and_dimension_preconditions() {
    let (f, _, path, input) = setup(0).await;
    let first = f.call("POST", &path, input, None, Some("one")).await;
    assert_eq!(first.0, 201, "{first:?}");
    let path = format!("/price-book-entries/{}", first.1["id"].as_str().unwrap());
    assert_eq!(f.call("PATCH", &path, json!({}), None, None).await.0, 400);
    assert_eq!(
        f.call(
            "PATCH",
            &path,
            json!({"invoice_line_override":"{phase}"}),
            Some(&first.2),
            None
        )
        .await
        .0,
        400
    );
    let changed = f
        .call(
            "PATCH",
            &path,
            json!({"invoice_line_override":"{sku} {unit}"}),
            Some(&first.2),
            None,
        )
        .await;
    assert_eq!(changed.0, 200, "{changed:?}");
    assert_eq!(
        f.call("PATCH", &path, json!({}), Some(&first.2), None)
            .await
            .0,
        409
    );
    assert_eq!(
        f.call(
            "PATCH",
            &path,
            json!({"invoice_line_override":null}),
            Some(&changed.2),
            None
        )
        .await
        .1["invoice_line_override"],
        Value::Null
    );
}

#[tokio::test]
async fn translated_dimension_key_change_rejected_and_approved_delete_refused() {
    use bss_pricing::infra::storage::repo::{price_book_entry_repo, price_repo};
    let (f, _, path, input) = setup(0).await;
    let (_, _, tag) = f
        .call("GET", "/dimension-keys", json!({}), None, None)
        .await;
    assert_eq!(
        f.call(
            "PUT",
            "/dimension-keys",
            json!({"items":[{"key":"region","values":["eu","us"]}]}),
            Some(&tag),
            None
        )
        .await
        .0,
        200
    );
    let first = f.call("POST", &path, input, None, Some("one")).await;
    let id = first.1["id"].as_str().unwrap().parse().unwrap();
    let path = format!("/price-book-entries/{id}");
    let changed = f
        .call(
            "PATCH",
            &path,
            json!({"dimension_key":"region"}),
            Some(&first.2),
            None,
        )
        .await;
    assert_eq!(changed.0, 200, "{changed:?}");
    let scope = toolkit_db::secure::AccessScope::for_tenant(f.ctx.subject_tenant_id());
    let conn = f.db.conn().unwrap();
    let p = price_book_entry_repo::find(&conn, &scope, f.ctx.subject_tenant_id(), id)
        .await
        .unwrap()
        .unwrap();
    let mut price = entry_support::price(&p);
    price.dim_value = Some("eu".into());
    price.state = "approved".into();
    price_repo::insert(&conn, &scope, price).await.unwrap();
    let refusal = f
        .call(
            "PATCH",
            &path,
            json!({"dimension_key":null}),
            Some(&changed.2),
            None,
        )
        .await;
    assert_eq!(refusal.0, 409);
    assert!(refusal.1.to_string().contains("DIMENSION_KEY_IN_USE"));
    // D-426: the entry now carries approved money, so its invoice line is locked.
    let locked = f
        .call(
            "PATCH",
            &path,
            json!({"invoice_line_override":"{sku}"}),
            Some(&changed.2),
            None,
        )
        .await;
    assert_eq!(locked.0, 409, "{locked:?}");
    assert!(
        locked.1.to_string().contains("INVOICE_LINE_LOCKED"),
        "{locked:?}"
    );
    assert_eq!(f.call("DELETE", &path, json!({}), None, None).await.0, 409);
}

// D-426 (owner 2026-09-26): an entry's `invoice_line_override` reaches consumers through resolve
// (D-421: the entry wins over the SKU's template), so once the entry carries money — an approved or
// a pending price — a change of it is refused 409 `INVOICE_LINE_LOCKED`; a draft price does not lock
// it, and sending the same value is no change.
#[tokio::test]
async fn an_entry_invoice_line_locks_once_the_entry_carries_money() {
    use bss_pricing::infra::storage::repo::{price_book_entry_repo, price_repo};
    for state in ["draft", "pending", "approved"] {
        let (f, _, path, input) = setup(0).await;
        let first = f.call("POST", &path, input, None, Some("one")).await;
        assert_eq!(first.0, 201, "{first:?}");
        let id: Uuid = first.1["id"].as_str().unwrap().parse().unwrap();
        let path = format!("/price-book-entries/{id}");
        let set = f
            .call(
                "PATCH",
                &path,
                json!({"invoice_line_override":"{sku} {unit}"}),
                Some(&first.2),
                None,
            )
            .await;
        assert_eq!(set.0, 200, "{state}: no price yet: {set:?}");
        let scope = toolkit_db::secure::AccessScope::for_tenant(f.ctx.subject_tenant_id());
        let conn = f.db.conn().unwrap();
        let p = price_book_entry_repo::find(&conn, &scope, f.ctx.subject_tenant_id(), id)
            .await
            .unwrap()
            .unwrap();
        let mut price = entry_support::price(&p);
        price.state = state.into();
        price_repo::insert(&conn, &scope, price).await.unwrap();
        let same = f
            .call(
                "PATCH",
                &path,
                json!({"invoice_line_override":"{sku} {unit}"}),
                Some(&set.2),
                None,
            )
            .await;
        assert_eq!(
            same.0, 200,
            "{state}: the same value is no change: {same:?}"
        );
        let other = f
            .call(
                "PATCH",
                &path,
                json!({"invoice_line_override":"{sku}"}),
                Some(&same.2),
                None,
            )
            .await;
        if state == "draft" {
            assert_eq!(
                other.0, 200,
                "a draft price does not lock the line: {other:?}"
            );
        } else {
            assert_eq!(other.0, 409, "{state}: {other:?}");
            assert!(
                other.1.to_string().contains("INVOICE_LINE_LOCKED"),
                "{state}: {other:?}"
            );
            let cleared = f
                .call(
                    "PATCH",
                    &path,
                    json!({"invoice_line_override":null}),
                    Some(&same.2),
                    None,
                )
                .await;
            assert_eq!(
                cleared.0, 409,
                "{state}: clearing is a change too: {cleared:?}"
            );
        }
    }
}

#[tokio::test]
async fn products_contention_and_rate_limits_are_unavailability_not_refusals() {
    for kind in KINDS {
        // A reserve answered 409 UNIT_CONTENDED or 429: nothing was written, the key is free.
        for mode in [17, 19] {
            let (f, script, t, input) = setup_kind(mode, kind).await;
            let c = f.caller();
            let first = t.create(&c, input.clone(), "one").await;
            assert_eq!(first.0, 503, "{kind:?} {mode}: {first:?}");
            script.set(0);
            let retry = t.create(&c, input, "one").await;
            assert_eq!(retry.0, 201, "{kind:?} {mode}: {retry:?}");
        }
        // The SKU re-read after a successful reserve answered 409 CONTENDED. The door answers
        // 503, and a 503 writes nothing (spec section 13) even with a receipt in hand: the create is
        // cancelled, its key is free again, and the cancellation releases the receipt it holds.
        let (f, script, t, input) = setup_kind(18, kind).await;
        let c = f.caller();
        let first = t.create(&c, input.clone(), "one").await;
        assert_eq!(first.0, 503, "{kind:?}: {first:?}");
        script.set(0);
        let retry = t.create(&c, input, "one").await;
        assert_eq!(
            retry.0, 201,
            "{kind:?}: a same-key retry runs afresh: {retry:?}"
        );
        bss_pricing::infra::reference_ticker::Ticker::new(
            f.state.clone(),
            Arc::new(LaterClock),
            10,
            100,
        )
        .tick()
        .await
        .unwrap();
        assert_eq!(
            Script::count(&script.releases),
            1,
            "{kind:?}: the abandoned create's reservation is released"
        );
    }
}
/// A clock past the in-flight grace, so the ticker may take over abandoned work.
struct LaterClock;
impl bss_pricing::infra::reference_work::Clock for LaterClock {
    fn now(&self) -> time::OffsetDateTime {
        time::OffsetDateTime::now_utc() + time::Duration::days(2)
    }
}

#[tokio::test]
async fn period_and_dimension_refusals_are_400_before_any_reservation() {
    // D-403: pure input refusals are 400, and they cost no Products reservation.
    for (mode, body, field, code) in [
        (11, json!({}), "period", "ENTRY_PERIOD_INVALID"),
        (
            0,
            json!({"period":"month"}),
            "period",
            "ENTRY_PERIOD_INVALID",
        ),
        (
            0,
            json!({"period":"week"}),
            "period",
            "ENTRY_PERIOD_INVALID",
        ),
        (
            0,
            json!({"dimension_key":"zone"}),
            "dimension_key",
            "DIM_NOT_DECLARED",
        ),
    ] {
        let (f, script, path, _) = setup(mode).await;
        let mut input = body.clone();
        input["sku_id"] = json!(Uuid::new_v4());
        let refused = f
            .call("POST", &path, input.clone(), None, Some("one"))
            .await;
        assert_eq!(refused.0, 400, "{body}: {refused:?}");
        assert!(refused.1.to_string().contains(code), "{refused:?}");
        assert!(refused.1.to_string().contains(field), "{refused:?}");
        assert_eq!(Script::count(&script.reserve_calls), 0, "{body}");
        // Nothing was claimed: the corrected request runs under the same key.
        let fixed = if mode == 11 {
            json!({"sku_id":input["sku_id"],"period":"month"})
        } else {
            json!({"sku_id":input["sku_id"]})
        };
        let created = f.call("POST", &path, fixed, None, Some("one")).await;
        assert_eq!(created.0, 201, "{created:?}");
        let patched = f
            .call(
                "PATCH",
                &format!("/price-book-entries/{}", created.1["id"].as_str().unwrap()),
                json!({"dimension_key":"zone"}),
                Some(&created.2),
                None,
            )
            .await;
        assert_eq!(patched.0, 400, "{patched:?}");
        assert!(
            patched.1.to_string().contains("DIM_NOT_DECLARED"),
            "{patched:?}"
        );
    }
}

#[tokio::test]
async fn removing_a_dimension_key_an_entry_names_is_refused_409() {
    let (f, _, path, input) = setup(0).await;
    let (_, _, tag) = f
        .call("GET", "/dimension-keys", json!({}), None, None)
        .await;
    let declared = f
        .call(
            "PUT",
            "/dimension-keys",
            json!({"items":[{"key":"region","values":["eu","us"]}]}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(declared.0, 200, "{declared:?}");
    let mut input = input;
    input["dimension_key"] = json!("region");
    let created = f.call("POST", &path, input, None, Some("one")).await;
    assert_eq!(created.0, 201, "{created:?}");
    // The entry names the key but has no valued price: the key itself is still in use.
    let refused = f
        .call(
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
    // Changing its values is still allowed: no price carries one.
    let changed = f
        .call(
            "PUT",
            "/dimension-keys",
            json!({"items":[{"key":"region","values":["ap","eu"]}]}),
            Some(&declared.2),
            None,
        )
        .await;
    assert_eq!(changed.0, 200, "{changed:?}");
}

// Docs F7, spec decision 4: a tenant's registry starts seeded with `region`, declared with no
// values yet; the first entry naming it stores the seed in the entry's own transaction.
#[tokio::test]
async fn the_registry_is_seeded_with_region_and_an_entry_naming_it_stores_the_seed() {
    let (f, _, path, input) = setup(0).await;
    let (status, dims, seed_tag) = f
        .call("GET", "/dimension-keys", json!({}), None, None)
        .await;
    assert_eq!(status, 200, "{dims}");
    assert_eq!(dims, json!({"items":[{"key":"region","values":[]}]}));
    let mut body = input.clone();
    body["dimension_key"] = json!("region");
    let (status, entry, _) = f.call("POST", &path, body, None, Some("region")).await;
    assert_eq!(status, 201, "{entry}");
    assert_eq!(entry["dimension_key"], "region");
    let (status, dims, stored_tag) = f
        .call("GET", "/dimension-keys", json!({}), None, None)
        .await;
    assert_eq!(status, 200, "{dims}");
    assert_eq!(dims, json!({"items":[{"key":"region","values":[]}]}));
    assert_ne!(stored_tag, seed_tag, "the seed is now a stored row");
    let prices = format!(
        "/price-book-entries/{}/prices",
        entry["id"].as_str().unwrap()
    );
    let eu = json!({"model":"per_unit","price":{"rate":"0.10"},"eligibility":"all",
        "effective_from":"2031-03-01","dim_value":"eu"});
    let (status, b, _) = f.call("POST", &prices, eu.clone(), None, Some("eu")).await;
    assert_eq!(status, 400, "no value is declared yet: {b}");
    assert!(b.to_string().contains("DIM_VALUE_UNKNOWN"), "{b}");
    let (status, b, _) = f
        .call(
            "PUT",
            "/dimension-keys",
            json!({"items":[{"key":"region","values":["eu","us"]}]}),
            Some(&stored_tag),
            None,
        )
        .await;
    assert_eq!(status, 200, "{b}");
    let (status, b, _) = f.call("POST", &prices, eu, None, Some("eu2")).await;
    assert_eq!(status, 201, "{b}");
    // PATCH names the seeded key on another new tenant the same way.
    let (f, _, path, input) = setup(0).await;
    let (status, entry, tag) = f.call("POST", &path, input, None, Some("plain")).await;
    assert_eq!(status, 201, "{entry}");
    let (status, b, _) = f
        .call(
            "PATCH",
            &format!("/price-book-entries/{}", entry["id"].as_str().unwrap()),
            json!({"dimension_key":"region"}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(status, 200, "{b}");
    let (_, _, stored) = f
        .call("GET", "/dimension-keys", json!({}), None, None)
        .await;
    let (_, _, empty) = setup(0)
        .await
        .0
        .call("GET", "/dimension-keys", json!({}), None, None)
        .await;
    assert_ne!(stored, empty, "PATCH stored the seed in its transaction");
}

async fn raw(f: &Fixture, sql: &str) {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    Database::connect(&f.dsn)
        .await
        .unwrap()
        .execute_raw(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .await
        .unwrap();
}

// Behaviour LOW-2: a door that fails after its reserve and before its entry write cancels the
// create before it answers, as a 503 does. Here Tx B stays contended past its retries, so the
// door answers 409 CONTENDED. No ticker pass turns that answer into an entry: the receipt is
// released, and a same-key retry runs afresh.
#[tokio::test]
async fn a_contended_entry_write_after_the_reserve_cancels_the_create() {
    let (f, script, path, input) = setup(0).await;
    raw(
        &f,
        "CREATE TRIGGER entry_busy BEFORE INSERT ON pricing_price_book_entry \
         BEGIN SELECT RAISE(ABORT, '(code: 5) database is locked'); END",
    )
    .await;
    let first = f
        .call("POST", &path, input.clone(), None, Some("one"))
        .await;
    assert_eq!(first.0, 409, "{first:?}");
    assert!(first.1.to_string().contains("CONTENDED"), "{first:?}");
    assert_eq!(
        Script::count(&script.reserve_calls),
        1,
        "the reserve succeeded"
    );
    raw(&f, "DROP TRIGGER entry_busy").await;
    bss_pricing::infra::reference_ticker::Ticker::new(
        f.state.clone(),
        Arc::new(LaterClock),
        10,
        100,
    )
    .tick()
    .await
    .unwrap();
    let (status, listed, _) = f.call("GET", &path, json!({}), None, None).await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(
        listed["items"],
        json!([]),
        "the answered 409 wrote no entry"
    );
    assert_eq!(
        Script::count(&script.releases),
        1,
        "the cancellation released the receipt"
    );
    let retry = f.call("POST", &path, input, None, Some("one")).await;
    assert_eq!(retry.0, 201, "a same-key retry runs afresh: {retry:?}");
    assert_eq!(retry.1["reference_state"], "confirmed");
    let (_, listed, _) = f.call("GET", &path, json!({}), None, None).await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
}

// Rename L1: a missing entry is a missing price_book_entry with ENTRY_NOT_FOUND on every entry
// door, never a missing price book or a missing price.
#[tokio::test]
async fn a_missing_entry_is_named_as_an_entry_on_every_door() {
    let (f, _, _, _) = setup(0).await;
    let path = format!("/price-book-entries/{}", Uuid::new_v4());
    let draft = json!({"model":"per_unit","price":{"rate":"0.10"},"eligibility":"all","effective_from":"2031-01-01"});
    for (method, path, body, tag, key) in [
        ("GET", path.clone(), json!({}), None, None),
        (
            "PATCH",
            path.clone(),
            json!({"invoice_line_override":"{sku}"}),
            Some("\"1\""),
            None,
        ),
        ("DELETE", path.clone(), json!({}), None, None),
        ("POST", format!("{path}/prices"), draft, None, Some("k")),
    ] {
        let (status, b, _) = f.call(method, &path, body, tag, key).await;
        assert_eq!(status, 404, "{method} {path}: {b}");
        assert_eq!(
            b["context"]["resource_name"], "price_book_entry",
            "{method}: {b}"
        );
        assert!(
            b["detail"]
                .as_str()
                .unwrap_or_default()
                .starts_with("ENTRY_NOT_FOUND"),
            "{method}: {b}"
        );
    }
}
