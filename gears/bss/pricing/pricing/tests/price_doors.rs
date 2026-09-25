//! Draft prices through the production router: create, temporary pairs, patch and delete.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod entry_support;
use bss_pricing::infra::storage::repo::{price_book_entry_repo, price_repo};
use entry_support::{Fixture, Script};
use serde_json::{Value, json};
use std::sync::Arc;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

/// A book, an optional `region` registry, and one confirmed usage entry.
async fn priced(dimension: bool) -> (Fixture, Value) {
    let script = Arc::new(Script::default());
    let f = Fixture::new(script).await;
    let (book, _) = f.book().await;
    if dimension {
        let (_, _, tag) = f
            .call("GET", "/dimension-keys", json!({}), None, None)
            .await;
        let saved = f
            .call(
                "PUT",
                "/dimension-keys",
                json!({"items":[{"key":"region","values":["eu","us"]}]}),
                Some(&tag),
                None,
            )
            .await;
        assert_eq!(saved.0, 200, "{saved:?}");
    }
    let body = if dimension {
        json!({"sku_id":Uuid::new_v4(),"dimension_key":"region"})
    } else {
        json!({"sku_id":Uuid::new_v4()})
    };
    let (status, entry, _) = f
        .call(
            "POST",
            &format!("/price-books/{}/entries", book["id"].as_str().unwrap()),
            body,
            None,
            Some("entry"),
        )
        .await;
    assert_eq!(status, 201, "{entry}");
    (f, entry)
}
fn prices_path(entry: &Value) -> String {
    format!(
        "/price-book-entries/{}/prices",
        entry["id"].as_str().unwrap()
    )
}
fn draft(from: &str) -> Value {
    json!({"model":"per_unit","price":{"rate":"0.10"},"eligibility":"all","effective_from":from})
}
/// An approved price written directly, as a unit's apply would leave it.
async fn approved(
    f: &Fixture,
    entry: &Value,
    version_no: i32,
    from: &str,
    dim: Option<&str>,
) -> Uuid {
    let tenant = f.ctx.subject_tenant_id();
    let scope = AccessScope::for_tenant(tenant);
    let conn = f.db.conn().unwrap();
    let p = price_book_entry_repo::find(
        &conn,
        &scope,
        tenant,
        entry["id"].as_str().unwrap().parse().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    let mut price = entry_support::price(&p);
    price.version_no = version_no;
    price.state = "approved".into();
    price.dim_value = dim.map(str::to_owned);
    price.price_json = json!({"rate":"0.20"});
    price.effective_from =
        time::Date::parse(from, &time::format_description::well_known::Iso8601::DATE).unwrap();
    price_repo::insert(&conn, &scope, price).await.unwrap().id
}
fn code_in(body: &Value, code: &str) -> bool {
    body.to_string().contains(code)
}

#[tokio::test]
async fn a_draft_price_is_created_once_per_key_and_numbered_per_entry() {
    let (f, entry) = priced(false).await;
    let path = prices_path(&entry);
    assert_eq!(
        f.call("POST", &path, draft("2031-01-01"), None, None)
            .await
            .0,
        400,
        "Idempotency-Key is required"
    );
    let first = f
        .call("POST", &path, draft("2031-01-01"), None, Some("r1"))
        .await;
    assert_eq!(first.0, 201, "{first:?}");
    assert_eq!(first.2, "\"1\"");
    let price = &first.1["items"][0];
    assert_eq!(first.1["items"].as_array().unwrap().len(), 1);
    assert_eq!(price["state"], "draft");
    assert_eq!(price["status"], "draft");
    assert_eq!(price["version_no"], 1);
    assert_eq!(price["price_json"], json!({"rate":"0.10"}));
    assert_eq!(price["created_by"], f.ctx.subject_id().to_string());
    assert!(price["pending_unit_id"].is_null());
    assert_eq!(
        f.call("POST", &path, draft("2031-01-01"), None, Some("r1"))
            .await,
        first,
        "an answered key replays"
    );
    let other = f
        .call("POST", &path, draft("2031-02-01"), None, Some("r1"))
        .await;
    assert_eq!(other.0, 409);
    assert!(code_in(&other.1, "IDEMPOTENCY_CONFLICT"), "{other:?}");
    let second = f
        .call("POST", &path, draft("2031-01-01"), None, Some("r2"))
        .await;
    assert_eq!(second.0, 201, "two drafts may share a start until approval");
    assert_eq!(second.1["items"][0]["version_no"], 2);
    let missing = f
        .call(
            "POST",
            &format!("/price-book-entries/{}/prices", Uuid::new_v4()),
            draft("2031-01-01"),
            None,
            Some("r3"),
        )
        .await;
    assert_eq!(missing.0, 404, "{missing:?}");
    assert_eq!(
        missing.1["context"]["resource_name"], "price_book_entry",
        "the missing object is the entry, not the price being created: {missing:?}"
    );
    assert!(
        missing.1["detail"]
            .as_str()
            .unwrap()
            .starts_with("ENTRY_NOT_FOUND"),
        "{missing:?}"
    );
}

#[tokio::test]
async fn a_temporary_price_on_an_owned_chain_is_a_pair_that_returns_to_the_price_in_force() {
    let (f, entry) = priced(false).await;
    let back = approved(&f, &entry, 1, "2031-01-01", None).await;
    let mut body = draft("2031-03-01");
    body["price"] = json!({"rate":"0.05"});
    body["temporary_until"] = json!("2031-03-11");
    body["note"] = json!("spring promo");
    let created = f
        .call("POST", &prices_path(&entry), body, None, Some("promo"))
        .await;
    assert_eq!(created.0, 201, "{created:?}");
    let items = created.1["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let (promo, ret) = (&items[0], &items[1]);
    assert_eq!(promo["version_no"], 2);
    assert_eq!(ret["version_no"], 3);
    assert_eq!(promo["temporary_until"], "2031-03-11");
    assert_eq!(promo["effective_to"], "2031-03-11");
    assert_eq!(promo["closed_explicitly"], false);
    assert_eq!(promo["paired_price_id"], ret["id"]);
    assert_eq!(ret["paired_price_id"], promo["id"]);
    assert_eq!(ret["return_of_price_id"], back.to_string());
    assert_eq!(ret["effective_from"], "2031-03-11");
    assert!(ret["effective_to"].is_null());
    assert_eq!(
        ret["price_json"],
        json!({"rate":"0.20"}),
        "the return copies the price in force"
    );
    assert_eq!(ret["note"], "spring promo");
}

#[tokio::test]
async fn a_temporary_price_on_a_value_without_its_own_chain_is_one_closed_price() {
    let (f, entry) = priced(true).await;
    approved(&f, &entry, 1, "2031-01-01", None).await;
    let mut body = draft("2031-03-01");
    body["dim_value"] = json!("eu");
    body["temporary_until"] = json!("2031-03-11");
    let created = f
        .call("POST", &prices_path(&entry), body, None, Some("eu"))
        .await;
    assert_eq!(created.0, 201, "{created:?}");
    let items = created.1["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "no return price copies the default");
    assert_eq!(items[0]["dim_value"], "eu");
    assert_eq!(items[0]["closed_explicitly"], true);
    assert_eq!(items[0]["effective_to"], "2031-03-11");
    assert!(items[0]["paired_price_id"].is_null());
}

#[tokio::test]
async fn pure_rule_refusals_answer_400_with_their_codes_and_no_price() {
    let (f, entry) = priced(true).await;
    approved(&f, &entry, 1, "2031-01-01", None).await;
    let cases: Vec<(Value, &str)> = vec![
        (
            json!({"model":"flat","price":{"amount":"5"},"eligibility":"all","effective_from":"2031-05-01"}),
            "MODEL_KIND_CHARGEKIND_MISMATCH",
        ),
        (
            json!({"model":"per_unit","price":{"amount":"5"},"eligibility":"all","effective_from":"2031-05-01"}),
            "PRICE_MISSING",
        ),
        (
            json!({"model":"per_unit","price":{"rate":"-1"},"eligibility":"all","effective_from":"2031-05-01"}),
            "AMOUNT_INVALID",
        ),
        (
            json!({"model":"package","price":{"package_size":"0","package_price":"5"},"eligibility":"all","effective_from":"2031-05-01"}),
            "PACKAGE_FIELDS_INVALID",
        ),
        (
            json!({"model":"graduated","price":{"tiers":[{"up_to":"10","rate":"1"},{"up_to":"5","rate":"1"},{"up_to":null,"rate":"1"}]},"eligibility":"all","effective_from":"2031-05-01"}),
            "TIER_BANDS_ORDER",
        ),
        (
            json!({"model":"volume","price":{"tiers":[{"up_to":"10","rate":"1"}]},"eligibility":"all","effective_from":"2031-05-01"}),
            "TIER_TOP_CLOSED",
        ),
        (
            json!({"model":"per_unit","price":{"rate":"1"},"eligibility":"all","effective_from":"2020-01-01"}),
            "WINDOW_START_IN_PAST",
        ),
        (
            json!({"model":"per_unit","price":{"rate":"1"},"eligibility":"all","effective_from":"2031-02-30"}),
            "WINDOW_START_INVALID",
        ),
        (
            json!({"model":"per_unit","price":{"rate":"1"},"eligibility":"all","effective_from":"2031-05-01","temporary_until":"2031-05-01"}),
            "WINDOW_END_INVALID",
        ),
        (
            json!({"model":"per_unit","price":{"rate":"1"},"eligibility":"all","effective_from":"2031-01-01"}),
            "WINDOW_OVERLAP",
        ),
        (
            json!({"model":"per_unit","price":{"rate":"1"},"eligibility":"all","effective_from":"2031-05-01","dim_value":"ap"}),
            "DIM_VALUE_UNKNOWN",
        ),
        (
            json!({"model":"per_unit","price":{"rate":"1"},"eligibility":"all","effective_from":"2031-05-01","min_fee":"-1"}),
            "MIN_FEE_INVALID",
        ),
        (
            json!({"model":"per_unit","price":{"rate":"1"},"eligibility":"all","effective_from":"2031-05-01","min_fee":"0.001"}),
            "MIN_FEE_INVALID",
        ),
        (
            json!({"model":"stair","price":{"rate":"1"},"eligibility":"all","effective_from":"2031-05-01"}),
            "MODEL_INVALID",
        ),
        (
            json!({"model":"per_unit","price":{"rate":"1"},"eligibility":"some","effective_from":"2031-05-01"}),
            "ELIGIBILITY_INVALID",
        ),
    ];
    for (n, (body, code)) in cases.into_iter().enumerate() {
        let refused = f
            .call(
                "POST",
                &prices_path(&entry),
                body,
                None,
                Some(&format!("k{n}")),
            )
            .await;
        assert_eq!(refused.0, 400, "{code}: {refused:?}");
        assert!(code_in(&refused.1, code), "{code}: {refused:?}");
    }
    let unknown = f
        .call(
            "POST",
            &prices_path(&entry),
            json!({"model":"per_unit","price":{"rate":"1"},"eligibility":"all","effective_from":"2031-05-01","variant":"x"}),
            None,
            Some("unknown"),
        )
        .await;
    assert_eq!(unknown.0, 400, "deny_unknown_fields");
    let (undimensioned, plain) = priced(false).await;
    let mut body = draft("2031-05-01");
    body["dim_value"] = json!("eu");
    let refused = undimensioned
        .call("POST", &prices_path(&plain), body, None, Some("dim"))
        .await;
    assert_eq!(refused.0, 400);
    assert!(code_in(&refused.1, "DIM_NOT_DECLARED"), "{refused:?}");
    let tenant = f.ctx.subject_tenant_id();
    let conn = f.db.conn().unwrap();
    let prices = price_repo::for_entry(
        &conn,
        &AccessScope::for_tenant(tenant),
        tenant,
        entry["id"].as_str().unwrap().parse().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(prices.len(), 1, "no refused request wrote a price");
}

#[tokio::test]
async fn a_lost_entry_refuses_prices_and_a_pending_confirmation_accepts_them() {
    let (f, entry) = priced(false).await;
    let tenant = f.ctx.subject_tenant_id();
    let scope = AccessScope::for_tenant(tenant);
    let conn = f.db.conn().unwrap();
    let id: Uuid = entry["id"].as_str().unwrap().parse().unwrap();
    let p = price_book_entry_repo::find(&conn, &scope, tenant, id)
        .await
        .unwrap()
        .unwrap();
    price_book_entry_repo::set_reference(
        &conn,
        &scope,
        tenant,
        id,
        p.version,
        bss_pricing::domain::price_book_entry::ReferenceState::ConfirmationPending,
        p.reservation_id,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    let pending = f
        .call(
            "POST",
            &prices_path(&entry),
            draft("2031-01-01"),
            None,
            Some("a"),
        )
        .await;
    assert_eq!(pending.0, 201, "{pending:?}");
    price_book_entry_repo::set_reference(
        &conn,
        &scope,
        tenant,
        id,
        p.version + 1,
        bss_pricing::domain::price_book_entry::ReferenceState::Lost,
        p.reservation_id,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    let lost = f
        .call(
            "POST",
            &prices_path(&entry),
            draft("2031-02-01"),
            None,
            Some("b"),
        )
        .await;
    assert_eq!(lost.0, 409, "{lost:?}");
    assert!(code_in(&lost.1, "ENTRY_REFERENCE_LOST"));
}

#[tokio::test]
async fn patch_and_delete_touch_only_unlocked_drafts_at_their_version() {
    let (f, entry) = priced(false).await;
    let created = f
        .call(
            "POST",
            &prices_path(&entry),
            draft("2031-01-01"),
            None,
            Some("a"),
        )
        .await;
    let id = created.1["items"][0]["id"].as_str().unwrap().to_owned();
    let path = format!("/prices/{id}");
    assert_eq!(
        f.call("PATCH", &path, json!({"note":"x"}), None, None)
            .await
            .0,
        400,
        "If-Match is required"
    );
    let changed = f
        .call(
            "PATCH",
            &path,
            json!({"price":{"rate":"0.12"},"min_fee":"30.00","note":"revised","effective_from":"2031-01-15"}),
            Some(&created.2),
            None,
        )
        .await;
    assert_eq!(changed.0, 200, "{changed:?}");
    assert_eq!(changed.2, "\"2\"");
    assert_eq!(changed.1["price_json"], json!({"rate":"0.12"}));
    assert_eq!(changed.1["min_fee"], "30.00");
    assert_eq!(changed.1["effective_from"], "2031-01-15");
    let stale = f
        .call(
            "PATCH",
            &path,
            json!({"note":"stale"}),
            Some(&created.2),
            None,
        )
        .await;
    assert_eq!(stale.0, 409, "{stale:?}");
    assert!(code_in(&stale.1, "STALE_REVISION"), "{stale:?}");
    let invalid = f
        .call(
            "PATCH",
            &path,
            json!({"model":"flat","price":{"amount":"1"}}),
            Some(&changed.2),
            None,
        )
        .await;
    assert_eq!(invalid.0, 400);
    assert!(code_in(&invalid.1, "MODEL_KIND_CHARGEKIND_MISMATCH"));
    assert_eq!(
        f.call(
            "PATCH",
            &path,
            json!({"state":"approved"}),
            Some(&changed.2),
            None
        )
        .await
        .0,
        400,
        "only business fields are patchable"
    );
    let approved_id = approved(&f, &entry, 9, "2031-06-01", None).await;
    for method in ["PATCH", "DELETE"] {
        let refused = f
            .call(
                method,
                &format!("/prices/{approved_id}"),
                json!({"note":"x"}),
                Some("\"1\""),
                None,
            )
            .await;
        assert_eq!(refused.0, 409, "{method} {refused:?}");
        assert!(code_in(&refused.1, "PRICE_NOT_DRAFT"), "{refused:?}");
    }
    assert_eq!(
        f.call("DELETE", &path, json!({}), Some(&created.2), None)
            .await
            .0,
        409,
        "a stale tag cannot delete"
    );
    assert_eq!(
        f.call("DELETE", &path, json!({}), Some(&changed.2), None)
            .await
            .0,
        204
    );
    assert_eq!(
        f.call("DELETE", &path, json!({}), Some(&changed.2), None)
            .await
            .0,
        404
    );
}

#[tokio::test]
async fn a_pair_is_deleted_whole_and_its_window_is_fixed() {
    let (f, entry) = priced(false).await;
    approved(&f, &entry, 1, "2031-01-01", None).await;
    let mut body = draft("2031-03-01");
    body["temporary_until"] = json!("2031-03-11");
    let created = f
        .call("POST", &prices_path(&entry), body, None, Some("promo"))
        .await;
    assert_eq!(created.0, 201, "{created:?}");
    let promo = created.1["items"][0]["id"].as_str().unwrap().to_owned();
    let ret = created.1["items"][1]["id"].as_str().unwrap().to_owned();
    for patch in [
        json!({"effective_from":"2031-03-02"}),
        json!({"dim_value":"eu"}),
    ] {
        let refused = f
            .call(
                "PATCH",
                &format!("/prices/{promo}"),
                patch,
                Some("\"1\""),
                None,
            )
            .await;
        assert_eq!(refused.0, 400, "{refused:?}");
        assert!(code_in(&refused.1, "TEMPORARY_PRICE_FIXED"), "{refused:?}");
    }
    let money = f
        .call(
            "PATCH",
            &format!("/prices/{ret}"),
            json!({"price":{"rate":"0.21"}}),
            Some("\"1\""),
            None,
        )
        .await;
    assert_eq!(
        money.0, 200,
        "money on a return half stays editable: {money:?}"
    );
    assert_eq!(
        f.call(
            "DELETE",
            &format!("/prices/{promo}"),
            json!({}),
            Some("\"1\""),
            None
        )
        .await
        .0,
        204
    );
    let tenant = f.ctx.subject_tenant_id();
    let conn = f.db.conn().unwrap();
    let left = price_repo::for_entry(
        &conn,
        &AccessScope::for_tenant(tenant),
        tenant,
        entry["id"].as_str().unwrap().parse().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        left.len(),
        1,
        "both halves are gone, the approved price stays"
    );
    assert_eq!(left[0].state, "approved");
}

#[tokio::test]
async fn deleting_an_entry_removes_its_door_made_drafts() {
    let (f, entry) = priced(false).await;
    let first = f
        .call(
            "POST",
            &prices_path(&entry),
            draft("2031-01-01"),
            None,
            Some("a"),
        )
        .await;
    assert_eq!(first.0, 201);
    let tenant = f.ctx.subject_tenant_id();
    let conn = f.db.conn().unwrap();
    let scope = AccessScope::for_tenant(tenant);
    // A pair needs an approved price, which blocks the delete; without one the
    // default chain yields a single explicitly closed price.
    let mut body = draft("2031-03-01");
    body["temporary_until"] = json!("2031-03-11");
    let single = f
        .call("POST", &prices_path(&entry), body, None, Some("b"))
        .await;
    assert_eq!(single.0, 201, "{single:?}");
    assert_eq!(single.1["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        f.call(
            "DELETE",
            &format!("/price-book-entries/{}", entry["id"].as_str().unwrap()),
            json!({}),
            None,
            None
        )
        .await
        .0,
        204
    );
    assert!(
        price_repo::for_entry(
            &conn,
            &scope,
            tenant,
            entry["id"].as_str().unwrap().parse().unwrap()
        )
        .await
        .unwrap()
        .is_empty()
    );
}

#[tokio::test]
async fn two_writers_creating_prices_at_once_get_distinct_version_numbers() {
    let (f, entry) = priced(false).await;
    let second = f.second_app().await;
    let path = prices_path(&entry);
    let (a, b) = tokio::join!(
        f.call("POST", &path, draft("2031-01-01"), None, Some("a")),
        entry_support::request(
            &second,
            &f.ctx,
            "POST",
            &path,
            draft("2031-02-01"),
            None,
            Some("b")
        )
    );
    assert_eq!(a.0, 201, "{a:?}");
    assert_eq!(b.0, 201, "{b:?}");
    let mut numbers = [
        a.1["items"][0]["version_no"].as_i64().unwrap(),
        b.1["items"][0]["version_no"].as_i64().unwrap(),
    ];
    numbers.sort_unstable();
    assert_eq!(numbers, [1, 2]);
}

// Chains LOW-4 at the door: a JSON number is refused 400 AMOUNT_INVALID, create and patch.
#[tokio::test]
async fn money_as_a_json_number_is_refused_and_the_detail_says_strings() {
    let (f, entry) = priced(false).await;
    let mut body = draft("2031-03-01");
    body["price"] = json!({"rate": 0.123_456_789_012_345_67});
    let (status, b, _) = f
        .call("POST", &prices_path(&entry), body, None, Some("n"))
        .await;
    assert_eq!(status, 400, "{b}");
    assert!(b.to_string().contains("AMOUNT_INVALID"), "{b}");
    assert!(
        b.to_string().contains("string"),
        "the detail says decimals are strings: {b}"
    );
    let (status, price, _) = f
        .call(
            "POST",
            &prices_path(&entry),
            draft("2031-03-01"),
            None,
            Some("s"),
        )
        .await;
    assert_eq!(status, 201, "{price}");
    let (status, b, _) = f
        .call(
            "PATCH",
            &format!("/prices/{}", price["items"][0]["id"].as_str().unwrap()),
            json!({"price":{"rate": 1}}),
            Some("\"1\""),
            None,
        )
        .await;
    assert_eq!(status, 400, "{b}");
    assert!(b.to_string().contains("AMOUNT_INVALID"), "{b}");
}

// dod-if-match-version (PRD AC #25): at every PATCH/PUT door a successful update moves the
// ETag, the old token is 409 STALE_REVISION with nothing written, and no token is a
// precondition failure.
#[tokio::test]
async fn every_conditional_door_answers_a_stale_token_with_stale_revision() {
    let (f, entry) = priced(false).await;
    let (_, book, book_tag) = f
        .call(
            "GET",
            &format!("/price-books/{}", entry["book_id"].as_str().unwrap()),
            json!({}),
            None,
            None,
        )
        .await;
    let (_, price, price_tag) = f
        .call(
            "POST",
            &prices_path(&entry),
            draft("2031-03-01"),
            None,
            Some("r"),
        )
        .await;
    let price_tag = if price_tag.is_empty() {
        "\"1\"".to_owned()
    } else {
        price_tag
    };
    let (_, _, entry_tag) = f
        .call(
            "GET",
            &format!("/price-book-entries/{}", entry["id"].as_str().unwrap()),
            json!({}),
            None,
            None,
        )
        .await;
    let (_, _, settings_tag) = f.call("GET", "/settings", json!({}), None, None).await;
    let (_, _, dims_tag) = f
        .call("GET", "/dimension-keys", json!({}), None, None)
        .await;
    let (_, _, policy_tag) = f
        .call("GET", "/approval-policy", json!({}), None, None)
        .await;
    let doors = [
        (
            "PATCH",
            format!("/price-books/{}", book["id"].as_str().unwrap()),
            json!({"name":"Renamed"}),
            book_tag,
        ),
        (
            "PATCH",
            format!("/price-book-entries/{}", entry["id"].as_str().unwrap()),
            json!({"invoice_line_override":null}),
            entry_tag,
        ),
        (
            "PATCH",
            format!("/prices/{}", price["items"][0]["id"].as_str().unwrap()),
            json!({"note":"changed"}),
            price_tag,
        ),
        (
            "PUT",
            "/settings".to_owned(),
            json!({"default_timing":"arrears","default_rounding":"half_up","invoice_line_templates":{}}),
            settings_tag,
        ),
        (
            "PUT",
            "/dimension-keys".to_owned(),
            json!({"items":[{"key":"region","values":["eu","us"]}]}),
            dims_tag,
        ),
        (
            "PUT",
            "/approval-policy".to_owned(),
            json!({"quorum":2}),
            policy_tag,
        ),
    ];
    for (method, path, body, tag) in doors {
        let (status, b, next) = f.call(method, &path, body.clone(), Some(&tag), None).await;
        assert_eq!(status, 200, "{method} {path}: {b}");
        assert_ne!(next, tag, "{method} {path}: the successor token changes");
        let (status, b, _) = f.call(method, &path, body.clone(), Some(&tag), None).await;
        assert_eq!(status, 409, "{method} {path}: {b}");
        assert!(code_in(&b, "STALE_REVISION"), "{method} {path}: {b}");
        let (status, b, _) = f.call(method, &path, body, None, None).await;
        assert_eq!(status, 400, "{method} {path}: no token: {b}");
    }
    let (status, b, _) = f
        .call(
            "DELETE",
            &format!("/prices/{}", price["items"][0]["id"].as_str().unwrap()),
            json!({}),
            Some("\"1\""),
            None,
        )
        .await;
    assert_eq!(status, 409, "{b}");
    assert!(code_in(&b, "STALE_REVISION"), "{b}");
}
