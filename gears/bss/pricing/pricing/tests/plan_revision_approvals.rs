//! The `plan_revision` approval kind through the production router (run 3.4, Task 3.4.2): the
//! submit door judges the revision with the same checks as its checks door and writes no unit
//! while one is red; the revision is locked while pending and returns to draft on reject or
//! withdraw; separation of duties excludes the submitter and the revision's author; apply
//! re-checks with fresh reads, supersedes the published revision first and advances the plan's
//! `published_rev`; descriptors and reference columns are never fingerprinted content (D-408).
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod plan_support;
use bss_pricing::infra::storage::repo::{plan_item_repo, price_book_entry_repo, price_repo};
use bss_products_sdk::models::{Lifecycle, SkuType};
use plan_support::{
    Catalog, Fixture, book, entry, entry_support::outbox_events, id_of, item, items, plan, raw,
    scope, setup, text,
};
use serde_json::{Value, json};
use toolkit_security::SecurityContext;
use uuid::Uuid;

const PUBLISHED: &str = "gts.cf.core.events.event.v1~cf.bss.pricing.plan_revision_published.v1~";
const DECIDED: &str = "gts.cf.core.events.event.v1~cf.bss.pricing.approval_unit_decided.v1~";

/// A plan whose draft revision 1 is green: one confirmed paid usage item on an entry of the
/// plan's book, priced from 2020 with an open tail.
struct Green {
    plan: Uuid,
    revision: Uuid,
    sku: Uuid,
    entry: Uuid,
}
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
async fn green(f: &Fixture, catalog: &Catalog, code: &str) -> Green {
    let eur = book(f, code).await;
    let (p, revision) = plan(f, code, eur).await;
    let sku = catalog.sku(SkuType::Usage);
    let e = entry(f, eur, sku, "usage", None).await;
    approved(f, e, "2020-01-01").await;
    item(f, revision, sku, Some(e), "paid").await;
    Green {
        plan: id_of(&p["id"]),
        revision,
        sku,
        entry: e,
    }
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
async fn submit(f: &Fixture, who: &SecurityContext, revision: Uuid, key: &str) -> (u16, Value) {
    let (s, b, _) = f
        .call_as(
            who,
            "POST",
            &format!("/plan-revisions/{revision}/submit"),
            json!({}),
            None,
            Some(key),
        )
        .await;
    (s, b)
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
async fn revision(f: &Fixture, id: Uuid) -> Value {
    let (s, b, _) = f
        .call(
            "GET",
            &format!("/plan-revisions/{id}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    b
}
async fn plan_of(f: &Fixture, id: Uuid) -> Value {
    let (s, b, _) = f
        .call("GET", &format!("/plans/{id}"), json!({}), None, None)
        .await;
    assert_eq!(s, 200, "{b}");
    b
}
async fn units(f: &Fixture) -> Vec<Value> {
    let (s, b, _) = f
        .call(
            "GET",
            "/approval-units?kind=plan_revision",
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    b["items"].as_array().unwrap().clone()
}
async fn checks(f: &Fixture, revision: Uuid) -> Value {
    let (s, b, _) = f
        .call(
            "GET",
            &format!("/plan-revisions/{revision}/checks"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    b
}
fn red_codes(checks: &Value) -> Vec<String> {
    checks["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["ok"] == false)
        .map(|c| c["code"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn a_green_revision_submits_under_its_key_and_quorum_zero_publishes_it_at_once() {
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 0).await;
    let path = format!("/plan-revisions/{}/submit", g.revision);
    assert_eq!(
        f.call("POST", &path, json!({}), None, None).await.0,
        400,
        "an Idempotency-Key is required"
    );
    assert_eq!(
        f.call("POST", &path, json!({"note":"x"}), None, Some("body"))
            .await
            .0,
        400,
        "the submit takes no body"
    );
    let (s, receipt) = submit(&f, &f.ctx, g.revision, "submit").await;
    assert_eq!(s, 201, "{receipt}");
    assert_eq!(receipt["applied"], true);
    let unit = &receipt["unit"];
    assert_eq!(unit["kind"], "plan_revision");
    assert_eq!(unit["ref_type"], "plan_revision");
    assert_eq!(unit["ref_id"], g.revision.to_string());
    assert_eq!(unit["state"], "approved");
    assert_eq!(unit["quorum_required"], 0);
    assert_eq!(unit["decisions"], json!([]));
    let snapshot = &unit["snapshot"];
    assert_eq!(
        snapshot["after"],
        json!({
            "book_id": revision(&f, g.revision).await["book_id"],
            "available_from": null,
            "items": [{
                "sku_id": g.sku, "price_book_entry_id": g.entry, "treatment": "paid",
                "included_qty": null, "qty_min": null,
            }],
        }),
        "the fingerprinted content is the business content only: {snapshot}"
    );
    assert_eq!(
        snapshot["before"],
        json!(null),
        "nothing was published before"
    );
    assert_eq!(snapshot["descriptors"][0]["sku_id"], g.sku.to_string());
    assert_eq!(
        snapshot["impact"],
        json!({"subscriptions":"unavailable until the Subscriptions integration"})
    );
    let r = &receipt["revision"];
    assert_eq!(r["state"], "published", "{receipt}");
    assert_eq!(r["approved_by_unit_id"], unit["id"]);
    assert_eq!(r["pending_unit_id"], json!(null));
    assert!(r["published_at"].is_string(), "{r}");
    assert_eq!(plan_of(&f, g.plan).await["published_rev"], 1);
    let (s, replay) = submit(&f, &f.ctx, g.revision, "submit").await;
    assert_eq!((s, &replay), (201, &receipt), "the key replays its answer");
    let published = outbox_events(&f.dsn, PUBLISHED).await;
    assert_eq!(published.len(), 1, "{published:?}");
    let data = &published[0]["data"];
    assert_eq!(data["tenantId"], f.ctx.subject_tenant_id().to_string());
    assert_eq!(data["planId"], g.plan.to_string());
    assert_eq!(data["revisionId"], g.revision.to_string());
    assert_eq!(data["revNo"], 1);
    assert_eq!(data["bookId"], r["book_id"]);
    assert_eq!(data["supersededRevisionId"], json!(null));
    assert_eq!(data["unitId"], unit["id"]);
    let decided = outbox_events(&f.dsn, DECIDED).await;
    assert_eq!(decided.len(), 1, "{decided:?}");
    assert_eq!(decided[0]["data"]["kind"], "plan_revision");
    assert_eq!(decided[0]["data"]["state"], "approved");
}

#[tokio::test]
async fn a_red_revision_is_refused_with_the_checks_doors_red_checks_and_no_unit() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    let sku = catalog.sku(SkuType::Usage);
    let e = entry(&f, eur, sku, "usage", None).await;
    item(&f, rev, sku, Some(e), "paid").await;
    policy(&f, 0).await;
    let expected = red_codes(&checks(&f, rev).await);
    assert_eq!(expected, vec!["ITEM_UNCOVERED".to_owned()]);
    let (s, b) = submit(&f, &f.ctx, rev, "submit").await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("REVISION_CHECKS_RED"), "{b}");
    let detail: Value = serde_json::from_str(b["detail"].as_str().unwrap()).unwrap();
    let codes: Vec<&str> = detail
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, expected, "the same checks as the checks door: {b}");
    for field in ["label", "detail", "blocked_by"] {
        assert!(!detail[0][field].is_null(), "{field}: {detail}");
    }
    assert!(units(&f).await.is_empty(), "no unit is written");
    assert_eq!(revision(&f, rev).await["state"], "draft");
    approved(&f, e, "2020-01-01").await;
    let (s, b) = submit(&f, &f.ctx, rev, "submit").await;
    assert_eq!(s, 201, "the refused key was not consumed: {b}");
}

#[tokio::test]
async fn a_red_revision_names_the_pending_price_unit_that_blocks_it() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (_, rev) = plan(&f, "pro", eur).await;
    let sku = catalog.sku(SkuType::Usage);
    let e = entry(&f, eur, sku, "usage", None).await;
    item(&f, rev, sku, Some(e), "paid").await;
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
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let (s, drafted, _) = f
        .call(
            "POST",
            &format!("/price-book-entries/{e}/prices"),
            json!({"model":"per_unit","price":{"rate":"0.10"},"eligibility":"all","effective_from":today}),
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
            Some("price-submit"),
        )
        .await;
    assert_eq!(s, 201, "{receipt}");
    let (s, b) = submit(&f, &f.ctx, rev, "submit").await;
    assert_eq!(s, 400, "{b}");
    let detail: Value = serde_json::from_str(b["detail"].as_str().unwrap()).unwrap();
    assert_eq!(detail[0]["code"], "ITEM_UNCOVERED");
    assert_eq!(detail[0]["blocked_by"], json!([receipt["unit"]["id"]]));
    assert!(
        text(&b).contains(receipt["unit"]["id"].as_str().unwrap()),
        "the field violation names the blocking unit too: {b}"
    );
}

#[tokio::test]
async fn submit_needs_an_unlocked_draft_of_the_tenant_and_an_answering_registry() {
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 1).await;
    let (s, b) = submit(&f, &f.ctx, Uuid::new_v4(), "missing").await;
    assert_eq!(s, 404, "{b}");
    catalog
        .down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (s, b) = submit(&f, &f.ctx, g.revision, "down").await;
    assert_eq!(s, 503, "a registry that cannot answer writes nothing: {b}");
    assert!(units(&f).await.is_empty());
    catalog
        .down
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let (s, receipt) = submit(&f, &f.ctx, g.revision, "first").await;
    assert_eq!(s, 201, "{receipt}");
    assert_eq!(receipt["applied"], false);
    assert_eq!(receipt["revision"]["state"], "pending");
    assert_eq!(
        receipt["revision"]["pending_unit_id"],
        receipt["unit"]["id"]
    );
    let (s, b) = submit(&f, &f.ctx, g.revision, "second").await;
    assert_eq!(s, 409, "{b}");
    assert!(text(&b).contains("REVISION_NOT_DRAFT"), "{b}");
    assert_eq!(units(&f).await.len(), 1);
    let (s, b) = submit(&f, &plan_support::stranger(), g.revision, "stranger").await;
    assert_eq!(s, 403, "a stranger may not submit here: {b}");
    // D-418: a plan revision is submitted under `plan:submit`, the plan label's own action, as a
    // price is under `price:submit`; neither authoring the plan nor `approval_unit:submit` is it.
    for grant in ["plan:author", "approval_unit:submit"] {
        let (s, b) = submit(&f, &plan_support::holding(&f, grant), g.revision, grant).await;
        assert_eq!(s, 403, "submitting needs plan:submit, not {grant}: {b}");
    }
    let other = green(&f, &catalog, "other").await;
    let (s, b) = submit(
        &f,
        &plan_support::holding(&f, "plan:submit"),
        other.revision,
        "submitter",
    )
    .await;
    assert_eq!(s, 201, "{b}");
}

#[tokio::test]
async fn a_lost_lock_race_is_row_locked_pending_and_writes_no_unit() {
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 1).await;
    // Another submit takes the revision between this one's read and its lock: the conditional
    // write matches no row.
    raw(
        &f,
        "CREATE TRIGGER lose_the_lock BEFORE UPDATE OF pending_unit_id ON pricing_plan_revision \
         WHEN NEW.pending_unit_id IS NOT NULL BEGIN SELECT RAISE(IGNORE); END",
    )
    .await;
    let (s, b) = submit(&f, &f.ctx, g.revision, "submit").await;
    assert_eq!(s, 409, "{b}");
    assert!(text(&b).contains("ROW_LOCKED_PENDING"), "{b}");
    assert!(units(&f).await.is_empty(), "the whole submit rolled back");
}

#[tokio::test]
async fn a_pending_revision_is_locked_and_reject_or_withdraw_return_it_to_draft() {
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 1).await;
    let (s, receipt) = submit(&f, &f.ctx, g.revision, "first").await;
    assert_eq!(s, 201, "{receipt}");
    let unit = &receipt["unit"]["id"];
    let path = format!("/plan-revisions/{}", g.revision);
    let (_, _, tag) = f.call("GET", &path, json!({}), None, None).await;
    let (s, b, _) = f
        .call(
            "PATCH",
            &path,
            json!({"available_from":"2031-01-01"}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 409, "a pending revision is not editable: {b}");
    assert!(text(&b).contains("REVISION_NOT_DRAFT"), "{b}");
    let reviewer = f.user();
    let (s, b) = vote(
        &f,
        &reviewer,
        unit,
        "reject",
        json!({"generation":1,"note":"not yet"}),
        "reject",
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["outcome"], "rejected");
    let stored = revision(&f, g.revision).await;
    assert_eq!(
        (stored["state"].clone(), stored["pending_unit_id"].clone()),
        (json!("draft"), json!(null)),
        "{stored}"
    );
    let (_, _, tag) = f.call("GET", &path, json!({}), None, None).await;
    let (s, b, _) = f
        .call(
            "PATCH",
            &path,
            json!({"available_from":null}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(
        s, 200,
        "a rejected revision is an editable draft again: {b}"
    );
    let (s, receipt) = submit(&f, &f.ctx, g.revision, "second").await;
    assert_eq!(s, 201, "{receipt}");
    let (s, b) = vote(
        &f,
        &f.user(),
        &receipt["unit"]["id"],
        "withdraw",
        json!({}),
        "foreign-withdraw",
    )
    .await;
    assert_eq!(s, 403, "only the submitter withdraws: {b}");
    let (s, b) = vote(
        &f,
        &f.ctx,
        &receipt["unit"]["id"],
        "withdraw",
        json!({}),
        "withdraw",
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["outcome"], "withdrawn");
    assert_eq!(revision(&f, g.revision).await["state"], "draft");
    let decided: Vec<String> = outbox_events(&f.dsn, DECIDED)
        .await
        .iter()
        .map(|e| {
            assert_eq!(e["data"]["kind"], "plan_revision");
            e["data"]["state"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(decided, vec!["rejected", "withdrawn"]);
    assert!(outbox_events(&f.dsn, PUBLISHED).await.is_empty());
    assert_eq!(plan_of(&f, g.plan).await["published_rev"], json!(null));
}

#[tokio::test]
async fn the_author_and_the_submitter_may_not_approve_and_an_independent_reviewer_publishes() {
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 1).await;
    let (author, submitter, reviewer) = (f.ctx.clone(), f.user(), f.user());
    let (s, receipt) = submit(&f, &submitter, g.revision, "submit").await;
    assert_eq!(s, 201, "{receipt}");
    let unit = &receipt["unit"]["id"];
    for (who, key) in [(&author, "author"), (&submitter, "submitter")] {
        let (s, b) = vote(&f, who, unit, "approve", json!({"generation":1}), key).await;
        assert_eq!(s, 403, "{key}: {b}");
        assert!(text(&b).contains("SOD_VIOLATION"), "{b}");
    }
    let (s, b) = vote(
        &f,
        &reviewer,
        unit,
        "approve",
        json!({"generation":1}),
        "reviewer",
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["outcome"], "applied");
    assert_eq!(b["unit"]["state"], "approved");
    let stored = revision(&f, g.revision).await;
    assert_eq!(stored["state"], "published");
    assert_eq!(stored["approved_by_unit_id"], *unit);
    assert_eq!(plan_of(&f, g.plan).await["published_rev"], 1);
    let published = outbox_events(&f.dsn, PUBLISHED).await;
    assert_eq!(published.len(), 1);
    assert_eq!(
        published[0]["data"]["actorRef"],
        reviewer.subject_id().to_string()
    );
    let decided = outbox_events(&f.dsn, DECIDED).await;
    assert_eq!(decided.len(), 1);
    assert_eq!(decided[0]["data"]["state"], "approved");
    let (s, card, _) = f
        .call(
            "GET",
            &format!("/approval-units/{}", unit.as_str().unwrap()),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{card}");
    assert_eq!(
        card["impact"],
        json!({"subscriptions":"unavailable until the Subscriptions integration"})
    );
}

// D-416: a voter without products `read`. The submit and the final approve run the checks, a
// rule, and read the SKUs as the caller: Products' own 403. A non-final approve and a reject apply
// no rule, and their descriptor read is information only: they pass.
#[tokio::test]
async fn a_voter_without_products_read_is_refused_only_by_the_checks() {
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 2).await;
    catalog.readers([f.ctx.subject_id()]);
    let (s, b) = submit(&f, &f.user(), g.revision, "foreign-submit").await;
    assert_eq!(s, 403, "the submit runs the checks as its caller: {b}");
    assert!(text(&b).contains("SKU_READ_DENIED"), "{b}");
    let (s, receipt) = submit(&f, &f.ctx, g.revision, "submit").await;
    assert_eq!(s, 201, "{receipt}");
    let unit = &receipt["unit"]["id"];
    let (one, two) = (f.user(), f.user());
    let (s, b) = vote(&f, &one, unit, "approve", json!({"generation":1}), "one").await;
    assert_eq!(s, 200, "a non-final approve reads no SKU for a rule: {b}");
    assert_eq!(b["outcome"], "pending");
    let (s, b) = vote(&f, &two, unit, "approve", json!({"generation":1}), "two").await;
    assert_eq!(s, 403, "the final approve re-runs the checks: {b}");
    assert!(
        text(&b).contains("SKU_READ_DENIED"),
        "Products' own code: {b}"
    );
    assert_eq!(revision(&f, g.revision).await["state"], "pending");
    let (s, b) = vote(
        &f,
        &two,
        unit,
        "reject",
        json!({"generation":1,"note":"not yet"}),
        "reject",
    )
    .await;
    assert_eq!(s, 200, "a reject reads no SKU for a rule: {b}");
    assert_eq!(b["outcome"], "rejected");
    assert_eq!(revision(&f, g.revision).await["state"], "draft");
}

// D-416: a registry outage refuses only what reads a SKU for a rule; a non-final approve and a
// reject of a plan_revision unit pass.
#[tokio::test]
async fn a_registry_outage_refuses_only_the_final_approve_of_a_revision() {
    use std::sync::atomic::Ordering::SeqCst;
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 2).await;
    let (s, receipt) = submit(&f, &f.ctx, g.revision, "submit").await;
    assert_eq!(s, 201, "{receipt}");
    let unit = &receipt["unit"]["id"];
    catalog.down.store(true, SeqCst);
    let (one, two) = (f.user(), f.user());
    let (s, b) = vote(&f, &one, unit, "approve", json!({"generation":1}), "one").await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["outcome"], "pending");
    let (s, b) = vote(&f, &two, unit, "approve", json!({"generation":1}), "two").await;
    assert_eq!(s, 503, "{b}");
    assert!(text(&b).contains("REGISTRY_UNAVAILABLE"), "{b}");
    let (s, b) = vote(
        &f,
        &two,
        unit,
        "reject",
        json!({"generation":1,"note":"not yet"}),
        "reject",
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["outcome"], "rejected");
    assert_eq!(revision(&f, g.revision).await["state"], "draft");
}

#[tokio::test]
async fn apply_supersedes_the_published_revision_first_and_advances_published_rev() {
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 0).await;
    let (s, b) = submit(&f, &f.ctx, g.revision, "rev1").await;
    assert_eq!(s, 201, "{b}");
    let (s, copy, _) = f
        .call(
            "POST",
            &format!("/plans/{}/revisions", g.plan),
            json!({}),
            None,
            Some("copy"),
        )
        .await;
    assert_eq!(s, 201, "{copy}");
    let rev2 = id_of(&copy["id"]);
    assert_eq!(
        items(&f, rev2).await[0].reference_state,
        "confirmed",
        "the copied item attached"
    );
    let (s, receipt) = submit(&f, &f.ctx, rev2, "rev2").await;
    assert_eq!(s, 201, "{receipt}");
    assert_eq!(receipt["revision"]["state"], "published");
    let before = &receipt["unit"]["snapshot"]["before"];
    assert_eq!(
        before["items"][0]["sku_id"],
        g.sku.to_string(),
        "before is the published revision's content: {before}"
    );
    let r1 = revision(&f, g.revision).await;
    assert_eq!(r1["state"], "superseded");
    assert!(
        !r1["approved_by_unit_id"].is_null(),
        "approval identity stays forever"
    );
    assert_eq!(plan_of(&f, g.plan).await["published_rev"], 2);
    let published = outbox_events(&f.dsn, PUBLISHED).await;
    assert_eq!(published.len(), 2);
    let second = published.iter().find(|e| e["data"]["revNo"] == 2).unwrap();
    assert_eq!(
        second["data"]["supersededRevisionId"],
        g.revision.to_string()
    );
}

#[tokio::test]
async fn apply_rechecks_with_fresh_reads_and_refuses_a_revision_that_turned_red() {
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 1).await;
    let (s, receipt) = submit(&f, &f.ctx, g.revision, "submit").await;
    assert_eq!(s, 201, "{receipt}");
    let unit = &receipt["unit"]["id"];
    catalog
        .down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (s, b) = vote(
        &f,
        &f.user(),
        unit,
        "approve",
        json!({"generation":1}),
        "down",
    )
    .await;
    assert_eq!(s, 503, "{b}");
    catalog
        .down
        .store(false, std::sync::atomic::Ordering::SeqCst);
    catalog.age(g.sku, Lifecycle::Retired);
    let (s, b) = vote(
        &f,
        &f.user(),
        unit,
        "approve",
        json!({"generation":1}),
        "red",
    )
    .await;
    assert_eq!(s, 409, "{b}");
    assert!(text(&b).contains("APPLY_REFUSED"), "{b}");
    assert!(text(&b).contains("ITEM_SKU_UNAVAILABLE"), "{b}");
    let stored = revision(&f, g.revision).await;
    assert_eq!(
        stored["state"], "pending",
        "the whole unit rolled back: {stored}"
    );
    assert_eq!(plan_of(&f, g.plan).await["published_rev"], json!(null));
    assert!(outbox_events(&f.dsn, PUBLISHED).await.is_empty());
    assert!(outbox_events(&f.dsn, DECIDED).await.is_empty());
}

#[tokio::test]
async fn descriptors_and_reference_columns_never_refresh_a_pending_unit() {
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    catalog.describe(g.sku, "4000");
    // The item still waits for its confirm, with a receipt: green (D-413).
    let tenant = f.ctx.subject_tenant_id();
    let conn = f.db.conn().unwrap();
    let it = items(&f, g.revision).await.remove(0);
    plan_item_repo::set_reference(
        &conn,
        &scope(&f),
        tenant,
        it.id,
        it.version,
        bss_pricing::domain::plan::ReferenceState::ConfirmationPending,
        it.reservation_id,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    policy(&f, 1).await;
    let (s, receipt) = submit(&f, &f.ctx, g.revision, "submit").await;
    assert_eq!(s, 201, "{receipt}");
    let snapshot = &receipt["unit"]["snapshot"];
    assert_eq!(snapshot["descriptors"][0]["gl_code"], "4000", "{snapshot}");
    // The attach confirms and the SKU's GL code changes while the unit is pending.
    let it = items(&f, g.revision).await.remove(0);
    plan_item_repo::set_reference(
        &conn,
        &scope(&f),
        tenant,
        it.id,
        it.version,
        bss_pricing::domain::plan::ReferenceState::Confirmed,
        it.reservation_id,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    catalog.describe(g.sku, "4100");
    let (s, b) = vote(
        &f,
        &f.user(),
        &receipt["unit"]["id"],
        "approve",
        json!({"generation":1}),
        "approve",
    )
    .await;
    assert_eq!(s, 200, "neither is content, so the unit is not stale: {b}");
    assert_eq!(b["outcome"], "applied");
    assert_eq!(b["unit"]["generation"], 1);
    assert_eq!(revision(&f, g.revision).await["state"], "published");
}

#[test]
fn plan_revision_published_is_a_typed_event_about_the_plan() {
    use bss_pricing::infra::events::{PLAN_SUBJECT_TYPE, PlanRevisionPublished};
    use event_broker_sdk::TypedEvent;
    assert_eq!(PlanRevisionPublished::TYPE_ID, PUBLISHED);
    assert_eq!(
        PLAN_SUBJECT_TYPE,
        "gts.cf.core.events.subject.v1~cf.bss.pricing.plan.v1"
    );
    assert_eq!(PlanRevisionPublished::SUBJECT_TYPE, PLAN_SUBJECT_TYPE);
    assert_eq!(PlanRevisionPublished::SOURCE, "bss-pricing");
    let plan = Uuid::new_v4();
    let event = PlanRevisionPublished {
        tenant_id: Uuid::new_v4(),
        plan_id: plan,
        revision_id: Uuid::new_v4(),
        rev_no: 2,
        book_id: Uuid::new_v4(),
        superseded_revision_id: None,
        unit_id: Uuid::new_v4(),
        actor_ref: Uuid::new_v4(),
    };
    assert_eq!(event.subject(), plan.to_string());
    assert_eq!(event.tenant_id(), Some(event.tenant_id));
    let wire = serde_json::to_value(&event).unwrap();
    let mut keys: Vec<&str> = wire
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "actorRef",
            "bookId",
            "planId",
            "revNo",
            "revisionId",
            "supersededRevisionId",
            "tenantId",
            "unitId"
        ]
    );
}

/// The red codes a refused submit answered in its `detail`.
fn refused_codes(b: &Value) -> Vec<String> {
    let detail: Value = serde_json::from_str(b["detail"].as_str().unwrap()).unwrap();
    detail
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap().to_owned())
        .collect()
}
async fn patch_revision(f: &Fixture, id: Uuid, body: Value) -> (u16, Value) {
    let path = format!("/plan-revisions/{id}");
    let (_, _, tag) = f.call("GET", &path, json!({}), None, None).await;
    let (s, b, _) = f.call("PATCH", &path, body, Some(&tag), None).await;
    (s, b)
}

/// The `plan-revision-book` definition of done (AC #15): a published revision owns its book. A
/// copy is rev 2 on a book of its own choosing while rev 1 keeps its book; rev 1 cannot be pointed
/// at another book directly; publishing rev 2 supersedes rev 1 without rewriting its binding.
#[tokio::test]
async fn a_new_revision_on_another_book_leaves_the_published_one_on_its_own() {
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 0).await;
    let (s, b) = submit(&f, &f.ctx, g.revision, "rev1").await;
    assert_eq!(s, 201, "{b}");
    let book_a = revision(&f, g.revision).await["book_id"].clone();
    let book_b = book(&f, "other").await;
    let entry_b = entry(&f, book_b, g.sku, "usage", None).await;
    approved(&f, entry_b, "2020-01-01").await;
    let (s, copy, _) = f
        .call(
            "POST",
            &format!("/plans/{}/revisions", g.plan),
            json!({}),
            None,
            Some("copy"),
        )
        .await;
    assert_eq!(s, 201, "{copy}");
    assert_eq!(copy["rev_no"], 2, "a copy takes the next revision number");
    let rev2 = id_of(&copy["id"]);
    let (s, moved) = patch_revision(&f, rev2, json!({"book_id":book_b})).await;
    assert_eq!(s, 200, "{moved}");
    assert_eq!(moved["book_id"], book_b.to_string());
    assert_eq!(
        moved["items"][0]["price_book_entry_id"],
        entry_b.to_string()
    );
    assert_eq!(revision(&f, g.revision).await["book_id"], book_a);
    let (s, refused) = patch_revision(&f, g.revision, json!({"book_id":book_b})).await;
    assert_eq!(s, 409, "a published revision is not re-bound: {refused}");
    assert!(text(&refused).contains("REVISION_NOT_DRAFT"), "{refused}");
    let (s, b) = submit(&f, &f.ctx, rev2, "rev2").await;
    assert_eq!(s, 201, "{b}");
    let r1 = revision(&f, g.revision).await;
    assert_eq!(r1["state"], "superseded");
    assert_eq!(r1["book_id"], book_a, "publishing rev 2 left rev 1's book");
    assert_eq!(r1["items"][0]["price_book_entry_id"], g.entry.to_string());
    let r2 = revision(&f, rev2).await;
    assert_eq!(
        (r2["state"].clone(), r2["book_id"].clone()),
        (json!("published"), json!(book_b.to_string()))
    );
    assert_eq!(plan_of(&f, g.plan).await["published_rev"], 2);
}

/// The `plan-item-rules` definition of done (AC #15): a second recurring period
/// (`FREQUENCY_MIXED`) or an item priced in another book (`ITEM_BOOK_FOREIGN`) blocks submit with
/// no unit; the valid set passes.
#[tokio::test]
async fn a_second_recurring_period_or_a_foreign_entry_blocks_submit_and_the_valid_set_passes() {
    let (f, catalog) = setup().await;
    let (eur, other) = (book(&f, "eur").await, book(&f, "other").await);
    let (_, rev) = plan(&f, "pro", eur).await;
    policy(&f, 0).await;
    let monthly = catalog.sku(SkuType::Recurring);
    let month = entry(&f, eur, monthly, "recurring", Some("month")).await;
    approved(&f, month, "2020-01-01").await;
    item(&f, rev, monthly, Some(month), "paid").await;
    assert_eq!(red_codes(&checks(&f, rev).await), Vec::<String>::new());
    let yearly = catalog.sku(SkuType::Recurring);
    let year = entry(&f, eur, yearly, "recurring", Some("year")).await;
    approved(&f, year, "2020-01-01").await;
    let second = item(&f, rev, yearly, Some(year), "paid").await;
    let (s, b) = submit(&f, &f.ctx, rev, "mixed").await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("REVISION_CHECKS_RED"), "{b}");
    assert_eq!(refused_codes(&b), vec!["FREQUENCY_MIXED".to_owned()], "{b}");
    assert!(units(&f).await.is_empty(), "no unit is written");
    let (s, b, _) = f
        .call(
            "DELETE",
            &format!("/plan-items/{}", second.id),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 204, "{b}");
    // The item has no entry in the other book, so it keeps its EUR entry: priced elsewhere.
    let (s, b) = patch_revision(&f, rev, json!({"book_id":other})).await;
    assert_eq!(s, 200, "{b}");
    let (s, b) = submit(&f, &f.ctx, rev, "foreign").await;
    assert_eq!(s, 400, "{b}");
    assert_eq!(
        refused_codes(&b),
        vec!["ITEM_BOOK_FOREIGN".to_owned()],
        "{b}"
    );
    assert!(units(&f).await.is_empty(), "no unit is written");
    let (s, b) = patch_revision(&f, rev, json!({"book_id":eur})).await;
    assert_eq!(s, 200, "{b}");
    let (s, b) = submit(&f, &f.ctx, rev, "valid").await;
    assert_eq!(s, 201, "the valid set passes: {b}");
    assert_eq!(b["revision"]["state"], "published", "{b}");
}

/// The `plan-revision-unit` definition of done (AC #15): an approved repricing reaches the
/// published revision through its book with no new revision, and a rejected revision is never
/// published.
#[tokio::test]
async fn an_approved_repricing_reaches_the_published_revision_and_a_rejected_one_is_not_published()
{
    let (f, catalog) = setup().await;
    let g = green(&f, &catalog, "pro").await;
    policy(&f, 0).await;
    let (s, b) = submit(&f, &f.ctx, g.revision, "rev1").await;
    assert_eq!(s, 201, "{b}");
    let published = revision(&f, g.revision).await;
    let book_id = published["book_id"].as_str().unwrap().to_owned();
    let (_, _, tag) = f
        .call("GET", "/approval-policy", json!({}), None, None)
        .await;
    let (s, b, _) = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"kind":"prices","quorum":0}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(s, 200, "{b}");
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let (s, drafted, _) = f
        .call(
            "POST",
            &format!("/price-book-entries/{}/prices", g.entry),
            json!({"model":"per_unit","price":{"rate":"0.25"},"eligibility":"all","effective_from":today}),
            None,
            Some("reprice"),
        )
        .await;
    assert_eq!(s, 201, "{drafted}");
    let price = drafted["items"][0]["id"].clone();
    let (s, receipt, _) = f
        .call(
            "POST",
            &format!("/prices/{}/submit", price.as_str().unwrap()),
            json!({}),
            None,
            Some("reprice-submit"),
        )
        .await;
    assert_eq!(s, 201, "{receipt}");
    assert_eq!(receipt["applied"], true, "{receipt}");
    // The published revision did not move: it still names the entry whose chain now carries the
    // new money from today.
    assert_eq!(revision(&f, g.revision).await, published);
    let (s, export, _) = f
        .call(
            "GET",
            &format!("/price-books/{book_id}/export"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{export}");
    let chain = export["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["entry"]["id"] == g.entry.to_string())
        .unwrap()["prices"]
        .clone();
    let repriced = chain
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == price)
        .unwrap_or_else(|| panic!("the new price is in the book: {chain}"));
    assert_eq!(repriced["state"], "approved");
    assert_eq!(repriced["effective_from"], today);
    assert_eq!(repriced["price_json"], json!({"rate":"0.25"}));
    assert_eq!(plan_of(&f, g.plan).await["published_rev"], 1);
    // A revision whose unit is rejected is never published.
    policy(&f, 1).await;
    let (s, copy, _) = f
        .call(
            "POST",
            &format!("/plans/{}/revisions", g.plan),
            json!({}),
            None,
            Some("copy"),
        )
        .await;
    assert_eq!(s, 201, "{copy}");
    let rev2 = id_of(&copy["id"]);
    let (s, receipt) = submit(&f, &f.ctx, rev2, "rev2").await;
    assert_eq!(s, 201, "{receipt}");
    assert_eq!(receipt["applied"], false);
    let (s, b) = vote(
        &f,
        &f.user(),
        &receipt["unit"]["id"],
        "reject",
        json!({"generation":1,"note":"not this one"}),
        "reject",
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let r2 = revision(&f, rev2).await;
    assert_eq!(r2["state"], "draft", "{r2}");
    for field in ["published_at", "approved_by_unit_id", "pending_unit_id"] {
        assert_eq!(r2[field], json!(null), "{field}: {r2}");
    }
    assert_eq!(revision(&f, g.revision).await["state"], "published");
    assert_eq!(plan_of(&f, g.plan).await["published_rev"], 1);
    assert_eq!(outbox_events(&f.dsn, PUBLISHED).await.len(), 1);
}
