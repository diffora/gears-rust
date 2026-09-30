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
    Catalog, Fixture, book, entry, holding, id_of, item, items, lock, ops_for, plan, publish,
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
/// A fresh usage SKU and its entry of `book`, written directly: an item's create body names both
/// (D-467).
async fn priced(f: &Fixture, catalog: &Catalog, book: Uuid) -> (Uuid, Uuid) {
    let sku = catalog.sku(SkuType::Usage);
    (sku, entry(f, book, sku, "usage", None).await)
}
/// The create body of an item: a SKU and its entry (D-467).
fn body(sku: Uuid, entry: Uuid) -> Value {
    json!({"sku_id":sku,"price_book_entry_id":entry})
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
    let body = body(seats, e);
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
    for removed in ["treatment", "included_qty", "qty_min"] {
        assert!(it.get(removed).is_none(), "D-467, no {removed}: {it}");
    }
    assert_eq!(it["created_by"], f.ctx.subject_id().to_string());
    assert_eq!(add(&f, rev, body, "one").await, created, "the key replays");
    let other_entry = entry(&f, eur, seats, "recurring", Some("year")).await;
    let other = add(
        &f,
        rev,
        json!({"sku_id":seats,"price_book_entry_id":other_entry}),
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
    let storage_eur = entry(&f, eur, storage, "usage", None).await;
    let old_eur = entry(&f, eur, old, "usage", None).await;
    let bundle_eur = entry(&f, eur, bundle, "one_time", None).await;
    for (body, status, code) in [
        (
            json!({"sku_id":seats,"price_book_entry_id":seats_other}),
            400,
            "ITEM_BOOK_FOREIGN",
        ),
        (
            json!({"sku_id":storage,"price_book_entry_id":seats_eur}),
            400,
            "ITEM_ENTRY_SKU_MISMATCH",
        ),
        (json!({"sku_id":storage}), 400, "ITEM_ENTRY_MISSING"),
        (
            json!({"sku_id":storage,"price_book_entry_id":null}),
            400,
            "ITEM_ENTRY_MISSING",
        ),
        (body(old, old_eur), 400, "ITEM_SKU_DEPRECATED"),
        (body(bundle, bundle_eur), 400, "ITEM_BUNDLE_SKU"),
        (body(storage, Uuid::new_v4()), 404, "ENTRY_NOT_FOUND"),
    ] {
        let key = Uuid::new_v4().to_string();
        let (s, b, _) = add(&f, rev, body.clone(), &key).await;
        assert_eq!(s, status, "{body}: {b}");
        assert!(text(&b).contains(code), "{body}: {b}");
    }
    assert_eq!(catalog.reserves(), 0, "no refusal cost a reservation");
    assert!(items(&f, rev).await.is_empty());
    let ok = add(&f, rev, body(storage, storage_eur), "ok").await;
    assert_eq!(ok.0, 201, "{ok:?}");
    let taken = add(&f, rev, body(storage, storage_eur), "taken").await;
    assert_eq!(taken.0, 409, "{taken:?}");
    assert!(text(&taken.1).contains("ITEM_SKU_TAKEN"), "{taken:?}");
    assert_eq!(
        catalog.reserves(),
        1,
        "the taken SKU cost no second reservation"
    );
}

/// D-467 (owner, 2026-09-30): a plan item is a SKU and its entry. Each of `treatment`,
/// `included_qty` and `qty_min` is 400 `BODY_UNEXPECTED` on that key at both item doors, with
/// nothing reserved or written; the create needs its entry (400 `ITEM_ENTRY_MISSING`), and so does
/// the PATCH, which refuses a null one.
#[tokio::test]
async fn the_item_doors_refuse_treatment_and_the_quantities() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    let (storage, e) = priced(&f, &catalog, eur).await;
    for (key, value) in [
        ("treatment", json!("paid")),
        ("included_qty", json!("10")),
        ("qty_min", json!(1)),
        ("treatment", json!(null)),
    ] {
        let mut create = body(storage, e);
        create[key] = value.clone();
        let (s, b, _) = add(&f, rev, create, &format!("{key}-{value}")).await;
        assert_eq!(s, 400, "{key}: {b}");
        assert!(text(&b).contains("BODY_UNEXPECTED"), "{key}: {b}");
        assert!(text(&b).contains(key), "the field names the key: {b}");
    }
    assert_eq!(catalog.reserves(), 0, "no refusal cost a reservation");
    assert!(items(&f, rev).await.is_empty());
    let (s, b, _) = add(&f, rev, json!({"sku_id":storage}), "no-entry").await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("ITEM_ENTRY_MISSING"), "{b}");
    let (s, it, tag) = add(&f, rev, body(storage, e), "ok").await;
    assert_eq!(s, 201, "{it}");
    let path = format!("/plan-items/{}", it["id"].as_str().unwrap());
    for (key, value) in [
        ("treatment", json!("optional")),
        ("included_qty", json!("250")),
        ("qty_min", json!(0)),
    ] {
        let (s, b, _) = f
            .call("PATCH", &path, json!({ key: value }), Some(&tag), None)
            .await;
        assert_eq!(s, 400, "{key}: {b}");
        assert!(text(&b).contains("BODY_UNEXPECTED"), "{key}: {b}");
    }
    let (s, b, _) = f
        .call(
            "PATCH",
            &path,
            json!({"price_book_entry_id":null}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("ITEM_ENTRY_MISSING"), "{b}");
    let stored = items(&f, rev).await.remove(0);
    assert_eq!(
        (
            stored.treatment.as_str(),
            stored.included_qty,
            stored.qty_min,
            stored.version
        ),
        ("paid", None, None, 2),
        "a new row stores paid and no quantity; the refused PATCHes wrote nothing"
    );
}

#[tokio::test]
async fn a_revision_holds_at_most_two_hundred_items_at_the_door_and_at_the_write() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    for _ in 0..200 {
        let (sku, e) = priced(&f, &catalog, eur).await;
        item(&f, rev, sku, Some(e), "paid").await;
    }
    let (extra, extra_entry) = priced(&f, &catalog, eur).await;
    let body = body(extra, extra_entry);
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
    let (sku, e) = priced(&f, &catalog, eur).await;
    let body = body(sku, e);
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
        json!({"sku_id":seats,"price_book_entry_id":month}),
        "seats",
    )
    .await;
    assert_eq!(s, 201, "{it}");
    let path = format!("/plan-items/{}", it["id"].as_str().unwrap());
    assert_eq!(
        f.call(
            "PATCH",
            &path,
            json!({"price_book_entry_id":year}),
            None,
            None
        )
        .await
        .0,
        400,
        "If-Match is required"
    );
    let stale = f
        .call(
            "PATCH",
            &path,
            json!({"price_book_entry_id":year}),
            Some("\"9\""),
            None,
        )
        .await;
    assert_eq!(stale.0, 409, "{stale:?}");
    assert!(text(&stale.1).contains("STALE_REVISION"), "{stale:?}");
    let colleague = f.user();
    let theirs = f
        .call_as(
            &colleague,
            "PATCH",
            &path,
            json!({"price_book_entry_id":year}),
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
        (json!({"treatment":"optional"}), "BODY_UNEXPECTED"),
        (json!({"qty_min":2}), "BODY_UNEXPECTED"),
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
            json!({"price_book_entry_id":year}),
            Some("\"2\""),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(tag, "\"3\"");
    assert_eq!(b["price_book_entry_id"], json!(year.to_string()));
    assert_eq!(b["sku_id"], seats.to_string(), "the SKU never changes");
    assert_eq!(b["reference_state"], "confirmed");
    let (s, b, tag) = f.call("PATCH", &path, json!({}), Some("\"3\""), None).await;
    assert_eq!(s, 200, "an empty PATCH keeps the entry: {b}");
    assert_eq!(tag, "\"4\"");
    assert_eq!(b["price_book_entry_id"], json!(year.to_string()));
    let gone = f
        .call_as(&colleague, "DELETE", &path, json!({}), None, None)
        .await;
    assert_eq!(gone.0, 403, "D-404: {gone:?}");
    assert!(text(&gone.1).contains("NOT_DRAFT_AUTHOR"), "{gone:?}");
    lock(&f, rev).await;
    let locked = f.call("PATCH", &path, json!({}), Some("\"4\""), None).await;
    assert_eq!(locked.0, 409, "{locked:?}");
    assert!(text(&locked.1).contains("REVISION_NOT_DRAFT"), "{locked:?}");
    let locked = f.call("DELETE", &path, json!({}), None, None).await;
    assert_eq!(locked.0, 409, "{locked:?}");
    assert!(text(&locked.1).contains("REVISION_NOT_DRAFT"), "{locked:?}");
    let unknown = f
        .call(
            "PATCH",
            &format!("/plan-items/{}", Uuid::new_v4()),
            json!({}),
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
    let (sku, e) = priced(&f, &catalog, eur).await;
    let (s, it, _) = add(&f, rev, body(sku, e), "one").await;
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
        json!({"sku_id":storage,"price_book_entry_id":e}),
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
    for field in [
        "ok",
        "label",
        "detail",
        "info",
        "blocked_by",
        "subjects",
        "blocked_by_prices",
    ] {
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
        json!({"sku_id":storage,"price_book_entry_id":e}),
        "one",
    )
    .await;
    assert_eq!(s, 201, "{b}");
    let item_id = b["id"].clone();
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
            json!({"price":{"rate":"0.10"},"eligibility":"all","effective_from":"2031-03-01"}),
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
    // D-466: the row names the item it is about and the pending price behind its unit.
    assert_eq!(
        uncovered["subjects"],
        json!([{"item_id":item_id,"sku_id":storage,"price_book_entry_id":e}])
    );
    assert_eq!(
        uncovered["blocked_by_prices"],
        json!([{"unit_id":unit,"price_id":price,"price_book_entry_id":e}])
    );
    assert_eq!(row(&red, "PLAN_NAME")["subjects"], json!([]), "plan-wide");
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
    assert_eq!(row(&green, "ITEM_UNCOVERED")["subjects"], json!([]));
    assert_eq!(
        row(&green, "ITEM_UNCOVERED")["blocked_by_prices"],
        json!([])
    );
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
        let (s, b, _) = add(&f, rev, json!({"sku_id":sku,"price_book_entry_id":e}), key).await;
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
    let (sku, e) = priced(&f, &catalog, eur).await;
    let (s, b, _) = add(&f, rev, body(sku, e), "one").await;
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
    let old_eur = entry(&f, eur, old, "usage", None).await;
    approved(&f, old_eur, "2020-01-01").await;
    item(&f, rev1, old, Some(old_eur), "paid").await;
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
    let (another, another_eur) = priced(&f, &catalog, eur).await;
    catalog.age(another, Lifecycle::Deprecated);
    let (s, b, _) = add(&f, rev2, body(another, another_eur), "new").await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("ITEM_SKU_DEPRECATED"), "{b}");
}

#[tokio::test]
async fn item_and_check_doors_need_the_plan_permissions() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    let (sku, e) = priced(&f, &catalog, eur).await;
    let (s, it, _) = add(&f, rev, body(sku, e), "one").await;
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
            body(Uuid::new_v4(), e),
            None,
            Some("k"),
        ),
        ("PATCH", item_path.clone(), json!({}), Some("\"2\""), None),
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

/// The `plan-blocked-by` definition of done (AC #15): `blocked_by` is computed from the current
/// pending prices, never stored: once the unit that would cover the item is rejected, or
/// withdrawn, the next check no longer names it and the item is simply uncovered.
#[tokio::test]
async fn a_rejected_or_withdrawn_price_unit_is_no_longer_named_by_the_next_check() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    available_from(&f, rev, "2031-03-01").await;
    let storage = catalog.sku(SkuType::Usage);
    let e = entry(&f, eur, storage, "usage", None).await;
    let (s, b, _) = add(
        &f,
        rev,
        json!({"sku_id":storage,"price_book_entry_id":e}),
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
    for (act, from) in [("reject", "2031-03-01"), ("withdraw", "2031-02-01")] {
        let (s, drafted, _) = f
            .call(
                "POST",
                &format!("/price-book-entries/{e}/prices"),
                json!({"price":{"rate":"0.10"},"eligibility":"all","effective_from":from}),
                None,
                Some(&format!("price-{act}")),
            )
            .await;
        assert_eq!(s, 201, "{act}: {drafted}");
        let price = drafted["items"][0]["id"].as_str().unwrap();
        let (s, receipt, _) = f
            .call(
                "POST",
                &format!("/prices/{price}/submit"),
                json!({}),
                None,
                Some(&format!("submit-{act}")),
            )
            .await;
        assert_eq!(s, 201, "{act}: {receipt}");
        let unit = receipt["unit"]["id"].as_str().unwrap();
        let (_, red) = checks(&f, rev).await;
        assert_eq!(
            row(&red, "ITEM_UNCOVERED")["blocked_by"],
            json!([unit]),
            "{act}: {red}"
        );
        let (who, body) = if act == "reject" {
            (f.user(), json!({"generation":1,"note":"not yet"}))
        } else {
            (f.ctx.clone(), json!({}))
        };
        let (s, b, _) = f
            .call_as(
                &who,
                "POST",
                &format!("/approval-units/{unit}/{act}"),
                body,
                None,
                Some(act),
            )
            .await;
        assert_eq!(s, 200, "{act}: {b}");
        let (_, next) = checks(&f, rev).await;
        let uncovered = row(&next, "ITEM_UNCOVERED");
        assert_eq!(uncovered["ok"], false, "{act}: still uncovered: {next}");
        assert_eq!(
            uncovered["blocked_by"],
            json!([]),
            "{act}: no dependency outlives the unit: {next}"
        );
        assert_eq!(next["ready"], false);
    }
}

/// D-465 (O-9b): D-408's "newly added" does not cover a re-add. A deprecated SKU that the plan's
/// published revision in effect carries may be added again to its draft after a removal (the door
/// and the create op's SKU re-read both admit it), and its checks stay green; any other deprecated
/// SKU is still 400 `ITEM_SKU_DEPRECATED`, and so is the carried one in a clone, a new plan with
/// no revision in effect.
#[tokio::test]
async fn a_deprecated_sku_the_plan_sells_may_be_added_again_and_no_other() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (p, rev1) = plan(&f, "pro", eur).await;
    let plan_id = id_of(&p["id"]);
    let (carried, carried_eur) = priced(&f, &catalog, eur).await;
    approved(&f, carried_eur, "2020-01-01").await;
    item(&f, rev1, carried, Some(carried_eur), "paid").await;
    publish(&f, plan_id, rev1).await;
    catalog.age(carried, Lifecycle::Deprecated);
    let (another, another_eur) = priced(&f, &catalog, eur).await;
    catalog.age(another, Lifecycle::Deprecated);
    let (s, copied, _) = f
        .call(
            "POST",
            &format!("/plans/{plan_id}/revisions"),
            json!({}),
            None,
            Some("copy"),
        )
        .await;
    assert_eq!(s, 201, "{copied}");
    let rev2 = id_of(&copied["id"]);
    let removed = copied["items"][0]["id"].as_str().unwrap().to_owned();
    let (s, b, _) = f
        .call(
            "DELETE",
            &format!("/plan-items/{removed}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 204, "{b}");
    let (s, b, _) = add(&f, rev2, body(carried, carried_eur), "again").await;
    assert_eq!(s, 201, "the published revision in effect carries it: {b}");
    assert_eq!(b["sku_id"], carried.to_string());
    assert_eq!(b["reference_state"], "confirmed", "{b}");
    let (s, b) = checks(&f, rev2).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(row(&b, "ITEM_SKU_DEPRECATED")["ok"], true, "{b}");
    let (s, b, _) = add(&f, rev2, body(another, another_eur), "another").await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("ITEM_SKU_DEPRECATED"), "{b}");
    let (s, cloned, _) = f
        .call(
            "POST",
            &format!("/plans/{plan_id}/clone"),
            json!({"code":"clone","name":"Clone"}),
            None,
            Some("clone"),
        )
        .await;
    assert_eq!(s, 201, "{cloned}");
    let clone_rev1 = id_of(&cloned["revisions"][0]["id"]);
    let copy = items(&f, clone_rev1).await;
    let (s, b, _) = f
        .call(
            "DELETE",
            &format!("/plan-items/{}", copy[0].id),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 204, "{b}");
    let (s, b, _) = add(&f, clone_rev1, body(carried, carried_eur), "clone-again").await;
    assert_eq!(
        s, 400,
        "a clone is a new plan with no revision in effect: {b}"
    );
    assert!(text(&b).contains("ITEM_SKU_DEPRECATED"), "{b}");
}
