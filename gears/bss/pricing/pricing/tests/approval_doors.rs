//! Submission, publish changes and the approval doors, mirroring products' governance suite.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod entry_support;
use bss_pricing::infra::storage::{
    entity::price,
    repo::{price_book_entry_repo, price_repo},
};
use entry_support::{Fixture, Script};
use sea_orm::EntityTrait;
use serde_json::{Value, json};
use std::sync::Arc;
use toolkit_db::secure::{AccessScope, SecureUpdateExt};
use toolkit_security::SecurityContext;
use uuid::Uuid;

struct Gov {
    f: Fixture,
    book: String,
    entry: Value,
}
async fn gov(quorum: u32) -> Gov {
    gov_on(quorum, Arc::new(Script::default())).await
}
/// A book with one entry over a scripted registry the test keeps: a usage SKU in the script's
/// default mode, a monthly recurring one in mode 11.
async fn gov_on(quorum: u32, script: Arc<Script>) -> Gov {
    let recurring = Script::count(&script.mode) == 11;
    let f = Fixture::new(script).await;
    let (book, _) = f.book().await;
    let book = book["id"].as_str().unwrap().to_owned();
    let entry = if recurring {
        json!({"sku_id":Uuid::new_v4(),"period":"month"})
    } else {
        json!({"sku_id":Uuid::new_v4()})
    };
    let (status, entry, _) = f
        .call(
            "POST",
            &format!("/price-books/{book}/entries"),
            entry,
            None,
            Some("entry"),
        )
        .await;
    assert_eq!(status, 201, "{entry}");
    let g = Gov { f, book, entry };
    g.policy(None, quorum).await;
    g
}
impl Gov {
    async fn policy(&self, kind: Option<&str>, quorum: u32) -> (u16, Value, String) {
        let (_, _, tag) = self
            .f
            .call("GET", "/approval-policy", json!({}), None, None)
            .await;
        let body = match kind {
            Some(kind) => json!({"kind":kind,"quorum":quorum}),
            None => json!({"quorum":quorum}),
        };
        let put = self
            .f
            .call("PUT", "/approval-policy", body, Some(&tag), None)
            .await;
        assert_eq!(put.0, 200, "{put:?}");
        put
    }
    fn price_book_entry_id(&self) -> Uuid {
        self.entry["id"].as_str().unwrap().parse().unwrap()
    }
    async fn draft_as(&self, who: &SecurityContext, key: &str, body: Value) -> Vec<Value> {
        let (status, b, _) = self
            .f
            .call_as(
                who,
                "POST",
                &format!("/price-book-entries/{}/prices", self.price_book_entry_id()),
                body,
                None,
                Some(key),
            )
            .await;
        assert_eq!(status, 201, "{b}");
        b["items"].as_array().unwrap().clone()
    }
    async fn draft(&self, key: &str, body: Value) -> Vec<Value> {
        self.draft_as(&self.f.ctx, key, body).await
    }
    async fn submit_as(
        &self,
        who: &SecurityContext,
        price: &Value,
        key: &str,
    ) -> (u16, Value, String) {
        self.f
            .call_as(
                who,
                "POST",
                &format!("/prices/{}/submit", price["id"].as_str().unwrap()),
                json!({}),
                None,
                Some(key),
            )
            .await
    }
    async fn vote(
        &self,
        who: &SecurityContext,
        unit: &Value,
        action: &str,
        body: Value,
        key: &str,
    ) -> (u16, Value, String) {
        self.f
            .call_as(
                who,
                "POST",
                &format!("/approval-units/{}/{action}", unit["id"].as_str().unwrap()),
                body,
                None,
                Some(key),
            )
            .await
    }
    async fn card(&self, unit: &Value) -> Value {
        let (status, b, _) = self
            .f
            .call(
                "GET",
                &format!("/approval-units/{}", unit["id"].as_str().unwrap()),
                json!({}),
                None,
                None,
            )
            .await;
        assert_eq!(status, 200, "{b}");
        b
    }
    async fn price(&self, id: &Value) -> price::Model {
        let tenant = self.f.ctx.subject_tenant_id();
        price_repo::find(
            &self.f.db.conn().unwrap(),
            &AccessScope::for_tenant(tenant),
            tenant,
            id.as_str().unwrap().parse().unwrap(),
        )
        .await
        .unwrap()
        .unwrap()
    }
    async fn approved(&self, version_no: i32, from: &str) -> Uuid {
        let tenant = self.f.ctx.subject_tenant_id();
        let scope = AccessScope::for_tenant(tenant);
        let conn = self.f.db.conn().unwrap();
        let p = price_book_entry_repo::find(&conn, &scope, tenant, self.price_book_entry_id())
            .await
            .unwrap()
            .unwrap();
        let mut r = entry_support::price(&p);
        r.version_no = version_no;
        r.state = "approved".into();
        r.price_json = json!({"rate":"0.20"});
        r.effective_from =
            time::Date::parse(from, &time::format_description::well_known::Iso8601::DATE).unwrap();
        price_repo::insert(&conn, &scope, r).await.unwrap().id
    }
}
fn body(from: &str) -> Value {
    json!({"model":"per_unit","price":{"rate":"0.10"},"eligibility":"all","effective_from":from})
}
fn code(b: &Value) -> String {
    b.to_string()
}

#[tokio::test]
async fn quorum_zero_applies_at_submit_and_records_the_unit() {
    let g = gov(0).await;
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let (status, receipt, _) = g.submit_as(&g.f.ctx, price, "submit").await;
    assert_eq!(status, 201, "{receipt}");
    assert_eq!(receipt["applied"], true);
    assert_eq!(receipt["unit"]["state"], "approved");
    assert_eq!(receipt["unit"]["kind"], "prices");
    assert_eq!(receipt["unit"]["ref_id"], g.book);
    assert_eq!(receipt["unit"]["decisions"], json!([]));
    assert!(receipt["unit"]["decided_at"].is_string());
    assert_eq!(receipt["prices"][0]["state"], "approved");
    assert_eq!(
        receipt["prices"][0]["approved_by_unit_id"],
        receipt["unit"]["id"]
    );
    for (query, count) in [
        ("?state=approved".to_owned(), 1),
        ("?state=pending".to_owned(), 0),
        (format!("?kind=prices&book_id={}", g.book), 1),
        (format!("?ref_id={}", Uuid::new_v4()), 0),
    ] {
        let (status, list, _) =
            g.f.call(
                "GET",
                &format!("/approval-units{query}"),
                json!({}),
                None,
                None,
            )
            .await;
        assert_eq!(status, 200, "{list}");
        assert_eq!(list["items"].as_array().unwrap().len(), count, "{query}");
    }
    assert_eq!(
        g.f.call("GET", "/approval-units?state=bogus", json!({}), None, None)
            .await
            .0,
        400
    );
}

#[tokio::test]
async fn quorum_one_neither_the_author_nor_the_submitter_may_approve() {
    let g = gov(1).await;
    let author = g.f.ctx.clone();
    let submitter = g.f.user();
    let reviewer = g.f.user();
    let price = &g.draft_as(&author, "a", body("2031-03-01")).await[0];
    let (status, receipt, _) = g.submit_as(&submitter, price, "submit").await;
    assert_eq!(status, 201, "{receipt}");
    assert_eq!(receipt["applied"], false);
    let unit = &receipt["unit"];
    for (who, key) in [(&author, "by-author"), (&submitter, "by-submitter")] {
        let (status, b, _) = g
            .vote(who, unit, "approve", json!({"generation":1}), key)
            .await;
        assert_eq!(status, 403, "{b}");
        assert!(code(&b).contains("SOD_VIOLATION"), "{b}");
    }
    let locked =
        g.f.call(
            "PATCH",
            &format!("/prices/{}", price["id"].as_str().unwrap()),
            json!({"note":"x"}),
            Some("\"1\""),
            None,
        )
        .await;
    assert_eq!(locked.0, 409);
    assert!(code(&locked.1).contains("PRICE_NOT_DRAFT"), "{locked:?}");
    let (status, b, _) = g
        .vote(
            &reviewer,
            unit,
            "approve",
            json!({"generation":1,"note":"ok"}),
            "ok",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["outcome"], "applied");
    assert_eq!(b["unit"]["state"], "approved");
    let stored = g.price(&price["id"]).await;
    assert_eq!(stored.state, "approved");
    assert_eq!(
        stored.approved_by_unit_id.map(|u| u.to_string()),
        unit["id"].as_str().map(str::to_owned)
    );
}

// Chains MEDIUM-1 = docs F2 (D-404): Bob, who may author and approve, cannot rewrite Alice's
// draft and then approve his own numbers under her name.
#[tokio::test]
async fn only_a_drafts_author_edits_or_deletes_it() {
    let g = gov(1).await;
    let (alice, bob) = (g.f.ctx.clone(), g.f.user());
    let mut ten = body("2031-03-01");
    ten["price"] = json!({"rate":"10"});
    let price = &g.draft_as(&alice, "a", ten).await[0];
    let path = format!("/prices/{}", price["id"].as_str().unwrap());
    let (status, b, _) =
        g.f.call_as(
            &bob,
            "PATCH",
            &path,
            json!({"price":{"rate":"1"}}),
            Some("\"1\""),
            None,
        )
        .await;
    assert_eq!(status, 403, "{b}");
    assert!(code(&b).contains("NOT_DRAFT_AUTHOR"), "{b}");
    assert_eq!(g.price(&price["id"]).await.price_json, json!({"rate":"10"}));
    let (status, b, _) =
        g.f.call_as(&bob, "DELETE", &path, json!({}), Some("\"1\""), None)
            .await;
    assert_eq!(status, 403, "{b}");
    assert!(code(&b).contains("NOT_DRAFT_AUTHOR"), "{b}");
    let mut promo = body("2031-04-01");
    promo["temporary_until"] = json!("2031-04-10");
    g.approved(9, "2031-01-01").await;
    let pair = g.draft_as(&alice, "pair", promo).await;
    let partner = format!("/prices/{}", pair[1]["id"].as_str().unwrap());
    let (status, b, _) =
        g.f.call_as(&bob, "DELETE", &partner, json!({}), Some("\"2\""), None)
            .await;
    assert_eq!(status, 403, "a pair's partner follows its creator: {b}");
    let (status, b, _) =
        g.f.call_as(
            &alice,
            "PATCH",
            &path,
            json!({"price":{"rate":"9"}}),
            Some("\"1\""),
            None,
        )
        .await;
    assert_eq!(status, 200, "the author still edits: {b}");
    let (status, b, _) = g.submit_as(&alice, price, "submit").await;
    assert_eq!(status, 201, "{b}");
    let (status, b, _) = g
        .vote(&bob, &b["unit"], "approve", json!({"generation":1}), "bob")
        .await;
    assert_eq!(status, 200, "Bob reviews money he did not write: {b}");
}

#[tokio::test]
async fn quorum_two_needs_two_reviewers_and_a_second_vote_by_one_is_refused() {
    let g = gov(2).await;
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let (_, receipt, _) = g.submit_as(&g.f.ctx, price, "submit").await;
    let unit = &receipt["unit"];
    let (one, two) = (g.f.user(), g.f.user());
    let (status, b, _) = g
        .vote(&one, unit, "approve", json!({"generation":1}), "1")
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(
        (b["outcome"].clone(), b["have"].clone(), b["need"].clone()),
        (json!("pending"), json!(1), json!(2))
    );
    let (status, b, _) = g
        .vote(&one, unit, "approve", json!({"generation":1}), "1-again")
        .await;
    assert_eq!(status, 409, "{b}");
    assert!(code(&b).contains("DUPLICATE_VOTE"), "{b}");
    let (status, b, _) = g
        .vote(&two, unit, "approve", json!({"generation":1}), "2")
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["outcome"], "applied");
    assert_eq!(b["unit"]["decisions"].as_array().unwrap().len(), 2);
    let (status, b, _) = g
        .vote(&g.f.user(), unit, "approve", json!({"generation":1}), "3")
        .await;
    assert_eq!(status, 409);
    assert!(code(&b).contains("UNIT_ALREADY_DECIDED"), "{b}");
}

#[tokio::test]
async fn content_drift_refreshes_the_generation_and_earlier_votes_go_stale() {
    let g = gov(2).await;
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let (_, receipt, _) = g.submit_as(&g.f.ctx, price, "submit").await;
    let unit = &receipt["unit"];
    let (one, two) = (g.f.user(), g.f.user());
    assert_eq!(
        g.vote(&one, unit, "approve", json!({"generation":1}), "1")
            .await
            .0,
        200
    );
    // The pending price's money changes underneath the unit.
    let tenant = g.f.ctx.subject_tenant_id();
    price::Entity::update_many()
        .secure()
        .scope_with(&AccessScope::for_tenant(tenant))
        .col_expr(
            price::Column::PriceJson,
            sea_orm::sea_query::Expr::value(json!({"rate":"0.11"})),
        )
        .filter(sea_orm::Condition::all().add(sea_orm::ColumnTrait::eq(
            &price::Column::Id,
            price["id"].as_str().unwrap().parse::<Uuid>().unwrap(),
        )))
        .exec(&g.f.db.conn().unwrap())
        .await
        .unwrap();
    let (status, b, _) = g
        .vote(&two, unit, "approve", json!({"generation":1}), "2")
        .await;
    assert_eq!(status, 400, "{b}");
    assert!(code(&b).contains("UNIT_STALE"), "{b}");
    assert_eq!(b["context"]["generation"], 2);
    let card = g.card(unit).await;
    assert_eq!(card["generation"], 2);
    assert_eq!(card["state"], "pending");
    assert_eq!(card["decisions"][0]["stale"], true);
    assert_eq!(
        card["snapshot"]["prices"][0]["after"]["price"],
        json!({"rate":"0.11"})
    );
    let (status, b, _) = g
        .vote(&one, unit, "approve", json!({"generation":1}), "1b")
        .await;
    assert_eq!(status, 400);
    assert!(code(&b).contains("GENERATION_MISMATCH"), "{b}");
    assert_eq!(b["context"]["generation"], 2);
    let (status, b, _) = g
        .vote(&one, unit, "approve", json!({"generation":2}), "1c")
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(
        b["outcome"], "pending",
        "the first reviewer votes again on what they now see"
    );
    let (status, b, _) = g
        .vote(&two, unit, "approve", json!({"generation":2}), "2b")
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["outcome"], "applied");
}

#[tokio::test]
async fn a_vote_must_name_the_generation_it_saw() {
    let g = gov(2).await;
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let (_, receipt, _) = g.submit_as(&g.f.ctx, price, "submit").await;
    let unit = &receipt["unit"];
    let reviewer = g.f.user();
    assert_eq!(
        g.vote(&reviewer, unit, "approve", json!({}), "none")
            .await
            .0,
        400
    );
    for action in ["approve", "reject"] {
        let (status, b, _) = g
            .vote(
                &reviewer,
                unit,
                action,
                json!({"generation":0,"note":"n"}),
                action,
            )
            .await;
        assert_eq!(status, 400, "{b}");
        assert!(code(&b).contains("GENERATION_MISMATCH"), "{b}");
        assert_eq!(b["context"]["generation"], 1);
    }
    assert_eq!(
        g.vote(&reviewer, unit, "approve", json!({"generation":1}), "ok")
            .await
            .0,
        200
    );
}

#[tokio::test]
async fn reject_needs_a_note_keeps_prices_rejected_and_withdraw_is_the_submitters() {
    let g = gov(1).await;
    let reviewer = g.f.user();
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let (_, receipt, _) = g.submit_as(&g.f.ctx, price, "submit").await;
    let unit = &receipt["unit"];
    let (status, b, _) = g
        .vote(
            &reviewer,
            unit,
            "reject",
            json!({"generation":1}),
            "no-note",
        )
        .await;
    assert_eq!(status, 400, "{b}");
    assert!(code(&b).contains("NOTE_REQUIRED"), "{b}");
    let (status, b, _) = g
        .vote(&reviewer, unit, "withdraw", json!({}), "not-mine")
        .await;
    assert_eq!(status, 403, "{b}");
    assert!(code(&b).contains("NOT_SUBMITTER"), "{b}");
    let (status, b, _) = g
        .vote(
            &reviewer,
            unit,
            "reject",
            json!({"generation":1,"note":"too cheap"}),
            "reject",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["outcome"], "rejected");
    assert_eq!(b["unit"]["decided_note"], "too cheap");
    let rejected = g.price(&price["id"]).await;
    assert_eq!(
        rejected.state, "rejected",
        "a rejected price keeps its history"
    );
    assert!(rejected.pending_unit_id.is_none());
    let other = &g.draft("b", body("2031-04-01")).await[0];
    let (_, receipt, _) = g.submit_as(&g.f.ctx, other, "submit-b").await;
    let (status, b, _) = g
        .vote(
            &g.f.ctx,
            &receipt["unit"],
            "withdraw",
            json!({}),
            "withdraw",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["outcome"], "withdrawn");
    assert_eq!(g.price(&other["id"]).await.state, "draft");
}

#[tokio::test]
async fn two_reviewers_approving_at_once_apply_exactly_once() {
    let g = gov(1).await;
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let (_, receipt, _) = g.submit_as(&g.f.ctx, price, "submit").await;
    let unit = receipt["unit"]["id"].as_str().unwrap().to_owned();
    let second = g.f.second_app().await;
    let (one, two) = (g.f.user(), g.f.user());
    let path = format!("/approval-units/{unit}/approve");
    let (a, b) = tokio::join!(
        g.f.call_as(
            &one,
            "POST",
            &path,
            json!({"generation":1}),
            None,
            Some("one")
        ),
        entry_support::request(
            &second,
            &two,
            "POST",
            &path,
            json!({"generation":1}),
            None,
            Some("two")
        )
    );
    let (winner, loser) = if a.0 == 200 { (a, b) } else { (b, a) };
    assert_eq!(winner.0, 200, "{winner:?}");
    assert_eq!(winner.1["outcome"], "applied");
    assert_eq!(loser.0, 409, "{loser:?}");
    assert!(
        code(&loser.1).contains("UNIT_ALREADY_DECIDED")
            || code(&loser.1).contains("UNIT_CONTENDED"),
        "{loser:?}"
    );
    assert_eq!(
        g.price(&price["id"]).await.version,
        4,
        "locked, approved, unlocked: once"
    );
}

#[tokio::test]
async fn publish_changes_lists_every_draft_and_pulls_a_pair_partner_in() {
    let g = gov(1).await;
    let back = g.approved(1, "2031-01-01").await;
    let single = g.draft("single", body("2031-05-01")).await;
    let mut promo = body("2031-03-01");
    promo["temporary_until"] = json!("2031-03-11");
    let pair = g.draft("pair", promo).await;
    let path = format!("/price-books/{}/publish-changes", g.book);
    let (status, listing, _) = g.f.call("GET", &path, json!({}), None, None).await;
    assert_eq!(status, 200, "{listing}");
    let prices = listing["prices"].as_array().unwrap();
    assert_eq!(prices.len(), 3);
    assert_eq!(
        prices
            .iter()
            .map(|r| r["price"]["effective_from"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("2031-03-01"),
            json!("2031-03-11"),
            json!("2031-05-01")
        ],
        "sorted by start, entry and version"
    );
    for r in prices {
        assert_eq!(r["selected"], true);
        assert_eq!(r["chain"], "default");
        assert_eq!(r["entry"]["sku_id"], g.entry["sku_id"]);
        assert_eq!(
            r["before"]["id"],
            back.to_string(),
            "the approved predecessor"
        );
    }
    assert_eq!(prices[0]["pair_partner_id"], pair[1]["id"]);
    let (status, b, _) = g.submit_as(&g.f.ctx, &pair[1], "half").await;
    assert_eq!(status, 400, "{b}");
    assert!(code(&b).contains("PAIR_SPLIT"), "{b}");
    let (status, receipt, _) =
        g.f.call(
            "POST",
            &path,
            json!({"price_ids":[pair[0]["id"]]}),
            None,
            Some("pub"),
        )
        .await;
    assert_eq!(status, 201, "{receipt}");
    let snapshot = &receipt["unit"]["snapshot"];
    assert_eq!(snapshot["prices"].as_array().unwrap().len(), 2);
    assert_eq!(snapshot["added_partner"], json!([pair[1]["id"]]));
    assert_eq!(
        g.price(&single[0]["id"]).await.state,
        "draft",
        "unticked prices stay drafts"
    );
    let (status, b, _) =
        g.f.call(
            "POST",
            &path,
            json!({"price_ids":[Uuid::new_v4()]}),
            None,
            Some("foreign"),
        )
        .await;
    assert_eq!(status, 400, "{b}");
    assert!(code(&b).contains("PRICE_NOT_IN_BOOK"), "{b}");
    let (status, b, _) =
        g.f.call(
            "POST",
            &path,
            json!({"price_ids":[pair[0]["id"]]}),
            None,
            Some("again"),
        )
        .await;
    assert_eq!(status, 409, "{b}");
    assert!(code(&b).contains("PRICE_NOT_DRAFT"), "{b}");
}

#[tokio::test]
async fn publish_all_with_a_common_date_and_the_card_shows_live_impact() {
    let g = gov(1).await;
    g.approved(1, "2031-01-01").await;
    let first = g.draft("one", body("2031-05-01")).await;
    let mut promo = body("2031-03-01");
    promo["temporary_until"] = json!("2031-03-11");
    let pair = g.draft("pair", promo).await;
    let path = format!("/price-books/{}/publish-changes", g.book);
    let (status, b, _) =
        g.f.call(
            "POST",
            &path,
            json!({"common_effective_date":"2031-04-01"}),
            None,
            Some("all"),
        )
        .await;
    assert_eq!(
        status, 400,
        "a single price and a promo both moved to one start overlap: {b}"
    );
    assert!(code(&b).contains("WINDOW_OVERLAP"), "{b}");
    let (status, receipt, _) =
        g.f.call(
            "POST",
            &path,
            json!({"price_ids":[pair[0]["id"]],"common_effective_date":"2031-04-01"}),
            None,
            Some("pair"),
        )
        .await;
    assert_eq!(status, 201, "{receipt}");
    let unit = &receipt["unit"];
    assert_eq!(unit["common_effective_date"], "2031-04-01");
    let card = g.card(unit).await;
    assert_eq!(
        card["impact"],
        json!({"prices":2,"entries":1,"plans":[],"subscriptions":"unavailable until the Subscriptions integration"})
    );
    let (status, b, _) = g
        .vote(&g.f.user(), unit, "approve", json!({"generation":1}), "ok")
        .await;
    assert_eq!(status, 200, "{b}");
    let (p, r) = (g.price(&pair[0]["id"]).await, g.price(&pair[1]["id"]).await);
    assert_eq!(p.effective_from.to_string(), "2031-04-01");
    assert_eq!(
        p.temporary_until.map(|d| d.to_string()),
        Some("2031-04-11".into())
    );
    assert_eq!(
        r.effective_from.to_string(),
        "2031-04-11",
        "the pair keeps its length"
    );
    let (status, rest, _) = g.f.call("POST", &path, json!({}), None, Some("rest")).await;
    assert_eq!(status, 201, "{rest}");
    assert_eq!(
        rest["unit"]["snapshot"]["prices"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        rest["unit"]["snapshot"]["prices"][0]["price_id"],
        first[0]["id"]
    );
    let (status, b, _) = g.f.call("POST", &path, json!({}), None, Some("none")).await;
    assert_eq!(status, 400, "{b}");
    assert!(code(&b).contains("NO_DRAFT_PRICES"), "{b}");
}

#[tokio::test]
async fn a_changed_usage_structure_is_refused_at_submit_with_400() {
    let g = gov(1).await;
    g.approved(1, "2031-01-01").await;
    let mut graduated = body("2031-03-01");
    graduated["model"] = json!("graduated");
    graduated["price"] = json!({"tiers":[{"up_to":null,"rate":"1"}]});
    let price = &g.draft("g", graduated).await[0];
    let (status, b, _) = g.submit_as(&g.f.ctx, price, "submit").await;
    assert_eq!(status, 400, "D-403: {b}");
    assert!(code(&b).contains("CHAIN_MODEL_CHANGED"), "{b}");
    assert_eq!(
        g.price(&price["id"]).await.state,
        "draft",
        "no unit was created"
    );
    let (_, list, _) =
        g.f.call("GET", "/approval-units", json!({}), None, None)
            .await;
    assert_eq!(list["items"], json!([]));
}

#[tokio::test]
async fn keyed_submissions_and_votes_replay_their_answer() {
    let g = gov(1).await;
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let first = g.submit_as(&g.f.ctx, price, "submit").await;
    assert_eq!(first.0, 201);
    assert_eq!(g.submit_as(&g.f.ctx, price, "submit").await, first);
    let reviewer = g.f.user();
    let unit = &first.1["unit"];
    let approve = g
        .vote(&reviewer, unit, "approve", json!({"generation":1}), "v")
        .await;
    assert_eq!(approve.0, 200);
    assert_eq!(
        g.vote(&reviewer, unit, "approve", json!({"generation":1}), "v")
            .await,
        approve,
        "a replay after success answers the stored receipt"
    );
    let (status, b, _) = g
        .vote(
            &reviewer,
            unit,
            "approve",
            json!({"generation":1,"note":"x"}),
            "v",
        )
        .await;
    assert_eq!(status, 409);
    assert!(code(&b).contains("IDEMPOTENCY_CONFLICT"), "{b}");
    assert_eq!(
        g.f.call(
            "POST",
            &format!("/prices/{}/submit", price["id"].as_str().unwrap()),
            json!({}),
            None,
            None
        )
        .await
        .0,
        400,
        "Idempotency-Key is required"
    );
}

#[tokio::test]
async fn the_approval_policy_is_read_and_written_under_if_match() {
    let f = Fixture::new(Arc::new(Script::default())).await;
    let (status, policy, tag) = f
        .call("GET", "/approval-policy", json!({}), None, None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        policy,
        json!({"default_quorum":1,"overrides":{}}),
        "fail-safe one"
    );
    assert!(!tag.is_empty());
    assert_eq!(
        f.call("PUT", "/approval-policy", json!({"quorum":0}), None, None)
            .await
            .0,
        400,
        "If-Match is required"
    );
    let saved = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"quorum":0}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(saved.0, 200, "{saved:?}");
    assert_eq!(saved.1["default_quorum"], 0);
    let stale = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"quorum":2}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(stale.0, 409);
    assert!(code(&stale.1).contains("STALE_REVISION"), "{stale:?}");
    for (bad, what) in [
        // `plan_revision` is a kind since run 3.4 (tests/approval_kinds.rs); promotions are
        // deferred (D-409), so their kind is still refused.
        (
            json!({"kind":"promotion","quorum":1}),
            "POLICY_KIND_INVALID",
        ),
        (json!({"quorum":3_000_000_000_u64}), "QUORUM_INVALID"),
    ] {
        let refused = f
            .call("PUT", "/approval-policy", bad, Some(&saved.2), None)
            .await;
        assert_eq!(refused.0, 400, "{refused:?}");
        assert!(code(&refused.1).contains(what), "{refused:?}");
    }
    let kind = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"kind":"prices","quorum":2}),
            Some(&saved.2),
            None,
        )
        .await;
    assert_eq!(kind.0, 200, "{kind:?}");
    assert_eq!(kind.1, json!({"default_quorum":0,"overrides":{"prices":2}}));
    assert_eq!(
        f.call("GET", "/approval-policy", json!({}), None, None)
            .await
            .1,
        kind.1
    );
}

/// A reviewer who holds `approval_unit:approve` and nothing else: no products `read`.
fn approve_only(g: &Gov) -> SecurityContext {
    SecurityContext::builder()
        .subject_id(Uuid::new_v4())
        .subject_tenant_id(g.f.ctx.subject_tenant_id())
        .subject_type("approval_unit:approve")
        .build()
        .unwrap()
}

// D-416: the registry double is armed, so Products refuses every SKU read of a caller without
// products `read`. A recurring unit's rules read no SKU: the approve-only reviewer's votes pass,
// and the descriptors its reads cannot see are information only.
#[tokio::test]
async fn a_units_quorum_is_its_own_snapshot_and_an_approve_only_reviewer_may_vote() {
    let script = Arc::new(Script::default());
    script.set(11);
    let g = gov_on(2, script.clone()).await;
    script.readers([g.f.ctx.subject_id()]);
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let (_, receipt, _) = g.submit_as(&g.f.ctx, price, "submit").await;
    let unit = &receipt["unit"];
    assert_eq!(unit["quorum_required"], 2);
    assert_eq!(
        g.card(unit).await["snapshot"]["descriptors"]
            .as_array()
            .map(Vec::len),
        Some(1),
        "the submitter holds products read: the entry SKU's descriptors"
    );
    g.policy(None, 0).await;
    let (status, b, _) = g
        .vote(
            &approve_only(&g),
            unit,
            "approve",
            json!({"generation":1}),
            "1",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(
        (b["outcome"].clone(), b["have"].clone(), b["need"].clone()),
        (json!("pending"), json!(1), json!(2)),
        "a lowered policy does not lower a submitted unit"
    );
    let (status, b, _) = g
        .vote(
            &approve_only(&g),
            unit,
            "approve",
            json!({"generation":1}),
            "2",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["outcome"], "applied");
}

// D-416: a price submitter without products `read` is not refused either; the snapshot says the
// descriptors are unavailable instead of naming them.
#[tokio::test]
async fn a_submitter_without_products_read_records_the_descriptors_unavailable() {
    let script = Arc::new(Script::default());
    script.set(11);
    let g = gov_on(1, script.clone()).await;
    script.readers([g.f.ctx.subject_id()]);
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let submitter = g.f.user();
    let (status, receipt, _) = g.submit_as(&submitter, price, "submit").await;
    assert_eq!(status, 201, "{receipt}");
    assert_eq!(
        g.card(&receipt["unit"]).await["snapshot"]["descriptors"],
        "unavailable"
    );
    let (status, b, _) = g
        .vote(
            &approve_only(&g),
            &receipt["unit"],
            "approve",
            json!({"generation":1}),
            "1",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["outcome"], "applied");
}

// D-416 / D-402: a usage chain's dated metering feeds the pair guard, a rule, so the final vote
// reads it as the voter and answers Products' own 403 (never 400); a reject applies no rule and
// passes; a non-final vote reads only descriptors and passes.
#[tokio::test]
async fn an_approve_only_reviewer_is_refused_only_where_a_rule_reads_the_sku() {
    let script = Arc::new(Script::default());
    let g = gov_on(1, script.clone()).await;
    script.readers([g.f.ctx.subject_id()]);
    g.approved(1, "2031-01-01").await;
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let (status, receipt, _) = g.submit_as(&g.f.ctx, price, "submit").await;
    assert_eq!(status, 201, "{receipt}");
    let unit = &receipt["unit"];
    let reviewer = approve_only(&g);
    let reads = Script::count(&script.version_reads);
    let (status, b, _) = g
        .vote(&reviewer, unit, "approve", json!({"generation":1}), "1")
        .await;
    assert_eq!(status, 403, "{b}");
    assert!(
        code(&b).contains("SKU_READ_DENIED"),
        "Products' own code: {b}"
    );
    assert!(
        Script::count(&script.version_reads) > reads,
        "the refused read is the dated metering of the pair guard"
    );
    assert_eq!(g.price(&price["id"]).await.state, "pending");
    let (status, b, _) = g
        .vote(
            &reviewer,
            unit,
            "reject",
            json!({"generation":1,"note":"not this one"}),
            "2",
        )
        .await;
    assert_eq!(status, 200, "a reject reads no SKU for a rule: {b}");
    assert_eq!(g.price(&price["id"]).await.state, "rejected");

    // Quorum 2: the first vote applies nothing, so no rule reads the SKU.
    g.policy(None, 2).await;
    let again = &g.draft("b", body("2031-04-01")).await[0];
    let (status, receipt, _) = g.submit_as(&g.f.ctx, again, "submit-2").await;
    assert_eq!(status, 201, "{receipt}");
    let (status, b, _) = g
        .vote(
            &approve_only(&g),
            &receipt["unit"],
            "approve",
            json!({"generation":1}),
            "3",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["outcome"], "pending");
}

// D-416: a registry outage takes only the descriptors. A recurring unit is submitted, approved and
// rejected while Products cannot answer, and the snapshot says the descriptors are unavailable.
#[tokio::test]
async fn a_registry_outage_leaves_a_recurring_unit_to_its_reviewers() {
    use std::sync::atomic::Ordering::SeqCst;
    let script = Arc::new(Script::default());
    script.set(11);
    let g = gov_on(1, script.clone()).await;
    let first = &g.draft("a", body("2031-03-01")).await[0];
    let (status, one, _) = g.submit_as(&g.f.ctx, first, "s1").await;
    assert_eq!(status, 201, "{one}");
    script.skus_down.store(true, SeqCst);
    let second = &g.draft("b", body("2031-06-01")).await[0];
    let (status, two, _) = g.submit_as(&g.f.ctx, second, "s2").await;
    assert_eq!(status, 201, "the submit is not refused: {two}");
    assert_eq!(
        g.card(&two["unit"]).await["snapshot"]["descriptors"],
        "unavailable"
    );
    let (status, b, _) = g
        .vote(
            &g.f.user(),
            &one["unit"],
            "approve",
            json!({"generation":1}),
            "1",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["outcome"], "applied");
    let (status, b, _) = g
        .vote(
            &g.f.user(),
            &two["unit"],
            "reject",
            json!({"generation":1,"note":"later"}),
            "2",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(g.price(&second["id"]).await.state, "rejected");
    assert_eq!(g.price(&first["id"]).await.state, "approved");
}

#[tokio::test]
async fn a_stale_refresh_is_the_keys_committed_answer() {
    let g = gov(2).await;
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let (_, receipt, _) = g.submit_as(&g.f.ctx, price, "submit").await;
    let unit = &receipt["unit"];
    let (one, two) = (g.f.user(), g.f.user());
    assert_eq!(
        g.vote(&one, unit, "approve", json!({"generation":1}), "1")
            .await
            .0,
        200
    );
    let tenant = g.f.ctx.subject_tenant_id();
    price::Entity::update_many()
        .secure()
        .scope_with(&AccessScope::for_tenant(tenant))
        .col_expr(
            price::Column::PriceJson,
            sea_orm::sea_query::Expr::value(json!({"rate":"0.11"})),
        )
        .filter(sea_orm::Condition::all().add(sea_orm::ColumnTrait::eq(
            &price::Column::Id,
            price["id"].as_str().unwrap().parse::<Uuid>().unwrap(),
        )))
        .exec(&g.f.db.conn().unwrap())
        .await
        .unwrap();
    let stale = g
        .vote(&two, unit, "approve", json!({"generation":1}), "2")
        .await;
    assert_eq!(stale.0, 400, "{stale:?}");
    assert!(code(&stale.1).contains("UNIT_STALE"), "{stale:?}");
    assert_eq!(
        g.vote(&two, unit, "approve", json!({"generation":1}), "2")
            .await,
        stale,
        "the refresh committed with its answer; the key replays it"
    );
    assert_eq!(g.card(unit).await["generation"], 2, "refreshed once");
}

#[tokio::test]
async fn simultaneous_claims_of_one_key_record_one_unit() {
    let g = gov(1).await;
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let second = g.f.second_app().await;
    let path = format!("/prices/{}/submit", price["id"].as_str().unwrap());
    let (a, b) = tokio::join!(
        g.f.call("POST", &path, json!({}), None, Some("same")),
        entry_support::request(
            &second,
            &g.f.ctx,
            "POST",
            &path,
            json!({}),
            None,
            Some("same")
        )
    );
    let (first, other) = if a.0 == 201 { (a, b) } else { (b, a) };
    assert_eq!(first.0, 201, "{first:?}");
    assert!(
        other == first || (other.0 == 409 && code(&other.1).contains("IDEMPOTENCY_KEY_IN_FLIGHT")),
        "the other claim replays or waits: {other:?}"
    );
    let (_, list, _) =
        g.f.call("GET", "/approval-units", json!({}), None, None)
            .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1, "one act");
}

#[tokio::test]
async fn every_act_is_audited_with_its_actor_subject_and_correlation() {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let g = gov(1).await;
    let (submitter, reviewer) = (g.f.user(), g.f.user());
    let price = &g.draft("a", body("2031-03-01")).await[0];
    let (status, receipt, _) = g.submit_as(&submitter, price, "submit").await;
    assert_eq!(status, 201, "{receipt}");
    let unit = &receipt["unit"];
    assert_eq!(
        g.vote(&reviewer, unit, "approve", json!({"generation":1}), "ok")
            .await
            .0,
        200
    );
    let prices = Database::connect(&g.f.dsn)
        .await
        .unwrap()
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT action, actor_ref, correlation_id FROM pricing_audit \
             WHERE subject_id = ? ORDER BY written_at, action",
            [unit["id"].as_str().unwrap().parse::<Uuid>().unwrap().into()],
        ))
        .await
        .unwrap();
    let acts: Vec<(String, Uuid, bool)> = prices
        .iter()
        .map(|r| {
            (
                r.try_get::<String>("", "action").unwrap(),
                r.try_get::<Uuid>("", "actor_ref").unwrap(),
                r.try_get::<Option<String>>("", "correlation_id")
                    .unwrap()
                    .is_some_and(|c| !c.is_empty()),
            )
        })
        .collect();
    assert_eq!(
        acts,
        vec![
            (
                "approval.submitted".to_owned(),
                submitter.subject_id(),
                true
            ),
            ("approval.approved".to_owned(), reviewer.subject_id(), true),
        ]
    );
}

#[tokio::test]
async fn an_entry_with_only_draft_or_rejected_prices_is_deleted_with_them() {
    // Rejected prices carry no approved money; their review history stays in the unit snapshot.
    let g = gov(1).await;
    let reviewer = g.f.user();
    let rejected = &g.draft("a", body("2031-03-01")).await[0];
    let (status, receipt, _) = g.submit_as(&g.f.ctx, rejected, "submit").await;
    assert_eq!(status, 201, "{receipt}");
    let unit = receipt["unit"].clone();
    let (status, b, _) = g
        .vote(
            &reviewer,
            &unit,
            "reject",
            json!({"generation":1,"note":"wrong sku"}),
            "reject",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(g.price(&rejected["id"]).await.state, "rejected");
    let draft = &g.draft("b", body("2031-04-01")).await[0];
    let path = format!("/price-book-entries/{}", g.price_book_entry_id());
    let deleted = g.f.call("DELETE", &path, json!({}), None, None).await;
    assert_eq!(deleted.0, 204, "{deleted:?}");
    assert_eq!(g.f.call("GET", &path, json!({}), None, None).await.0, 404);
    let tenant = g.f.ctx.subject_tenant_id();
    for price in [rejected, draft] {
        assert!(
            price_repo::find(
                &g.f.db.conn().unwrap(),
                &AccessScope::for_tenant(tenant),
                tenant,
                price["id"].as_str().unwrap().parse().unwrap(),
            )
            .await
            .unwrap()
            .is_none()
        );
    }
    let card = g.card(&unit).await;
    assert_eq!(card["state"], "rejected");
    assert_eq!(
        card["snapshot"]["prices"][0]["after"]["price"],
        json!({"rate":"0.10"}),
        "the rejected proposal's history survives its price"
    );
}

#[tokio::test]
async fn an_entry_with_pending_or_approved_prices_refuses_deletion() {
    let g = gov(1).await;
    let pending = &g.draft("a", body("2031-03-01")).await[0];
    let (status, receipt, _) = g.submit_as(&g.f.ctx, pending, "submit").await;
    assert_eq!(status, 201, "{receipt}");
    let path = format!("/price-book-entries/{}", g.price_book_entry_id());
    let refused = g.f.call("DELETE", &path, json!({}), None, None).await;
    assert_eq!(refused.0, 409, "{refused:?}");
    assert!(
        code(&refused.1).contains("ENTRY_PRICES_IN_USE"),
        "{refused:?}"
    );
    let (status, b, _) = g
        .vote(
            &g.f.user(),
            &receipt["unit"],
            "approve",
            json!({"generation":1}),
            "approve",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    let refused = g.f.call("DELETE", &path, json!({}), None, None).await;
    assert_eq!(refused.0, 409, "{refused:?}");
    assert!(
        code(&refused.1).contains("ENTRY_PRICES_IN_USE"),
        "{refused:?}"
    );
    assert_eq!(g.f.call("GET", &path, json!({}), None, None).await.0, 200);
}

// Chains HIGH-1 on the wire: the stale return is a 400 with its code, and no unit is made.
#[tokio::test]
async fn a_common_date_past_an_approved_change_answers_400_pair_return_stale() {
    let g = gov(1).await;
    g.approved(1, "2031-01-01").await;
    g.approved(2, "2031-06-01").await;
    let mut promo = body("2031-02-01");
    promo["temporary_until"] = json!("2031-03-01");
    let pair = g.draft("pair", promo).await;
    let path = format!("/price-books/{}/publish-changes", g.book);
    let (status, b, _) =
        g.f.call(
            "POST",
            &path,
            json!({"common_effective_date":"2031-07-01"}),
            None,
            Some("late"),
        )
        .await;
    assert_eq!(status, 400, "{b}");
    assert!(code(&b).contains("PAIR_RETURN_STALE"), "{b}");
    assert_eq!(g.price(&pair[0]["id"]).await.state, "draft");
    let (status, list, _) =
        g.f.call("GET", "/approval-units", json!({}), None, None)
            .await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["items"], json!([]), "no unit was recorded");
}

// Docs F10 (D-392): the queue list and the publish-changes listing carry the same impact
// object as the unit card.
#[tokio::test]
async fn the_queue_list_and_the_publish_listing_carry_impact() {
    let g = gov(1).await;
    g.approved(1, "2031-01-01").await;
    g.draft("one", body("2031-05-01")).await;
    let mut promo = body("2031-03-01");
    promo["temporary_until"] = json!("2031-03-11");
    g.draft("pair", promo).await;
    let path = format!("/price-books/{}/publish-changes", g.book);
    // Phase 3 measures the plans (none name this book's entries here); subscriptions wait for
    // the Subscriptions integration.
    let unavailable = "unavailable until the Subscriptions integration";
    let (status, listing, _) = g.f.call("GET", &path, json!({}), None, None).await;
    assert_eq!(status, 200, "{listing}");
    assert_eq!(
        listing["impact"],
        json!({"prices":3,"entries":1,"plans":[],"subscriptions":unavailable})
    );
    let (status, receipt, _) = g.f.call("POST", &path, json!({}), None, Some("all")).await;
    assert_eq!(status, 201, "{receipt}");
    let (status, list, _) =
        g.f.call("GET", "/approval-units", json!({}), None, None)
            .await;
    assert_eq!(status, 200, "{list}");
    let card = g.card(&receipt["unit"]).await;
    assert_eq!(
        list["items"][0]["impact"],
        json!({"prices":3,"entries":1,"plans":[],"subscriptions":unavailable})
    );
    assert_eq!(list["items"][0]["impact"], card["impact"]);
}

// Behaviour LOW-3 (D-404): an entry delete cannot take another author's draft with it. Bob, who
// may author entries, is refused 403 NOT_DRAFT_AUTHOR, and the answer names Alice's draft.
// A rejected price is history and never blocks: Alice deletes the entry, Bob's rejected
// proposal included.
#[tokio::test]
async fn an_entry_delete_honours_each_drafts_author() {
    let g = gov(1).await;
    let (alice, bob) = (g.f.ctx.clone(), g.f.user());
    let bobs = &g.draft_as(&bob, "b", body("2031-02-01")).await[0];
    let (status, receipt, _) = g.submit_as(&bob, bobs, "bs").await;
    assert_eq!(status, 201, "{receipt}");
    let (status, b, _) = g
        .vote(
            &alice,
            &receipt["unit"],
            "reject",
            json!({"generation":1,"note":"no"}),
            "rj",
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(g.price(&bobs["id"]).await.state, "rejected");
    let alices = &g.draft_as(&alice, "a", body("2031-03-01")).await[0];
    let path = format!("/price-book-entries/{}", g.price_book_entry_id());
    let (status, b, _) =
        g.f.call_as(&bob, "DELETE", &path, json!({}), None, None)
            .await;
    assert_eq!(status, 403, "{b}");
    assert!(code(&b).contains("NOT_DRAFT_AUTHOR"), "{b}");
    assert!(
        code(&b).contains(alices["id"].as_str().unwrap()),
        "the answer names the draft: {b}"
    );
    assert_eq!(
        g.price(&alices["id"]).await.state,
        "draft",
        "nothing deleted"
    );
    assert_eq!(g.f.call("GET", &path, json!({}), None, None).await.0, 200);
    let (status, b, _) =
        g.f.call_as(&alice, "DELETE", &path, json!({}), None, None)
            .await;
    assert_eq!(
        status, 204,
        "her own drafts and a rejected price go with it: {b}"
    );
    assert_eq!(g.f.call("GET", &path, json!({}), None, None).await.0, 404);
}
