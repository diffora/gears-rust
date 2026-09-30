//! What the plans list names for the screens (phase 9 run 9.1): each plan's current revision and
//! the one in effect (D-460), on the reads and on every write answer that carries the plan DTO, in
//! a fixed number of statements.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod plan_support;
use bss_pricing::infra::storage::{
    entity::plan as plan_entity,
    repo::{plan_repo, plan_revision_repo, price_book_entry_repo, price_repo},
};
use bss_products_sdk::models::SkuType;
use plan_support::{Catalog, Fixture, book, entry, entry_support, id_of, item, plan, scope, setup};
use serde_json::{Value, json};
use std::sync::Arc;
use time::{Date, Duration, OffsetDateTime};
use toolkit_security::SecurityContext;
use uuid::Uuid;

// ------------------------------------------------------------------ fixture

fn today() -> Date {
    OffsetDateTime::now_utc().date()
}
fn days(n: i64) -> Date {
    today() + Duration::days(n)
}
async fn get(f: &Fixture, path: &str) -> Value {
    let (s, b, _) = f.call("GET", path, json!({}), None, None).await;
    assert_eq!(s, 200, "{path}: {b}");
    b
}
async fn policy(f: &Fixture, quorum: u32) {
    let (_, _, tag) = f
        .call("GET", "/approval-policy", json!({}), None, None)
        .await;
    let (s, b, _) = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"kind":"plan_revision","quorum":quorum}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
}
async fn approved(f: &Fixture, entry: Uuid) {
    let tenant = f.ctx.subject_tenant_id();
    let conn = f.db.conn().unwrap();
    let e = price_book_entry_repo::find(&conn, &scope(f), tenant, entry)
        .await
        .unwrap()
        .unwrap();
    let mut p = entry_support::price(&e);
    p.state = "approved".into();
    p.effective_from = Date::from_calendar_date(2020, time::Month::January, 1).unwrap();
    price_repo::insert(&conn, &scope(f), p).await.unwrap();
}
/// A plan whose draft rev 1 holds one confirmed paid usage item on a priced entry of its own
/// book: green, ready to submit.
struct Fresh {
    plan: Uuid,
    rev1: Uuid,
    sku: Uuid,
    created: Value,
}
async fn fresh(f: &Fixture, catalog: &Catalog, code: &str) -> Fresh {
    let eur = book(f, code).await;
    let (created, rev1) = plan(f, code, eur).await;
    let sku = catalog.sku(SkuType::Usage);
    let priced = entry(f, eur, sku, "usage", None).await;
    approved(f, priced).await;
    item(f, rev1, sku, Some(priced), "paid").await;
    Fresh {
        plan: id_of(&created["id"]),
        rev1,
        sku,
        created,
    }
}
async fn submit(f: &Fixture, revision: Uuid, key: &str) -> Value {
    let (s, b, _) = f
        .call(
            "POST",
            &format!("/plan-revisions/{revision}/submit"),
            json!({}),
            None,
            Some(key),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    b
}
async fn vote(
    f: &Fixture,
    who: &SecurityContext,
    unit: &Value,
    action: &str,
    body: Value,
    key: &str,
) -> (u16, Value) {
    let (s, b, _) = f
        .call_as(
            who,
            "POST",
            &format!("/approval-units/{}/{action}", unit.as_str().unwrap()),
            body,
            None,
            Some(key),
        )
        .await;
    (s, b)
}
/// A plan whose rev 1 is published at once (quorum 0).
async fn live(f: &Fixture, catalog: &Catalog, code: &str) -> Fresh {
    let p = fresh(f, catalog, code).await;
    policy(f, 0).await;
    let receipt = submit(f, p.rev1, &format!("{code}-rev1")).await;
    assert_eq!(receipt["revision"]["state"], "published", "{receipt}");
    p
}
async fn copy(f: &Fixture, plan: Uuid, key: &str) -> Value {
    let (s, b, _) = f
        .call(
            "POST",
            &format!("/plans/{plan}/revisions"),
            json!({}),
            None,
            Some(key),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    b
}
/// Set a draft's sale date through its PATCH (null for "at publish").
async fn sale_date(f: &Fixture, revision: Uuid, from: Option<Date>) {
    let path = format!("/plan-revisions/{revision}");
    let (_, _, tag) = f.call("GET", &path, json!({}), None, None).await;
    let (s, b, _) = f
        .call(
            "PATCH",
            &path,
            json!({"available_from": from.map(|d| d.to_string())}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
}
fn current(p: &Value) -> (Value, Value, Value) {
    (
        p["current"]["revision_id"].clone(),
        p["current"]["rev_no"].clone(),
        p["current"]["state"].clone(),
    )
}
fn in_effect(p: &Value) -> (Value, Value) {
    (
        p["in_effect"]["revision_id"].clone(),
        p["in_effect"]["rev_no"].clone(),
    )
}

// ------------------------------------------------------------------ #27 the current revision

/// D-460: `current` is the draft or pending revision, else the scheduled one, else the published
/// one in effect, chosen over the states the revisions read today (D-447); `in_effect` is the
/// published one in effect. A due scheduled revision whose switch is not persisted is both.
/// Both are null without revisions. The plan read agrees with its list row.
#[tokio::test]
async fn the_current_revision_and_the_one_in_effect_over_every_state_mix() {
    let (f, catalog) = setup().await;
    let published = live(&f, &catalog, "published").await;
    let beside_draft = live(&f, &catalog, "draft-beside").await;
    let beside_pending = live(&f, &catalog, "pending-beside").await;
    let beside_scheduled = live(&f, &catalog, "scheduled-beside").await;
    let due = live(&f, &catalog, "due").await;
    let draft = fresh(&f, &catalog, "draft").await;
    let draft_rev2 = id_of(&copy(&f, beside_draft.plan, "copy-draft").await["id"]);
    // A due revision: seeded scheduled with today's date through the repository (the apply
    // publishes such a date at once), so no job or door has persisted its switch.
    let due_rev2 = id_of(&copy(&f, due.plan, "copy-due").await["id"]);
    sale_date(&f, due_rev2, Some(today())).await;
    let due_unit = plan_support::lock(&f, due_rev2).await;
    plan_revision_repo::schedule(
        &f.db.conn().unwrap(),
        &scope(&f),
        f.ctx.subject_tenant_id(),
        due_rev2,
        due_unit,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    policy(&f, 1).await;
    let pending = fresh(&f, &catalog, "pending").await;
    submit(&f, pending.rev1, "pending-rev1").await;
    let pending_rev2 = id_of(&copy(&f, beside_pending.plan, "copy-pending").await["id"]);
    submit(&f, pending_rev2, "pending-rev2").await;
    let scheduled_rev2 = id_of(&copy(&f, beside_scheduled.plan, "copy-scheduled").await["id"]);
    sale_date(&f, scheduled_rev2, Some(days(3))).await;
    let receipt = submit(&f, scheduled_rev2, "scheduled-rev2").await;
    let (s, b) = vote(
        &f,
        &f.user(),
        &receipt["unit"]["id"],
        "approve",
        json!({"generation":1}),
        "approve-scheduled",
    )
    .await;
    assert_eq!((s, b["outcome"].clone()), (200, json!("applied")), "{b}");
    let empty = plan_repo::insert(
        &f.db.conn().unwrap(),
        &scope(&f),
        plan_entity::Model {
            id: Uuid::now_v7(),
            tenant_id: f.ctx.subject_tenant_id(),
            code: "empty".into(),
            name: "Empty".into(),
            published_rev: None,
            version: 1,
            created_by: f.ctx.subject_id(),
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        },
    )
    .await
    .unwrap()
    .id;

    let s = |id: Uuid| json!(id.to_string());
    let expected = [
        (
            "draft only",
            draft.plan,
            (s(draft.rev1), json!(1), json!("draft")),
            (json!(null), json!(null)),
        ),
        (
            "pending only",
            pending.plan,
            (s(pending.rev1), json!(1), json!("pending")),
            (json!(null), json!(null)),
        ),
        (
            "published only",
            published.plan,
            (s(published.rev1), json!(1), json!("published")),
            (s(published.rev1), json!(1)),
        ),
        (
            "a draft beside the published",
            beside_draft.plan,
            (s(draft_rev2), json!(2), json!("draft")),
            (s(beside_draft.rev1), json!(1)),
        ),
        (
            "a pending beside the published",
            beside_pending.plan,
            (s(pending_rev2), json!(2), json!("pending")),
            (s(beside_pending.rev1), json!(1)),
        ),
        (
            "a waiting scheduled beside the published",
            beside_scheduled.plan,
            (s(scheduled_rev2), json!(2), json!("scheduled")),
            (s(beside_scheduled.rev1), json!(1)),
        ),
        (
            "a due scheduled, its switch not persisted",
            due.plan,
            (s(due_rev2), json!(2), json!("published")),
            (s(due_rev2), json!(2)),
        ),
        (
            "no revision",
            empty,
            (json!(null), json!(null), json!(null)),
            (json!(null), json!(null)),
        ),
    ];
    let listed = get(&f, "/plans").await;
    let rows = listed["items"].as_array().unwrap();
    assert_eq!(rows.len(), expected.len(), "{listed}");
    for (name, plan_id, want_current, want_in_effect) in expected {
        let row = rows
            .iter()
            .find(|p| p["id"] == plan_id.to_string())
            .unwrap_or_else(|| panic!("{name}: not listed"));
        assert_eq!(current(row), want_current, "{name}: {row}");
        assert_eq!(in_effect(row), want_in_effect, "{name}: {row}");
        let read = get(&f, &format!("/plans/{plan_id}")).await;
        assert_eq!(read["current"], row["current"], "{name}: the read agrees");
        assert_eq!(
            read["in_effect"], row["in_effect"],
            "{name}: the read agrees"
        );
    }
    let stored = plan_revision_repo::find(
        &f.db.conn().unwrap(),
        &scope(&f),
        f.ctx.subject_tenant_id(),
        due_rev2,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(stored.state, "scheduled", "the read derived, never wrote");
}

/// D-460: `item_count` and `sku_ids` count every item of the current revision, an included item
/// without an entry too, while `GET /plans?sku_id=` keeps a plan only for an item with an entry,
/// judged on the stored state (D-434): the two may differ. `created_by` is the current
/// revision's author, not the plan's.
#[tokio::test]
async fn the_current_revisions_skus_count_every_item_and_may_differ_from_the_sku_filter() {
    let (f, catalog) = setup().await;
    let p = fresh(&f, &catalog, "pro").await;
    let included = catalog.sku(SkuType::Usage);
    plan_support::item_with_qty(&f, p.rev1, included, "5").await;
    let listed = get(&f, "/plans").await;
    let row = &listed["items"][0];
    assert_eq!(row["current"]["item_count"], 2, "{row}");
    let mut skus = vec![p.sku.to_string(), included.to_string()];
    skus.sort();
    assert_eq!(row["current"]["sku_ids"], json!(skus), "ascending: {row}");
    assert_eq!(row["current"]["created_by"], f.ctx.subject_id().to_string());
    let by_entry = get(&f, &format!("/plans?sku_id={}", p.sku)).await;
    assert_eq!(by_entry["items"].as_array().unwrap().len(), 1);
    assert_eq!(by_entry["items"][0]["current"], row["current"]);
    let by_included = get(&f, &format!("/plans?sku_id={included}")).await;
    assert!(
        by_included["items"].as_array().unwrap().is_empty(),
        "the SKU filter needs an entry, the current revision names every item: {by_included}"
    );
    // The copy is authored by another principal: `current.created_by` follows the revision.
    policy(&f, 0).await;
    submit(&f, p.rev1, "rev1").await;
    let other = f.user();
    let (s, b, _) = f
        .call_as(
            &other,
            "POST",
            &format!("/plans/{}/revisions", p.plan),
            json!({}),
            None,
            Some("copy"),
        )
        .await;
    assert_eq!(s, 201, "{b}");
    let read = get(&f, &format!("/plans/{}", p.plan)).await;
    assert_eq!(
        read["current"]["created_by"],
        other.subject_id().to_string()
    );
    assert_eq!(
        read["current"]["item_count"], 2,
        "the copy carries both items"
    );
    assert_eq!(read["created_by"], f.ctx.subject_id().to_string());
}

/// The statements on pricing's tables one plan list makes.
async fn list_statements(
    f: &Fixture,
    recorder: &toolkit_db::test_support::QueryRecorder,
    n: usize,
) -> Vec<String> {
    recorder.clear();
    let listed = get(f, "/plans").await;
    assert_eq!(listed["items"].as_array().unwrap().len(), n);
    recorder
        .events()
        .into_iter()
        .filter(|q| {
            q.table
                .as_deref()
                .is_some_and(|t| t.starts_with("pricing_"))
        })
        .map(|q| q.sql)
        .collect()
}
/// `n` plans on one book, every other one published (a unit approves it) and the rest pending
/// under a unit, each revision with one item.
async fn seeded_plans(f: &Fixture, catalog: &Catalog, eur: Uuid, from: usize, n: usize) {
    for i in from..from + n {
        let (created, rev1) = plan(f, &format!("p-{i:03}"), eur).await;
        item(f, rev1, catalog.sku(SkuType::Usage), None, "included").await;
        if i % 2 == 0 {
            plan_support::publish(f, id_of(&created["id"]), rev1).await;
        } else {
            plan_support::lock(f, rev1).await;
        }
    }
}

// Probed in run 9.1: a per-plan read of the items is red here.
/// D-460 (amending D-434 and D-453): `GET /plans` makes three statements whatever the number of
/// plans: the plans, their revisions and the current revisions' items. The same statements for 10
/// and for 100 plans.
#[tokio::test]
async fn the_plan_list_reads_in_three_statements_for_10_and_100_plans() {
    let (db, recorder, tenant, dsn) = entry_support::recorded_db().await;
    let catalog = Arc::new(Catalog::default());
    let f = Fixture::on(db, tenant, dsn, catalog.clone()).await;
    let eur = book(&f, "eur").await;
    seeded_plans(&f, &catalog, eur, 0, 10).await;
    let ten = list_statements(&f, &recorder, 10).await;
    seeded_plans(&f, &catalog, eur, 10, 90).await;
    let hundred = list_statements(&f, &recorder, 100).await;
    for (i, sql) in hundred.iter().enumerate() {
        eprintln!("plan list statement {i}: {sql}");
    }
    assert_eq!(ten.len(), 3, "{ten:#?}");
    assert_eq!(ten, hundred, "the same statements, whatever the size");
    let listed = get(&f, "/plans").await;
    for p in listed["items"].as_array().unwrap() {
        assert_eq!(p["current"]["item_count"], 1, "{p}");
    }
}

// ------------------------------------------------------------------ M7 the write answers

/// D-460 (plan review M7): the plan create and clone and the rename answer the current revision
/// and the one in effect, from the rows the write holds.
#[tokio::test]
async fn every_write_answer_carries_the_new_fields() {
    let (f, catalog) = setup().await;
    let p = fresh(&f, &catalog, "pro").await;
    let created = &p.created;
    let rev1 = p.rev1.to_string();
    assert_eq!(
        created["current"],
        json!({"revision_id":rev1,"rev_no":1,"state":"draft","item_count":0,"sku_ids":[],
               "created_by":f.ctx.subject_id()}),
        "the create answers its empty draft: {created}"
    );
    assert_eq!(created["in_effect"], json!(null));
    policy(&f, 0).await;
    submit(&f, p.rev1, "submit").await;
    let (s, renamed, _) = {
        let path = format!("/plans/{}", p.plan);
        let (_, _, tag) = f.call("GET", &path, json!({}), None, None).await;
        f.call("PATCH", &path, json!({"name":"Pro 2"}), Some(&tag), None)
            .await
    };
    assert_eq!(s, 200, "{renamed}");
    assert_eq!(renamed["current"]["revision_id"], rev1);
    assert_eq!(renamed["current"]["state"], "published");
    assert_eq!(renamed["in_effect"], json!({"revision_id":rev1,"rev_no":1}));
    let (s, cloned, _) = f
        .call(
            "POST",
            &format!("/plans/{}/clone", p.plan),
            json!({"code":"clone","name":"Clone"}),
            None,
            Some("clone"),
        )
        .await;
    assert_eq!(s, 201, "{cloned}");
    assert_eq!(cloned["current"]["state"], "draft");
    assert_eq!(
        cloned["current"]["item_count"], 1,
        "the clone's copied item"
    );
    assert_eq!(cloned["current"]["sku_ids"], json!([p.sku]));
    assert_eq!(cloned["in_effect"], json!(null));
}
