//! Plan items and revision checks through the production router (run 3.3, Task 3.3.2): the item
//! door's refusals before any reservation, its create op (D-407), the draft-only PATCH and DELETE
//! of the revision's author (D-404), and `GET /plan-revisions/{id}/checks` over fresh SKU reads
//! (D-408), with `blocked_by` naming the pending price unit (spec §8).
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod plan_support;
use bss_pricing::{
    api::rest::authoring::{dto, plan_items},
    infra::storage::repo::{price_book_entry_repo, price_repo},
};
use bss_products_sdk::models::{Lifecycle, ReferenceKind, SkuType};
use plan_support::{
    Fixture, book, entry, holding, id_of, item, item_with_qty, items, lock, ops_for, plan, publish,
    request, scope, setup, stranger, text,
};
use serde_json::{Value, json};
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

async fn add(f: &Fixture, revision: Uuid, body: Value, key: &str) -> (u16, Value, String) {
    f.call(
        "POST",
        &format!("/plan-revisions/{revision}/items"),
        body,
        None,
        Some(key),
    )
    .await
}
async fn checks(f: &Fixture, revision: Uuid) -> (u16, Value) {
    let (s, b, _) = f
        .call(
            "GET",
            &format!("/plan-revisions/{revision}/checks"),
            json!({}),
            None,
            None,
        )
        .await;
    (s, b)
}
fn row<'a>(body: &'a Value, code: &str) -> &'a Value {
    body["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["code"] == code)
        .unwrap_or_else(|| panic!("no {code} in {body}"))
}
/// An approved price of `entry` from `from`, written directly.
async fn approved(f: &Fixture, entry: Uuid, from: &str) {
    let tenant = f.ctx.subject_tenant_id();
    let conn = f.db.conn().unwrap();
    let e = price_book_entry_repo::find(&conn, &scope(f), tenant, entry)
        .await
        .unwrap()
        .unwrap();
    let mut p = plan_support::entry_support::price(&e);
    p.state = "approved".into();
    p.effective_from =
        time::Date::parse(from, &time::format_description::well_known::Iso8601::DATE).unwrap();
    price_repo::insert(&conn, &scope(f), p).await.unwrap();
}
async fn available_from(f: &Fixture, revision: Uuid, date: &str) {
    let path = format!("/plan-revisions/{revision}");
    let (_, _, tag) = f.call("GET", &path, json!({}), None, None).await;
    let (s, b, _) = f
        .call(
            "PATCH",
            &path,
            json!({"available_from":date}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
}

#[tokio::test]
async fn an_item_is_added_through_its_door_confirmed_and_its_key_replays() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    let seats = catalog.sku(SkuType::Recurring);
    let e = entry(&f, eur, seats, "recurring", Some("month")).await;
    let body = json!({"sku_id":seats,"price_book_entry_id":e,"treatment":"paid","qty_min":1});
    let path = format!("/plan-revisions/{rev}/items");
    assert_eq!(
        f.call("POST", &path, body.clone(), None, None).await.0,
        400,
        "an Idempotency-Key is required"
    );
    let created = add(&f, rev, body.clone(), "one").await;
    assert_eq!(created.0, 201, "{created:?}");
    assert_eq!(created.2, "\"2\"", "written, then confirmed");
    let it = &created.1;
    assert_eq!(it["reference_state"], "confirmed");
    assert_eq!(it["revision_id"], rev.to_string());
    assert_eq!(it["sku_id"], seats.to_string());
    assert_eq!(it["price_book_entry_id"], e.to_string());
    assert_eq!(it["treatment"], "paid");
    assert_eq!(it["qty_min"], 1);
    assert_eq!(it["created_by"], f.ctx.subject_id().to_string());
    assert_eq!(add(&f, rev, body, "one").await, created, "the key replays");
    let other = add(
        &f,
        rev,
        json!({"sku_id":seats,"price_book_entry_id":e,"treatment":"optional"}),
        "one",
    )
    .await;
    assert_eq!(other.0, 409, "{other:?}");
    assert!(text(&other.1).contains("IDEMPOTENCY_CONFLICT"), "{other:?}");
    assert_eq!(
        *catalog.reserve_kinds.lock().unwrap(),
        [ReferenceKind::PlanItem]
    );
    let (_, r, _) = f
        .call(
            "GET",
            &format!("/plan-revisions/{rev}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(r["items"], json!([it]));
}

#[tokio::test]
async fn item_door_refusals_are_answered_before_any_reservation() {
    let (f, catalog) = setup().await;
    let (eur, other) = (book(&f, "eur").await, book(&f, "other").await);
    let (_, rev) = plan(&f, "pro", eur).await;
    let (seats, storage, bundle) = (
        catalog.sku(SkuType::Recurring),
        catalog.sku(SkuType::Usage),
        catalog.sku(SkuType::Bundle),
    );
    let old = catalog.sku(SkuType::Usage);
    catalog.age(old, Lifecycle::Deprecated);
    let seats_eur = entry(&f, eur, seats, "recurring", Some("month")).await;
    let seats_other = entry(&f, other, seats, "recurring", Some("month")).await;
    for (body, status, code) in [
        (
            json!({"sku_id":seats,"price_book_entry_id":seats_other,"treatment":"paid"}),
            400,
            "ITEM_BOOK_FOREIGN",
        ),
        (
            json!({"sku_id":storage,"price_book_entry_id":seats_eur,"treatment":"paid"}),
            400,
            "ITEM_ENTRY_SKU_MISMATCH",
        ),
        (
            json!({"sku_id":storage,"treatment":"paid"}),
            400,
            "ITEM_ENTRY_MISSING",
        ),
        (
            json!({"sku_id":storage,"price_book_entry_id":null,"treatment":"optional"}),
            400,
            "ITEM_ENTRY_MISSING",
        ),
        (
            json!({"sku_id":old,"treatment":"included","included_qty":"5"}),
            400,
            "ITEM_SKU_DEPRECATED",
        ),
        (
            json!({"sku_id":bundle,"treatment":"included"}),
            400,
            "ITEM_BUNDLE_SKU",
        ),
        (
            json!({"sku_id":storage,"treatment":"free"}),
            400,
            "TREATMENT_INVALID",
        ),
        (
            json!({"sku_id":storage,"treatment":"included","included_qty":"-1"}),
            400,
            "INCLUDED_QTY_INVALID",
        ),
        (
            json!({"sku_id":storage,"treatment":"included","included_qty":"1e3"}),
            400,
            "INCLUDED_QTY_INVALID",
        ),
        (
            json!({"sku_id":storage,"treatment":"included","qty_min":-1}),
            400,
            "QTY_MIN_INVALID",
        ),
        (
            json!({"sku_id":storage,"price_book_entry_id":Uuid::new_v4(),"treatment":"paid"}),
            404,
            "ENTRY_NOT_FOUND",
        ),
    ] {
        let key = Uuid::new_v4().to_string();
        let (s, b, _) = add(&f, rev, body.clone(), &key).await;
        assert_eq!(s, status, "{body}: {b}");
        assert!(text(&b).contains(code), "{body}: {b}");
    }
    assert_eq!(catalog.reserves(), 0, "no refusal cost a reservation");
    assert!(items(&f, rev).await.is_empty());
    let ok = add(
        &f,
        rev,
        json!({"sku_id":storage,"treatment":"included","included_qty":"2.5"}),
        "ok",
    )
    .await;
    assert_eq!(ok.0, 201, "{ok:?}");
    assert_eq!(ok.1["included_qty"], "2.5");
    let taken = add(
        &f,
        rev,
        json!({"sku_id":storage,"treatment":"included"}),
        "taken",
    )
    .await;
    assert_eq!(taken.0, 409, "{taken:?}");
    assert!(text(&taken.1).contains("ITEM_SKU_TAKEN"), "{taken:?}");
    assert_eq!(
        catalog.reserves(),
        1,
        "the taken SKU cost no second reservation"
    );
}

#[tokio::test]
async fn a_revision_holds_at_most_two_hundred_items_at_the_door_and_at_the_write() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    for _ in 0..200 {
        item(&f, rev, catalog.sku(SkuType::Usage), None, "included").await;
    }
    let extra = catalog.sku(SkuType::Usage);
    let body = json!({"sku_id":extra,"treatment":"included","included_qty":"1"});
    let (s, b, _) = add(&f, rev, body.clone(), "extra").await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("REVISION_ITEMS_TOO_MANY"), "{b}");
    assert_eq!(catalog.reserves(), 0);
    // Below the door, the write itself refuses the 201st item: the create op cancels and its
    // receipt is released.
    let input: dto::PricingPlanItemCreate = serde_json::from_value(body.clone()).unwrap();
    let digest = bss_pricing::api::rest::preconditions::request_digest(&body).unwrap();
    let (s, b, _) = plan_support::entry_support::answer(
        plan_items::create(
            f.state.clone(),
            AccessScope::for_tenant(f.ctx.subject_tenant_id()),
            f.ctx.clone(),
            rev,
            Uuid::now_v7(),
            "below".into(),
            digest,
            input,
        )
        .await,
    )
    .await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("REVISION_ITEMS_TOO_MANY"), "{b}");
    assert_eq!((catalog.reserves(), catalog.releases()), (1, 1));
    assert_eq!(items(&f, rev).await.len(), 200);
}

#[tokio::test]
async fn items_are_added_only_to_an_unlocked_draft_by_its_author() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    let body =
        json!({"sku_id":catalog.sku(SkuType::Usage),"treatment":"included","included_qty":"1"});
    let colleague = f.user();
    let (s, b, _) = f
        .call_as(
            &colleague,
            "POST",
            &format!("/plan-revisions/{rev}/items"),
            body.clone(),
            None,
            Some("theirs"),
        )
        .await;
    assert_eq!(s, 403, "D-404: {b}");
    assert!(text(&b).contains("NOT_DRAFT_AUTHOR"), "{b}");
    let (s, b, _) = add(&f, Uuid::new_v4(), body.clone(), "unknown").await;
    assert_eq!(s, 404, "{b}");
    lock(&f, rev).await;
    let (s, b, _) = add(&f, rev, body, "locked").await;
    assert_eq!(s, 409, "{b}");
    assert!(text(&b).contains("REVISION_NOT_DRAFT"), "{b}");
    assert_eq!(catalog.reserves(), 0);
}

#[tokio::test]
async fn an_item_is_edited_under_if_match_only_as_a_draft_by_its_revision_author() {
    let (f, catalog) = setup().await;
    let (eur, other) = (book(&f, "eur").await, book(&f, "other").await);
    let (_, rev) = plan(&f, "pro", eur).await;
    let (seats, storage) = (catalog.sku(SkuType::Recurring), catalog.sku(SkuType::Usage));
    let month = entry(&f, eur, seats, "recurring", Some("month")).await;
    let year = entry(&f, eur, seats, "recurring", Some("year")).await;
    let foreign = entry(&f, other, seats, "recurring", Some("month")).await;
    let storage_eur = entry(&f, eur, storage, "usage", None).await;
    let (s, it, _) = add(
        &f,
        rev,
        json!({"sku_id":seats,"price_book_entry_id":month,"treatment":"paid"}),
        "seats",
    )
    .await;
    assert_eq!(s, 201, "{it}");
    let path = format!("/plan-items/{}", it["id"].as_str().unwrap());
    assert_eq!(
        f.call("PATCH", &path, json!({"qty_min":2}), None, None)
            .await
            .0,
        400,
        "If-Match is required"
    );
    let stale = f
        .call("PATCH", &path, json!({"qty_min":2}), Some("\"9\""), None)
        .await;
    assert_eq!(stale.0, 409, "{stale:?}");
    assert!(text(&stale.1).contains("STALE_REVISION"), "{stale:?}");
    let colleague = f.user();
    let theirs = f
        .call_as(
            &colleague,
            "PATCH",
            &path,
            json!({"qty_min":2}),
            Some("\"2\""),
            None,
        )
        .await;
    assert_eq!(theirs.0, 403, "D-404: {theirs:?}");
    assert!(text(&theirs.1).contains("NOT_DRAFT_AUTHOR"), "{theirs:?}");
    for (body, code) in [
        (json!({"sku_id":storage}), "unknown field"),
        (json!({"price_book_entry_id":foreign}), "ITEM_BOOK_FOREIGN"),
        (
            json!({"price_book_entry_id":storage_eur}),
            "ITEM_ENTRY_SKU_MISMATCH",
        ),
        (json!({"price_book_entry_id":null}), "ITEM_ENTRY_MISSING"),
        (json!({"treatment":"free"}), "TREATMENT_INVALID"),
        (json!({"included_qty":"x"}), "INCLUDED_QTY_INVALID"),
        (json!({"qty_min":-3}), "QTY_MIN_INVALID"),
    ] {
        let (s, b, _) = f
            .call("PATCH", &path, body.clone(), Some("\"2\""), None)
            .await;
        assert_eq!(s, 400, "{body}: {b}");
        assert!(text(&b).contains(code), "{body}: {b}");
    }
    let (s, b, tag) = f
        .call(
            "PATCH",
            &path,
            json!({"treatment":"optional","price_book_entry_id":year,"qty_min":2}),
            Some("\"2\""),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(tag, "\"3\"");
    assert_eq!(
        (&b["treatment"], &b["price_book_entry_id"], &b["qty_min"]),
        (&json!("optional"), &json!(year.to_string()), &json!(2))
    );
    assert_eq!(b["sku_id"], seats.to_string(), "the SKU never changes");
    assert_eq!(b["reference_state"], "confirmed");
    let (s, b, tag) = f
        .call(
            "PATCH",
            &path,
            json!({"treatment":"included","price_book_entry_id":null,"qty_min":null}),
            Some("\"3\""),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(tag, "\"4\"");
    assert_eq!(
        (&b["price_book_entry_id"], &b["qty_min"]),
        (&json!(null), &json!(null))
    );
    let gone = f
        .call_as(&colleague, "DELETE", &path, json!({}), None, None)
        .await;
    assert_eq!(gone.0, 403, "D-404: {gone:?}");
    assert!(text(&gone.1).contains("NOT_DRAFT_AUTHOR"), "{gone:?}");
    lock(&f, rev).await;
    let locked = f
        .call("PATCH", &path, json!({"qty_min":1}), Some("\"4\""), None)
        .await;
    assert_eq!(locked.0, 409, "{locked:?}");
    assert!(text(&locked.1).contains("REVISION_NOT_DRAFT"), "{locked:?}");
    let locked = f.call("DELETE", &path, json!({}), None, None).await;
    assert_eq!(locked.0, 409, "{locked:?}");
    assert!(text(&locked.1).contains("REVISION_NOT_DRAFT"), "{locked:?}");
    let unknown = f
        .call(
            "PATCH",
            &format!("/plan-items/{}", Uuid::new_v4()),
            json!({"qty_min":1}),
            Some("\"1\""),
            None,
        )
        .await;
    assert_eq!(unknown.0, 404, "{unknown:?}");
}

#[tokio::test]
async fn an_item_delete_writes_a_delete_op_and_releases_its_reference() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    let (s, it, _) = add(
        &f,
        rev,
        json!({"sku_id":catalog.sku(SkuType::Usage),"treatment":"included","included_qty":"1"}),
        "one",
    )
    .await;
    assert_eq!(s, 201, "{it}");
    let id = id_of(&it["id"]);
    let path = format!("/plan-items/{id}");
    let (s, b, _) = f.call("DELETE", &path, json!({}), None, None).await;
    assert_eq!(s, 204, "{b}");
    assert!(items(&f, rev).await.is_empty());
    let ops = ops_for(&f, id).await;
    let delete = ops.iter().find(|op| op.kind == "delete").unwrap();
    assert_eq!(delete.state, "done");
    assert_eq!(delete.reservation_id, Some(id_of(&it["reservation_id"])));
    assert_eq!(catalog.releases(), 1);
    assert_eq!(f.call("DELETE", &path, json!({}), None, None).await.0, 404);
}

// D-408 probe: every check reads each item's SKU fresh; a cached SKU would pass a deprecated one.
#[tokio::test]
async fn checks_read_every_sku_fresh_and_answer_the_sale_date() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    available_from(&f, rev, "2031-03-01").await;
    let storage = catalog.sku(SkuType::Usage);
    let e = entry(&f, eur, storage, "usage", None).await;
    approved(&f, e, "2031-01-01").await;
    let (s, b, _) = add(
        &f,
        rev,
        json!({"sku_id":storage,"price_book_entry_id":e,"treatment":"paid"}),
        "one",
    )
    .await;
    assert_eq!(s, 201, "{b}");
    let reads = catalog.reads();
    let (s, green) = checks(&f, rev).await;
    assert_eq!(s, 200, "{green}");
    assert_eq!(green["ready"], true, "{green}");
    assert_eq!(green["sale_date"], "2031-03-01");
    assert_eq!(catalog.reads(), reads + 1, "one fresh read per item SKU");
    let first = &green["checks"][0];
    assert_eq!(first["code"], "PLAN_NAME");
    for field in ["ok", "label", "detail", "info", "blocked_by"] {
        assert!(!first[field].is_null(), "{field}: {first}");
    }
    assert_eq!(row(&green, "APPROVAL")["info"], true);
    catalog.age(storage, Lifecycle::Deprecated);
    let (s, red) = checks(&f, rev).await;
    assert_eq!(s, 200, "{red}");
    assert_eq!(
        row(&red, "ITEM_SKU_DEPRECATED")["ok"],
        false,
        "a SKU deprecated since the last read is red at once: {red}"
    );
    assert_eq!(red["ready"], false);
    assert_eq!(catalog.reads(), reads + 2, "read again, never cached");
    catalog.age(storage, Lifecycle::Retired);
    let (_, red) = checks(&f, rev).await;
    assert_eq!(row(&red, "ITEM_SKU_UNAVAILABLE")["ok"], false, "{red}");
    catalog.skus.lock().unwrap().remove(&storage);
    let (s, red) = checks(&f, rev).await;
    assert_eq!(
        s, 200,
        "a SKU Products no longer knows is unavailable: {red}"
    );
    assert_eq!(row(&red, "ITEM_SKU_UNAVAILABLE")["ok"], false, "{red}");
}

// Spec §8: a revision is red with ITEM_UNCOVERED naming the pending price unit, and turns green
// once that unit is approved; blocked_by is computed, never stored.
#[tokio::test]
async fn checks_name_the_pending_price_unit_that_would_cover_an_item() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    available_from(&f, rev, "2031-03-01").await;
    let storage = catalog.sku(SkuType::Usage);
    let e = entry(&f, eur, storage, "usage", None).await;
    let (s, b, _) = add(
        &f,
        rev,
        json!({"sku_id":storage,"price_book_entry_id":e,"treatment":"paid"}),
        "one",
    )
    .await;
    assert_eq!(s, 201, "{b}");
    let (_, _, tag) = f
        .call("GET", "/approval-policy", json!({}), None, None)
        .await;
    let (s, b, _) = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"kind":"prices","quorum":1}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    let (s, drafted, _) = f
        .call(
            "POST",
            &format!("/price-book-entries/{e}/prices"),
            json!({"model":"per_unit","price":{"rate":"0.10"},"eligibility":"all","effective_from":"2031-03-01"}),
            None,
            Some("price"),
        )
        .await;
    assert_eq!(s, 201, "{drafted}");
    let price = drafted["items"][0]["id"].as_str().unwrap();
    let (s, receipt, _) = f
        .call(
            "POST",
            &format!("/prices/{price}/submit"),
            json!({}),
            None,
            Some("submit"),
        )
        .await;
    assert_eq!(s, 201, "{receipt}");
    assert_eq!(receipt["applied"], false);
    let unit = receipt["unit"]["id"].as_str().unwrap();
    let (s, red) = checks(&f, rev).await;
    assert_eq!(s, 200, "{red}");
    assert_eq!(red["ready"], false);
    let uncovered = row(&red, "ITEM_UNCOVERED");
    assert_eq!(uncovered["ok"], false, "{red}");
    assert_eq!(uncovered["blocked_by"], json!([unit]));
    let (s, b, _) = f
        .call_as(
            &f.user(),
            "POST",
            &format!("/approval-units/{unit}/approve"),
            json!({"generation":1}),
            None,
            Some("approve"),
        )
        .await;
    assert_eq!(s, 200, "{b}");
    let (_, green) = checks(&f, rev).await;
    assert_eq!(row(&green, "ITEM_UNCOVERED")["ok"], true, "{green}");
    assert_eq!(row(&green, "ITEM_UNCOVERED")["blocked_by"], json!([]));
    assert_eq!(green["ready"], true, "{green}");
}

#[tokio::test]
async fn a_book_change_leaves_an_unmatched_item_foreign_in_the_checks() {
    let (f, catalog) = setup().await;
    let (eur, other) = (book(&f, "eur").await, book(&f, "other").await);
    let (_, rev) = plan(&f, "pro", eur).await;
    let (seats, storage) = (catalog.sku(SkuType::Recurring), catalog.sku(SkuType::Usage));
    let seats_eur = entry(&f, eur, seats, "recurring", Some("month")).await;
    entry(&f, other, seats, "recurring", Some("month")).await;
    let storage_eur = entry(&f, eur, storage, "usage", None).await;
    for (sku, e, key) in [
        (seats, seats_eur, "seats"),
        (storage, storage_eur, "storage"),
    ] {
        let (s, b, _) = add(
            &f,
            rev,
            json!({"sku_id":sku,"price_book_entry_id":e,"treatment":"paid"}),
            key,
        )
        .await;
        assert_eq!(s, 201, "{b}");
    }
    let (_, before) = checks(&f, rev).await;
    assert_eq!(row(&before, "ITEM_BOOK_FOREIGN")["ok"], true, "{before}");
    let path = format!("/plan-revisions/{rev}");
    let (_, _, tag) = f.call("GET", &path, json!({}), None, None).await;
    let (s, b, _) = f
        .call("PATCH", &path, json!({"book_id":other}), Some(&tag), None)
        .await;
    assert_eq!(s, 200, "{b}");
    let (_, after) = checks(&f, rev).await;
    let foreign = row(&after, "ITEM_BOOK_FOREIGN");
    assert_eq!(foreign["ok"], false, "{after}");
    let detail = foreign["detail"].as_str().unwrap();
    assert!(detail.contains(&catalog.name(storage)), "{detail}");
    assert!(
        !detail.contains(&catalog.name(seats)),
        "the remapped item is not foreign: {detail}"
    );
}

#[tokio::test]
async fn checks_answer_503_when_the_registry_is_down() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    let (s, b, _) = add(
        &f,
        rev,
        json!({"sku_id":catalog.sku(SkuType::Usage),"treatment":"included","included_qty":"1"}),
        "one",
    )
    .await;
    assert_eq!(s, 201, "{b}");
    catalog
        .down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (s, b) = checks(&f, rev).await;
    assert_eq!(s, 503, "{b}");
    assert!(text(&b).contains("REGISTRY_UNAVAILABLE"), "{b}");
    let (s, b) = checks(&f, Uuid::new_v4()).await;
    assert_eq!(
        s, 404,
        "an unknown revision is 404 before any registry read: {b}"
    );
}

// D-408: a deprecated SKU may stay in a new revision of the same plan that carries it over from
// the published revision; it cannot be added.
#[tokio::test]
async fn a_copy_keeps_a_carried_over_deprecated_sku_green_and_a_new_one_is_refused() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (p, rev1) = plan(&f, "pro", eur).await;
    let old = catalog.sku(SkuType::Usage);
    item_with_qty(&f, rev1, old, "10").await;
    publish(&f, id_of(&p["id"]), rev1).await;
    catalog.age(old, Lifecycle::Deprecated);
    let (s, copied, _) = f
        .call(
            "POST",
            &format!("/plans/{}/revisions", p["id"].as_str().unwrap()),
            json!({}),
            None,
            Some("copy"),
        )
        .await;
    assert_eq!(s, 201, "{copied}");
    let rev2 = id_of(&copied["id"]);
    let (s, b) = checks(&f, rev2).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        row(&b, "ITEM_SKU_DEPRECATED")["ok"],
        true,
        "carried over: {b}"
    );
    assert_eq!(
        row(&b, "ITEM_REFERENCE_PENDING")["ok"],
        true,
        "the attach confirmed: {b}"
    );
    assert_eq!(b["ready"], true, "{b}");
    let another = catalog.sku(SkuType::Usage);
    catalog.age(another, Lifecycle::Deprecated);
    let (s, b, _) = add(
        &f,
        rev2,
        json!({"sku_id":another,"treatment":"included","included_qty":"1"}),
        "new",
    )
    .await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("ITEM_SKU_DEPRECATED"), "{b}");
}

#[tokio::test]
async fn item_and_check_doors_need_the_plan_permissions() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    let (s, it, _) = add(
        &f,
        rev,
        json!({"sku_id":catalog.sku(SkuType::Usage),"treatment":"included","included_qty":"1"}),
        "one",
    )
    .await;
    assert_eq!(s, 201, "{it}");
    let item_path = format!("/plan-items/{}", it["id"].as_str().unwrap());
    let checks_path = format!("/plan-revisions/{rev}/checks");
    let reader = holding(&f, "plan:read");
    let (s, b, _) = request(&f.app, &reader, "GET", &checks_path, json!({}), None, None).await;
    assert_eq!(s, 200, "{b}");
    let (s, _, _) = request(
        &f.app,
        &holding(&f, "price_book:read"),
        "GET",
        &checks_path,
        json!({}),
        None,
        None,
    )
    .await;
    assert_eq!(s, 403);
    let (s, _, _) = request(
        &f.app,
        &stranger(),
        "GET",
        &checks_path,
        json!({}),
        None,
        None,
    )
    .await;
    assert_eq!(s, 404, "another tenant's revision is not disclosed");
    for (method, path, body, tag, key) in [
        (
            "POST",
            format!("/plan-revisions/{rev}/items"),
            json!({"sku_id":Uuid::new_v4(),"treatment":"included"}),
            None,
            Some("k"),
        ),
        (
            "PATCH",
            item_path.clone(),
            json!({"qty_min":1}),
            Some("\"2\""),
            None,
        ),
        ("DELETE", item_path.clone(), json!({}), None, None),
    ] {
        let (s, b, _) = request(&f.app, &reader, method, &path, body.clone(), tag, key).await;
        assert_eq!(s, 403, "a reader may not author: {method} {path}: {b}");
        let (s, b, _) = request(&f.app, &stranger(), method, &path, body, tag, key).await;
        assert_eq!(
            s, 403,
            "a stranger may not write here: {method} {path}: {b}"
        );
    }
    assert_eq!(items(&f, rev).await.len(), 1);
}
