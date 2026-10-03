//! D-522 on `SQLite`: a finished price book is archived, and archiving it releases its entries'
//! SKU references (ask 58).
//!
//! - `POST /price-books/{id}/archive` under If-Match: refused while a plan revision that is not
//!   superseded names the book (`BOOK_IN_PLAN`) or a price of it is pending (`BOOK_HAS_PENDING`).
//!   Each entry's reference is released through a `release` op, driven after the commit; the
//!   ticker finishes a release the door could not.
//! - The archived book's entries and prices are read-only: `BOOK_ARCHIVED`.
//! - `POST /price-books/{id}/unarchive` re-reserves each released entry whose SKU still admits a
//!   reference and lists the entries it could not.
//! - `GET /price-books` hides an archived book unless asked `archived eq true`.
//!
//! The Postgres twin is `postgres_book_archive.rs`. Products' side of ask 58 (the SKU then
//! retires and archives) runs in products' `tests/book_archive_e2e.rs`, where both gears run.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod book_support;
mod plan_support;
use book_support::bare_revision;
use bss_pricing::infra::{
    reference_ticker::Ticker,
    reference_work::Clock,
    storage::repo::{price_book_entry_repo, price_repo},
};
use bss_products_sdk::models::{Lifecycle, ReferenceState, SkuType};
use plan_support::{Catalog, Fixture, entry_support, id_of, item, ops_for, plan, publish, scope};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use uuid::Uuid;

struct Later;
impl Clock for Later {
    fn now(&self) -> time::OffsetDateTime {
        time::OffsetDateTime::now_utc() + time::Duration::days(2)
    }
}

fn future(days: i64) -> String {
    (time::OffsetDateTime::now_utc().date() + time::Duration::days(days)).to_string()
}

async fn quorum(f: &Fixture, quorum: u32) {
    let (_, _, tag) = f
        .call("GET", "/approval-policy", json!({}), None, None)
        .await;
    let (s, b, _) = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"quorum": quorum}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
}

async fn new_book(f: &Fixture, code: &str) -> Uuid {
    plan_support::book(f, code).await
}

/// A recurring entry of `book` for `sku`, through its door: reserved, written and confirmed.
async fn door_entry(f: &Fixture, book: Uuid, sku: Uuid) -> Uuid {
    let (s, b, _) = f
        .call(
            "POST",
            &format!("/price-books/{book}/entries"),
            json!({"sku_id": sku, "period": "month", "model": "per_unit"}),
            None,
            Some(&Uuid::new_v4().to_string()),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    assert_eq!(b["reference_state"], "confirmed", "{b}");
    id_of(&b["id"])
}

/// An approved price of `entry` that starts `days` from today, written through the repository.
async fn approved_from(f: &Fixture, entry: Uuid, days: i64) -> Uuid {
    let tenant = f.ctx.subject_tenant_id();
    let conn = f.db.conn().unwrap();
    let e = price_book_entry_repo::find(&conn, &scope(f), tenant, entry)
        .await
        .unwrap()
        .unwrap();
    let mut price = entry_support::price(&e);
    price.state = "approved".into();
    price.effective_from = time::OffsetDateTime::now_utc().date() + time::Duration::days(days);
    price_repo::insert(&conn, &scope(f), price)
        .await
        .unwrap()
        .id
}

/// An approved price of `entry`, written through the repository.
async fn approved(f: &Fixture, entry: Uuid) -> Uuid {
    let tenant = f.ctx.subject_tenant_id();
    let conn = f.db.conn().unwrap();
    let e = price_book_entry_repo::find(&conn, &scope(f), tenant, entry)
        .await
        .unwrap()
        .unwrap();
    let mut price = entry_support::price(&e);
    price.state = "approved".into();
    price.effective_from = time::OffsetDateTime::now_utc().date() - time::Duration::days(10);
    price_repo::insert(&conn, &scope(f), price)
        .await
        .unwrap()
        .id
}

/// A draft price of `entry`, through its door.
async fn draft(f: &Fixture, entry: Uuid) -> Value {
    let (s, b, _) = f
        .call(
            "POST",
            &format!("/price-book-entries/{entry}/prices"),
            json!({"price": {"rate": "0.10"}, "eligibility": "all", "effective_from": future(40)}),
            None,
            Some(&Uuid::new_v4().to_string()),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    b["items"][0].clone()
}

/// One price of `entry` as the entry's price list reads it.
async fn price_of(f: &Fixture, entry: Uuid, price: &str) -> Value {
    let (s, b, _) = f
        .call(
            "GET",
            &format!("/price-book-entries/{entry}/prices"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    b["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == price)
        .unwrap_or_else(|| panic!("{price} is listed: {b}"))
        .clone()
}

async fn book_tag(f: &Fixture, book: Uuid) -> String {
    let (s, b, tag) = f
        .call(
            "GET",
            &format!("/price-books/{book}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    tag
}

async fn mark(f: &Fixture, book: Uuid, what: &str, tag: Option<&str>) -> (u16, Value, String) {
    f.call(
        "POST",
        &format!("/price-books/{book}/{what}"),
        json!({}),
        tag,
        None,
    )
    .await
}

/// Archive at the book's current tag; it must answer 200.
async fn archive(f: &Fixture, book: Uuid) -> Value {
    let tag = book_tag(f, book).await;
    let (s, b, _) = mark(f, book, "archive", Some(&tag)).await;
    assert_eq!(s, 200, "{b}");
    b
}

async fn reference_state(f: &Fixture, entry: Uuid) -> String {
    let (s, b, _) = f
        .call(
            "GET",
            &format!("/price-book-entries/{entry}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    b["reference_state"].as_str().unwrap().to_owned()
}

/// The catalog's state of `entry`'s reference.
fn held(catalog: &Catalog, entry: Uuid) -> ReferenceState {
    catalog.refs.lock().unwrap()[&entry].1
}

fn codes(page: &Value) -> Vec<String> {
    let mut codes: Vec<String> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["code"].as_str().unwrap().to_owned())
        .collect();
    codes.sort_unstable();
    codes
}

async fn listed(f: &Fixture, query: &str) -> Vec<String> {
    let (s, b, _) = f
        .call(
            "GET",
            &format!("/price-books{query}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    codes(&b)
}

/// The answer's status and its code, exactly: a conflict's reason, or a 400's first violation.
fn refused(answer: &(u16, Value, String), status: u16, code: &str) {
    assert_eq!(answer.0, status, "{answer:?}");
    let context = &answer.1["context"];
    let found = context["reason"]
        .as_str()
        .or_else(|| context["field_violations"][0]["reason"].as_str());
    assert_eq!(found, Some(code), "{answer:?}");
}

/// Ask 58: a book with an entry, an approved price and only a superseded plan revision is
/// archived. Each entry's reference is released in Products, the entries read `released`, and
/// the book leaves the list, reading by id as archived.
#[tokio::test]
async fn ask_58_a_finished_book_archives_and_releases_its_sku_references() {
    let (f, catalog) = plan_support::setup().await;
    let sold = catalog.sku(SkuType::Recurring);
    let planned = catalog.sku(SkuType::Recurring);
    let book = new_book(&f, "finished").await;
    let other = new_book(&f, "other").await;
    let e1 = door_entry(&f, book, sold).await;
    let e2 = door_entry(&f, book, planned).await;
    approved(&f, e1).await;
    // A plan whose revision on the book is superseded by one on another book.
    let (p, first) = plan(&f, "moved", book).await;
    let p = id_of(&p["id"]);
    item(&f, first, planned, Some(e2), "paid").await;
    publish(&f, p, first).await;
    let second = bare_revision(&f, p, 2, other).await;
    publish(&f, p, second).await;
    assert_eq!(listed(&f, "").await, ["finished", "other"]);

    let tag = book_tag(&f, book).await;
    assert_eq!(tag, "\"1\"");
    let (s, b, new_tag) = mark(&f, book, "archive", Some(&tag)).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(new_tag, "\"2\"", "the mark is a write of the book");
    assert!(b["archived_at"].is_string(), "{b}");
    assert_eq!(b["archived_by"], json!(f.ctx.subject_id()), "{b}");

    for entry in [e1, e2] {
        assert_eq!(reference_state(&f, entry).await, "released");
        assert_eq!(held(&catalog, entry), ReferenceState::Released, "{entry}");
        let ops = ops_for(&f, entry).await;
        let release = ops.iter().find(|op| op.kind == "release").unwrap();
        assert_eq!(release.state, "done", "{release:?}");
    }
    // The op journal says why.
    let (s, journal, _) = f.call("GET", "/reference-ops", json!({}), None, None).await;
    assert_eq!(s, 200, "{journal}");
    let released: Vec<&Value> = journal["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|op| op["kind"] == "release")
        .collect();
    assert_eq!(released.len(), 2, "{journal}");
    assert!(
        released.iter().all(|op| op["reason"] == "book_archived"),
        "{journal}"
    );

    assert_eq!(listed(&f, "").await, ["other"]);
    assert_eq!(
        listed(&f, "?$filter=archived%20eq%20false").await,
        ["other"]
    );
    assert_eq!(
        listed(&f, "?$filter=archived%20eq%20true").await,
        ["finished"]
    );
    let (s, read, _) = f
        .call(
            "GET",
            &format!("/price-books/{book}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "a read by id ignores the mark: {read}");
    assert!(read["archived_at"].is_string(), "{read}");
    // Archiving an archived book answers it unchanged.
    let (s, again, tag) = mark(&f, book, "archive", Some("\"2\"")).await;
    assert_eq!(s, 200, "{again}");
    assert_eq!(tag, "\"2\"");
}

/// The refusals: a missing If-Match is 400, a stale one 409 `STALE_REVISION`, an unknown book 404;
/// a plan revision that is not superseded is `BOOK_IN_PLAN`, a pending price `BOOK_HAS_PENDING`.
/// A refused archive writes nothing.
#[tokio::test]
async fn a_book_in_use_is_refused_and_kept() {
    let (f, catalog) = plan_support::setup().await;
    quorum(&f, 1).await;
    let free = new_book(&f, "free").await;
    assert_eq!(mark(&f, free, "archive", None).await.0, 400);
    refused(
        &mark(&f, free, "archive", Some("\"9\"")).await,
        409,
        "STALE_REVISION",
    );
    assert_eq!(
        mark(&f, Uuid::now_v7(), "archive", Some("\"1\"")).await.0,
        404
    );

    let drafted = new_book(&f, "drafted").await;
    plan(&f, "draft", drafted).await;
    refused(
        &mark(&f, drafted, "archive", Some("\"1\"")).await,
        409,
        "BOOK_IN_PLAN",
    );
    let live = new_book(&f, "live").await;
    let (p, first) = plan(&f, "live", live).await;
    publish(&f, id_of(&p["id"]), first).await;
    refused(
        &mark(&f, live, "archive", Some("\"1\"")).await,
        409,
        "BOOK_IN_PLAN",
    );

    let pending = new_book(&f, "pending").await;
    let entry = door_entry(&f, pending, catalog.sku(SkuType::Recurring)).await;
    let price = draft(&f, entry).await;
    let (s, b, _) = f
        .call(
            "POST",
            &format!("/prices/{}/submit", price["id"].as_str().unwrap()),
            json!({}),
            None,
            Some("submit"),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    assert_eq!(b["unit"]["state"], "pending", "{b}");
    refused(
        &mark(&f, pending, "archive", Some("\"1\"")).await,
        409,
        "BOOK_HAS_PENDING",
    );
    // A cancel in review keeps the book too: it is a prices unit of the book.
    let cancelling = new_book(&f, "cancelling").await;
    let scheduled = door_entry(&f, cancelling, catalog.sku(SkuType::Recurring)).await;
    let target = approved_from(&f, scheduled, 30).await;
    let (s, change, _) = f
        .call(
            "POST",
            &format!("/prices/{target}/cancel"),
            json!({}),
            None,
            Some("cancel"),
        )
        .await;
    assert_eq!(s, 201, "{change}");
    let (s, b, _) = f
        .call(
            "POST",
            &format!("/prices/{}/submit", change["id"].as_str().unwrap()),
            json!({}),
            None,
            Some("submit-cancel"),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    refused(
        &mark(&f, cancelling, "archive", Some("\"1\"")).await,
        409,
        "BOOK_HAS_PENDING",
    );
    for book in [drafted, live, pending, cancelling] {
        let (_, read, tag) = f
            .call(
                "GET",
                &format!("/price-books/{book}"),
                json!({}),
                None,
                None,
            )
            .await;
        assert!(read["archived_at"].is_null(), "{read}");
        assert_eq!(tag, "\"1\"");
    }
    assert_eq!(reference_state(&f, entry).await, "confirmed");
}

/// An archived book's entries and prices are read-only: an entry create or PATCH, a price
/// create, a cancel, an end, a submit, and a plan item naming one of its entries are 409
/// `BOOK_ARCHIVED`. A delete still runs: it adds no money.
#[tokio::test]
async fn an_archived_books_entries_and_prices_are_read_only() {
    let (f, catalog) = plan_support::setup().await;
    quorum(&f, 0).await;
    let sku = catalog.sku(SkuType::Recurring);
    let book = new_book(&f, "shelved").await;
    let entry = door_entry(&f, book, sku).await;
    let spare = door_entry(&f, book, catalog.sku(SkuType::Recurring)).await;
    let live = approved(&f, entry).await;
    let pending_draft = draft(&f, entry).await;
    archive(&f, book).await;

    let other = catalog.sku(SkuType::Recurring);
    refused(
        &f.call(
            "POST",
            &format!("/price-books/{book}/entries"),
            json!({"sku_id": other, "period": "month", "model": "per_unit"}),
            None,
            Some("late-entry"),
        )
        .await,
        409,
        "BOOK_ARCHIVED",
    );
    refused(
        &f.call(
            "POST",
            &format!("/price-book-entries/{entry}/prices"),
            json!({"price": {"rate": "0.20"}, "eligibility": "all", "effective_from": future(50)}),
            None,
            Some("late-price"),
        )
        .await,
        409,
        "BOOK_ARCHIVED",
    );
    refused(
        &f.call(
            "POST",
            &format!("/prices/{live}/cancel"),
            json!({}),
            None,
            Some("late-cancel"),
        )
        .await,
        409,
        "BOOK_ARCHIVED",
    );
    refused(
        &f.call(
            "POST",
            &format!("/prices/{live}/end"),
            json!({"effective_to": future(60)}),
            None,
            Some("late-end"),
        )
        .await,
        409,
        "BOOK_ARCHIVED",
    );
    let pending_id = pending_draft["id"].as_str().unwrap();
    refused(
        &f.call(
            "POST",
            &format!("/prices/{pending_id}/submit"),
            json!({}),
            None,
            Some("late-submit"),
        )
        .await,
        409,
        "BOOK_ARCHIVED",
    );
    let kept = price_of(&f, entry, pending_id).await;
    assert_eq!(
        kept["state"], "draft",
        "the refused submit wrote nothing: {kept}"
    );
    assert!(kept["pending_unit_id"].is_null(), "{kept}");
    let (_, read, tag) = f
        .call(
            "GET",
            &format!("/price-book-entries/{entry}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(read["reference_state"], "released");
    refused(
        &f.call(
            "PATCH",
            &format!("/price-book-entries/{entry}"),
            json!({"invoice_line_override": "{name}"}),
            Some(&tag),
            None,
        )
        .await,
        409,
        "BOOK_ARCHIVED",
    );
    // A plan item naming the archived book's entry, created or patched onto it.
    let (_, revision) = plan(&f, "after", book).await;
    refused(
        &f.call(
            "POST",
            &format!("/plan-revisions/{revision}/items"),
            json!({"sku_id": sku, "price_book_entry_id": entry}),
            None,
            Some("late-item"),
        )
        .await,
        409,
        "BOOK_ARCHIVED",
    );
    let written = item(&f, revision, sku, Some(entry), "paid").await;
    refused(
        &f.call(
            "PATCH",
            &format!("/plan-items/{}", written.id),
            json!({"price_book_entry_id": entry}),
            Some("\"1\""),
            None,
        )
        .await,
        409,
        "BOOK_ARCHIVED",
    );
    // A delete adds no money: the spare entry goes.
    let (s, b, _) = f
        .call(
            "DELETE",
            &format!("/price-book-entries/{spare}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 204, "{b}");
}

/// Unarchive re-reserves each released entry whose SKU still admits a reference, and lists the
/// entries it could not: a retired SKU leaves its entry `released`, and that entry stays
/// read-only (`ENTRY_REFERENCE_RELEASED`) in the book, which is listed again.
#[tokio::test]
async fn unarchive_rereserves_the_live_skus_and_lists_the_others() {
    let (f, catalog) = plan_support::setup().await;
    let live = catalog.sku(SkuType::Recurring);
    let gone = catalog.sku(SkuType::Recurring);
    let book = new_book(&f, "back").await;
    let e1 = door_entry(&f, book, live).await;
    let e2 = door_entry(&f, book, gone).await;
    let stranded = draft(&f, e2).await;
    let first_receipt = catalog.refs.lock().unwrap()[&e1].0;
    archive(&f, book).await;
    catalog.age(gone, Lifecycle::Retired);

    let tag = book_tag(&f, book).await;
    let (s, b, new_tag) = mark(&f, book, "unarchive", Some(&tag)).await;
    assert_eq!(s, 200, "{b}");
    assert!(b["archived_at"].is_null(), "{b}");
    assert_eq!(b["released_entries"], json!([e2]), "{b}");
    assert_eq!(new_tag, "\"3\"");
    assert_eq!(reference_state(&f, e1).await, "confirmed");
    assert_eq!(reference_state(&f, e2).await, "released");
    let (receipt, state) = catalog.refs.lock().unwrap()[&e1];
    assert_ne!(receipt, first_receipt, "a new reservation");
    assert_eq!(state, ReferenceState::Confirmed);
    assert_eq!(held(&catalog, e2), ReferenceState::Released);
    assert_eq!(listed(&f, "").await, ["back"]);
    refused(
        &f.call(
            "POST",
            &format!("/price-book-entries/{e2}/prices"),
            json!({"price": {"rate": "0.20"}, "eligibility": "all", "effective_from": future(50)}),
            None,
            Some("released-price"),
        )
        .await,
        409,
        "ENTRY_REFERENCE_RELEASED",
    );
    // A draft written before the archive is not submitted either: the prices unit answers the
    // released entry as the door does.
    let stranded_id = stranded["id"].as_str().unwrap();
    refused(
        &f.call(
            "POST",
            &format!("/prices/{stranded_id}/submit"),
            json!({}),
            None,
            Some("released-submit"),
        )
        .await,
        409,
        "ENTRY_REFERENCE_RELEASED",
    );
    let kept = price_of(&f, e2, stranded_id).await;
    assert_eq!(kept["state"], "draft", "{kept}");
    assert!(kept["pending_unit_id"].is_null(), "{kept}");
    draft(&f, e1).await;
}

/// The release survives a failed drive: with Products down the archive still answers 200, its
/// entry reads `released`, and the ticker finishes the release once Products answers.
#[tokio::test]
async fn the_release_survives_a_failed_drive() {
    let (f, catalog) = plan_support::setup().await;
    let book = new_book(&f, "offline").await;
    let entry = door_entry(&f, book, catalog.sku(SkuType::Recurring)).await;
    catalog.down.store(true, Ordering::SeqCst);
    archive(&f, book).await;
    assert_eq!(reference_state(&f, entry).await, "released");
    assert_eq!(held(&catalog, entry), ReferenceState::Confirmed);
    let ops = ops_for(&f, entry).await;
    let release = ops.iter().find(|op| op.kind == "release").unwrap();
    assert_eq!(release.state, "releasing", "{release:?}");

    catalog.down.store(false, Ordering::SeqCst);
    Ticker::new(f.state.clone(), Arc::new(Later), 10, 100)
        .tick()
        .await
        .unwrap();
    assert_eq!(held(&catalog, entry), ReferenceState::Released);
    let ops = ops_for(&f, entry).await;
    assert_eq!(
        ops.iter().find(|op| op.kind == "release").unwrap().state,
        "done"
    );
}

/// A re-reservation that an unarchive started and could not finish never writes into a book
/// archived again meanwhile: its write is refused, its new reservation released, and the entry
/// stays `released`.
#[tokio::test]
async fn a_rereserve_never_writes_into_an_archived_book() {
    let (f, catalog) = plan_support::setup().await;
    let book = new_book(&f, "again").await;
    let entry = door_entry(&f, book, catalog.sku(SkuType::Recurring)).await;
    archive(&f, book).await;
    catalog.down.store(true, Ordering::SeqCst);
    let tag = book_tag(&f, book).await;
    let (s, b, _) = mark(&f, book, "unarchive", Some(&tag)).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        b["released_entries"],
        json!([entry]),
        "the drive did not finish: {b}"
    );
    archive(&f, book).await;
    catalog.down.store(false, Ordering::SeqCst);
    Ticker::new(f.state.clone(), Arc::new(Later), 10, 100)
        .tick()
        .await
        .unwrap();
    assert_eq!(reference_state(&f, entry).await, "released");
    assert_eq!(held(&catalog, entry), ReferenceState::Released);
    let ops = ops_for(&f, entry).await;
    let rereserve = ops.iter().find(|op| op.kind == "rereserve").unwrap();
    assert_eq!(rereserve.state, "done", "{rereserve:?}");
}
