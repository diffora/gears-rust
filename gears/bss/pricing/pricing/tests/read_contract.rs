//! The consumer read contract through the production routers (run 4.3): `GET /resolve` (D-419,
//! D-420, D-421) and the pinned price read `GET /prices/{id}` (D-422).
//!
//! Approved money is written through the repositories at FIXED past dates (every door refuses a
//! start before today), the revisions are published as an applied unit publishes them, and the
//! reads go through the doors only. The Products double answers dated SKU versions when armed.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod plan_support;
use bss_pricing::infra::storage::{
    entity::{price, price_book_entry},
    repo::{price_book_entry_repo, price_repo},
};
use bss_products_sdk::models::{BillingTiming, SkuType};
use plan_support::{
    Catalog, Fixture, book, holding, id_of, item, lock, plan, publish, scope, setup, stranger, text,
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::Ordering::SeqCst};
use uuid::Uuid;

fn date(text: &str) -> time::Date {
    time::Date::parse(text, &time::format_description::well_known::Iso8601::DATE).unwrap()
}
/// One stored price row; the defaults are an approved open flat price of the default chain.
#[derive(Clone)]
struct Row {
    dim: Option<&'static str>,
    model: &'static str,
    price: Value,
    min_fee: Option<&'static str>,
    from: &'static str,
    to: Option<&'static str>,
    eligibility: &'static str,
    state: &'static str,
    keep: bool,
    closed: bool,
    temporary_until: Option<&'static str>,
    version_no: i32,
}
impl Default for Row {
    fn default() -> Self {
        Self {
            dim: None,
            model: "flat",
            price: json!({"amount":"30.00"}),
            min_fee: None,
            from: "2026-09-01",
            to: None,
            eligibility: "all",
            state: "approved",
            keep: false,
            closed: false,
            temporary_until: None,
            version_no: 1,
        }
    }
}
fn flat(amount: &str) -> Value {
    json!({ "amount": amount })
}
/// Write one price of `entry` straight through the repository.
async fn put(f: &Fixture, entry: Uuid, row: Row) -> Uuid {
    let now = time::OffsetDateTime::now_utc();
    let pending_unit_id = if row.state == "pending" {
        Some(plan_support::unit_of_kind(f, "prices").await)
    } else {
        None
    };
    price_repo::insert(
        &f.db.conn().unwrap(),
        &scope(f),
        price::Model {
            id: Uuid::now_v7(),
            tenant_id: f.ctx.subject_tenant_id(),
            price_book_entry_id: entry,
            version_no: row.version_no,
            dim_value: row.dim.map(str::to_owned),
            model: row.model.into(),
            price_json: row.price,
            min_fee: row.min_fee.map(str::to_owned),
            eligibility: row.eligibility.into(),
            effective_from: date(row.from),
            effective_to: row.to.map(date),
            keep_for_bound: row.keep,
            closed_explicitly: row.closed,
            temporary_until: row.temporary_until.map(date),
            paired_price_id: None,
            return_of_price_id: None,
            state: row.state.into(),
            pending_unit_id,
            approved_by_unit_id: None,
            note: Some("authoring note".into()),
            created_by: f.ctx.subject_id(),
            approved_at: None,
            version: 3,
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .unwrap()
    .id
}
/// An entry of `book` for `sku`, written directly, with a dimension key and an invoice-line
/// override when asked.
async fn entry_of(
    f: &Fixture,
    book: Uuid,
    sku: Uuid,
    kind: &str,
    shape: (Option<&str>, Option<&str>, Option<&str>),
) -> Uuid {
    let (period, key, line) = shape;
    let now = time::OffsetDateTime::now_utc();
    price_book_entry_repo::insert(
        &f.db.conn().unwrap(),
        &scope(f),
        price_book_entry::Model {
            id: Uuid::now_v7(),
            tenant_id: f.ctx.subject_tenant_id(),
            book_id: book,
            sku_id: sku,
            charge_kind: kind.into(),
            period: period.map(str::to_owned),
            dimension_key: key.map(str::to_owned),
            invoice_line_override: line.map(str::to_owned),
            reservation_id: Uuid::new_v4(),
            reference_state: "confirmed".into(),
            version: 1,
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .unwrap()
    .id
}
/// Register one dimension key through its door.
async fn dimension(f: &Fixture, key: &str, values: &[&str]) {
    let (_, _, tag) = f
        .call("GET", "/dimension-keys", json!({}), None, None)
        .await;
    let (s, b, _) = f
        .call(
            "PUT",
            "/dimension-keys",
            json!({"items":[{"key":key,"values":values}]}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
}
/// Write the tenant settings through their door.
async fn settings(f: &Fixture, body: Value) {
    let (_, _, tag) = f.call("GET", "/settings", json!({}), None, None).await;
    let (s, b, _) = f.call("PUT", "/settings", body, Some(&tag), None).await;
    assert_eq!(s, 200, "{b}");
}
async fn resolve(f: &Fixture, query: &str) -> (u16, Value) {
    let (s, b, tag) = f
        .call("GET", &format!("/resolve?{query}"), json!({}), None, None)
        .await;
    assert_eq!(tag, "", "a resolve answer carries no ETag");
    (s, b)
}
async fn resolve_as(
    f: &Fixture,
    ctx: &toolkit_security::SecurityContext,
    query: &str,
) -> (u16, Value) {
    let (s, b, _) = f
        .call_as(
            ctx,
            "GET",
            &format!("/resolve?{query}"),
            json!({}),
            None,
            None,
        )
        .await;
    (s, b)
}
/// Rows of the tables a mutation writes: audit, idempotency, outbox and reference ops.
async fn written(f: &Fixture) -> Vec<i64> {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let db = Database::connect(&f.dsn).await.unwrap();
    let mut counts = Vec::new();
    for table in [
        "pricing_audit",
        "pricing_idempotency",
        "bss_pricing_outbox_incoming",
        "pricing_reference_op",
    ] {
        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT COUNT(*) AS n FROM {table}"),
            ))
            .await
            .unwrap()
            .unwrap();
        counts.push(row.try_get::<i64>("", "n").unwrap());
    }
    counts
}

/// A published plan: one monthly recurring SKU priced €30 from 2026-09-01, as one paid item.
struct World {
    f: Fixture,
    catalog: Arc<Catalog>,
    book: Uuid,
    plan: Uuid,
    revision: Uuid,
    sku: Uuid,
    entry: Uuid,
    item: Uuid,
    price: Uuid,
}
async fn world() -> World {
    let (f, catalog) = setup().await;
    let book = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Recurring);
    let entry = entry_of(&f, book, sku, "recurring", (Some("month"), None, None)).await;
    let price = put(&f, entry, Row::default()).await;
    let (created, revision) = plan(&f, "pro", book).await;
    let plan = id_of(&created["id"]);
    let item = item(&f, revision, sku, Some(entry), "paid").await.id;
    publish(&f, plan, revision).await;
    World {
        f,
        catalog,
        book,
        plan,
        revision,
        sku,
        entry,
        item,
        price,
    }
}

// ------------------------------------------------------------------ Task 4.3.1: the answer

#[tokio::test]
async fn a_published_revision_resolves_every_item_in_the_frozen_shape_and_writes_nothing() {
    let w = world().await;
    let before = written(&w.f).await;
    let (s, b) = resolve(
        &w.f,
        &format!("plan_revision_id={}&date=2026-10-05", w.revision),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let binding = json!({
        "price_id": w.price, "dim_used": null, "pinned_from": null,
        "model": "flat", "price": {"amount": "30.00"}, "min_fee": null,
        "eligibility": "all", "effective_from": "2026-09-01",
        "effective_to": null, "temporary_until": null, "ends_on": null,
        "keep_for_bound": false
    });
    let item = json!({
        "item_id": w.item, "sku_id": w.sku, "treatment": "paid", "included_qty": null,
        "qty_min": null, "price_book_entry_id": w.entry, "charge_kind": "recurring",
        "period": "month", "sku_version": null,
        "invoice_line_template": {"value": null, "source": null},
        "gl_code": {"value": null, "source": null},
        "tax_category": {"value": null, "source": null},
        "billing_timing": {"value": "advance", "source": "tenant"},
        "meter": {"usage_type_ref": null, "unit": null},
        "chains": [{"dim_value": null, "uncovered": false, "binding": binding}]
    });
    assert_eq!(
        b,
        json!({
            "plan_revision_id": w.revision, "plan_id": w.plan, "rev_no": 1, "state": "published",
            "book_id": w.book, "currency": "EUR", "currency_minor_digits": 2,
            "rounding_policy": "half_up", "date": "2026-10-05", "items": [item]
        })
    );
    assert_eq!(
        written(&w.f).await,
        before,
        "a read writes no audit row, no key, no event and no reference op"
    );
    assert_eq!(w.catalog.calls(), 1, "one dated read per distinct SKU");
}

#[tokio::test]
async fn a_superseded_revision_still_resolves() {
    let w = world().await;
    let (s, b, _) =
        w.f.call(
            "POST",
            &format!("/plans/{}/revisions", w.plan),
            json!({}),
            None,
            Some("copy"),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    let rev2 = id_of(&b["id"]);
    publish(&w.f, w.plan, rev2).await;
    for (revision, state, rev_no) in [(w.revision, "superseded", 1), (rev2, "published", 2)] {
        let (s, b) = resolve(
            &w.f,
            &format!("plan_revision_id={revision}&date=2026-10-05"),
        )
        .await;
        assert_eq!(s, 200, "{b}");
        assert_eq!(b["state"], state, "{b}");
        assert_eq!(b["rev_no"], rev_no, "{b}");
        assert_eq!(
            b["items"][0]["chains"][0]["binding"]["price_id"],
            json!(w.price),
            "{b}"
        );
    }
}

#[tokio::test]
async fn a_draft_or_a_pending_revision_is_409_revision_not_published() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Recurring);
    let entry = entry_of(&f, eur, sku, "recurring", (Some("month"), None, None)).await;
    put(&f, entry, Row::default()).await;
    let (_, draft) = plan(&f, "draft", eur).await;
    item(&f, draft, sku, Some(entry), "paid").await;
    let (_, pending) = plan(&f, "pending", eur).await;
    item(&f, pending, sku, Some(entry), "paid").await;
    lock(&f, pending).await;
    for revision in [draft, pending] {
        let (s, b) = resolve(&f, &format!("plan_revision_id={revision}&date=2026-10-05")).await;
        assert_eq!(s, 409, "{b}");
        assert!(text(&b).contains("REVISION_NOT_PUBLISHED"), "{b}");
    }
    assert_eq!(
        catalog.calls(),
        0,
        "no Products read for a revision that does not resolve"
    );
}

#[tokio::test]
async fn another_tenants_or_an_unknown_revision_is_404_before_any_products_read() {
    let w = world().await;
    let calls = w.catalog.calls();
    let query = format!("plan_revision_id={}&date=2026-10-05", w.revision);
    let (s, b) = resolve_as(&w.f, &stranger(), &query).await;
    assert_eq!(s, 404, "{b}");
    let (s, unknown) = resolve(
        &w.f,
        &format!("plan_revision_id={}&date=2026-10-05", Uuid::new_v4()),
    )
    .await;
    assert_eq!(s, 404, "{unknown}");
    assert!(
        text(&b).contains("plan_revision"),
        "a pricing 404, not a missing route: {b}"
    );
    assert_eq!(b, unknown, "a foreign revision reads as an unknown one");
    assert_eq!(w.catalog.calls(), calls, "no registry call before the 404");
}

#[tokio::test]
async fn a_bad_or_missing_date_is_400_date_invalid() {
    let w = world().await;
    for query in [
        format!("plan_revision_id={}", w.revision),
        format!("plan_revision_id={}&date=", w.revision),
        format!("plan_revision_id={}&date=2026-13-01", w.revision),
        format!("plan_revision_id={}&date=20261005", w.revision),
        format!("plan_revision_id={}&date=2026-10-05T00:00:00Z", w.revision),
    ] {
        let (s, b) = resolve(&w.f, &query).await;
        assert_eq!(s, 400, "{query}: {b}");
        assert!(text(&b).contains("DATE_INVALID"), "{query}: {b}");
    }
}

#[tokio::test]
async fn an_unknown_parameter_or_a_malformed_id_is_400() {
    let w = world().await;
    for (query, code) in [
        (
            format!(
                "plan_revision_id={}&date=2026-10-05&as_of=2026-10-05",
                w.revision
            ),
            "QUERY_INVALID",
        ),
        (
            format!("planRevisionId={}&date=2026-10-05", w.revision),
            "QUERY_INVALID",
        ),
        ("date=2026-10-05".to_owned(), "QUERY_INVALID"),
        (
            "plan_revision_id=not-a-uuid&date=2026-10-05".to_owned(),
            "QUERY_INVALID",
        ),
        (
            format!(
                "plan_revision_id={}&date=2026-10-05&item_id=nope",
                w.revision
            ),
            "QUERY_INVALID",
        ),
        (
            format!(
                "plan_revision_id={0}&plan_revision_id={0}&date=2026-10-05",
                w.revision
            ),
            "QUERY_INVALID",
        ),
    ] {
        let (s, b) = resolve(&w.f, &query).await;
        assert_eq!(s, 400, "{query}: {b}");
        assert!(text(&b).contains(code), "{query}: {b}");
    }
}

#[tokio::test]
async fn a_pin_that_does_not_parse_is_400_pin_foreign() {
    let w = world().await;
    for pins in [
        "nope".to_owned(),
        format!("{}:", w.price),
        format!("{},", w.price),
        format!(",{}", w.price),
        "not-a-uuid:eu".to_owned(),
        format!("{}:EU", w.price),
        format!("{}:eu:us", w.price),
        format!("{}:e%20u", w.price),
    ] {
        let (s, b) = resolve(
            &w.f,
            &format!(
                "plan_revision_id={}&date=2026-10-05&pins={pins}",
                w.revision
            ),
        )
        .await;
        assert_eq!(s, 400, "{pins}: {b}");
        assert!(text(&b).contains("PIN_FOREIGN"), "{pins}: {b}");
    }
}

// ------------------------------------------------------------------ pins, the matrix, the walk

/// Spec §7.1 through the door: pinned 10 → all 12 → new 15 renews at 12 and signs up at 15.
#[tokio::test]
async fn a_renewal_walks_to_the_all_successor_and_a_signup_binds_the_new_price() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Recurring);
    let entry = entry_of(&f, eur, sku, "recurring", (Some("month"), None, None)).await;
    let ten = put(
        &f,
        entry,
        Row {
            price: flat("10.00"),
            to: Some("2026-10-01"),
            ..Row::default()
        },
    )
    .await;
    let twelve = put(
        &f,
        entry,
        Row {
            price: flat("12.00"),
            from: "2026-10-01",
            to: Some("2026-11-01"),
            keep: true,
            version_no: 2,
            ..Row::default()
        },
    )
    .await;
    let fifteen = put(
        &f,
        entry,
        Row {
            price: flat("15.00"),
            from: "2026-11-01",
            eligibility: "new",
            version_no: 3,
            ..Row::default()
        },
    )
    .await;
    let (created, revision) = plan(&f, "pro", eur).await;
    item(&f, revision, sku, Some(entry), "paid").await;
    publish(&f, id_of(&created["id"]), revision).await;
    let base = format!("plan_revision_id={revision}&date=2026-11-05");
    let (s, renewal) = resolve(&f, &format!("{base}&pins={ten}")).await;
    assert_eq!(s, 200, "{renewal}");
    let binding = &renewal["items"][0]["chains"][0]["binding"];
    assert_eq!(binding["price_id"], json!(twelve), "{renewal}");
    assert_eq!(binding["price"], flat("12.00"));
    assert_eq!(binding["pinned_from"], json!(ten));
    assert_eq!(binding["keep_for_bound"], true);
    assert_eq!(binding["eligibility"], "all");
    assert_eq!(binding["effective_to"], "2026-11-01", "the stored window");
    assert_eq!(
        binding.get("ends_on"),
        Some(&json!(null)),
        "15's start is not an end for the pin (D-425)"
    );
    let (s, signup) = resolve(&f, &base).await;
    assert_eq!(s, 200, "{signup}");
    let binding = &signup["items"][0]["chains"][0]["binding"];
    assert_eq!(binding["price_id"], json!(fifteen), "{signup}");
    assert_eq!(binding["pinned_from"], json!(null));
    assert_eq!(binding["eligibility"], "new");
}

/// D-425: a binding says where it ends for its holder — a temporary price at its
/// `temporary_until`, an explicitly closed price at its end, an open price nowhere.
#[tokio::test]
async fn a_binding_says_where_it_ends_for_its_holder() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    dimension(&f, "region", &["eu", "us"]).await;
    let sku = catalog.sku(SkuType::Usage);
    let entry = entry_of(&f, eur, sku, "usage", (None, Some("region"), None)).await;
    let usage = |rate: &str, from, version_no| Row {
        model: "per_unit",
        price: json!({ "rate": rate }),
        from,
        version_no,
        ..Row::default()
    };
    let promo = put(
        &f,
        entry,
        Row {
            to: Some("2026-11-10"),
            temporary_until: Some("2026-11-10"),
            ..usage("0.07", "2026-09-01", 1)
        },
    )
    .await;
    let returned = put(&f, entry, usage("0.10", "2026-11-10", 2)).await;
    let closed = put(
        &f,
        entry,
        Row {
            dim: Some("eu"),
            to: Some("2026-12-01"),
            closed: true,
            ..usage("0.08", "2026-09-01", 3)
        },
    )
    .await;
    let (created, revision) = plan(&f, "pro", eur).await;
    item(&f, revision, sku, Some(entry), "paid").await;
    publish(&f, id_of(&created["id"]), revision).await;
    let (s, b) = resolve(&f, &format!("plan_revision_id={revision}&date=2026-10-05")).await;
    assert_eq!(s, 200, "{b}");
    let chains = &b["items"][0]["chains"];
    assert_eq!(chains[0]["binding"]["price_id"], json!(promo), "{b}");
    assert_eq!(chains[0]["binding"]["ends_on"], "2026-11-10", "{b}");
    assert_eq!(chains[1]["binding"]["price_id"], json!(closed), "{b}");
    assert_eq!(chains[1]["binding"]["ends_on"], "2026-12-01", "{b}");
    let (s, b) = resolve(&f, &format!("plan_revision_id={revision}&date=2026-11-15")).await;
    assert_eq!(s, 200, "{b}");
    let binding = &b["items"][0]["chains"][0]["binding"];
    assert_eq!(binding["price_id"], json!(returned), "{b}");
    assert_eq!(binding.get("ends_on"), Some(&json!(null)), "{b}");
}

/// The whole matrix: the default chain, then every registered value in the registry's order;
/// a value without its own price reads the default (`dim_used` null); a value pin names a
/// default-chain price; with no default price a value no chain covers is `uncovered`.
#[tokio::test]
async fn the_matrix_carries_every_registered_value_and_an_uncovered_chain_is_explicit() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    dimension(&f, "region", &["eu", "us"]).await;
    let sku = catalog.sku(SkuType::Usage);
    let entry = entry_of(&f, eur, sku, "usage", (None, Some("region"), None)).await;
    let usage = |dim, rate: &str, version_no| Row {
        dim,
        model: "per_unit",
        price: json!({ "rate": rate }),
        min_fee: Some("5.00"),
        version_no,
        ..Row::default()
    };
    let default = put(&f, entry, usage(None, "0.10", 1)).await;
    let eu = put(&f, entry, usage(Some("eu"), "0.08", 2)).await;
    let bare_sku = catalog.sku(SkuType::Usage);
    let bare = entry_of(&f, eur, bare_sku, "usage", (None, Some("region"), None)).await;
    let bare_eu = put(&f, bare, usage(Some("eu"), "0.07", 1)).await;
    let (created, revision) = plan(&f, "pro", eur).await;
    item(&f, revision, sku, Some(entry), "paid").await;
    item(&f, revision, bare_sku, Some(bare), "paid").await;
    publish(&f, id_of(&created["id"]), revision).await;

    let (s, b) = resolve(
        &f,
        &format!("plan_revision_id={revision}&date=2026-10-05&pins={default}:us"),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let chains = |i: usize| b["items"][i]["chains"].as_array().unwrap().clone();
    let first = chains(0);
    assert_eq!(
        first
            .iter()
            .map(|c| (
                c["dim_value"].clone(),
                c["binding"]["price_id"].clone(),
                c["binding"]["dim_used"].clone(),
                c["binding"]["pinned_from"].clone(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (json!(null), json!(default), json!(null), json!(null)),
            (json!("eu"), json!(eu), json!("eu"), json!(null)),
            (json!("us"), json!(default), json!(null), json!(default)),
        ],
        "{b}"
    );
    assert_eq!(first[1]["binding"]["price"], json!({"rate":"0.08"}));
    assert_eq!(first[1]["binding"]["min_fee"], "5.00", "exact decimal text");
    let second = chains(1);
    assert_eq!(
        second
            .iter()
            .map(|c| (
                c["dim_value"].clone(),
                c["uncovered"].clone(),
                c["binding"]["price_id"].clone()
            ))
            .collect::<Vec<_>>(),
        vec![
            (json!(null), json!(true), json!(null)),
            (json!("eu"), json!(false), json!(bare_eu)),
            (json!("us"), json!(true), json!(null)),
        ],
        "an uncovered chain is explicit, never a refusal: {b}"
    );
    assert!(b["items"][1]["chains"][0]["binding"].is_null(), "{b}");
}

#[tokio::test]
async fn a_pin_foreign_to_the_revision_or_not_approved_is_400_pin_foreign() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    dimension(&f, "region", &["eu", "us"]).await;
    let sku = catalog.sku(SkuType::Usage);
    let entry = entry_of(&f, eur, sku, "usage", (None, Some("region"), None)).await;
    put(&f, entry, Row::default()).await;
    let eu = put(
        &f,
        entry,
        Row {
            dim: Some("eu"),
            version_no: 2,
            ..Row::default()
        },
    )
    .await;
    let mut unapproved = Vec::new();
    for (state, from, version_no) in [
        ("draft", "2031-01-01", 3),
        ("pending", "2031-02-01", 4),
        ("rejected", "2031-03-01", 5),
    ] {
        unapproved.push(
            put(
                &f,
                entry,
                Row {
                    from,
                    state,
                    version_no,
                    ..Row::default()
                },
            )
            .await,
        );
    }
    let other_sku = catalog.sku(SkuType::Usage);
    let other = entry_of(&f, eur, other_sku, "usage", (None, None, None)).await;
    let elsewhere = put(&f, other, Row::default()).await;
    let (created, revision) = plan(&f, "pro", eur).await;
    item(&f, revision, sku, Some(entry), "paid").await;
    publish(&f, id_of(&created["id"]), revision).await;
    let mut pins = vec![
        elsewhere.to_string(),
        format!("{eu}:us"),
        Uuid::new_v4().to_string(),
    ];
    pins.extend(unapproved.iter().map(Uuid::to_string));
    for pin in pins {
        let (s, b) = resolve(
            &f,
            &format!("plan_revision_id={revision}&date=2026-10-05&pins={pin}"),
        )
        .await;
        assert_eq!(s, 400, "{pin}: {b}");
        assert!(text(&b).contains("PIN_FOREIGN"), "{pin}: {b}");
    }
    // Another tenant's approved price is foreign too.
    let (other_f, other_catalog) = setup().await;
    let their_book = book(&other_f, "eur").await;
    let their_sku = other_catalog.sku(SkuType::Usage);
    let their_entry = entry_of(&other_f, their_book, their_sku, "usage", (None, None, None)).await;
    let theirs = put(&other_f, their_entry, Row::default()).await;
    let (s, b) = resolve(
        &f,
        &format!("plan_revision_id={revision}&date=2026-10-05&pins={theirs}"),
    )
    .await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("PIN_FOREIGN"), "{b}");
    assert_eq!(
        catalog.calls(),
        0,
        "the pins are judged before any Products read"
    );
}

#[tokio::test]
async fn two_pins_for_one_item_and_value_or_more_than_a_thousand_pins_are_refused() {
    let w = world().await;
    let base = format!("plan_revision_id={}&date=2026-10-05", w.revision);
    let (s, b) = resolve(&w.f, &format!("{base}&pins={0},{0}", w.price)).await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("PIN_DUPLICATE"), "{b}");
    let many: Vec<String> = (0..1_001).map(|_| Uuid::new_v4().to_string()).collect();
    let (s, b) = resolve(&w.f, &format!("{base}&pins={}", many.join(","))).await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("PINS_TOO_MANY"), "{b}");
    let (s, b) = resolve(&w.f, &format!("{base}&pins={}", w.price)).await;
    assert_eq!(s, 200, "one pin is fine: {b}");
    assert_eq!(
        b["items"][0]["chains"][0]["binding"]["pinned_from"],
        json!(w.price)
    );
}

#[tokio::test]
async fn item_id_resolves_that_item_only_and_an_item_the_revision_lacks_is_404() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Recurring);
    let entry = entry_of(&f, eur, sku, "recurring", (Some("month"), None, None)).await;
    put(&f, entry, Row::default()).await;
    let included_sku = catalog.sku(SkuType::Usage);
    let (created, revision) = plan(&f, "pro", eur).await;
    let paid = item(&f, revision, sku, Some(entry), "paid").await.id;
    let included = item(&f, revision, included_sku, None, "included").await.id;
    publish(&f, id_of(&created["id"]), revision).await;
    let base = format!("plan_revision_id={revision}&date=2026-10-05");
    let (s, b) = resolve(&f, &base).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["items"].as_array().unwrap().len(), 2, "{b}");
    assert_eq!(catalog.calls(), 2, "one dated read per distinct SKU");
    let (s, b) = resolve(&f, &format!("{base}&item_id={included}")).await;
    assert_eq!(s, 200, "{b}");
    let items = b["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{b}");
    assert_eq!(items[0]["item_id"], json!(included));
    assert_eq!(items[0]["treatment"], "included");
    assert_eq!(items[0]["price_book_entry_id"], json!(null));
    assert_eq!(items[0]["charge_kind"], json!(null));
    assert_eq!(
        items[0]["chains"],
        json!([]),
        "an item without an entry has no chains"
    );
    assert_eq!(catalog.calls(), 3, "only the resolved item's SKU is read");
    let calls = catalog.calls();
    for missing in [Uuid::new_v4(), revision] {
        let (s, b) = resolve(&f, &format!("{base}&item_id={missing}")).await;
        assert_eq!(s, 404, "{b}");
    }
    let (s, b) = resolve(&f, &format!("{base}&item_id={paid}")).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["items"][0]["item_id"], json!(paid));
    assert_eq!(catalog.calls(), calls + 1);
}

/// The refusal order: authn, authz, the query, the revision (404, then 409), the item, the
/// pins, and only then Products.
#[tokio::test]
async fn the_refusals_come_in_their_order() {
    let w = world().await;
    let (_, draft) = plan(&w.f, "draft", w.book).await;
    let foreign_pin = Uuid::new_v4();
    let unauthenticated =
        w.f.call_as(
            &toolkit_security::SecurityContext::anonymous(),
            "GET",
            "/resolve?date=bad",
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(unauthenticated.0, 401, "{unauthenticated:?}");
    let (s, b) = resolve_as(&w.f, &holding(&w.f, "price:read"), "date=bad").await;
    assert_eq!(s, 403, "authz before the query: {b}");
    let (s, b) = resolve(
        &w.f,
        &format!(
            "plan_revision_id={}&date=bad&pins={foreign_pin}",
            Uuid::new_v4()
        ),
    )
    .await;
    assert_eq!(s, 400, "the query before the revision: {b}");
    assert!(text(&b).contains("DATE_INVALID"), "{b}");
    let (s, b) = resolve(
        &w.f,
        &format!(
            "plan_revision_id={}&date=2026-10-05&pins={foreign_pin}",
            Uuid::new_v4()
        ),
    )
    .await;
    assert_eq!(s, 404, "the revision before the pins: {b}");
    let (s, b) = resolve(
        &w.f,
        &format!(
            "plan_revision_id={draft}&date=2026-10-05&item_id={}&pins={foreign_pin}",
            Uuid::new_v4()
        ),
    )
    .await;
    assert_eq!(s, 409, "the state before the item and the pins: {b}");
    let (s, b) = resolve(
        &w.f,
        &format!(
            "plan_revision_id={}&date=2026-10-05&item_id={}&pins={foreign_pin}",
            w.revision,
            Uuid::new_v4()
        ),
    )
    .await;
    assert_eq!(s, 404, "the item before the pins: {b}");
    w.catalog.down.store(true, SeqCst);
    let calls = w.catalog.calls();
    let (s, b) = resolve(
        &w.f,
        &format!(
            "plan_revision_id={}&date=2026-10-05&pins={foreign_pin}",
            w.revision
        ),
    )
    .await;
    assert_eq!(s, 400, "the pins before Products: {b}");
    assert!(text(&b).contains("PIN_FOREIGN"), "{b}");
    assert_eq!(w.catalog.calls(), calls);
    let (s, b) = resolve(
        &w.f,
        &format!("plan_revision_id={}&date=2026-10-05", w.revision),
    )
    .await;
    assert_eq!(s, 503, "{b}");
}

// ------------------------------------------------------------------ authz and Products

#[tokio::test]
async fn resolve_needs_plan_read() {
    let w = world().await;
    let query = format!("plan_revision_id={}&date=2026-10-05", w.revision);
    for grant in ["price:read", "plan:author", "config:read"] {
        let (s, b) = resolve_as(&w.f, &holding(&w.f, grant), &query).await;
        assert_eq!(s, 403, "{grant}: {b}");
    }
    let (s, b) = resolve_as(&w.f, &holding(&w.f, "plan:read"), &query).await;
    assert_eq!(s, 200, "{b}");
    let denied = plan_support::request(
        &w.f.denied,
        &w.f.ctx,
        "GET",
        &format!("/resolve?{query}"),
        json!({}),
        None,
        None,
    )
    .await;
    assert_eq!(denied.0, 403, "{denied:?}");
}

/// D-421 as D-424 leaves it: Products that cannot answer is 503, and a definite refusal of the
/// read keeps Products' own status and code. Resolve reads as pricing's system actor (D-424), so
/// the refusal a consumer can meet is one Products gives that actor: here Products bound its
/// registry to another owner.
#[tokio::test]
async fn products_unavailable_is_503_and_its_refusal_keeps_its_own_status_and_code() {
    let w = world().await;
    let query = format!("plan_revision_id={}&date=2026-10-05", w.revision);
    w.catalog.down.store(true, SeqCst);
    let (s, b) = resolve(&w.f, &query).await;
    assert_eq!(s, 503, "{b}");
    assert!(text(&b).contains("REGISTRY_UNAVAILABLE"), "{b}");
    w.catalog.down.store(false, SeqCst);
    w.catalog.foreign_owner.store(true, SeqCst);
    let (s, b) = resolve(&w.f, &query).await;
    assert_eq!(s, 403, "{b}");
    assert!(text(&b).contains("REFERENCE_OWNER_MISMATCH"), "{b}");
    w.catalog.foreign_owner.store(false, SeqCst);
    let (s, b) = resolve(&w.f, &query).await;
    assert_eq!(s, 200, "{b}");
}

// ------------------------------------------------------------------ D-424: SKU versions as pricing's system actor

/// A published plan whose SKU Products holds one dated version.
async fn versioned_world() -> World {
    let w = world().await;
    w.catalog
        .version(w.sku, 1, "2026-01-01", w.catalog.content(w.sku));
    w
}
fn read_version() -> Value {
    json!({"published_version": 1, "effective_from": "2026-01-01"})
}

/// D-424: Rating and Subscriptions call as `*.system` subjects, which Products' registry refuses
/// outright (it admits no system subject but pricing's); resolve reads the SKU versions as
/// pricing's system actor, so such a caller holding only pricing `plan:read` gets its answer.
#[tokio::test]
async fn a_system_caller_other_than_pricing_resolves_with_its_sku_versions() {
    use bss_products_sdk::ReferenceRegistryV1;
    let w = versioned_world().await;
    let tenant = w.f.ctx.subject_tenant_id();
    let app = plan_support::entry_support::app_granting_every_subject(w.f.state.clone(), tenant);
    for consumer in ["bss-rating.system", "bss-subscriptions.system"] {
        let caller = toolkit_security::SecurityContext::builder()
            .subject_id(Uuid::new_v4())
            .subject_tenant_id(tenant)
            .subject_type(consumer)
            .build()
            .unwrap();
        let own = w
            .catalog
            .sku_version_as_of(&caller, tenant, w.sku, date("2026-10-05"))
            .await
            .expect_err("Products refuses this subject a read of its own");
        assert_eq!(own.status_code(), 403, "{consumer}");
        let (s, b, _) = plan_support::request(
            &app,
            &caller,
            "GET",
            &format!("/resolve?plan_revision_id={}&date=2026-10-05", w.revision),
            json!({}),
            None,
            None,
        )
        .await;
        assert_eq!(s, 200, "{consumer}: {b}");
        assert_eq!(
            b["items"][0]["sku_version"],
            read_version(),
            "{consumer}: {b}"
        );
    }
}

/// D-424: a consumer needs pricing `plan:read` only — a caller Products would refuse a SKU read
/// (no products `read`) resolves with the SKU version.
#[tokio::test]
async fn a_caller_without_products_read_resolves_with_its_sku_versions() {
    use bss_products_sdk::ReferenceRegistryV1;
    let w = versioned_world().await;
    let query = format!("plan_revision_id={}&date=2026-10-05", w.revision);
    let reader = holding(&w.f, "plan:read");
    w.catalog.readers([w.f.ctx.subject_id()]);
    let own = w
        .catalog
        .sku_version_as_of(
            &reader,
            reader.subject_tenant_id(),
            w.sku,
            date("2026-10-05"),
        )
        .await
        .expect_err("Products refuses this caller a read of its own");
    assert_eq!(own.status_code(), 403);
    let (s, b) = resolve_as(&w.f, &reader, &query).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["items"][0]["sku_version"], read_version(), "{b}");
}

/// D-424: every dated read is made as pricing's system actor, for the caller's tenant, and only
/// after the caller passed `plan:read` and the revision was found in its tenant.
#[tokio::test]
async fn resolve_reads_every_sku_version_as_the_pricing_system_actor() {
    let w = versioned_world().await;
    let tenant = w.f.ctx.subject_tenant_id();
    let query = format!("plan_revision_id={}&date=2026-10-05", w.revision);
    let (s, b) = resolve_as(&w.f, &holding(&w.f, "price:read"), &query).await;
    assert_eq!(s, 403, "{b}");
    let (s, b) = resolve_as(&w.f, &stranger(), &query).await;
    assert_eq!(s, 404, "{b}");
    assert!(
        w.catalog.version_readers.lock().unwrap().is_empty(),
        "no Products read before plan:read and the revision"
    );
    let (s, b) = resolve(&w.f, &query).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        *w.catalog.version_readers.lock().unwrap(),
        vec![(
            bss_products_sdk::PRICING_SYSTEM_ACTOR,
            Some("bss-pricing.system".to_owned()),
            tenant
        )]
    );
}

#[tokio::test]
async fn a_sku_products_does_not_know_resolves_with_a_null_sku_version() {
    let (f, catalog) = setup().await;
    catalog.dated();
    let eur = book(&f, "eur").await;
    let gone = Uuid::new_v4();
    let entry = entry_of(&f, eur, gone, "recurring", (Some("month"), None, None)).await;
    let price = put(&f, entry, Row::default()).await;
    let (created, revision) = plan(&f, "pro", eur).await;
    item(&f, revision, gone, Some(entry), "paid").await;
    publish(&f, id_of(&created["id"]), revision).await;
    settings(
        &f,
        json!({"default_timing":"arrears","default_rounding":"half_even","default_gl":"9000",
               "default_tax_category":"std","invoice_line_templates":{"recurring":"{sku} per {period}"}}),
    )
    .await;
    let (s, b) = resolve(&f, &format!("plan_revision_id={revision}&date=2026-10-05")).await;
    assert_eq!(s, 200, "{b}");
    let item = &b["items"][0];
    assert_eq!(item["sku_version"], json!(null), "{b}");
    assert_eq!(item["meter"], json!({"usage_type_ref": null, "unit": null}));
    assert_eq!(
        item["invoice_line_template"],
        json!({"value":"{sku} per {period}","source":"tenant"})
    );
    assert_eq!(item["gl_code"], json!({"value":"9000","source":"tenant"}));
    assert_eq!(
        item["tax_category"],
        json!({"value":"std","source":"tenant"})
    );
    assert_eq!(
        item["billing_timing"],
        json!({"value":"arrears","source":"tenant"})
    );
    assert_eq!(b["rounding_policy"], "half_even");
    assert_eq!(item["chains"][0]["binding"]["price_id"], json!(price));
}

/// AC `dod-binding-sku-version`: an October 1 SKU change already published does not reach a
/// September resolve; October binds the new version; a pin does not change the price.
#[tokio::test]
async fn september_binds_sku_version_one_and_october_version_two() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Usage);
    let mut v1 = catalog.content(sku);
    v1.gl_code = Some("4000".into());
    v1.billing_timing = Some(BillingTiming::Advance);
    v1.invoice_line_template = Some("Storage {unit}".into());
    v1.usage_type_ref = Some("gts.cf.core.uc.usage_record.v1~cf.test.storage.v1".into());
    v1.unit = Some("GB".into());
    let mut v2 = v1.clone();
    v2.gl_code = Some("5000".into());
    v2.invoice_line_template = None;
    v2.unit = Some("TB".into());
    catalog.version(sku, 1, "2026-09-01", v1);
    catalog.version(sku, 2, "2026-10-01", v2);
    settings(
        &f,
        json!({"default_timing":"arrears","default_rounding":"half_up","default_gl":"9000",
               "default_tax_category":"std","invoice_line_templates":{"usage":"{sku} usage"}}),
    )
    .await;
    let entry = entry_of(&f, eur, sku, "usage", (None, None, None)).await;
    let price = put(
        &f,
        entry,
        Row {
            model: "per_unit",
            price: json!({"rate":"0.10"}),
            ..Row::default()
        },
    )
    .await;
    let (created, revision) = plan(&f, "pro", eur).await;
    item(&f, revision, sku, Some(entry), "paid").await;
    publish(&f, id_of(&created["id"]), revision).await;

    let (s, september) = resolve(&f, &format!("plan_revision_id={revision}&date=2026-09-15")).await;
    assert_eq!(s, 200, "{september}");
    let item = &september["items"][0];
    assert_eq!(
        item["sku_version"],
        json!({"published_version": 1, "effective_from": "2026-09-01"})
    );
    assert_eq!(item["gl_code"], json!({"value":"4000","source":"sku"}));
    assert_eq!(
        item["invoice_line_template"],
        json!({"value":"Storage {unit}","source":"sku"})
    );
    assert_eq!(
        item["tax_category"],
        json!({"value":"std","source":"tenant"})
    );
    assert_eq!(
        item["billing_timing"],
        json!({"value":"advance","source":"sku"}),
        "PRD AC #13: advance on the SKU beats arrears as the tenant default"
    );
    assert_eq!(
        item["meter"],
        json!({"usage_type_ref":"gts.cf.core.uc.usage_record.v1~cf.test.storage.v1","unit":"GB"})
    );

    let (s, october) = resolve(
        &f,
        &format!("plan_revision_id={revision}&date=2026-10-05&pins={price}"),
    )
    .await;
    assert_eq!(s, 200, "{october}");
    let item = &october["items"][0];
    assert_eq!(
        item["sku_version"],
        json!({"published_version": 2, "effective_from": "2026-10-01"})
    );
    assert_eq!(item["gl_code"], json!({"value":"5000","source":"sku"}));
    assert_eq!(
        item["invoice_line_template"],
        json!({"value":"{sku} usage","source":"tenant"}),
        "the tenant template of the entry's charge kind"
    );
    assert_eq!(item["meter"]["unit"], "TB");
    let binding = &item["chains"][0]["binding"];
    assert_eq!(binding["price_id"], json!(price), "the pin keeps its price");
    assert_eq!(binding["pinned_from"], json!(price));
    assert_eq!(
        september["items"][0]["chains"][0]["binding"]["price"],
        binding["price"]
    );
}

#[tokio::test]
async fn the_entry_override_is_the_first_invoice_line_source() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Recurring);
    let mut content = catalog.content(sku);
    content.invoice_line_template = Some("SKU line".into());
    catalog.version(sku, 1, "2026-01-01", content);
    let entry = entry_of(
        &f,
        eur,
        sku,
        "recurring",
        (Some("month"), None, Some("Pro {period}")),
    )
    .await;
    put(&f, entry, Row::default()).await;
    let (created, revision) = plan(&f, "pro", eur).await;
    item(&f, revision, sku, Some(entry), "paid").await;
    publish(&f, id_of(&created["id"]), revision).await;
    let (s, b) = resolve(&f, &format!("plan_revision_id={revision}&date=2026-10-05")).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        b["items"][0]["invoice_line_template"],
        json!({"value":"Pro {period}","source":"entry"})
    );
}

// ------------------------------------------------------------------ Task 4.3.2: the pinned price

async fn price_read(
    f: &Fixture,
    ctx: &toolkit_security::SecurityContext,
    id: Uuid,
) -> (u16, Value) {
    let (s, b, tag) = f
        .call_as(ctx, "GET", &format!("/prices/{id}"), json!({}), None, None)
        .await;
    assert_eq!(tag, "", "the pinned price read carries no ETag");
    (s, b)
}

/// AC `dod-price-read-forever`: an approved price is served with its original money whatever
/// its window, with its entry's SKU, charge kind, period, book and currency, and nothing
/// computed from today or of its authoring (D-422).
#[tokio::test]
async fn an_approved_price_is_served_forever_whatever_its_window() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Recurring);
    let entry = entry_of(&f, eur, sku, "recurring", (Some("month"), None, None)).await;
    let closed = put(
        &f,
        entry,
        Row {
            price: flat("10.00"),
            from: "2025-01-01",
            to: Some("2025-06-01"),
            closed: true,
            ..Row::default()
        },
    )
    .await;
    let kept = put(
        &f,
        entry,
        Row {
            price: flat("12.00"),
            from: "2025-06-01",
            to: Some("2026-01-01"),
            keep: true,
            version_no: 2,
            ..Row::default()
        },
    )
    .await;
    let open = put(
        &f,
        entry,
        Row {
            price: flat("15.00"),
            min_fee: Some("1.50"),
            from: "2026-01-01",
            eligibility: "new",
            version_no: 3,
            ..Row::default()
        },
    )
    .await;
    let before = written(&f).await;
    let (s, b) = price_read(&f, &f.ctx, open).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        b,
        json!({
            "price_id": open, "price_book_entry_id": entry, "sku_id": sku,
            "charge_kind": "recurring", "period": "month", "book_id": eur, "currency": "EUR",
            "version_no": 3, "dim_value": null, "model": "flat", "price": {"amount": "15.00"},
            "min_fee": "1.50", "eligibility": "new", "effective_from": "2026-01-01",
            "effective_to": null, "temporary_until": null, "keep_for_bound": false,
            "closed_explicitly": false, "paired_price_id": null, "return_of_price_id": null,
            "approved_by_unit_id": null, "approved_at": null
        })
    );
    for (id, money, to, keep) in [
        (closed, "10.00", json!("2025-06-01"), false),
        (kept, "12.00", json!("2026-01-01"), true),
    ] {
        let (s, b) = price_read(&f, &f.ctx, id).await;
        assert_eq!(s, 200, "{b}");
        assert_eq!(b["price_id"], json!(id));
        assert_eq!(b["price"], flat(money), "the original money: {b}");
        assert_eq!(b["effective_to"], to);
        assert_eq!(b["keep_for_bound"], keep);
    }
    let (_, b) = price_read(&f, &f.ctx, closed).await;
    assert_eq!(b["closed_explicitly"], true);
    for internal in [
        "status",
        "state",
        "version",
        "pending_unit_id",
        "note",
        "created_by",
        "created_at",
        "updated_at",
    ] {
        assert!(b.get(internal).is_none(), "{internal} is not served: {b}");
    }
    assert_eq!(written(&f).await, before, "a read writes nothing");
}

#[tokio::test]
async fn a_draft_pending_rejected_unknown_or_foreign_price_is_one_and_the_same_404() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let sku = catalog.sku(SkuType::Recurring);
    let entry = entry_of(&f, eur, sku, "recurring", (Some("month"), None, None)).await;
    let approved = put(&f, entry, Row::default()).await;
    let mut hidden = vec![Uuid::new_v4()];
    for (state, from, version_no) in [
        ("draft", "2031-01-01", 2),
        ("pending", "2031-02-01", 3),
        ("rejected", "2031-03-01", 4),
    ] {
        hidden.push(
            put(
                &f,
                entry,
                Row {
                    from,
                    state,
                    version_no,
                    ..Row::default()
                },
            )
            .await,
        );
    }
    let (s, _) = price_read(&f, &f.ctx, approved).await;
    assert_eq!(s, 200);
    let (s, first) = price_read(&f, &stranger(), approved).await;
    assert_eq!(s, 404, "another tenant's approved price: {first}");
    assert!(text(&first).contains("price"), "a pricing 404: {first}");
    for id in hidden {
        let (s, b) = price_read(&f, &f.ctx, id).await;
        assert_eq!(s, 404, "{id}: {b}");
        assert_eq!(b, first, "one body for every hidden price");
    }
}

#[tokio::test]
async fn the_pinned_price_read_needs_price_read() {
    let w = world().await;
    for grant in ["plan:read", "price:author", "price_book:read"] {
        let (s, b) = price_read(&w.f, &holding(&w.f, grant), w.price).await;
        assert_eq!(s, 403, "{grant}: {b}");
    }
    let (s, b) = price_read(&w.f, &holding(&w.f, "price:read"), w.price).await;
    assert_eq!(s, 200, "{b}");
    let denied = plan_support::request(
        &w.f.denied,
        &w.f.ctx,
        "GET",
        &format!("/prices/{}", w.price),
        json!({}),
        None,
        None,
    )
    .await;
    assert_eq!(denied.0, 403, "{denied:?}");
    assert_eq!(
        w.catalog.calls(),
        0,
        "the pinned read asks Products nothing"
    );
}

// ------------------------------------------------------------------ Task 4.3.3: ETag, measured

/// `module_test` pins which operations declare an `ETag`; this measures it: every GET of the two
/// production routers is called on a live fixture, and exactly those that declare the header on
/// their 200 answer one.
#[tokio::test]
async fn exactly_the_reads_that_declare_an_etag_answer_one() {
    let w = world().await;
    let unit = plan_support::unit_of_kind(&w.f, "prices").await;
    let registry = toolkit::api::OpenApiRegistryImpl::new();
    let _mounted = bss_pricing::api::rest::authoring::router(w.f.state.clone(), &registry).merge(
        bss_pricing::api::rest::read_contract::router(w.f.state.clone(), &registry),
    );
    let mut measured = 0;
    for entry in &registry.operation_specs {
        let (method, template) = entry.key().split_once(':').unwrap();
        if method != "GET" {
            continue;
        }
        let declares = entry.value().responses.iter().any(|r| {
            r.status == 200
                && r.headers
                    .iter()
                    .any(|h| h.name.eq_ignore_ascii_case("etag"))
        });
        let path = template.trim_start_matches("/bss-pricing/v1");
        let id = match path.split('/').nth(1).unwrap() {
            "price-books" => w.book.to_string(),
            "price-book-entries" => w.entry.to_string(),
            "approval-units" => unit.to_string(),
            "plans" => w.plan.to_string(),
            "plan-revisions" => w.revision.to_string(),
            "prices" => w.price.to_string(),
            _ => String::new(),
        };
        let mut path = path.replace("{id}", &id);
        if path == "/resolve" {
            path = format!("/resolve?plan_revision_id={}&date=2026-10-05", w.revision);
        }
        let (s, b, tag) = w.f.call("GET", &path, json!({}), None, None).await;
        assert_eq!(s, 200, "{path}: {b}");
        assert_eq!(!tag.is_empty(), declares, "{path}: ETag {tag:?}");
        measured += 1;
    }
    assert_eq!(measured, 18, "every GET operation is measured");
}

// ------------------------------------------------------------------ phase 4 review F1: what was refused

const PLAN_RESOURCE: &str = "gts.cf.bss.pricing.plan.v1~";
const PRICE_RESOURCE: &str = "gts.cf.bss.pricing.price.v1~";

/// Every refusal of the two consumer doors names the type of what it refused: `GET /resolve` a
/// plan (the query, the pins, the revision, its item, its state and the grant), `GET
/// /prices/{id}` a price (the id, the price and the grant). A consumer routes a refusal by it.
#[tokio::test]
async fn every_read_contract_refusal_names_the_resource_it_refused() {
    let w = world().await;
    let rev = w.revision;
    let (_, draft) = plan(&w.f, "draft", w.book).await;
    item(&w.f, draft, w.sku, Some(w.entry), "paid").await;
    let many = vec![w.price.to_string(); 1_001].join(",");
    let on = "date=2026-10-05";
    for (query, status, code) in [
        (
            format!("plan_revision_id={rev}&{on}&as_of=x"),
            400,
            "QUERY_INVALID",
        ),
        ("date=2026-10-05".to_owned(), 400, "QUERY_INVALID"),
        (format!("plan_revision_id={rev}"), 400, "DATE_INVALID"),
        (
            format!("plan_revision_id={rev}&{on}&pins=nope"),
            400,
            "PIN_FOREIGN",
        ),
        (
            format!("plan_revision_id={rev}&{on}&pins={}", Uuid::new_v4()),
            400,
            "PIN_FOREIGN",
        ),
        (
            format!("plan_revision_id={rev}&{on}&pins={0},{0}", w.price),
            400,
            "PIN_DUPLICATE",
        ),
        (
            format!("plan_revision_id={rev}&{on}&pins={many}"),
            400,
            "PINS_TOO_MANY",
        ),
        (
            format!("plan_revision_id={}&{on}", Uuid::new_v4()),
            404,
            "plan_revision",
        ),
        (
            format!("plan_revision_id={rev}&{on}&item_id={}", Uuid::new_v4()),
            404,
            "plan_item",
        ),
        (
            format!("plan_revision_id={draft}&{on}"),
            409,
            "REVISION_NOT_PUBLISHED",
        ),
    ] {
        let (s, b) = resolve(&w.f, &query).await;
        assert_eq!(s, status, "{query}: {b}");
        assert!(text(&b).contains(code), "{query}: {b}");
        assert_eq!(b["context"]["resource_type"], PLAN_RESOURCE, "{query}: {b}");
    }
    let (s, b) = resolve_as(
        &w.f,
        &holding(&w.f, "price:read"),
        &format!("plan_revision_id={rev}&{on}"),
    )
    .await;
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["context"]["resource_type"], PLAN_RESOURCE, "{b}");

    let (s, b) = price_read(&w.f, &w.f.ctx, Uuid::new_v4()).await;
    assert_eq!(s, 404, "{b}");
    assert_eq!(b["context"]["resource_name"], "price", "{b}");
    assert_eq!(b["context"]["resource_type"], PRICE_RESOURCE, "{b}");
    let (s, b, _) =
        w.f.call("GET", "/prices/not-an-id", json!({}), None, None)
            .await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("ID_INVALID"), "{b}");
    assert_eq!(b["context"]["resource_type"], PRICE_RESOURCE, "{b}");
    let (s, b) = price_read(&w.f, &holding(&w.f, "plan:read"), w.price).await;
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["context"]["resource_type"], PRICE_RESOURCE, "{b}");

    // Phase 4 second review B-1: a read transaction whose contention outlasts the retries is the
    // door's own 409 CONTENDED. The classifier's contention path is forced as the support tests
    // force it, with SQLite's busy signature as the driver error: every read of `pricing_price`
    // (both doors read it inside their transaction) now fails with that text.
    busy_prices(&w.f).await;
    let (s, b) = resolve(&w.f, &format!("plan_revision_id={rev}&{on}")).await;
    assert_eq!(s, 409, "{b}");
    assert!(text(&b).contains("CONTENDED"), "{b}");
    assert_eq!(b["context"]["resource_type"], PLAN_RESOURCE, "{b}");
    let (s, b) = price_read(&w.f, &w.f.ctx, w.price).await;
    assert_eq!(s, 409, "{b}");
    assert!(text(&b).contains("CONTENDED"), "{b}");
    assert_eq!(b["context"]["resource_type"], PRICE_RESOURCE, "{b}");
}
/// Park `pricing_price` behind a view whose every read fails with `SQLite`'s busy signature —
/// `(code: 5) database is locked`, the text the toolkit's retry classifier calls contention.
async fn busy_prices(f: &Fixture) {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let db = Database::connect(&f.dsn).await.unwrap();
    for sql in [
        "ALTER TABLE pricing_price RENAME TO pricing_price_parked",
        "CREATE VIEW pricing_price AS \
         SELECT [(code: 5) database is locked] AS id FROM pricing_price_parked",
    ] {
        db.execute_raw(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
            .await
            .unwrap();
    }
}
