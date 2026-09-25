//! Clone a plan through the production router (run 3.4, Task 3.4.3): a new plan whose draft rev 1
//! copies the source's PUBLISHED revision (book, sale date, items) under D-413, with no approval
//! identity, decision or pin of the source; a deprecated SKU is carried and is red in the new
//! plan's checks (D-408).
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod plan_support;
use bss_products_sdk::models::{Lifecycle, ReferenceKind, SkuType};
use plan_support::{
    Fixture, book, entry, holding, id_of, item, item_with_qty, items, ops_for, plan, publish, raw,
    request, setup, stranger, text,
};
use serde_json::{Value, json};
use uuid::Uuid;

async fn clone(f: &Fixture, source: Uuid, body: Value, key: Option<&str>) -> (u16, Value, String) {
    f.call("POST", &format!("/plans/{source}/clone"), body, None, key)
        .await
}
async fn plans(f: &Fixture) -> Vec<Value> {
    let (s, b, _) = f.call("GET", "/plans", json!({}), None, None).await;
    assert_eq!(s, 200, "{b}");
    b["items"].as_array().unwrap().clone()
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
fn row<'a>(body: &'a Value, code: &str) -> &'a Value {
    body["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["code"] == code)
        .unwrap_or_else(|| panic!("no {code} in {body}"))
}

#[tokio::test]
async fn a_clone_copies_the_published_revision_into_a_new_draft_rev_1_and_attaches_its_items() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (p, rev1) = plan(&f, "pro", eur).await;
    let source = id_of(&p["id"]);
    let (s, _, _) = f
        .call(
            "PATCH",
            &format!("/plan-revisions/{rev1}"),
            json!({"available_from":"2031-03-01"}),
            Some("\"1\""),
            None,
        )
        .await;
    assert_eq!(s, 200);
    let (seats, storage) = (catalog.sku(SkuType::Recurring), catalog.sku(SkuType::Usage));
    let seats_entry = entry(&f, eur, seats, "recurring", Some("month")).await;
    let paid = item(&f, rev1, seats, Some(seats_entry), "paid").await;
    let free = item_with_qty(&f, rev1, storage, "100").await;
    publish(&f, source, rev1).await;
    let published = revision(&f, rev1).await;
    assert!(published["approved_by_unit_id"].is_string());
    let path_body = json!({"code":"pro-2","name":"Pro 2"});
    assert_eq!(
        clone(&f, source, path_body.clone(), None).await.0,
        400,
        "an Idempotency-Key is required"
    );
    let reviewer = f.user();
    let cloned = request(
        &f.app,
        &reviewer,
        "POST",
        &format!("/plans/{source}/clone"),
        path_body.clone(),
        None,
        Some("clone"),
    )
    .await;
    assert_eq!(cloned.0, 201, "{cloned:?}");
    assert_eq!(cloned.2, "\"1\"");
    let p2 = &cloned.1;
    assert_ne!(p2["id"], p["id"]);
    assert_eq!(p2["code"], "pro-2");
    assert_eq!(p2["name"], "Pro 2");
    assert_eq!(
        p2["published_rev"],
        json!(null),
        "nothing of the new plan is published"
    );
    assert_eq!(p2["created_by"], reviewer.subject_id().to_string());
    let revisions = p2["revisions"].as_array().unwrap();
    assert_eq!(revisions.len(), 1, "{p2}");
    assert_eq!(revisions[0]["rev_no"], 1);
    assert_eq!(revisions[0]["state"], "draft");
    let rev = revision(&f, id_of(&revisions[0]["id"])).await;
    assert_eq!(rev["plan_id"], p2["id"]);
    assert_eq!(rev["book_id"], eur.to_string());
    assert_eq!(rev["available_from"], "2031-03-01");
    assert_eq!(rev["created_by"], reviewer.subject_id().to_string());
    for field in ["approved_by_unit_id", "published_at", "pending_unit_id"] {
        assert_eq!(rev[field], json!(null), "{field} is not cloned: {rev}");
    }
    let copies = rev["items"].as_array().unwrap();
    assert_eq!(copies.len(), 2, "{rev}");
    for (copy, source_item) in copies.iter().zip([&paid, &free]) {
        assert_ne!(copy["id"], source_item.id.to_string());
        assert_eq!(copy["sku_id"], source_item.sku_id.to_string());
        assert_eq!(
            copy["price_book_entry_id"],
            json!(source_item.price_book_entry_id.map(|e| e.to_string()))
        );
        assert_eq!(copy["treatment"], source_item.treatment);
        assert_eq!(copy["included_qty"], json!(source_item.included_qty));
        assert_eq!(copy["qty_min"], json!(source_item.qty_min));
        assert_eq!(copy["created_by"], reviewer.subject_id().to_string());
        // D-413: written unreserved, then attached by the door's best-effort drive.
        assert_eq!(copy["reference_state"], "confirmed", "{copy}");
        assert_ne!(copy["reservation_id"], json!(source_item.reservation_id));
        let ops = ops_for(&f, id_of(&copy["id"])).await;
        assert_eq!(ops.len(), 1, "one attach op per item");
        assert_eq!(
            (ops[0].kind.as_str(), ops[0].state.as_str()),
            ("attach", "done")
        );
    }
    assert_eq!(
        *catalog.reserve_kinds.lock().unwrap(),
        [ReferenceKind::PlanItem, ReferenceKind::PlanItem]
    );
    let replay = request(
        &f.app,
        &reviewer,
        "POST",
        &format!("/plans/{source}/clone"),
        path_body,
        None,
        Some("clone"),
    )
    .await;
    assert_eq!(replay, cloned, "the key replays the recorded answer");
    let src = revision(&f, rev1).await;
    assert_eq!(src, published, "the source is untouched");
    assert_eq!(plans(&f).await.len(), 2);
}

#[tokio::test]
async fn a_clone_needs_a_published_source_and_a_code_of_its_own() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (p, rev1) = plan(&f, "pro", eur).await;
    let source = id_of(&p["id"]);
    item(&f, rev1, catalog.sku(SkuType::Usage), None, "included").await;
    let (s, b, _) = clone(&f, source, json!({"code":"x","name":"X"}), Some("draft")).await;
    assert_eq!(s, 409, "a draft is not a published source: {b}");
    assert!(text(&b).contains("CLONE_SOURCE_UNPUBLISHED"), "{b}");
    publish(&f, source, rev1).await;
    let (s, b, _) = clone(&f, source, json!({"code":"  ","name":"X"}), Some("blank")).await;
    assert_eq!(s, 400, "{b}");
    assert!(text(&b).contains("PLAN_CODE_REQUIRED"), "{b}");
    let (s, b, _) = clone(&f, source, json!({"code":"pro","name":"X"}), Some("taken")).await;
    assert_eq!(s, 409, "{b}");
    assert!(text(&b).contains("PLAN_CODE_TAKEN"), "{b}");
    let (s, b, _) = clone(
        &f,
        Uuid::new_v4(),
        json!({"code":"y","name":"Y"}),
        Some("unknown"),
    )
    .await;
    assert_eq!(s, 404, "{b}");
    let (s, b, _) = clone(&f, source, json!({"code":"z"}), Some("no-name")).await;
    assert_eq!(s, 400, "a clone names its plan: {b}");
    let (s, b, _) = clone(
        &f,
        source,
        json!({"code":"z","name":"Z","book_id":eur}),
        Some("extra"),
    )
    .await;
    assert_eq!(s, 400, "a clone takes code and name only: {b}");
    assert_eq!(plans(&f).await.len(), 1, "no refused clone wrote a plan");
    let (s, b, _) = clone(&f, source, json!({"code":"x","name":"X"}), Some("draft")).await;
    assert_eq!(s, 201, "a refused key was never claimed: {b}");
}

#[tokio::test]
async fn a_clone_carries_a_deprecated_sku_and_the_new_plans_checks_show_it_red() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (p, rev1) = plan(&f, "pro", eur).await;
    let source = id_of(&p["id"]);
    let legacy = catalog.sku(SkuType::Usage);
    item(&f, rev1, legacy, None, "included").await;
    publish(&f, source, rev1).await;
    catalog.age(legacy, Lifecycle::Deprecated);
    let (s, cloned, _) = clone(
        &f,
        source,
        json!({"code":"pro-2","name":"Pro 2"}),
        Some("c"),
    )
    .await;
    assert_eq!(s, 201, "a deprecated SKU is carried, not refused: {cloned}");
    let rev = id_of(&cloned["revisions"][0]["id"]);
    assert_eq!(
        items(&f, rev).await[0].reference_state,
        "confirmed",
        "Products admits a deprecated SKU for an attach"
    );
    let (s, checks, _) = f
        .call(
            "GET",
            &format!("/plan-revisions/{rev}/checks"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{checks}");
    assert_eq!(
        row(&checks, "ITEM_SKU_DEPRECATED")["ok"],
        false,
        "a new plan never carries a deprecated SKU over: {checks}"
    );
    // The source's own copy carries it over green (D-408).
    let (s, copy, _) = f
        .call(
            "POST",
            &format!("/plans/{source}/revisions"),
            json!({}),
            None,
            Some("copy"),
        )
        .await;
    assert_eq!(s, 201, "{copy}");
    let (_, same_plan, _) = f
        .call(
            "GET",
            &format!("/plan-revisions/{}/checks", copy["id"].as_str().unwrap()),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(
        row(&same_plan, "ITEM_SKU_DEPRECATED")["ok"],
        true,
        "{same_plan}"
    );
}

// D-413: the clone writes its plan, its revision, every item and every attach op in ONE
// transaction. A trigger refuses the second copied item: nothing of the clone survives.
#[tokio::test]
async fn a_clone_writes_the_plan_the_revision_and_every_item_in_one_transaction() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (p, rev1) = plan(&f, "pro", eur).await;
    let source = id_of(&p["id"]);
    item(&f, rev1, catalog.sku(SkuType::Usage), None, "included").await;
    item(&f, rev1, catalog.sku(SkuType::Usage), None, "included").await;
    publish(&f, source, rev1).await;
    raw(
        &f,
        &format!(
            "CREATE TRIGGER second_copy BEFORE INSERT ON pricing_plan_item \
             WHEN NEW.revision_id != '{rev1}' AND (SELECT count(*) FROM pricing_plan_item \
             WHERE revision_id = NEW.revision_id) >= 1 BEGIN SELECT RAISE(ABORT, 'probe'); END"
        ),
    )
    .await;
    let body = json!({"code":"pro-2","name":"Pro 2"});
    let (s, b, _) = clone(&f, source, body.clone(), Some("clone")).await;
    assert_eq!(s, 500, "{b}");
    assert_eq!(
        plans(&f).await.len(),
        1,
        "no plan of the failed clone survives"
    );
    let (_, all_ops, _) = f.call("GET", "/reference-ops", json!({}), None, None).await;
    assert_eq!(all_ops["items"], json!([]), "no attach op survives");
    assert_eq!(catalog.reserves(), 0);
    raw(&f, "DROP TRIGGER second_copy").await;
    let (s, b, _) = clone(&f, source, body, Some("clone")).await;
    assert_eq!(s, 201, "the key was never claimed: {b}");
}

#[tokio::test]
async fn the_clone_door_needs_plan_author() {
    let (f, catalog) = setup().await;
    let eur = book(&f, "eur").await;
    let (p, rev1) = plan(&f, "pro", eur).await;
    let source = id_of(&p["id"]);
    item(&f, rev1, catalog.sku(SkuType::Usage), None, "included").await;
    publish(&f, source, rev1).await;
    let path = format!("/plans/{source}/clone");
    for (who, status) in [
        (holding(&f, "plan:read"), 403),
        (stranger(), 403),
        (holding(&f, "plan:author"), 201),
    ] {
        let (s, b, _) = request(
            &f.app,
            &who,
            "POST",
            &path,
            json!({"code":format!("c-{status}-{}", Uuid::new_v4()),"name":"C"}),
            None,
            Some("k"),
        )
        .await;
        assert_eq!(s, status, "{b}");
    }
}
