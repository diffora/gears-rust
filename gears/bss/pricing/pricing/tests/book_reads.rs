//! The Price Books screen's reads (phase 7, run 7.1): an entry's prices with their date-derived
//! status and the entry reads' price in force and dated counts (D-440); every book read's stats
//! (D-441); the book list on the toolkit's `OData` pager, with `q` and `sku_id` (D-442) — each
//! list in a fixed number of statements, whatever the number of rows.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod book_support;
mod plan_support;
use book_support::{
    Row, bare_revision, code_of, codes, days, door_book, encode, get, ids, instant, ok,
    stored_book, stored_entry, stored_price, today, unit_on,
};
use bss_approval::UnitState;
use bss_pricing::infra::storage::entity::price;
use bss_products_sdk::models::SkuType;
use plan_support::{
    Catalog, Fixture, entry_support, holding, id_of, item, plan, publish, request, setup, stranger,
};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

// ------------------------------------------------------------------ the policies of the money

/// A policy where every subject of `tenant` holds every grant, `price_book read` narrowed to
/// `books` when given, or unavailable altogether when `unavailable`.
struct Money {
    tenant: Uuid,
    books: Option<Vec<Uuid>>,
    unavailable: bool,
}
#[async_trait::async_trait]
impl authz_resolver_sdk::AuthZResolverApi for Money {
    async fn evaluate(
        &self,
        _: toolkit_security::PlatformSecurityContext,
        request: authz_resolver_sdk::EvaluationRequest,
    ) -> Result<authz_resolver_sdk::EvaluationResponse, toolkit_canonical_errors::CanonicalError>
    {
        use authz_resolver_sdk::*;
        let money = request.resource.resource_type == "gts.cf.bss.pricing.price_book.v1~"
            && request.action.name == "read";
        if money && self.unavailable {
            return Err(toolkit_canonical_errors::CanonicalError::service_unavailable().create());
        }
        let mut predicates = vec![Predicate::In(InPredicate::new(
            toolkit_security::pep_properties::OWNER_TENANT_ID,
            vec![self.tenant],
        ))];
        if money && let Some(books) = &self.books {
            predicates.push(Predicate::In(InPredicate::new(
                toolkit_security::pep_properties::RESOURCE_ID,
                books.clone(),
            )));
        }
        Ok(EvaluationResponse {
            decision: true,
            context: EvaluationResponseContext {
                constraints: vec![Constraint { predicates }],
                deny_reason: None,
            },
        })
    }
}
fn money_app(f: &Fixture, books: Option<Vec<Uuid>>, unavailable: bool) -> axum::Router {
    entry_support::production(f.state.clone()).layer(axum::Extension(
        authz_resolver_sdk::PolicyEnforcer::new(Arc::new(Money {
            tenant: f.ctx.subject_tenant_id(),
            books,
            unavailable,
        })),
    ))
}

// ------------------------------------------------------------------ a plan's book (PS-08)

/// The number of plans of the fixture tenant, read with every grant.
async fn plan_count(f: &Fixture) -> usize {
    ok(f, "/plans").await["items"].as_array().unwrap().len()
}

/// A plan names only a book its author may read (whole-branch review PS-08, D-456): plan create,
/// clone and a revision PATCH that names a book judge `price_book` read on that book, as D-440
/// judges the money. A grant that does not admit the book is 403 `PRICE_BOOK_READ_REQUIRED`, a
/// policy that cannot judge is 503, and nothing is written; a grant that admits it lets all three
/// through, and a PATCH that names no book asks nothing of the money's policy.
#[tokio::test]
async fn a_plan_names_only_a_book_its_author_may_read() {
    let (f, catalog) = setup().await;
    let eur = plan_support::book(&f, "eur").await;
    let other = plan_support::book(&f, "other").await;
    let sku = catalog.sku(SkuType::Usage);
    let entry = plan_support::entry(&f, eur, sku, "usage", None).await;
    let (source, rev) = plan(&f, "source", eur).await;
    let source = id_of(&source["id"]);
    item(&f, rev, sku, Some(entry), "paid").await;
    publish(&f, source, rev).await;
    let (_, draft) = plan(&f, "draft", eur).await;
    let plans = plan_count(&f).await;
    let tries = |app: axum::Router, tag: &'static str| {
        let f = &f;
        async move {
            [
                request(
                    &app,
                    &f.ctx,
                    "POST",
                    "/plans",
                    json!({"code":format!("new-{tag}"),"name":"New","book_id":eur}),
                    None,
                    Some(&format!("create-{tag}")),
                )
                .await,
                request(
                    &app,
                    &f.ctx,
                    "POST",
                    &format!("/plans/{source}/clone"),
                    json!({"code":format!("clone-{tag}"),"name":"Clone"}),
                    None,
                    Some(&format!("clone-{tag}")),
                )
                .await,
                request(
                    &app,
                    &f.ctx,
                    "PATCH",
                    &format!("/plan-revisions/{draft}"),
                    json!({"book_id":eur}),
                    Some("\"1\""),
                    None,
                )
                .await,
            ]
        }
    };
    for (app, status, tag) in [
        (money_app(&f, Some(vec![other]), false), 403, "narrow"),
        (money_app(&f, None, true), 503, "down"),
    ] {
        for (s, b, _) in tries(app, tag).await {
            assert_eq!(s, status, "{tag}: {b}");
            if status == 403 {
                assert!(code_of(&b).contains("PRICE_BOOK_READ_REQUIRED"), "{b}");
            }
        }
        assert_eq!(plan_count(&f).await, plans, "{tag}: nothing was written");
    }
    let revision = ok(&f, &format!("/plan-revisions/{draft}")).await;
    assert_eq!(
        revision["version"], 1,
        "the PATCH wrote nothing: {revision}"
    );
    // A PATCH that names no book needs no judgement of the money: the policy may be down.
    let (s, b, _) = request(
        &money_app(&f, None, true),
        &f.ctx,
        "PATCH",
        &format!("/plan-revisions/{draft}"),
        json!({"available_from":null}),
        Some("\"1\""),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let admitted = money_app(&f, Some(vec![eur]), false);
    let [created, cloned, patched] = tries(admitted, "admitted").await;
    assert_eq!(created.0, 201, "{created:?}");
    assert_eq!(cloned.0, 201, "{cloned:?}");
    assert_eq!(patched.0, 409, "the draft moved to version 2: {patched:?}");
    assert!(
        code_of(&patched.1).contains("STALE_REVISION"),
        "{patched:?}"
    );
    assert_eq!(plan_count(&f).await, plans + 2);
    // The second review of W1a, L2: the PATCH judges the book it names, not the draft's current
    // one. Under a grant for `eur` alone, moving the `eur` draft to `other` is 403 and writes
    // nothing; naming `eur` at the draft's current version passes.
    let only_eur = money_app(&f, Some(vec![eur]), false);
    let (s, b, _) = request(
        &only_eur,
        &f.ctx,
        "PATCH",
        &format!("/plan-revisions/{draft}"),
        json!({"book_id":other}),
        Some("\"2\""),
        None,
    )
    .await;
    assert_eq!(s, 403, "{b}");
    assert!(code_of(&b).contains("PRICE_BOOK_READ_REQUIRED"), "{b}");
    let revision = ok(&f, &format!("/plan-revisions/{draft}")).await;
    assert_eq!(
        revision["version"], 2,
        "the refused move wrote nothing: {revision}"
    );
    let (s, b, _) = request(
        &only_eur,
        &f.ctx,
        "PATCH",
        &format!("/plan-revisions/{draft}"),
        json!({"book_id":eur}),
        Some("\"2\""),
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
}

// ------------------------------------------------------------------ D-440: an entry's prices

/// An entry whose default chain holds one price of every status and whose value chains `eu` and
/// `apac` hold one price each: `(entry, the stored prices by name)`.
async fn priced_entry(f: &Fixture, catalog: &Catalog, book: Uuid) -> (Uuid, Vec<price::Model>) {
    let t = today();
    let e = stored_entry(
        f,
        book,
        catalog.sku(SkuType::Usage),
        "per_unit",
        time::OffsetDateTime::now_utc(),
    )
    .await;
    let mut prices = Vec::new();
    for row in [
        // 0: superseded, 1: active (the price in force), 2: scheduled — the approved default chain.
        Row::new(1, "approved", t - days(30)).to(t - days(10)),
        Row::new(2, "approved", t - days(10)).to(t + days(10)),
        Row::new(3, "approved", t + days(10)),
        // 3: a draft on the scheduled price's start, after it by version.
        Row::new(9, "draft", t + days(10)),
        Row::new(4, "draft", t + days(20)),
        Row::new(5, "pending", t + days(21)),
        Row::new(6, "rejected", t + days(22)),
        // 7: eu's active price; 8: apac's draft.
        Row::new(7, "approved", t - days(5)).on("eu"),
        Row::new(8, "draft", t + days(5)).on("apac"),
    ] {
        prices.push(stored_price(f, e, row).await);
    }
    (e, prices)
}

#[tokio::test]
async fn an_entrys_prices_are_every_state_with_its_status_default_chain_first() {
    let (f, catalog) = setup().await;
    let eur = plan_support::book(&f, "eur").await;
    let (entry, prices) = priced_entry(&f, &catalog, eur).await;
    let b = ok(&f, &format!("/price-book-entries/{entry}/prices")).await;
    // The default chain by start then version, then apac, then eu.
    let expected: Vec<String> = [0, 1, 2, 3, 4, 5, 6, 8, 7]
        .into_iter()
        .map(|i| prices[i].id.to_string())
        .collect();
    assert_eq!(ids(&b), expected, "{b:#}");
    let statuses: Vec<&str> = b["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        [
            "superseded",
            "active",
            "scheduled",
            "draft",
            "draft",
            "pending",
            "rejected",
            "draft",
            "active"
        ]
    );
    // Each item is the price as every price read answers it, with its entry's model.
    let first = &b["items"][0];
    assert_eq!(first["price_book_entry_id"], entry.to_string());
    assert_eq!(first["model"], "per_unit");
    assert_eq!(first["state"], "approved");
    assert_eq!(first["version_no"], 1);
    assert!(first["dim_value"].is_null());
    assert_eq!(b["items"][8]["dim_value"], "eu");
    // An entry with no prices lists none; another tenant's entry and an unknown one are 404.
    let bare = stored_entry(
        &f,
        eur,
        catalog.sku(SkuType::Usage),
        "per_unit",
        time::OffsetDateTime::now_utc(),
    )
    .await;
    assert_eq!(
        ok(&f, &format!("/price-book-entries/{bare}/prices")).await,
        json!({"items": []})
    );
    for (who, id) in [(stranger(), entry), (f.ctx.clone(), Uuid::new_v4())] {
        let (s, b, _) = f
            .call_as(
                &who,
                "GET",
                &format!("/price-book-entries/{id}/prices"),
                json!({}),
                None,
                None,
            )
            .await;
        assert_eq!(s, 404, "{b}");
        assert!(code_of(&b).contains("ENTRY_NOT_FOUND"), "{b}");
    }
}

#[tokio::test]
async fn the_status_filter_takes_one_or_several_values_and_refuses_the_rest() {
    let (f, catalog) = setup().await;
    let eur = plan_support::book(&f, "eur").await;
    let (entry, prices) = priced_entry(&f, &catalog, eur).await;
    let path = format!("/price-book-entries/{entry}/prices");
    let of = |i: &[usize]| -> Vec<String> { i.iter().map(|i| prices[*i].id.to_string()).collect() };
    for (status, expected) in [
        ("active", of(&[1, 7])),
        ("scheduled", of(&[2])),
        ("superseded", of(&[0])),
        ("pending", of(&[5])),
        ("rejected", of(&[6])),
        ("draft", of(&[3, 4, 8])),
        // Several values, in chain order whatever order they are named in.
        ("draft,superseded", of(&[0, 3, 4, 8])),
        ("active,active", of(&[1, 7])),
        (
            "rejected,pending,draft,scheduled,active,superseded",
            of(&[0, 1, 2, 3, 4, 5, 6, 8, 7]),
        ),
    ] {
        let b = ok(&f, &format!("{path}?status={status}")).await;
        assert_eq!(ids(&b), expected, "status={status}: {b:#}");
    }
    for query in [
        "?status=live",
        "?status=",
        "?status=active,",
        "?status=ACTIVE",
        "?status=active&status=draft",
        "?state=draft",
        "?limit=10",
    ] {
        let (s, b, _) = get(&f, &format!("{path}{query}")).await;
        assert_eq!(s, 400, "{query}: {b}");
        assert!(code_of(&b).contains("QUERY_INVALID"), "{query}: {b}");
    }
    // Authorization is judged first.
    let (s, _, _) = request(
        &f.denied,
        &f.ctx,
        "GET",
        &format!("{path}?status=live"),
        json!({}),
        None,
        None,
    )
    .await;
    assert_eq!(s, 403);
}

// Probed in run 7.1: the prices list without the second grant.
#[tokio::test]
async fn an_entrys_prices_are_money_read_with_price_book_read_on_its_book() {
    let (f, catalog) = setup().await;
    let eur = plan_support::book(&f, "eur").await;
    let usd = plan_support::book(&f, "usd").await;
    let (e, _) = priced_entry(&f, &catalog, eur).await;
    let path = format!("/price-book-entries/{e}/prices");
    // Entry read alone reaches the entry — an unknown one is 404 — but not its money: 403.
    let entry_reader = holding(&f, "price_book_entry:read");
    let (s, b, _) = f
        .call_as(&entry_reader, "GET", &path, json!({}), None, None)
        .await;
    assert_eq!(s, 403, "{b}");
    let (s, _, _) = f
        .call_as(
            &entry_reader,
            "GET",
            &format!("/price-book-entries/{}/prices", Uuid::new_v4()),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 404, "the entry is judged before its money");
    // Price-book read alone does not reach entries.
    let (s, _, _) = f
        .call_as(
            &holding(&f, "price_book:read"),
            "GET",
            &path,
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 403);
    // A price_book grant scoped to other books does not show this book's money.
    let app = money_app(&f, Some(vec![usd]), false);
    let (s, b, _) = request(&app, &f.ctx, "GET", &path, json!({}), None, None).await;
    assert_eq!(s, 403, "{b}");
    let app = money_app(&f, Some(vec![eur]), false);
    let (s, b, _) = request(&app, &f.ctx, "GET", &path, json!({}), None, None).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["items"].as_array().unwrap().len(), 9);
    // A policy that cannot judge the money fails the read.
    let app = money_app(&f, None, true);
    let (s, b, _) = request(&app, &f.ctx, "GET", &path, json!({}), None, None).await;
    assert_eq!(s, 503, "{b}");
}

// ------------------------------------------------------------------ D-440: the entry reads

fn dated(
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

#[tokio::test]
async fn the_entry_reads_carry_the_price_in_force_and_the_approved_prices_by_date() {
    let (f, catalog) = setup().await;
    let eur = plan_support::book(&f, "eur").await;
    let (entry, prices) = priced_entry(&f, &catalog, eur).await;
    // An entry whose default chain has nothing in force: only a value chain's price and a
    // scheduled one.
    let quiet = stored_entry(
        &f,
        eur,
        catalog.sku(SkuType::Usage),
        "per_unit",
        time::OffsetDateTime::now_utc(),
    )
    .await;
    stored_price(
        &f,
        quiet,
        Row::new(1, "approved", today() - days(1)).on("eu"),
    )
    .await;
    stored_price(&f, quiet, Row::new(2, "approved", today() + days(1))).await;
    let read = ok(&f, &format!("/price-book-entries/{entry}")).await;
    assert_eq!(read["usage"], dated((1, 2, 1), 1, 3, 0, 0), "{read:#}");
    assert_eq!(
        read["current_price"]["id"],
        prices[1].id.to_string(),
        "{read:#}"
    );
    assert_eq!(read["current_price"]["status"], "active");
    assert_eq!(read["current_price"]["model"], "per_unit");
    let listed = ok(&f, &format!("/price-books/{eur}/entries")).await;
    let item = |id: Uuid| {
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id.to_string())
            .unwrap()
            .clone()
    };
    assert_eq!(item(entry)["usage"], read["usage"]);
    assert_eq!(item(entry)["current_price"], read["current_price"]);
    assert_eq!(item(quiet)["usage"], dated((1, 1, 0), 0, 0, 0, 0));
    assert!(
        item(quiet)
            .as_object()
            .unwrap()
            .contains_key("current_price")
            && item(quiet)["current_price"].is_null(),
        "no default-chain price in force: null, never absent"
    );
    // The SKU's entries (D-434) carry the same dated counts.
    let sku = read["sku_id"].as_str().unwrap();
    let across = ok(&f, &format!("/price-book-entries?sku_id={sku}")).await;
    assert_eq!(across["items"][0]["usage"], read["usage"]);
    // Entry read alone: the entry and its counts, and no money.
    let entry_reader = holding(&f, "price_book_entry:read");
    for path in [
        format!("/price-book-entries/{entry}"),
        format!("/price-books/{eur}/entries"),
    ] {
        let (s, b, _) = f
            .call_as(&entry_reader, "GET", &path, json!({}), None, None)
            .await;
        assert_eq!(s, 200, "{path}: {b}");
        let body = if b["items"].is_array() {
            b["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == entry.to_string())
                .unwrap()
                .clone()
        } else {
            b
        };
        assert!(body["current_price"].is_null(), "{path}: {body}");
        assert_eq!(body["usage"], read["usage"], "{path}");
    }
    // A price_book grant on another book hides the money; an unavailable policy fails the read.
    let other = plan_support::book(&f, "other").await;
    for (app, status) in [
        (money_app(&f, Some(vec![other]), false), 200),
        (money_app(&f, None, true), 503),
    ] {
        for path in [
            format!("/price-book-entries/{entry}"),
            format!("/price-books/{eur}/entries"),
        ] {
            let (s, b, _) = request(&app, &f.ctx, "GET", &path, json!({}), None, None).await;
            assert_eq!(s, status, "{path}: {b}");
            if s == 200 {
                assert!(
                    !code_of(&b).contains(&prices[1].id.to_string()),
                    "{path}: {b}"
                );
            }
        }
    }
}

/// `approved` is the sum of `scheduled`, `active` and `superseded` for every entry, whatever its
/// chains hold: value chains, an explicitly closed price, a temporary pair and a gap.
// Probed in run 7.1: approved counted apart from its three dates.
#[tokio::test]
async fn approved_is_always_scheduled_plus_active_plus_superseded() {
    let (f, catalog) = setup().await;
    let eur = plan_support::book(&f, "eur").await;
    let t = today();
    let mut entries = Vec::new();
    for rows in [
        vec![
            Row::new(1, "approved", t - days(40)).to(t - days(20)),
            Row::new(2, "approved", t - days(20)).to(t),
            Row::new(3, "approved", t).to(t + days(1)),
            Row::new(4, "approved", t + days(1)),
        ],
        // A temporary promo in force with its scheduled return, and an ended gap before it.
        vec![
            Row::new(1, "approved", t - days(9)).to(t - days(5)),
            Row::new(2, "approved", t - days(2)).to(t + days(3)),
            Row::new(3, "approved", t + days(3)),
            Row::new(4, "approved", t - days(3))
                .on("eu")
                .to(t - days(1)),
            Row::new(5, "approved", t + days(30)).on("us"),
        ],
        vec![Row::new(1, "draft", t + days(1))],
        vec![],
    ] {
        let e = stored_entry(
            &f,
            eur,
            catalog.sku(SkuType::Usage),
            "per_unit",
            time::OffsetDateTime::now_utc(),
        )
        .await;
        for row in rows {
            stored_price(&f, e, row).await;
        }
        entries.push(e);
    }
    let listed = ok(&f, &format!("/price-books/{eur}/entries")).await;
    let mut seen = Vec::new();
    for item in listed["items"].as_array().unwrap() {
        let p = &item["usage"]["prices"];
        let split = ["scheduled", "active", "superseded"]
            .iter()
            .map(|k| p[k].as_u64().unwrap())
            .sum::<u64>();
        assert_eq!(p["approved"].as_u64().unwrap(), split, "{item}");
        seen.push((
            item["id"].as_str().unwrap().to_owned(),
            p["scheduled"].as_u64().unwrap(),
            p["active"].as_u64().unwrap(),
            p["superseded"].as_u64().unwrap(),
        ));
    }
    seen.sort();
    let mut expected = vec![
        (entries[0].to_string(), 1, 1, 2),
        (entries[1].to_string(), 2, 1, 2),
        (entries[2].to_string(), 0, 0, 0),
        (entries[3].to_string(), 0, 0, 0),
    ];
    expected.sort();
    assert_eq!(seen, expected);
    // The book's own counts add up the same way.
    let book = ok(&f, &format!("/price-books/{eur}")).await;
    let p = &book["stats"]["prices"];
    assert_eq!(
        (
            p["approved"].as_u64(),
            p["scheduled"].as_u64(),
            p["active"].as_u64(),
            p["superseded"].as_u64()
        ),
        (Some(9), Some(3), Some(2), Some(4)),
        "{book:#}"
    );
}

// ------------------------------------------------------------------ D-441: the book stats

fn stats(
    entries: u64,
    skus: u64,
    [plans, plans_superseded_only]: [u64; 2],
    prices: [u64; 7],
    pending_units: u64,
    last_change_at: &str,
) -> Value {
    let [
        draft,
        pending,
        approved,
        scheduled,
        active,
        superseded,
        rejected,
    ] = prices;
    json!({
        "entries": entries,
        "skus": skus,
        "plans": plans,
        "plans_superseded_only": plans_superseded_only,
        "prices": {
            "draft": draft,
            "pending": pending,
            "approved": approved,
            "scheduled": scheduled,
            "active": active,
            "superseded": superseded,
            "rejected": rejected,
        },
        "pending_units": pending_units,
        "last_change_at": last_change_at,
    })
}
/// The book as the list answers it (`q` = its unique code).
async fn listed_book(f: &Fixture, code: &str) -> Value {
    let b = ok(f, &format!("/price-books?q={code}")).await;
    let found: Vec<&Value> = b["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["code"] == code)
        .collect();
    assert_eq!(found.len(), 1, "{b:#}");
    found[0].clone()
}

#[tokio::test]
async fn a_books_stats_count_its_entries_skus_plans_prices_and_units() {
    let (f, catalog) = setup().await;
    let day = today();
    let at = |s: &str| instant(s);
    let book = stored_book(&f, "stats", at("2026-09-01T09:00:00Z")).await;
    let other = stored_book(&f, "other", at("2026-09-01T09:00:00Z")).await;
    // Three entries over two SKUs; another book's entry of the same SKU is not counted.
    let sku = catalog.sku(SkuType::Usage);
    let e1 = stored_entry(&f, book, sku, "per_unit", at("2026-09-02T09:00:00Z")).await;
    let e2 = stored_entry(&f, book, sku, "graduated", at("2026-09-02T09:00:00Z")).await;
    let e3 = stored_entry(
        &f,
        book,
        catalog.sku(SkuType::Usage),
        "per_unit",
        at("2026-09-02T09:00:00Z"),
    )
    .await;
    stored_entry(&f, other, sku, "per_unit", at("2026-09-20T09:00:00Z")).await;
    let old = at("2026-09-02T09:00:00Z");
    for (entry, row) in [
        (
            e1,
            Row::new(1, "approved", day - days(30)).to(day - days(10)),
        ),
        (e1, Row::new(2, "approved", day - days(10))),
        (e1, Row::new(3, "draft", day + days(3))),
        (e1, Row::new(4, "rejected", day + days(4))),
        (e2, Row::new(1, "approved", day + days(10))),
        (e2, Row::new(2, "pending", day + days(11))),
        (e2, Row::new(3, "draft", day + days(12))),
        (e3, Row::new(1, "rejected", day + days(13))),
    ] {
        stored_price(&f, entry, row.updated(old)).await;
    }
    // Plans: a (a published and a superseded revision on the book: once), b (a draft on it),
    // c (on it only through a superseded revision: not in `plans`, the one plan of
    // `plans_superseded_only`), d (on the other book).
    let (plan_a, a1) = plan(&f, "a", book).await;
    let plan_a = id_of(&plan_a["id"]);
    item(&f, a1, sku, Some(e1), "paid").await;
    publish(&f, plan_a, a1).await;
    let a2 = bare_revision(&f, plan_a, 2, book).await;
    publish(&f, plan_a, a2).await;
    plan(&f, "b", book).await;
    let (plan_c, c1) = plan(&f, "c", book).await;
    let plan_c = id_of(&plan_c["id"]);
    publish(&f, plan_c, c1).await;
    let c2 = bare_revision(&f, plan_c, 2, other).await;
    publish(&f, plan_c, c2).await;
    plan(&f, "d", other).await;
    // Units: two pending prices units, a decided one, and a plan revision unit naming the id.
    let submitted = at("2026-09-03T09:00:00Z");
    unit_on(&f, "prices", book, UnitState::Pending, submitted, None).await;
    unit_on(&f, "prices", book, UnitState::Pending, submitted, None).await;
    unit_on(
        &f,
        "prices",
        book,
        UnitState::Approved,
        submitted,
        Some(submitted),
    )
    .await;
    unit_on(
        &f,
        "plan_revision",
        book,
        UnitState::Pending,
        at("2026-09-09T09:00:00Z"),
        None,
    )
    .await;
    let expected = stats(
        3,
        2,
        [2, 1],
        [2, 1, 3, 1, 1, 1, 2],
        2,
        "2026-09-03T09:00:00Z",
    );
    assert_eq!(listed_book(&f, "stats").await["stats"], expected);
    let read = ok(&f, &format!("/price-books/{book}")).await;
    assert_eq!(read["stats"], expected, "{read:#}");
    assert_eq!(
        read["id"],
        book.to_string(),
        "the book's own fields stay flat"
    );
    assert_eq!(read["code"], "stats");
    // The other book: its own entry and plans (c's live revision, d).
    assert_eq!(
        ok(&f, &format!("/price-books/{other}")).await["stats"],
        stats(1, 1, [2, 0], [0; 7], 0, "2026-09-20T09:00:00Z")
    );
    // A book with nothing reads zeros and its own last change.
    let empty = stored_book(&f, "empty", at("2026-08-01T09:00:00.25Z")).await;
    assert_eq!(
        ok(&f, &format!("/price-books/{empty}")).await["stats"],
        stats(0, 0, [0, 0], [0; 7], 0, "2026-08-01T09:00:00.25Z")
    );
    // The write answers keep the book alone.
    let (s, created, _) = f
        .call(
            "POST",
            "/price-books",
            json!({"code":"new","name":"New","currency":"EUR"}),
            None,
            Some("book-new"),
        )
        .await;
    assert_eq!(s, 201, "{created}");
    assert!(created.get("stats").is_none(), "{created}");
    let (_, _, tag) = get(
        &f,
        &format!("/price-books/{}", created["id"].as_str().unwrap()),
    )
    .await;
    let (s, patched, _) = f
        .call(
            "PATCH",
            &format!("/price-books/{}", created["id"].as_str().unwrap()),
            json!({"name":"Renamed"}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 200, "{patched}");
    assert!(patched.get("stats").is_none(), "{patched}");
}

/// `last_change_at` is the latest of the book's, its entries' and its prices' `updated_at` and
/// its units' `submitted_at` and `decided_at` — compared as instants, within one source too, where
/// `SQLite`'s RFC 3339 text does not sort as time within one second (`…00.41868Z` after
/// `…00.418681Z`, `…00Z` after `…00.5Z`).
// Probed in run 7.1: the maximum of a source's stored text.
#[tokio::test]
async fn the_last_change_is_the_latest_instant_of_every_source() {
    let (f, catalog) = setup().await;
    let at = |s: &str| instant(s);
    let book = stored_book(&f, "dated", at("2026-09-01T09:00:00Z")).await;
    let last = || async { listed_book(&f, "dated").await["stats"]["last_change_at"].clone() };
    assert_eq!(last().await, "2026-09-01T09:00:00Z");
    let entry = stored_entry(
        &f,
        book,
        catalog.sku(SkuType::Usage),
        "per_unit",
        at("2026-09-02T08:00:00Z"),
    )
    .await;
    assert_eq!(last().await, "2026-09-02T08:00:00Z");
    // Two prices in one second: the later one's text sorts first.
    for (n, updated) in [
        (1, "2026-09-02T09:00:00.418681Z"),
        (2, "2026-09-02T09:00:00.41868Z"),
    ] {
        stored_price(
            &f,
            entry,
            Row::new(n, "draft", today() + days(i64::from(n))).updated(at(updated)),
        )
        .await;
    }
    assert_eq!(last().await, "2026-09-02T09:00:00.418681Z");
    // A unit's submission, then decisions in one second: the later one's text sorts first.
    unit_on(
        &f,
        "prices",
        book,
        UnitState::Pending,
        at("2026-09-03T09:00:00Z"),
        None,
    )
    .await;
    assert_eq!(last().await, "2026-09-03T09:00:00Z");
    for (state, decided) in [
        (UnitState::Rejected, "2026-09-04T09:00:00.5Z"),
        (UnitState::Approved, "2026-09-04T09:00:00Z"),
    ] {
        unit_on(
            &f,
            "prices",
            book,
            state,
            at("2026-09-03T08:00:00Z"),
            Some(at(decided)),
        )
        .await;
    }
    assert_eq!(last().await, "2026-09-04T09:00:00.5Z");
    // The entry again, a quarter second later: the latest across the sources.
    let mut changed = bss_pricing::infra::storage::repo::price_book_entry_repo::find(
        &f.db.conn().unwrap(),
        &plan_support::scope(&f),
        f.ctx.subject_tenant_id(),
        entry,
    )
    .await
    .unwrap()
    .unwrap();
    changed.updated_at = at("2026-09-04T09:00:00.75Z");
    bss_pricing::infra::storage::repo::price_book_entry_repo::update(
        &f.db.conn().unwrap(),
        &plan_support::scope(&f),
        changed,
    )
    .await
    .unwrap();
    assert_eq!(last().await, "2026-09-04T09:00:00.75Z");
    // Another book's rows and a plan revision unit naming the id move nothing.
    let other = stored_book(&f, "elsewhere", at("2026-09-30T09:00:00Z")).await;
    stored_entry(
        &f,
        other,
        catalog.sku(SkuType::Usage),
        "per_unit",
        at("2026-10-01T09:00:00Z"),
    )
    .await;
    unit_on(
        &f,
        "plan_revision",
        book,
        UnitState::Pending,
        at("2026-10-02T09:00:00Z"),
        None,
    )
    .await;
    assert_eq!(last().await, "2026-09-04T09:00:00.75Z");
    assert_eq!(
        ok(&f, &format!("/price-books/{book}")).await["stats"]["last_change_at"],
        "2026-09-04T09:00:00.75Z"
    );
}

// ------------------------------------------------------------------ D-442: the book list

#[tokio::test]
async fn the_book_list_filters_orders_and_pages() {
    let (f, _) = setup().await;
    door_book(&f, "c-usd", "Dollars", "USD", Some("2026-01-01"), None).await;
    door_book(
        &f,
        "a-eur",
        "Euro 2026",
        "EUR",
        Some("2026-01-01"),
        Some("2027-01-01"),
    )
    .await;
    door_book(&f, "b-eur", "Euro open", "EUR", None, None).await;
    door_book(&f, "d-gbp", "Alpha pounds", "GBP", Some("2027-01-01"), None).await;
    // The default order is the code; the page is 200 and a larger $top is clamped at 500.
    let all = ok(&f, "/price-books").await;
    assert_eq!(codes(&all), ["a-eur", "b-eur", "c-usd", "d-gbp"]);
    assert_eq!(all["page_info"]["limit"], 200, "{all}");
    assert!(all["page_info"]["next_cursor"].is_null(), "{all}");
    assert_eq!(
        ok(&f, "/price-books?$top=1000").await["page_info"]["limit"],
        500
    );
    for (query, expected) in [
        ("$filter=currency eq 'EUR'", vec!["a-eur", "b-eur"]),
        ("$filter=code eq 'c-usd'", vec!["c-usd"]),
        ("$filter=startswith(name, 'Euro')", vec!["a-eur", "b-eur"]),
        ("$filter=valid_from eq null", vec!["b-eur"]),
        ("$filter=valid_until ne null", vec!["a-eur"]),
        ("$filter=valid_from ge 2026-06-01", vec!["d-gbp"]),
        (
            "$filter=valid_from le 2026-01-01 and currency ne 'EUR'",
            vec!["c-usd"],
        ),
        ("$orderby=name", vec!["d-gbp", "c-usd", "a-eur", "b-eur"]),
        (
            "$orderby=code desc",
            vec!["d-gbp", "c-usd", "b-eur", "a-eur"],
        ),
        (
            "$orderby=name desc",
            vec!["b-eur", "a-eur", "c-usd", "d-gbp"],
        ),
    ] {
        let b = ok(&f, &format!("/price-books?{}", encode(query))).await;
        assert_eq!(codes(&b), expected, "{query}: {b:#}");
    }
    // Paging: `$top` or its alias `limit`, then the cursor from page_info.
    for top in ["$top", "limit"] {
        let first = ok(&f, &format!("/price-books?{top}=3")).await;
        assert_eq!(codes(&first), ["a-eur", "b-eur", "c-usd"], "{top}");
        let cursor = first["page_info"]["next_cursor"].as_str().unwrap();
        let second = ok(&f, &format!("/price-books?{top}=3&cursor={cursor}")).await;
        assert_eq!(codes(&second), ["d-gbp"], "{top}: {second:#}");
        assert!(second["page_info"]["next_cursor"].is_null());
    }
    // Every item is the book with its stats.
    let item = &all["items"][0];
    assert_eq!(item["currency"], "EUR");
    assert_eq!(item["valid_from"], "2026-01-01");
    assert_eq!(item["valid_until"], "2027-01-01");
    assert_eq!(item["stats"]["entries"], 0);
    // Another tenant lists nothing.
    let (s, b, _) = f
        .call_as(&stranger(), "GET", "/price-books", json!({}), None, None)
        .await;
    assert_eq!(s, 200);
    assert_eq!(b["items"], json!([]));
}

#[tokio::test]
async fn q_matches_a_code_or_a_name_whatever_its_case_and_literally() {
    let (f, _) = setup().await;
    door_book(&f, "eur-main", "Main", "EUR", None, None).await;
    door_book(&f, "usd", "Dollars for EUROPE", "USD", None, None).await;
    door_book(&f, "gbp", "Pounds", "GBP", None, None).await;
    door_book(&f, "pct", "Ten 100% off", "EUR", None, None).await;
    door_book(&f, "under_score", "Under", "EUR", None, None).await;
    for (q, expected) in [
        ("EUR", vec!["eur-main", "usd"]),
        ("eUr", vec!["eur-main", "usd"]),
        ("pounds", vec!["gbp"]),
        ("100%", vec!["pct"]),
        ("%", vec!["pct"]),
        ("_", vec!["under_score"]),
        ("r_s", vec!["under_score"]),
        ("nothing", vec![]),
    ] {
        let b = ok(&f, &format!("/price-books?q={}", encode(q))).await;
        assert_eq!(codes(&b), expected, "q={q}: {b:#}");
    }
    // An empty q is no search.
    assert_eq!(codes(&ok(&f, "/price-books?q=").await).len(), 5);
}

#[tokio::test]
async fn sku_id_keeps_the_books_that_price_the_sku() {
    let (f, catalog) = setup().await;
    let (a, b, c) = (
        plan_support::book(&f, "a").await,
        plan_support::book(&f, "b").await,
        plan_support::book(&f, "c").await,
    );
    let sku = catalog.sku(SkuType::Usage);
    let other = catalog.sku(SkuType::Usage);
    plan_support::entry(&f, a, sku, "usage", None).await;
    plan_support::entry_in(&f, a, sku, "usage", None, "graduated").await;
    plan_support::entry(&f, c, sku, "usage", None).await;
    plan_support::entry(&f, b, other, "usage", None).await;
    let listed = ok(&f, &format!("/price-books?sku_id={sku}")).await;
    assert_eq!(codes(&listed), ["a", "c"], "each book once: {listed:#}");
    assert_eq!(listed["items"][0]["stats"]["entries"], 2);
    assert_eq!(
        codes(&ok(&f, &format!("/price-books?sku_id={other}&q=b")).await),
        ["b"]
    );
    assert_eq!(
        codes(&ok(&f, &format!("/price-books?sku_id={other}&q=a")).await),
        Vec::<String>::new()
    );
    assert_eq!(
        codes(&ok(&f, &format!("/price-books?sku_id={}", Uuid::new_v4())).await),
        Vec::<String>::new()
    );
}

// Probed in run 7.1: a cursor replayed with another q or sku_id accepted.
#[tokio::test]
async fn a_cursor_replayed_under_another_narrowing_is_refused() {
    let (f, catalog) = setup().await;
    let sku = catalog.sku(SkuType::Usage);
    for code in ["x-1", "x-2", "x-3"] {
        let b = plan_support::book(&f, code).await;
        plan_support::entry(&f, b, sku, "usage", None).await;
    }
    let first = ok(&f, &format!("/price-books?q=x&sku_id={sku}&$top=1")).await;
    let cursor = first["page_info"]["next_cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let next = ok(
        &f,
        &format!("/price-books?q=x&sku_id={sku}&cursor={cursor}"),
    )
    .await;
    assert_eq!(codes(&next), ["x-2", "x-3"]);
    for query in [
        format!("q=x-&sku_id={sku}&cursor={cursor}"),
        format!("sku_id={sku}&cursor={cursor}"),
        format!("q=x&sku_id={}&cursor={cursor}", Uuid::new_v4()),
        format!("q=x&cursor={cursor}"),
        format!("q=x&sku_id={sku}&$filter=currency eq 'EUR'&cursor={cursor}"),
    ] {
        let (s, b, _) = get(&f, &format!("/price-books?{}", encode(&query))).await;
        assert_eq!(s, 400, "{query}: {b}");
        assert!(code_of(&b).contains("FILTER_MISMATCH"), "{query}: {b}");
    }
}

#[tokio::test]
async fn the_book_list_refuses_what_it_does_not_take() {
    let (f, _) = setup().await;
    for (query, code) in [
        ("book=1", "QUERY_INVALID"),
        ("q=a&q=b", "QUERY_INVALID"),
        ("sku_id=nope", "QUERY_INVALID"),
        ("sku_id=", "QUERY_INVALID"),
        ("$select=code", "UNSUPPORTED_QUERY_PARAM"),
        ("$count=true", "UNSUPPORTED_QUERY_PARAM"),
        ("$orderby=currency", "INVALID_ORDERBY_FIELD"),
        ("$orderby=valid_from", "INVALID_ORDERBY_FIELD"),
        ("$filter=version eq 1", "INVALID_FILTER"),
        ("$filter=code eq null", "INVALID_FILTER"),
        ("cursor=garbage", "INVALID_CURSOR"),
    ] {
        let (s, b, _) = get(&f, &format!("/price-books?{}", encode(query))).await;
        assert_eq!(s, 400, "{query}: {b}");
        assert!(code_of(&b).contains(code), "{query}: {b}");
    }
    // Authorization is judged first.
    let (s, _, _) = request(
        &f.denied,
        &f.ctx,
        "GET",
        "/price-books?book=1",
        json!({}),
        None,
        None,
    )
    .await;
    assert_eq!(s, 403);
}

// ------------------------------------------------------------------ fixed statements

/// The statements on pricing's tables one read makes, and the items it answered.
async fn statements(
    f: &Fixture,
    recorder: &toolkit_db::test_support::QueryRecorder,
    path: &str,
    n: usize,
) -> Vec<(String, usize)> {
    recorder.clear();
    let b = ok(f, path).await;
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
fn same(what: &str, ten: &[(String, usize)], hundred: &[(String, usize)]) {
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
async fn recorded() -> (
    Fixture,
    Arc<Catalog>,
    toolkit_db::test_support::QueryRecorder,
) {
    let (db, recorder, tenant, dsn) = entry_support::recorded_db().await;
    let catalog = Arc::new(Catalog::default());
    let f = Fixture::on(db, tenant, dsn, catalog.clone()).await;
    (f, catalog, recorder)
}

#[tokio::test]
async fn an_entrys_prices_read_in_the_same_statements_for_10_and_100_prices() {
    let (f, catalog, recorder) = recorded().await;
    let eur = plan_support::book(&f, "eur").await;
    let mut lists = Vec::new();
    for n in [10, 100] {
        let e = stored_entry(
            &f,
            eur,
            catalog.sku(SkuType::Usage),
            "per_unit",
            time::OffsetDateTime::now_utc(),
        )
        .await;
        for i in 0..n {
            let version = i32::try_from(i).unwrap() + 1;
            let from = today() + days(i64::from(version));
            let state = ["approved", "draft", "pending"][i % 3];
            stored_price(&f, e, Row::new(version, state, from)).await;
        }
        lists.push(statements(&f, &recorder, &format!("/price-book-entries/{e}/prices"), n).await);
    }
    same("prices", &lists[0], &lists[1]);
}

/// `n` more books, each with an entry, an approved price in force, a draft and a pending unit.
/// An even book is in a plan (a draft revision with an item); an odd one is named only by a plan's
/// superseded revision, the plan having moved to `moved`: `plans` 0, `plans_superseded_only` 1.
async fn seed_books(f: &Fixture, catalog: &Catalog, tag: &str, n: usize, moved: Uuid) {
    for i in 0..n {
        let code = format!("{tag}-{i:03}");
        let b = plan_support::book(f, &code).await;
        let sku = catalog.sku(SkuType::Usage);
        let e = plan_support::entry(f, b, sku, "usage", None).await;
        stored_price(f, e, Row::new(1, "approved", today() - days(1))).await;
        stored_price(f, e, Row::new(2, "draft", today() + days(1))).await;
        let (created, revision) = plan(f, &format!("plan-{code}"), b).await;
        if i % 2 == 0 {
            item(f, revision, sku, Some(e), "paid").await;
        } else {
            let plan_id = id_of(&created["id"]);
            publish(f, plan_id, revision).await;
            let second = bare_revision(f, plan_id, 2, moved).await;
            publish(f, plan_id, second).await;
        }
        unit_on(
            f,
            "prices",
            b,
            UnitState::Pending,
            time::OffsetDateTime::now_utc(),
            None,
        )
        .await;
    }
}

// Probed in run 7.1: a per-book statement in the stats.
#[tokio::test]
async fn the_book_reads_count_their_stats_in_the_same_statements_for_10_and_100_books() {
    let (f, catalog, recorder) = recorded().await;
    // The plans that moved away land on a book the seeded books' search (`q=-`) leaves out.
    let moved = plan_support::book(&f, "moved").await;
    seed_books(&f, &catalog, "s", 10, moved).await;
    let seeded = "/price-books?q=-&$top=200";
    let ten = statements(&f, &recorder, seeded, 10).await;
    seed_books(&f, &catalog, "l", 90, moved).await;
    let hundred = statements(&f, &recorder, seeded, 100).await;
    same("books", &ten, &hundred);
    // The page, then one grouped statement per source: entries, prices, plans (the live ones and
    // those only history holds, together), units.
    assert_eq!(ten.len(), 5, "{ten:#?}");
    let listed = ok(&f, seeded).await;
    for b in listed["items"].as_array().unwrap() {
        let s = &b["stats"];
        let odd = b["code"]
            .as_str()
            .unwrap()
            .ends_with(['1', '3', '5', '7', '9']);
        assert_eq!(
            (
                s["entries"].as_u64(),
                s["skus"].as_u64(),
                s["plans"].as_u64(),
                s["plans_superseded_only"].as_u64(),
                s["prices"]["active"].as_u64(),
                s["prices"]["draft"].as_u64(),
                s["pending_units"].as_u64()
            ),
            (
                Some(1),
                Some(1),
                Some(u64::from(!odd)),
                Some(u64::from(odd)),
                Some(1),
                Some(1),
                Some(1)
            ),
            "{b}"
        );
    }
    // One book's read counts it with the same statements, a book in a plan (`l-000`) and a book
    // only history holds (`l-001`) alike.
    let mut reads = Vec::new();
    for book in &listed["items"].as_array().unwrap()[..2] {
        recorder.clear();
        let read = ok(
            &f,
            &format!("/price-books/{}", book["id"].as_str().unwrap()),
        )
        .await;
        assert_eq!(read["stats"], book["stats"], "{read:#}");
        reads.push(
            recorder
                .events()
                .into_iter()
                .filter(|q| {
                    q.table
                        .as_deref()
                        .is_some_and(|t| t.starts_with("pricing_"))
                })
                .map(|q| q.sql)
                .collect::<Vec<String>>(),
        );
    }
    assert_eq!(reads[0].len(), 5, "{:#?}", reads[0]);
    assert_eq!(reads[0], reads[1], "the same statements for either book");
    assert_eq!(
        (
            listed["items"][0]["stats"]["plans_superseded_only"].as_u64(),
            listed["items"][1]["stats"]["plans_superseded_only"].as_u64()
        ),
        (Some(0), Some(1))
    );
    // The entry reads stay set-based with their price in force (D-434's two extra reads).
    let mut lists = Vec::new();
    for n in [10, 100] {
        let b = plan_support::book(&f, &format!("entries-{n}")).await;
        for _ in 0..n {
            let e = plan_support::entry(&f, b, catalog.sku(SkuType::Usage), "usage", None).await;
            stored_price(&f, e, Row::new(1, "approved", today() - days(1))).await;
        }
        lists.push(statements(&f, &recorder, &format!("/price-books/{b}/entries"), n).await);
    }
    same("entries", &lists[0], &lists[1]);
}

// ------------------------------------------------------------------ set-based reads (whole-branch review)

/// The statements on pricing's tables the last request made, in order, with their binds.
fn pricing_statements(recorder: &toolkit_db::test_support::QueryRecorder) -> Vec<(String, usize)> {
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

/// `GET /resolve` reads the revision's entries and their prices set-based (PS-15): the same
/// statements for 10 and for 100 items, each item on its own entry with its own price.
#[tokio::test]
async fn resolve_reads_in_the_same_statements_for_10_and_100_items() {
    let (f, catalog, recorder) = recorded().await;
    let eur = plan_support::book(&f, "eur").await;
    let now = time::OffsetDateTime::now_utc();
    let mut runs = Vec::new();
    for n in [10, 100] {
        let (created, revision) = plan(&f, &format!("resolve-{n}"), eur).await;
        for _ in 0..n {
            let sku = catalog.sku(SkuType::Usage);
            let e = stored_entry(&f, eur, sku, "per_unit", now).await;
            stored_price(&f, e, Row::new(1, "approved", today() - days(1))).await;
            item(&f, revision, sku, Some(e), "paid").await;
        }
        publish(&f, id_of(&created["id"]), revision).await;
        recorder.clear();
        let b = ok(
            &f,
            &format!("/resolve?plan_revision_id={revision}&date={}", today()),
        )
        .await;
        assert_eq!(b["items"].as_array().unwrap().len(), n);
        runs.push(pricing_statements(&recorder));
    }
    same("resolve", &runs[0], &runs[1]);
}

/// A book's publish-changes listing and its export read the book's prices, and the listing the
/// plans that read its entries, set-based (PS-14, PS-16): the same statements for 10 and for 100
/// entries, each with an approved price and a draft, and each named by its own plan.
#[tokio::test]
async fn publish_changes_and_the_export_read_in_the_same_statements_for_10_and_100_entries() {
    let (f, catalog, recorder) = recorded().await;
    let now = time::OffsetDateTime::now_utc();
    let (mut listings, mut exports) = (Vec::new(), Vec::new());
    for n in [10, 100] {
        let b = plan_support::book(&f, &format!("changes-{n}")).await;
        for i in 0..n {
            let sku = catalog.sku(SkuType::Usage);
            let e = stored_entry(&f, b, sku, "per_unit", now).await;
            stored_price(&f, e, Row::new(1, "approved", today() - days(1))).await;
            stored_price(&f, e, Row::new(2, "draft", today() + days(1))).await;
            let (_, revision) = plan(&f, &format!("reader-{n}-{i}"), b).await;
            item(&f, revision, sku, Some(e), "paid").await;
        }
        recorder.clear();
        let listing = ok(&f, &format!("/price-books/{b}/publish-changes")).await;
        assert_eq!(
            listing["prices"].as_array().unwrap().len(),
            n,
            "{listing:#}"
        );
        assert_eq!(listing["impact"]["plans"].as_array().unwrap().len(), n);
        listings.push(pricing_statements(&recorder));
        recorder.clear();
        let export = ok(&f, &format!("/price-books/{b}/export")).await;
        assert_eq!(export["entries"].as_array().unwrap().len(), n);
        exports.push(pricing_statements(&recorder));
    }
    same("publish-changes", &listings[0], &listings[1]);
    same("export", &exports[0], &exports[1]);
}

/// A submit reads its prices set-based (PS-39): publishing 10 or 100 drafts of one entry makes
/// the same reads of `pricing_price`, under quorum 1 (recorded) and quorum 0 (applied at once,
/// with its event); only its per-price writes (the locks, the approvals) grow.
#[tokio::test]
async fn a_submit_reads_its_prices_in_the_same_statements_for_10_and_100_drafts() {
    let (f, catalog, recorder) = recorded().await;
    let now = time::OffsetDateTime::now_utc();
    for quorum in [1, 0] {
        let (_, _, tag) = f
            .call("GET", "/approval-policy", json!({}), None, None)
            .await;
        let (s, b, _) = f
            .call(
                "PUT",
                "/approval-policy",
                json!({ "quorum": quorum }),
                Some(&tag),
                None,
            )
            .await;
        assert_eq!(s, 200, "{b}");
        submit_reads(&f, &catalog, &recorder, now, quorum).await;
    }
}
async fn submit_reads(
    f: &Fixture,
    catalog: &Catalog,
    recorder: &toolkit_db::test_support::QueryRecorder,
    now: time::OffsetDateTime,
    quorum: u32,
) {
    let mut runs = Vec::new();
    for n in [10_i32, 100] {
        let b = plan_support::book(f, &format!("submit-{quorum}-{n}")).await;
        let e = stored_entry(f, b, catalog.sku(SkuType::Usage), "per_unit", now).await;
        for i in 1..=n {
            stored_price(f, e, Row::new(i, "draft", today() + days(i64::from(i)))).await;
        }
        recorder.clear();
        let (s, body, _) = f
            .call(
                "POST",
                &format!("/price-books/{b}/publish-changes"),
                json!({}),
                None,
                Some(&format!("submit-{quorum}-{n}")),
            )
            .await;
        assert_eq!(s, 201, "{body}");
        assert_eq!(body["applied"], quorum == 0, "{body}");
        runs.push(
            recorder
                .events()
                .into_iter()
                .filter(|q| {
                    q.table.as_deref() == Some("pricing_price")
                        && q.sql
                            .trim_start()
                            .to_ascii_uppercase()
                            .starts_with("SELECT")
                })
                .map(|q| (q.sql, q.param_count))
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(
        runs[0].len(),
        runs[1].len(),
        "quorum {quorum}: the price reads grow with the drafts: {:#?}",
        runs[1]
    );
}

// ------------------------------------------------------------------ the unit list (PS-13, D-458)

/// A `prices` unit on `book` whose one price item names `entry`, with one current vote.
async fn priced_unit(
    f: &Fixture,
    book: Uuid,
    entry: Uuid,
    submitted: time::OffsetDateTime,
) -> Uuid {
    use bss_approval::{Decision, ItemRef, Store, Unit, Verdict};
    use bss_pricing::infra::storage::repo::{approval_repo::PricingApprovalStore, price_repo};
    let (id, tenant) = (Uuid::now_v7(), f.ctx.subject_tenant_id());
    let scope = plan_support::scope(f);
    price_repo::transaction(&f.db.db(), move |tx| {
        let scope = scope.clone();
        Box::pin(async move {
            let store = PricingApprovalStore {
                scope,
                tenant_id: tenant,
            };
            let err = |e: bss_approval::ApprovalError| {
                bss_pricing::infra::storage::RepoError::Db(e.to_string())
            };
            store
                .insert_unit(
                    tx,
                    &Unit {
                        id,
                        tenant_id: tenant,
                        kind: "prices".into(),
                        ref_type: "price_book".into(),
                        ref_id: book,
                        state: UnitState::Pending,
                        common_effective_date: None,
                        quorum_required: 2,
                        generation: 1,
                        submitted_by: Uuid::new_v4(),
                        submitted_at: submitted,
                        submit_note: None,
                        decided_at: None,
                        decided_note: None,
                        snapshot: json!({}),
                        snapshot_hash: "hash".into(),
                        version: 1,
                    },
                    &[ItemRef {
                        item_type: "price".into(),
                        item_id: Uuid::now_v7(),
                        created_by: Uuid::new_v4(),
                        before: None,
                        after: json!({ "price_book_entry_id": entry }),
                    }],
                )
                .await
                .map_err(err)?;
            store
                .insert_decision(
                    tx,
                    &Decision {
                        unit_id: id,
                        actor: Uuid::new_v4(),
                        generation: 1,
                        verdict: Verdict::Approve,
                        note: None,
                        at: submitted,
                        stale: false,
                    },
                )
                .await
                .map_err(err)
        })
    })
    .await
    .unwrap();
    id
}

/// `GET /approval-units` reads one page set-based (PS-13, D-458): its units, their items, their
/// decisions and their impact's plans in the same statements for 10 and for 100 units, each with
/// an item on its own entry, a vote, and a plan that names the entry.
#[tokio::test]
async fn the_unit_list_reads_a_page_in_the_same_statements_for_10_and_100_units() {
    let (f, catalog, recorder) = recorded().await;
    let now = time::OffsetDateTime::now_utc();
    let mut runs = Vec::new();
    for n in [10, 100] {
        let b = plan_support::book(&f, &format!("units-{n}")).await;
        for i in 0..n {
            let sku = catalog.sku(SkuType::Usage);
            let e = stored_entry(&f, b, sku, "per_unit", now).await;
            let (_, revision) = plan(&f, &format!("reads-{n}-{i}"), b).await;
            item(&f, revision, sku, Some(e), "paid").await;
            priced_unit(&f, b, e, now).await;
        }
        recorder.clear();
        let page = ok(&f, &format!("/approval-units?book_id={b}")).await;
        let items = page["items"].as_array().unwrap();
        assert_eq!(items.len(), n);
        for unit in items {
            assert_eq!(unit["decisions"].as_array().unwrap().len(), 1, "{unit}");
            assert_eq!(
                unit["impact"]["plans"].as_array().unwrap().len(),
                1,
                "{unit}"
            );
        }
        runs.push(pricing_statements(&recorder));
    }
    same("approval units", &runs[0], &runs[1]);
}

/// The unit list pages (PS-13, D-458): `limit` (default 200, clamped at 500) and the opaque
/// `cursor` of `page_info`, in submission order with the id breaking a tie; the pages together are
/// the whole list, and a cursor replayed under another filter is 400.
#[tokio::test]
async fn the_unit_list_pages_in_submission_order() {
    let (f, _catalog) = setup().await;
    let book = plan_support::book(&f, "paged").await;
    let t0 = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    let mut submitted = Vec::new();
    for minutes in [0_i64, 1, 1, 2, 3] {
        submitted.push(
            unit_on(
                &f,
                "prices",
                book,
                UnitState::Pending,
                t0 + time::Duration::minutes(minutes),
                None,
            )
            .await,
        );
    }
    // Minute 1 holds two units: the id orders them.
    submitted[1..3].sort();
    let whole = ok(&f, &format!("/approval-units?book_id={book}")).await;
    assert_eq!(
        ids(&whole),
        submitted.iter().map(Uuid::to_string).collect::<Vec<_>>()
    );
    assert_eq!(whole["page_info"]["limit"], 200, "{whole}");
    assert!(whole["page_info"]["next_cursor"].is_null(), "{whole}");
    let mut seen = Vec::new();
    let mut path = format!("/approval-units?book_id={book}&limit=2");
    let mut pages = 0;
    loop {
        let page = ok(&f, &path).await;
        pages += 1;
        assert!(page["items"].as_array().unwrap().len() <= 2, "{page}");
        seen.extend(ids(&page));
        match page["page_info"]["next_cursor"].as_str() {
            Some(cursor) => {
                path = format!(
                    "/approval-units?book_id={book}&limit=2&cursor={}",
                    encode(cursor)
                );
            }
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(seen, ids(&whole), "the pages are the whole list, in order");
    let first = ok(&f, &format!("/approval-units?book_id={book}&limit=2")).await;
    let cursor = encode(first["page_info"]["next_cursor"].as_str().unwrap());
    let (s, b, _) = get(
        &f,
        &format!("/approval-units?book_id={book}&state=pending&limit=2&cursor={cursor}"),
    )
    .await;
    assert_eq!(s, 400, "a cursor of another filter: {b}");
    assert!(code_of(&b).contains("FILTER_MISMATCH"), "{b}");
    let (s, b, _) = get(&f, "/approval-units?cursor=not-a-cursor").await;
    assert_eq!(s, 400, "{b}");
    let (s, b, _) = get(&f, "/approval-units?limit=many").await;
    assert_eq!(s, 400, "{b}");
    assert!(code_of(&b).contains("QUERY_INVALID"), "{b}");
    let clamped = ok(&f, &format!("/approval-units?book_id={book}&limit=1000")).await;
    assert_eq!(clamped["page_info"]["limit"], 500, "{clamped}");
}
