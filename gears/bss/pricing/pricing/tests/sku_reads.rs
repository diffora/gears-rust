//! Where a SKU is priced and sold (D-434): the SKU's entries across the tenant's books with their
//! usage and the default chain's price in force today, the plans that name the SKU through an
//! entry, and one plan item with its revision and plan — each list in a fixed number of
//! statements, whatever the number of rows.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod plan_support;
use bss_pricing::infra::storage::{
    entity::{plan_revision, price},
    repo::{plan_revision_repo, price_book_entry_repo, price_repo},
};
use bss_products_sdk::models::SkuType;
use plan_support::{
    Catalog, Fixture, book, entry, entry_support, holding, id_of, item, plan, publish, scope,
    setup, stranger,
};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}
/// A price of `entry` written straight through the repository (the doors cannot write an
/// approved, pending or rejected price without a unit).
#[allow(
    clippy::too_many_arguments,
    reason = "a stored price row, spelled out where the test reads it"
)]
async fn price_at(
    f: &Fixture,
    entry: Uuid,
    version_no: i32,
    state: &str,
    dim: Option<&str>,
    from: time::Date,
    to: Option<time::Date>,
    rate: &str,
) -> price::Model {
    let conn = f.db.conn().unwrap();
    let e = price_book_entry_repo::find(&conn, &scope(f), f.ctx.subject_tenant_id(), entry)
        .await
        .unwrap()
        .unwrap();
    let mut p = entry_support::price(&e);
    p.version_no = version_no;
    p.state = state.into();
    p.dim_value = dim.map(str::to_owned);
    p.effective_from = from;
    p.effective_to = to;
    p.price_json = json!({ "rate": rate });
    price_repo::insert(&conn, &scope(f), p).await.unwrap()
}
/// A draft revision `rev_no` of `plan` on `book`, with no items, written through the repository.
async fn bare_revision(f: &Fixture, plan: Uuid, rev_no: i32, book: Uuid) -> Uuid {
    let now = time::OffsetDateTime::now_utc();
    plan_revision_repo::insert(
        &f.db.conn().unwrap(),
        &scope(f),
        plan_revision::Model {
            id: Uuid::now_v7(),
            tenant_id: f.ctx.subject_tenant_id(),
            plan_id: plan,
            rev_no,
            book_id: book,
            state: "draft".into(),
            available_from: None,
            pending_unit_id: None,
            approved_by_unit_id: None,
            published_at: None,
            version: 1,
            created_by: f.ctx.subject_id(),
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .unwrap()
    .id
}
async fn get(f: &Fixture, path: &str) -> (u16, Value, String) {
    f.call("GET", path, json!({}), None, None).await
}
/// The SKU's entries by id, as the list answers them.
async fn sku_entries(f: &Fixture, sku: Uuid) -> Vec<Value> {
    let (s, b, _) = get(f, &format!("/price-book-entries?sku_id={sku}")).await;
    assert_eq!(s, 200, "{b}");
    b["items"].as_array().unwrap().clone()
}
/// An entry's usage (D-428): its approved prices as `(scheduled, active, superseded)` today, whose
/// sum is `approved` (D-440), its pending and draft prices, and its plans.
fn usage(
    approved: (u64, u64, u64),
    pending: u64,
    draft: u64,
    plans: u64,
    superseded_only: u64,
) -> Value {
    let (scheduled, active, superseded) = approved;
    json!({
        "prices": {
            "approved": scheduled + active + superseded,
            "pending": pending,
            "draft": draft,
            "scheduled": scheduled,
            "active": active,
            "superseded": superseded,
        },
        "plans": plans,
        "plans_superseded_only": superseded_only,
    })
}
async fn usd_book(f: &Fixture) -> Uuid {
    let (s, b, _) = f
        .call(
            "POST",
            "/price-books",
            json!({"code":"a-usd","name":"Dollars","currency":"USD"}),
            None,
            Some("book-usd"),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    id_of(&b["id"])
}

// ------------------------------------------------------------------ GET /price-book-entries

#[tokio::test]
async fn a_skus_entries_are_listed_across_books_with_their_usage_and_the_price_in_force() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "b-eur").await;
    let usd = usd_book(&f).await;
    let sku = catalog.sku(SkuType::Usage);
    let other = catalog.sku(SkuType::Usage);
    let e_eur = entry(&f, eur, sku, "usage", None).await;
    let e_usd = entry(&f, usd, sku, "usage", None).await;
    let e_other = entry(&f, eur, other, "usage", None).await;
    let t = today();
    let day = time::Duration::days(1);
    // The default chain of e_eur: one price before, the one in force today, one scheduled; a
    // value chain's price and a pending price are never "the default chain's price in force".
    price_at(
        &f,
        e_eur,
        1,
        "approved",
        None,
        t - day * 30,
        Some(t - day * 10),
        "0.10",
    )
    .await;
    let current = price_at(
        &f,
        e_eur,
        2,
        "approved",
        None,
        t - day * 10,
        Some(t + day * 10),
        "0.20",
    )
    .await;
    price_at(&f, e_eur, 3, "approved", None, t + day * 10, None, "0.30").await;
    price_at(
        &f,
        e_eur,
        4,
        "approved",
        Some("eu"),
        t - day * 3,
        None,
        "0.25",
    )
    .await;
    price_at(&f, e_eur, 5, "pending", None, t + day * 20, None, "0.40").await;
    price_at(&f, e_eur, 6, "rejected", None, t + day * 21, None, "0.50").await;
    // e_usd: only a value chain and a draft — nothing of the default chain in force.
    price_at(
        &f,
        e_usd,
        1,
        "approved",
        Some("eu"),
        t - day * 3,
        None,
        "0.26",
    )
    .await;
    price_at(&f, e_usd, 2, "draft", None, t + day, None, "0.27").await;
    price_at(&f, e_other, 1, "approved", None, t - day, None, "9.99").await;
    // A live plan names e_eur; an included item of the SKU without an entry counts nothing.
    let (_, revision) = plan(&f, "pro", eur).await;
    item(&f, revision, sku, Some(e_eur), "paid").await;

    let items = sku_entries(&f, sku).await;
    assert_eq!(items.len(), 2, "{items:#?}");
    // Ordered by book code: "a-usd" before "b-eur".
    let (first, second) = (&items[0], &items[1]);
    assert_eq!(first["id"], e_usd.to_string());
    assert_eq!(first["book_id"], usd.to_string());
    assert_eq!(first["book_code"], "a-usd");
    assert_eq!(first["book_name"], "Dollars");
    assert_eq!(first["currency"], "USD");
    assert_eq!(first["sku_id"], sku.to_string());
    assert_eq!(first["charge_kind"], "usage");
    assert_eq!(first["model"], "per_unit");
    assert_eq!(first["reference_state"], "confirmed");
    assert!(first["period"].is_null());
    assert_eq!(first["usage"], usage((0, 1, 0), 0, 1, 0, 0));
    assert!(
        first["current_price"].is_null(),
        "no default-chain price in force: {first}"
    );
    assert_eq!(second["id"], e_eur.to_string());
    assert_eq!(second["book_name"], "b-eur");
    assert_eq!(second["currency"], "EUR");
    assert_eq!(second["usage"], usage((1, 2, 1), 1, 0, 1, 0));
    assert_eq!(second["current_price"]["id"], current.id.to_string());
    assert_eq!(
        second["current_price"]["price_json"],
        json!({"rate":"0.20"})
    );
    assert_eq!(second["current_price"]["status"], "active");
    assert_eq!(second["current_price"]["model"], "per_unit");
    assert!(second["current_price"]["dim_value"].is_null());

    // Another tenant's caller sees nothing of this SKU; an unknown SKU is an empty list.
    let (s, b, _) = f
        .call_as(
            &stranger(),
            "GET",
            &format!("/price-book-entries?sku_id={sku}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!((s, b), (200, json!({"items": []})));
    assert!(sku_entries(&f, Uuid::new_v4()).await.is_empty());
}

// Probed in run 6.4: the money is shown only to a holder of price_book read (the export's grant).
#[tokio::test]
async fn the_price_in_force_is_shown_only_to_a_holder_of_price_book_read() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Usage);
    let e = entry(&f, eur, sku, "usage", None).await;
    let in_force = price_at(
        &f,
        e,
        1,
        "approved",
        None,
        today() - time::Duration::days(1),
        None,
        "0.10",
    )
    .await;
    // The user holds every grant: the money is there.
    assert_eq!(
        sku_entries(&f, sku).await[0]["current_price"]["id"],
        in_force.id.to_string()
    );
    // Entry read alone: the entry, its book and its usage, and no money.
    let (s, b, _) = f
        .call_as(
            &holding(&f, "price_book_entry:read"),
            "GET",
            &format!("/price-book-entries?sku_id={sku}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["items"][0]["id"], e.to_string());
    assert_eq!(b["items"][0]["currency"], "EUR");
    assert_eq!(b["items"][0]["usage"], usage((0, 1, 0), 0, 0, 0, 0));
    assert!(
        b["items"][0]
            .as_object()
            .unwrap()
            .contains_key("current_price")
            && b["items"][0]["current_price"].is_null(),
        "the field is there, null: {b}"
    );
    // Price-book read alone does not read entries.
    let (s, _, _) = f
        .call_as(
            &holding(&f, "price_book:read"),
            "GET",
            &format!("/price-book-entries?sku_id={sku}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn the_entry_list_needs_exactly_one_well_formed_sku_id() {
    let (f, _) = setup().await;
    let sku = Uuid::new_v4();
    for query in [
        String::new(),
        "?sku_id=".to_owned(),
        "?sku_id=not-a-uuid".to_owned(),
        format!("?sku_id={sku}&sku_id={sku}"),
        format!("?sku_id={sku}&book_id={sku}"),
        "?limit=10".to_owned(),
    ] {
        let (s, b, _) = get(&f, &format!("/price-book-entries{query}")).await;
        assert_eq!(s, 400, "{query}: {b}");
        assert!(b.to_string().contains("QUERY_INVALID"), "{query}: {b}");
    }
    // Authorization is judged first.
    let (s, _, _) = plan_support::request(
        &f.denied,
        &f.ctx,
        "GET",
        "/price-book-entries",
        json!({}),
        None,
        None,
    )
    .await;
    assert_eq!(s, 403);
}

// ------------------------------------------------------------------ GET /plans?sku_id=

#[tokio::test]
async fn the_plans_of_a_sku_are_those_that_name_it_through_an_entry() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Usage);
    let other = catalog.sku(SkuType::Usage);
    let e = entry(&f, eur, sku, "usage", None).await;
    let e_other = entry(&f, eur, other, "usage", None).await;
    // draft: a draft revision names the entry.
    let (draft, d1) = plan(&f, "c-draft", eur).await;
    item(&f, d1, sku, Some(e), "paid").await;
    // live: published, names it.
    let (live, l1) = plan(&f, "a-live", eur).await;
    let live_id = id_of(&live["id"]);
    item(&f, l1, sku, Some(e), "optional").await;
    publish(&f, live_id, l1).await;
    // history: named it only through a superseded revision.
    let (history, h1) = plan(&f, "b-history", eur).await;
    let history_id = id_of(&history["id"]);
    item(&f, h1, sku, Some(e), "paid").await;
    publish(&f, history_id, h1).await;
    let h2 = bare_revision(&f, history_id, 2, eur).await;
    publish(&f, history_id, h2).await;
    // included: names the SKU only through an included item without an entry.
    let (_, i1) = plan(&f, "d-included", eur).await;
    item(&f, i1, sku, None, "included").await;
    // elsewhere: names another SKU.
    let (_, o1) = plan(&f, "e-elsewhere", eur).await;
    item(&f, o1, other, Some(e_other), "paid").await;

    let (s, b, _) = get(&f, &format!("/plans?sku_id={sku}")).await;
    assert_eq!(s, 200, "{b}");
    let codes: Vec<&str> = b["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["a-live", "c-draft"], "{b}");
    // One response schema: each plan as GET /plans answers it, with its revision headers.
    let (_, all, _) = get(&f, "/plans").await;
    let whole = |code: &str| {
        all["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["code"] == code)
            .unwrap()
            .clone()
    };
    assert_eq!(b["items"][0], whole("a-live"));
    assert_eq!(b["items"][1], whole("c-draft"));
    assert_eq!(b["items"][1]["id"], draft["id"]);
    assert_eq!(all["items"].as_array().unwrap().len(), 5);
    assert_eq!(
        whole("b-history")["revisions"].as_array().unwrap().len(),
        2,
        "the plain list keeps every revision header"
    );
    // The plans of the other SKU; an unknown SKU has none; another tenant sees none.
    let (_, b, _) = get(&f, &format!("/plans?sku_id={other}")).await;
    assert_eq!(b["items"].as_array().unwrap().len(), 1);
    let (_, b, _) = get(&f, &format!("/plans?sku_id={}", Uuid::new_v4())).await;
    assert_eq!(b, json!({"items": []}));
    let (s, b, _) = f
        .call_as(
            &stranger(),
            "GET",
            &format!("/plans?sku_id={sku}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!((s, b), (200, json!({"items": []})));
    for query in ["?sku_id=nope", "?sku=1", "?sku_id="] {
        let (s, b, _) = get(&f, &format!("/plans{query}")).await;
        assert_eq!(s, 400, "{query}: {b}");
        assert!(b.to_string().contains("QUERY_INVALID"), "{query}: {b}");
    }
}

// ------------------------------------------------------------------ GET /plan-items/{id}

#[tokio::test]
async fn a_plan_item_reads_with_its_revision_and_plan() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Usage);
    let e = entry(&f, eur, sku, "usage", None).await;
    let (created, r1) = plan(&f, "pro", eur).await;
    let stored = item(&f, r1, sku, Some(e), "paid").await;
    let (s, b, tag) = get(&f, &format!("/plan-items/{}", stored.id)).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(tag, "\"1\"", "the item's version, the PATCH's If-Match");
    assert_eq!(b["id"], stored.id.to_string());
    assert_eq!(b["plan_id"], created["id"]);
    assert_eq!(b["revision_id"], r1.to_string());
    assert_eq!(b["rev_no"], 1);
    assert_eq!(b["state"], "draft");
    assert_eq!(b["sku_id"], sku.to_string());
    assert_eq!(b["price_book_entry_id"], e.to_string());
    assert_eq!(b["treatment"], "paid");
    assert_eq!(b["reference_state"], "confirmed");
    assert_eq!(b["version"], 1);
    // The tag is the one the PATCH takes.
    let patched = f
        .call(
            "PATCH",
            &format!("/plan-items/{}", stored.id),
            json!({"treatment":"optional"}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(patched.0, 200, "{patched:?}");
    publish(&f, id_of(&created["id"]), r1).await;
    let (_, b, tag) = get(&f, &format!("/plan-items/{}", stored.id)).await;
    assert_eq!(
        (b["state"].as_str(), tag.as_str()),
        (Some("published"), "\"2\"")
    );
    for (who, id) in [(stranger(), stored.id), (f.ctx.clone(), Uuid::new_v4())] {
        let (s, _, _) = f
            .call_as(
                &who,
                "GET",
                &format!("/plan-items/{id}"),
                json!({}),
                None,
                None,
            )
            .await;
        assert_eq!(s, 404);
    }
    let (s, _, _) = f
        .call_as(
            &holding(&f, "price_book_entry:read"),
            "GET",
            &format!("/plan-items/{}", stored.id),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 403, "plan read is the grant");
}

// ------------------------------------------------------------------ fixed statements

/// The statements on pricing's tables one read makes.
async fn statements(
    f: &Fixture,
    recorder: &toolkit_db::test_support::QueryRecorder,
    path: &str,
    n: usize,
) -> Vec<(String, usize)> {
    recorder.clear();
    let (s, b, _) = get(f, path).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["items"].as_array().unwrap().len(), n, "{path}");
    recorder
        .events()
        .into_iter()
        .filter(|q| {
            q.table
                .as_deref()
                .is_some_and(|t| t.starts_with("pricing_"))
        })
        .map(|q| (q.sql, q.param_count))
        .collect()
}
/// A SKU with `n` entries, one per book, each with an approved default-chain price in force and
/// named by the paid item of a draft revision of its own plan.
async fn seeded_sku(f: &Fixture, catalog: &Catalog, tag: &str, n: usize) -> Uuid {
    let sku = catalog.sku(SkuType::Usage);
    for i in 0..n {
        let own = book(f, &format!("{tag}-{i:03}")).await;
        let e = entry(f, own, sku, "usage", None).await;
        price_at(
            f,
            e,
            1,
            "approved",
            None,
            today() - time::Duration::days(1),
            None,
            "0.10",
        )
        .await;
        let (_, revision) = plan(f, &format!("{tag}-plan-{i:03}"), own).await;
        item(f, revision, sku, Some(e), "paid").await;
    }
    sku
}

// Probed in run 6.4: a per-row read in either list is red here.
#[tokio::test]
async fn the_two_sku_lists_read_in_the_same_statements_for_10_and_100_rows() {
    let (db, recorder, tenant, dsn) = entry_support::recorded_db().await;
    let catalog = Arc::new(Catalog::default());
    let f = Fixture::on(db, tenant, dsn, catalog.clone()).await;
    let small = seeded_sku(&f, &catalog, "s", 10).await;
    let large = seeded_sku(&f, &catalog, "l", 100).await;
    for (what, ten, hundred) in [
        (
            "entries",
            statements(
                &f,
                &recorder,
                &format!("/price-book-entries?sku_id={small}"),
                10,
            )
            .await,
            statements(
                &f,
                &recorder,
                &format!("/price-book-entries?sku_id={large}"),
                100,
            )
            .await,
        ),
        (
            "plans",
            statements(&f, &recorder, &format!("/plans?sku_id={small}"), 10).await,
            statements(&f, &recorder, &format!("/plans?sku_id={large}"), 100).await,
        ),
    ] {
        for (i, (sql, binds)) in hundred.iter().enumerate() {
            eprintln!("{what} statement {i} ({binds} binds): {sql}");
        }
        assert_eq!(
            ten.len(),
            hundred.len(),
            "{what}: the statements grow with the rows: {ten:#?} vs {hundred:#?}"
        );
        assert_eq!(
            ten.iter().map(|(sql, _)| sql).collect::<Vec<_>>(),
            hundred.iter().map(|(sql, _)| sql).collect::<Vec<_>>(),
            "{what}: the same statements, whatever the size"
        );
    }
    // The plain plan list is set-based too: 110 plans in the statements of the SKU's list.
    let plain = statements(&f, &recorder, "/plans", 110).await;
    assert_eq!(plain.len(), 2, "{plain:#?}");
}
