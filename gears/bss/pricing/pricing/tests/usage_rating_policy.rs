//! Entry-owned immutable policy authoring and identity.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod pg_support;
mod plan_support;
mod seam_support;
use plan_support::entry_support::{Fixture, Script};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

fn policy() -> Value {
    json!({
        "rating_window":{"kind":"calendar_hour","timezone":"UTC"},
        "aggregation_scope":"subscription_line","reset":"rating_window_start",
        "quantity_semantics":{"meter":{"usage_type_id":"vm-hours","version":"v1"},
            "unit":"VM\u{b7}hour","fold":"SUM","accrual_policy_version":"integrated-v1"},
        "partial_window":"actual_quantity_full_thresholds"
    })
}

#[tokio::test]
async fn policy_is_immutable_deduplicated_and_part_of_the_entry_key() {
    authoring_case(Fixture::new(Arc::new(Script::default())).await).await;
}

#[expect(
    clippy::cognitive_complexity,
    reason = "one ordered authoring and replay contract, shared by both databases"
)]
async fn authoring_case(f: Fixture) {
    let (book, _) = f.book().await;
    let path = format!("/price-books/{}/entries", book["id"].as_str().unwrap());
    let body = json!({"sku_id":Uuid::new_v4(),"model":"per_unit","usage_rating_policy":policy()});
    let first = f
        .call("POST", &path, body.clone(), None, Some("hour"))
        .await;
    assert_eq!(first.0, 201, "{first:?}");
    let original = first.1["usage_rating_policy"].clone();
    assert_eq!(original["content"], policy());
    assert_eq!(original["version"], "1");
    assert_eq!(original["digest"].as_str().unwrap().len(), 64);
    let read_path = format!("/price-book-entries/{}", first.1["id"].as_str().unwrap());
    assert_immutable(&f, &read_path, &first.2, &original).await;
    assert_policy_views(&f, &book["id"], &body["sku_id"], &original).await;
    let duplicate = f
        .call("POST", &path, body.clone(), None, Some("duplicate"))
        .await;
    assert_eq!(duplicate.0, 409, "{duplicate:?}");
    assert!(duplicate.1.to_string().contains("ENTRY_KEY_TAKEN"));
    let mut monthly = body.clone();
    monthly["usage_rating_policy"]["rating_window"] = json!({"kind":"billing_cycle"});
    assert_eq!(
        f.call("POST", &path, monthly, None, Some("month")).await.0,
        201
    );
    let mut resource = body.clone();
    resource["usage_rating_policy"]["aggregation_scope"] = json!("resource");
    let second = f
        .call("POST", &path, resource, None, Some("resource"))
        .await;
    assert_eq!(second.0, 201, "{second:?}");
    assert_ne!(
        second.1["usage_rating_policy"]["policy_id"],
        original["policy_id"]
    );
    let mut another_sku = body.clone();
    another_sku["sku_id"] = json!(Uuid::new_v4());
    let reused = f
        .call("POST", &path, another_sku, None, Some("reuse"))
        .await;
    assert_eq!(reused.0, 201, "{reused:?}");
    assert_eq!(reused.1["usage_rating_policy"], original);
    assert_eq!(f.call("POST", &path, body, None, Some("hour")).await, first);
    assert_eq!(
        f.call("GET", &read_path, json!({}), None, None).await.1["usage_rating_policy"],
        original
    );
}

async fn assert_policy_views(f: &Fixture, book: &Value, sku: &Value, original: &Value) {
    let book = book.as_str().unwrap();
    let sku = sku.as_str().unwrap();
    for (path, pointer) in [
        (
            format!("/price-books/{book}/entries"),
            "/items/0/usage_rating_policy",
        ),
        (
            format!("/price-books/{book}/export"),
            "/entries/0/entry/usage_rating_policy",
        ),
        (
            format!("/price-book-entries?sku_id={sku}"),
            "/items/0/usage_rating_policy",
        ),
    ] {
        let answer = f.call("GET", &path, json!({}), None, None).await;
        assert_eq!(answer.0, 200, "{answer:?}");
        assert_eq!(
            answer.1.pointer(pointer),
            Some(original),
            "{path}: {answer:?}"
        );
    }
}

async fn assert_immutable(f: &Fixture, read_path: &str, tag: &str, original: &Value) {
    assert_eq!(
        f.call("GET", read_path, json!({}), None, None).await.1["usage_rating_policy"],
        *original
    );
    for replacement in [Value::Null, policy()] {
        assert_eq!(
            f.call(
                "PATCH",
                read_path,
                json!({"usage_rating_policy":replacement}),
                Some(tag),
                None
            )
            .await
            .0,
            400
        );
    }
}

#[test]
fn a_window_change_is_a_different_entry_key() {
    use bss_pricing::domain::usage_policy::entry_policy_key;
    use bss_pricing_sdk::terms::{RatingWindow, Timezone};
    let monthly = seam_support::vm_hour_policy().content;
    let mut hourly = monthly.clone();
    hourly.rating_window = RatingWindow::CalendarHour {
        timezone: Timezone::Utc,
    };
    assert_ne!(
        entry_policy_key(Some(&hourly)),
        entry_policy_key(Some(&monthly))
    );
    assert_eq!(
        entry_policy_key(Some(&hourly)),
        entry_policy_key(Some(&hourly.clone()))
    );
    assert_eq!(entry_policy_key(None), None);
}

#[tokio::test]
async fn policy_shape_and_ownership_are_enforced_before_reservation() {
    let script = Arc::new(Script::default());
    let f = Fixture::new(script.clone()).await;
    let (book, _) = f.book().await;
    let path = format!("/price-books/{}/entries", book["id"].as_str().unwrap());
    let base = json!({"sku_id":Uuid::new_v4(),"model":"per_unit"});
    let missing = f
        .call("POST", &path, base.clone(), None, Some("missing"))
        .await;
    assert_eq!(missing.0, 400);
    assert!(missing.1.to_string().contains("MISSING_RATING_POLICY"));
    let mut valid = base.clone();
    valid["usage_rating_policy"] = policy();
    for (key, value) in [
        ("policy_id", json!(Uuid::new_v4())),
        ("version", json!("1")),
        ("digest", json!("0".repeat(64))),
        ("included_qty", json!("1")),
    ] {
        let mut body = valid.clone();
        body["usage_rating_policy"][key] = value;
        assert_eq!(f.call("POST", &path, body, None, Some(key)).await.0, 400);
    }
    for pointer in [
        "/quantity_semantics/meter/usage_type_id",
        "/quantity_semantics/meter/version",
        "/quantity_semantics/unit",
        "/quantity_semantics/accrual_policy_version",
    ] {
        let mut body = valid.clone();
        *body["usage_rating_policy"].pointer_mut(pointer).unwrap() = json!(" \t");
        let result = f.call("POST", &path, body, None, Some(pointer)).await;
        assert_eq!(result.0, 400, "{result:?}");
        assert!(result.1.to_string().contains("METER_POLICY_MISMATCH"));
    }
    for (pointer, value) in [
        ("/rating_window/kind", "rolling"),
        ("/rating_window/timezone", "Europe/Madrid"),
        ("/aggregation_scope", "tenant"),
        ("/quantity_semantics/fold", "MAX"),
        ("/reset", "never"),
        ("/partial_window", "prorate_thresholds"),
    ] {
        let mut body = valid.clone();
        *body["usage_rating_policy"].pointer_mut(pointer).unwrap() = json!(value);
        assert_eq!(
            f.call("POST", &path, body, None, Some(pointer)).await.0,
            400
        );
    }
    let duplicate = valid.to_string().replace(
        "\"version\":\"v1\"",
        "\"version\":\"v1\",\"version\":\"v2\"",
    );
    assert_eq!(
        plan_support::entry_support::request_raw(
            &f.app,
            &f.ctx,
            "POST",
            &path,
            &duplicate,
            Some("duplicate-field")
        )
        .await
        .0,
        400
    );
    script.set(11);
    valid["period"] = json!("month");
    let nonusage = f.call("POST", &path, valid, None, Some("nonusage")).await;
    assert_eq!(nonusage.0, 400);
    assert!(nonusage.1.to_string().contains("UNEXPECTED_RATING_POLICY"));
    assert_eq!(Script::count(&script.reserve_calls), 0);
}

#[test]
fn items_prices_and_entry_patch_refuse_all_policy_fields() {
    use bss_pricing::api::rest::authoring::dto::*;
    for key in [
        "usage_rating_policy",
        "usage_policy_id",
        "usage_policy_version",
        "usage_policy_digest",
    ] {
        let mut body = json!({"sku_id":Uuid::new_v4(),"price_book_entry_id":Uuid::new_v4()});
        body[key] = policy();
        assert!(serde_json::from_value::<PricingPlanItemCreate>(body).is_err());
        let mut body = json!({"price_book_entry_id":Uuid::new_v4()});
        body[key] = Value::Null;
        assert!(serde_json::from_value::<PricingPlanItemPatch>(body).is_err());
        let mut body = json!({});
        body[key] = Value::Null;
        assert!(serde_json::from_value::<PricingPriceBookEntryPatch>(body.clone()).is_err());
        assert!(serde_json::from_value::<PricingPricePatch>(body.clone()).is_err());
        body["price"] = json!({"rate":"1"});
        body["eligibility"] = json!("all");
        body["effective_from"] = json!("2026-09-01");
        assert!(serde_json::from_value::<PricingPriceCreate>(body).is_err());
    }
}

async fn create_entry(f: &Fixture, book: Uuid, sku: Uuid, content: Value) -> Uuid {
    let result = f
        .call(
            "POST",
            &format!("/price-books/{book}/entries"),
            json!({"sku_id":sku,"model":"per_unit","usage_rating_policy":content}),
            None,
            Some(&Uuid::new_v4().to_string()),
        )
        .await;
    assert_eq!(result.0, 201, "{result:?}");
    plan_support::id_of(&result.1["id"])
}

#[tokio::test]
async fn book_switch_matches_policy_and_dimension_and_preserves_unmatched_entries() {
    use bss_products_sdk::models::SkuType;
    use plan_support::{book, id_of, item, items, plan, publish, setup};
    let (f, catalog) = setup().await;
    let sku = catalog.sku(SkuType::Usage);
    let source = book(&f, "SOURCE").await;
    let target = book(&f, "TARGET").await;
    let wrong = book(&f, "WRONG").await;
    let dimension = book(&f, "DIMENSION").await;
    let hourly = create_entry(&f, source, sku, policy()).await;
    let mut monthly = policy();
    monthly["rating_window"] = json!({"kind":"billing_cycle"});
    // Insert the wrong policy first: selection must match content, never first SKU/model.
    create_entry(&f, target, sku, monthly.clone()).await;
    let twin = create_entry(&f, target, sku, policy()).await;
    create_entry(&f, wrong, sku, monthly).await;
    let dimension_twin = create_entry(&f, dimension, sku, policy()).await;
    let epath = format!("/price-book-entries/{dimension_twin}");
    let tag = f.call("GET", &epath, json!({}), None, None).await.2;
    assert_eq!(
        f.call(
            "PATCH",
            &epath,
            json!({"dimension_key":"region"}),
            Some(&tag),
            None
        )
        .await
        .0,
        200
    );
    for (code, destination, expected) in [
        ("EXACT", target, twin),
        ("WRONG", wrong, hourly),
        ("DIM", dimension, hourly),
    ] {
        let (_, revision) = plan(&f, code, source).await;
        item(&f, revision, sku, Some(hourly), "paid").await;
        let path = format!("/plan-revisions/{revision}");
        let tag = f.call("GET", &path, json!({}), None, None).await.2;
        let patched = f
            .call(
                "PATCH",
                &path,
                json!({"book_id":destination}),
                Some(&tag),
                None,
            )
            .await;
        assert_eq!(patched.0, 200, "{patched:?}");
        assert_eq!(
            items(&f, revision).await[0].price_book_entry_id,
            Some(expected)
        );
        if expected == hourly {
            let checks = f
                .call("GET", &format!("{path}/checks"), json!({}), None, None)
                .await;
            assert_eq!(checks.0, 200, "{checks:?}");
            assert!(
                checks.1.to_string().contains("ITEM_BOOK_FOREIGN"),
                "{checks:?}"
            );
        }
    }
    // Copy and clone preserve the exact selected entry in the same book.
    let (p, revision) = plan(&f, "CLONESOURCE", source).await;
    item(&f, revision, sku, Some(hourly), "paid").await;
    publish(&f, id_of(&p["id"]), revision).await;
    for (suffix, body) in [
        ("revisions", json!({})),
        ("clone", json!({"code":"CLONED","name":"Cloned"})),
    ] {
        let response = f
            .call(
                "POST",
                &format!("/plans/{}/{suffix}", p["id"].as_str().unwrap()),
                body,
                None,
                Some(suffix),
            )
            .await;
        assert_eq!(response.0, 201, "{response:?}");
        let revision = if suffix == "clone" {
            id_of(&response.1["revisions"][0]["id"])
        } else {
            id_of(&response.1["id"])
        };
        assert_eq!(
            items(&f, revision).await[0].price_book_entry_id,
            Some(hourly)
        );
    }
}

struct RecoveryClock(time::OffsetDateTime);
impl bss_pricing::infra::reference_work::Clock for RecoveryClock {
    fn now(&self) -> time::OffsetDateTime {
        self.0
    }
}

#[tokio::test]
async fn crash_windows_preserve_policy_input_and_confirmed_receipt() {
    use bss_pricing::infra::{
        reference_ticker::Ticker,
        reference_work::{Target, Work},
        storage::repo::{price_book_entry_repo, reference_op_repo},
    };
    use toolkit_db::secure::AccessScope;
    for mode in [1, 2] {
        let script = Arc::new(Script::default());
        let f = Fixture::new(script.clone()).await;
        let (book, _) = f.book().await;
        let path = format!("/price-books/{}/entries", book["id"].as_str().unwrap());
        let body =
            json!({"sku_id":Uuid::new_v4(),"model":"per_unit","usage_rating_policy":policy()});
        script.set(mode);
        let mut door = Box::pin(f.call("POST", &path, body.clone(), None, Some("crash")));
        tokio::select! { result = &mut door => panic!("did not park: {result:?}"), () = script.parked.notified() => {} }
        drop(door);
        let now = time::OffsetDateTime::now_utc() + time::Duration::days(2);
        let scope = AccessScope::for_tenant(f.ctx.subject_tenant_id());
        let ops = reference_op_repo::due(&f.db.conn().unwrap(), &scope, now, 10)
            .await
            .unwrap();
        assert_eq!(ops.len(), 1);
        let Target::PriceBookEntry { input, .. } = Work::read(&ops[0]).unwrap().target else {
            panic!("entry op")
        };
        assert_eq!(input.schema_version, Some(1));
        assert_eq!(
            serde_json::to_value(input.usage_rating_policy).unwrap(),
            policy()
        );
        let before = price_book_entry_repo::find(
            &f.db.conn().unwrap(),
            &scope,
            f.ctx.subject_tenant_id(),
            ops[0].ref_id,
        )
        .await
        .unwrap();
        assert_eq!(before.is_some(), mode == 2);
        script.set(0);
        Ticker::new(f.state.clone(), Arc::new(RecoveryClock(now)), 10, 100)
            .tick()
            .await
            .unwrap();
        let result = f
            .call("POST", &path, body.clone(), None, Some("crash"))
            .await;
        assert_eq!(result.0, 201, "{result:?}");
        assert_eq!(result.1["usage_rating_policy"]["content"], policy());
        if let Some(before) = before {
            assert_eq!(
                result.1["usage_rating_policy"]["policy_id"],
                before.usage_policy_id.unwrap().to_string()
            );
        }
        let calls = Script::count(&script.reserve_calls);
        script
            .skus_down
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            f.call("POST", &path, body, None, Some("crash")).await,
            result
        );
        assert_eq!(Script::count(&script.reserve_calls), calls);
    }
}

#[tokio::test]
async fn legacy_published_usage_upgrades_without_inventing_policy_and_keeps_resolving() {
    use bss_pricing::{infra::storage::repo::price_book_entry_repo, module::BssPricingGear};
    use bss_products_sdk::models::SkuType;
    use plan_support::entry_support::TestDsn;
    use plan_support::{Catalog, book, id_of, plan, publish, scope};
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    use toolkit::contracts::DatabaseCapability;
    use toolkit_db::migration_runner::run_migrations_for_testing;
    let dsn = TestDsn::new("pricing-legacy-policy-");
    let db = toolkit_db::connect_db(&dsn, toolkit_db::ConnectOpts::default())
        .await
        .unwrap();
    let prior = BssPricingGear::default()
        .migrations()
        .into_iter()
        .filter(|m| m.name() != "m20260930_000018_usage_rating_policy")
        .collect();
    run_migrations_for_testing(&db, prior).await.unwrap();
    let catalog = Arc::new(Catalog::default());
    let sku = catalog.sku(SkuType::Usage);
    let f = Fixture::on(
        toolkit_db::DBProvider::new(db),
        Uuid::new_v4(),
        dsn,
        catalog,
    )
    .await;
    let book_id = book(&f, "LEGACY").await;
    let (p, revision) = plan(&f, "LEGACY", book_id).await;
    let entry = Uuid::new_v4();
    let price = Uuid::new_v4();
    let tenant = f.ctx.subject_tenant_id();
    let now = time::OffsetDateTime::now_utc();
    let raw = Database::connect(&f.dsn).await.unwrap();
    raw.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO pricing_price_book_entry (id,tenant_id,book_id,sku_id,charge_kind,model,reservation_id,reference_state,version,created_at,updated_at) VALUES (?,?,?,?,'usage','per_unit',?,'confirmed',1,?,?)",
        vec![entry.into(),tenant.into(),book_id.into(),sku.into(),Uuid::new_v4().into(),now.into(),now.into()])).await.unwrap();
    raw.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO pricing_plan_item (id,tenant_id,revision_id,sku_id,price_book_entry_id,treatment,reservation_id,reference_state,version,created_by,created_at,updated_at) VALUES (?,?,?,?,?,'paid',?,'confirmed',1,?,?,?)",
        vec![Uuid::new_v4().into(),tenant.into(),revision.into(),sku.into(),entry.into(),Uuid::new_v4().into(),f.ctx.subject_id().into(),now.into(),now.into()])).await.unwrap();
    raw.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO pricing_price (id,tenant_id,price_book_entry_id,version_no,price_json,eligibility,effective_from,state,created_by,version,created_at,updated_at) VALUES (?,?,?,1,?,'all','2026-09-01','approved',?,1,?,?)",
        vec![price.into(),tenant.into(),entry.into(),json!({"rate":"1"}).to_string().into(),f.ctx.subject_id().into(),now.into(),now.into()])).await.unwrap();
    publish(&f, id_of(&p["id"]), revision).await;
    run_migrations_for_testing(&f.db.db(), BssPricingGear::default().migrations())
        .await
        .unwrap();
    let stored = price_book_entry_repo::find(&f.db.conn().unwrap(), &scope(&f), tenant, entry)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            stored.usage_policy_id,
            stored.usage_policy_version,
            stored.usage_policy_digest
        ),
        (None, None, None)
    );
    let read = f
        .call(
            "GET",
            &format!("/price-book-entries/{entry}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(read.0, 200, "{read:?}");
    assert_eq!(read.1["usage_rating_policy"], Value::Null);
    let resolved = f
        .call(
            "GET",
            &format!("/resolve?plan_revision_id={revision}&date=2026-10-01"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(resolved.0, 200, "{resolved:?}");
    assert!(
        resolved.1.to_string().contains(&price.to_string()),
        "{resolved:?}"
    );
    // A legacy null-policy source cannot switch to a modern policy-bearing entry implicitly.
    let target = book(&f, "MODERN").await;
    create_entry(&f, target, sku, policy()).await;
    let copied = f
        .call(
            "POST",
            &format!("/plans/{}/revisions", p["id"].as_str().unwrap()),
            json!({}),
            None,
            Some("copy"),
        )
        .await;
    assert_eq!(copied.0, 201, "{copied:?}");
    let draft = id_of(&copied.1["id"]);
    let patched = f
        .call(
            "PATCH",
            &format!("/plan-revisions/{draft}"),
            json!({"book_id":target}),
            Some(&copied.2),
            None,
        )
        .await;
    assert_eq!(patched.0, 200, "{patched:?}");
    assert_eq!(
        plan_support::items(&f, draft).await[0].price_book_entry_id,
        Some(entry)
    );
    // Null-policy uniqueness remains enforced by the normalized-period key.
    assert!(raw.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO pricing_price_book_entry (id,tenant_id,book_id,sku_id,charge_kind,model,reservation_id,reference_state,version,created_at,updated_at) VALUES (?,?,?,?,'usage','per_unit',?,'confirmed',1,?,?)",
        vec![Uuid::new_v4().into(),tenant.into(),book_id.into(),sku.into(),Uuid::new_v4().into(),now.into(),now.into()])).await.is_err());
    assert!(
        raw.query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA foreign_key_check"
        ))
        .await
        .unwrap()
        .is_empty()
    );
}

#[tokio::test]
async fn database_rejects_partial_foreign_and_mismatched_references_and_policy_mutation() {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let f = Fixture::new(Arc::new(Script::default())).await;
    let (book, _) = f.book().await;
    let entry = create_entry(
        &f,
        plan_support::id_of(&book["id"]),
        Uuid::new_v4(),
        policy(),
    )
    .await;
    let raw = Database::connect(&f.dsn).await.unwrap();
    for assignment in [
        "usage_policy_id = NULL",
        "usage_policy_version = NULL",
        "usage_policy_digest = NULL",
        "usage_policy_digest = ''",
        "usage_policy_digest = 'bad'",
        "usage_policy_version = 2",
    ] {
        assert!(
            raw.execute_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                format!("UPDATE pricing_price_book_entry SET {assignment} WHERE id = ?"),
                [entry.into()]
            ))
            .await
            .is_err(),
            "{assignment}"
        );
    }
    assert!(
        raw.execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "UPDATE pricing_usage_rating_policy SET content = '{}'"
        ))
        .await
        .is_err()
    );
    assert!(
        raw.execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM pricing_usage_rating_policy"
        ))
        .await
        .is_err()
    );
    let foreign = Uuid::new_v4();
    assert!(
        raw.execute_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE pricing_price_book_entry SET tenant_id = ? WHERE id = ?",
            [foreign.into(), entry.into()]
        ))
        .await
        .is_err()
    );
    // The same canonical content in another tenant has its own policy identity.
    let other = Fixture::on(
        f.db.clone(),
        foreign,
        f.dsn.clone(),
        Arc::new(Script::default()),
    )
    .await;
    let (other_book, _) = other.book().await;
    let other_entry = create_entry(
        &other,
        plan_support::id_of(&other_book["id"]),
        Uuid::new_v4(),
        policy(),
    )
    .await;
    let first = f
        .call(
            "GET",
            &format!("/price-book-entries/{entry}"),
            json!({}),
            None,
            None,
        )
        .await;
    let second = other
        .call(
            "GET",
            &format!("/price-book-entries/{other_entry}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_ne!(
        first.1["usage_rating_policy"]["policy_id"],
        second.1["usage_rating_policy"]["policy_id"]
    );
    assert_eq!(
        first.1["usage_rating_policy"]["digest"],
        second.1["usage_rating_policy"]["digest"]
    );
    assert_eq!(
        other
            .call(
                "GET",
                &format!("/price-book-entries/{entry}"),
                json!({}),
                None,
                None
            )
            .await
            .0,
        404
    );
}

#[tokio::test]
async fn a_digest_collision_is_an_integrity_error_not_content_reuse() {
    use bss_pricing::infra::{
        storage::repo::usage_policy_repo,
        usage_policy_wire::{UsageRatingPolicyInput, digest_text},
    };
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let f = Fixture::new(Arc::new(Script::default())).await;
    let input: UsageRatingPolicyInput = serde_json::from_value(policy()).unwrap();
    let digest = digest_text(bss_pricing_sdk::digest::policy_digest(&(&input).into()));
    let mut corrupt = policy();
    corrupt["aggregation_scope"] = json!("resource");
    let raw = Database::connect(&f.dsn).await.unwrap();
    raw.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO pricing_usage_rating_policy (tenant_id,policy_id,version,digest,content,created_at,created_by) VALUES (?,?,1,?,?,?,?)",
        vec![f.ctx.subject_tenant_id().into(),Uuid::new_v4().into(),digest.into(),corrupt.to_string().into(),time::OffsetDateTime::now_utc().into(),f.ctx.subject_id().into()])).await.unwrap();
    let error = usage_policy_repo::intern(
        &f.db.conn().unwrap(),
        &plan_support::scope(&f),
        f.ctx.subject_tenant_id(),
        f.ctx.subject_id(),
        &input,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        bss_pricing::infra::storage::RepoError::CorruptRow(_)
    ));
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_policy_authoring_uses_the_same_content_key_and_receipt() {
    let pg = pg_support::Pg::applied().await;
    let f = Fixture::on(
        toolkit_db::DBProvider::new(pg.db().await),
        Uuid::new_v4(),
        plan_support::entry_support::TestDsn::of(pg.url(true)),
        Arc::new(Script::default()),
    )
    .await;
    authoring_case(f).await;
}

#[tokio::test]
async fn only_unversioned_persisted_creates_can_recover_without_a_policy() {
    use bss_pricing::{
        domain::reference_op::{OpKind, RefKind},
        infra::{
            reference_work::{self, Caller, EntryInput, Ref, Target, WallClock, Work},
            storage::repo::{price_book_entry_repo, reference_op_repo},
        },
    };
    use bss_products_sdk::{ReferenceRegistryV1, models::ReferenceKind};
    for versioned in [false, true] {
        let script = Arc::new(Script::default());
        let f = Fixture::new(script.clone()).await;
        let (book, _) = f.book().await;
        let sku = Uuid::new_v4();
        let id = Uuid::new_v4();
        let mut persisted = json!({"sku_id":sku,"model":"per_unit","period":null,"dimension_key":null,"invoice_line_override":null});
        if versioned {
            persisted["schema_version"] = json!(1);
        }
        let input: EntryInput = serde_json::from_value(persisted).unwrap();
        let work = Work {
            target: Target::PriceBookEntry {
                book_id: plan_support::id_of(&book["id"]),
                input,
            },
            correlation: Uuid::new_v4(),
            refusal: None,
            receipt: None,
            outcome: None,
        };
        let reservation = script
            .reserve(
                &f.ctx,
                f.ctx.subject_tenant_id(),
                sku,
                ReferenceKind::PriceBookEntry,
                id,
            )
            .await
            .unwrap();
        let op = reference_work::new_op(
            &f.ctx,
            Ref {
                kind: RefKind::Entry,
                id,
                sku_id: sku,
            },
            &work,
            OpKind::Create,
            Some(reservation.reservation_id),
            None,
            time::OffsetDateTime::now_utc() - time::Duration::days(1),
        )
        .unwrap();
        let op_id = op.op_id;
        reference_op_repo::insert(&f.db.conn().unwrap(), &plan_support::scope(&f), op)
            .await
            .unwrap();
        let receipt =
            reference_work::drive(&f.state, &f.ctx, op_id, Arc::new(WallClock), Caller::Ticker)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            receipt.status,
            if versioned { 400 } else { 201 },
            "{}",
            receipt.body
        );
        let entry = price_book_entry_repo::find(
            &f.db.conn().unwrap(),
            &plan_support::scope(&f),
            f.ctx.subject_tenant_id(),
            id,
        )
        .await
        .unwrap();
        if versioned {
            assert!(entry.is_none());
            assert!(receipt.body.contains("MISSING_RATING_POLICY"));
        } else {
            let entry = entry.unwrap();
            assert_eq!(
                (
                    entry.usage_policy_id,
                    entry.usage_policy_version,
                    entry.usage_policy_digest
                ),
                (None, None, None)
            );
            assert_eq!(
                serde_json::from_str::<Value>(&receipt.body).unwrap()["usage_rating_policy"],
                Value::Null
            );
        }
    }
}

#[tokio::test]
async fn rereserve_and_delete_retain_the_original_immutable_policy() {
    use bss_pricing::infra::{
        reference_work::{self, Caller, Target, WallClock, Work},
        storage::repo::{price_book_entry_repo, reference_op_repo, usage_policy_repo},
    };
    use bss_products_sdk::ReferenceRegistryV1;
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let script = Arc::new(Script::default());
    let f = Fixture::new(script.clone()).await;
    let (book, _) = f.book().await;
    let id = create_entry(
        &f,
        plan_support::id_of(&book["id"]),
        Uuid::new_v4(),
        policy(),
    )
    .await;
    let scope = plan_support::scope(&f);
    let tenant = f.ctx.subject_tenant_id();
    let original = price_book_entry_repo::find(&f.db.conn().unwrap(), &scope, tenant, id)
        .await
        .unwrap()
        .unwrap();
    let reference = Some((
        original.usage_policy_id.unwrap(),
        original.usage_policy_version.unwrap(),
        original.usage_policy_digest.clone().unwrap(),
    ));
    let now = time::OffsetDateTime::now_utc();
    script
        .release(&f.ctx, tenant, original.reservation_id)
        .await
        .unwrap();
    let op = reference_work::rereserve_op(&f.ctx, &original, now, now).unwrap();
    let Target::PriceBookEntry { input, .. } = Work::read(&op).unwrap().target else {
        panic!("entry op")
    };
    assert_eq!(input.usage_policy_reference, reference);
    let op_id = op.op_id;
    reference_op_repo::insert(&f.db.conn().unwrap(), &scope, op)
        .await
        .unwrap();
    reference_work::drive(&f.state, &f.ctx, op_id, Arc::new(WallClock), Caller::Door)
        .await
        .unwrap();
    let after = price_book_entry_repo::find(&f.db.conn().unwrap(), &scope, tenant, id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(after.reservation_id, original.reservation_id);
    assert_eq!(
        (
            after.usage_policy_id,
            after.usage_policy_version,
            after.usage_policy_digest
        ),
        (
            original.usage_policy_id,
            original.usage_policy_version,
            original.usage_policy_digest.clone()
        )
    );
    assert_eq!(
        f.call(
            "DELETE",
            &format!("/price-book-entries/{id}"),
            json!({}),
            None,
            None
        )
        .await
        .0,
        204
    );
    let ops = plan_support::ops_for(&f, id).await;
    let deleted = ops.iter().find(|o| o.kind == "delete").unwrap();
    let Target::PriceBookEntry { input, .. } = Work::read(deleted).unwrap().target else {
        panic!("entry op")
    };
    assert_eq!(input.usage_policy_reference, reference);
    let remaining = usage_policy_repo::for_entries(&f.db.conn().unwrap(), tenant, &[original])
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&remaining[&id].content).unwrap(),
        policy()
    );
    // No entry references this policy now; its delete is still forbidden by append-only storage.
    let raw = Database::connect(&f.dsn).await.unwrap();
    assert!(
        raw.execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DELETE FROM pricing_usage_rating_policy"
        ))
        .await
        .is_err()
    );
}
