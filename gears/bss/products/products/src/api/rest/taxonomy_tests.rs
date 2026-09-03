//! `02`'s four doors, exercised through the router.
//!
//! # Every ceiling case reads the configured number, never a literal
//!
//! The caps are **P-D-107 arm 1**'s interim values and the NFR workshop
//! overrides them by configuration. A case asserting `50` would redden the
//! day the workshop rules, for no reason a reader could connect to the rule
//! under test, so each case derives its fixture from `ProductsConfig`'s own
//! field.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use serde_json::{Value as JsonValue, json};
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use tower::ServiceExt;
use uuid::Uuid;

use sea_orm_migration::MigratorTrait;

use super::router;
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::domain::taxonomy::DefinitionState;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo;
use crate::test_support::{authed_ctx, flat_in_enforcer};

const TENANT: Uuid = Uuid::from_u128(0x7e_44);
const BRAND: Uuid = Uuid::from_u128(0xb1_02);

struct TestHarness {
    dsn: String,
    db: DBProvider<DbError>,
    outbox: Arc<Outbox>,
    #[allow(dead_code)]
    _outbox_handle: OutboxHandle,
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        if let Some(rest) = self.dsn.strip_prefix("sqlite://") {
            let path = rest.split('?').next().unwrap_or(rest);
            std::fs::remove_file(path).ok();
        }
    }
}

async fn harness() -> TestHarness {
    let path = std::env::temp_dir().join(format!(
        "bss-products-taxonomy-door-tests-{}.sqlite3",
        Uuid::new_v4()
    ));
    let dsn = format!("sqlite://{}?mode=rwc", path.display());
    let opts = ConnectOpts {
        max_conns: Some(1),
        min_conns: Some(1),
        ..Default::default()
    };
    let db = connect_db(&dsn, opts)
        .await
        .expect("connect the file-backed sqlite mirror");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run this gear's own migrator");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX)
            .expect("OUTBOX_TABLE_PREFIX is a fixed, valid identifier"),
    )
    .await
    .expect("run the outbox facility's own migrator");
    let outbox_handle = Outbox::builder(db.clone())
        .table_prefix(events::OUTBOX_TABLE_PREFIX)
        .expect("OUTBOX_TABLE_PREFIX is a fixed, valid identifier")
        .queue(events::QUEUE_NAME, Partitions::of(events::PARTITIONS))
        .leased(events::PendingBrokerProducer)
        .start()
        .await
        .expect("start the outbox pipeline");
    let outbox = Arc::clone(outbox_handle.outbox());
    TestHarness {
        dsn,
        db: DBProvider::<DbError>::new(db),
        outbox,
        _outbox_handle: outbox_handle,
    }
}

fn app(harness: &TestHarness) -> Router {
    app_with_caps(
        harness,
        crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
    )
}

/// The same router with the ceilings tightened.
///
/// The configured fan-out is a thousand, so a door case proving the rule
/// fires would have to make a thousand and one categories -- a minute of test
/// time to assert a comparison. Tightening the **configuration** is the same
/// rule on the same path with a fixture a reader can hold, and it is what the
/// caps being configuration is for.
fn app_with_caps(harness: &TestHarness, caps: crate::api::rest::TaxonomyCaps) -> Router {
    let state = Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        taxonomy_caps: caps,
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: ProductsConfig::default().bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: ProductsConfig::default()
            .bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: ProductsConfig::default().watermark_skew_tolerance(),
    });
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(TENANT)))
}

async fn send(
    app: Router,
    method: &str,
    uri: &str,
    body: &JsonValue,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(uri)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(authed_ctx(TENANT))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn body_json(response: axum::http::Response<Body>) -> JsonValue {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read the body"),
    )
    .expect("the body is JSON")
}

/// The code of a **409**, which the conflict class puts in `context.reason`
/// rather than in a violation -- `error_mapping`'s `aborted(..)` shape, not
/// its `precondition(..)` one. Two readers because the envelope has two
/// places, and reading the wrong one answers an empty string on a body that
/// carried the code correctly.
async fn conflict_code(response: axum::http::Response<Body>) -> String {
    let body = body_json(response).await;
    body["context"]["reason"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

async fn error_code(response: axum::http::Response<Body>) -> String {
    let body = body_json(response).await;
    body["context"]["violations"][0]["type"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn now() -> chrono::DateTime<chrono::Utc> {
    crate::domain::canonical::write_instant(chrono::Utc::now())
}

/// Seed a live Product head, the operand the metadata door needs.
async fn seed_product(harness: &TestHarness) -> Uuid {
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let product_id = Uuid::now_v7();
    repo::insert_product(
        &conn,
        &scope,
        repo::NewProduct {
            product_id,
            tenant_id: TENANT,
            brand_id: BRAND,
            name: "Fibre 500".to_owned(),
            name_normalized: "fibre 500".to_owned(),
            product_code: None,
            region_scope: String::new(),
            brand_scope: String::new(),
            created_by: "principal:alice".to_owned(),
            created_at: now(),
            cloned_from: None,
            cloned_from_version: None,
        },
    )
    .await
    .expect("seed a product head");
    product_id
}

async fn seed_definition(harness: &TestHarness, key: &str) -> Uuid {
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let definition_id = Uuid::now_v7();
    repo::insert_attribute_definition(
        &conn,
        &scope,
        repo::NewAttributeDefinition {
            tenant_id: TENANT,
            definition_id,
            key,
            value_type: "string",
            localized: false,
            region_scope: "",
            brand_scope: "",
            seeded_by: None,
        },
        now(),
    )
    .await
    .expect("seed a definition");
    definition_id
}

async fn create_category(app: Router, name: &str, parent: Option<Uuid>) -> JsonValue {
    let response = send(
        app,
        "POST",
        "/bss-products/v1/categories",
        &json!({ "name": name, "parent_id": parent }),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    body_json(response).await
}

// ---------------------------------------------------------------- metadata

/// **The merge is per key: an absent key is untouched and a `null` removes
/// one.**
///
/// All three arms in one case, because a door that replaced the whole map
/// would pass the set arm alone, and one that ignored `null` would pass the
/// untouched arm alone.
#[tokio::test]
async fn the_metadata_merge_sets_leaves_and_removes_per_key() {
    let h = harness().await;
    let product = seed_product(&h).await;
    let uri = format!("/bss-products/v1/products/{product}/metadata");

    let first = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({ "entries": { "owner": "team-a", "tier": "gold" } }),
    )
    .await;
    assert_eq!(first.status(), axum::http::StatusCode::OK);

    let second = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({ "entries": { "owner": null, "note": "migrated" } }),
    )
    .await;
    assert_eq!(second.status(), axum::http::StatusCode::OK);
    let body = body_json(second).await;

    assert!(body["entries"]["owner"].is_null(), "removed: {body}");
    assert_eq!(body["entries"]["tier"], "gold", "untouched: {body}");
    assert_eq!(body["entries"]["note"], "migrated", "set: {body}");
}

/// **A map standing at the key cap can still be reduced** — the test
/// `dod-metadata-door` names in as many words.
///
/// The cap is read from configuration, and the reduction is asserted through
/// the door rather than the store, because it is the door that judges the
/// ceiling: judging the **request** instead of the map the merge would leave
/// is precisely the bug that leaves a full map with no exit.
#[tokio::test]
async fn a_map_at_the_key_cap_can_still_be_reduced() {
    let h = harness().await;
    let product = seed_product(&h).await;
    let uri = format!("/bss-products/v1/products/{product}/metadata");
    let cap = ProductsConfig::default().metadata_max_keys;

    let mut full = serde_json::Map::new();
    for i in 0..cap {
        full.insert(format!("k{i}"), json!("v"));
    }
    let filled = send(app(&h), "PATCH", &uri, &json!({ "entries": full })).await;
    assert_eq!(
        filled.status(),
        axum::http::StatusCode::OK,
        "the cap itself is admitted"
    );

    let over = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({ "entries": { "one-too-many": "v" } }),
    )
    .await;
    assert_eq!(over.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(error_code(over).await, "METADATA_LIMIT");

    let reduced = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({ "entries": { "k0": null } }),
    )
    .await;
    assert_eq!(
        reduced.status(),
        axum::http::StatusCode::OK,
        "a full map has an exit, which is what the per-key merge is for"
    );
}

/// **The key-length and value-length ceilings each refuse**, and each names
/// which one it was: an operator told only *"a cap was exceeded"* cannot tell
/// a key problem from a value problem, and the two have different fixes.
#[tokio::test]
async fn the_byte_ceilings_refuse_and_say_which() {
    let h = harness().await;
    let product = seed_product(&h).await;
    let uri = format!("/bss-products/v1/products/{product}/metadata");
    let cfg = ProductsConfig::default();

    let long_key = "k".repeat(cfg.metadata_max_key_bytes as usize + 1);
    let over_key = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({ "entries": { long_key: "v" } }),
    )
    .await;
    assert_eq!(over_key.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = body_json(over_key).await;
    assert_eq!(body["context"]["violations"][0]["type"], "METADATA_LIMIT");
    assert!(
        body["context"]["violations"][0]["description"]
            .as_str()
            .unwrap_or_default()
            .contains("key"),
        "the refusal names the key ceiling: {body}"
    );

    let long_value = "v".repeat(cfg.metadata_max_value_bytes as usize + 1);
    let over_value = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({ "entries": { "k": long_value } }),
    )
    .await;
    assert_eq!(over_value.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(error_code(over_value).await, "METADATA_LIMIT");
}

/// **The ceilings count bytes, not characters.**
///
/// The sibling case above builds its over-long strings with `"k".repeat(n)`,
/// which is ASCII — so bytes and characters are the same number and an
/// implementation using `chars().count()` would satisfy it. This one is under
/// the ceiling by character count and over it by bytes, so it fails on
/// `chars().count()` and passes on `len()`.
///
/// Ported from a domain rule that briefly held this property and was removed
/// when this door made it dead: it was the one thing the door's own tests did
/// not pin.
#[tokio::test]
async fn the_byte_ceilings_count_bytes_and_not_characters() {
    let h = harness().await;
    let product = seed_product(&h).await;
    let uri = format!("/bss-products/v1/products/{product}/metadata");
    let cfg = ProductsConfig::default();

    // Three bytes per character, so half the cap in characters is over it in
    // bytes. Escaped because `clippy::non_ascii_literal` is denied here.
    // `div_ceil` rather than `/`: `clippy::integer_division` is denied here,
    // and the `+ 1` keeps it over the ceiling when the cap divides evenly.
    let per_char = 3_usize;
    let chars = (cfg.metadata_max_value_bytes as usize).div_ceil(per_char) + 1;
    let value = "\u{20ac}".repeat(chars);
    assert!(
        value.chars().count() <= cfg.metadata_max_value_bytes as usize,
        "the premise: under the ceiling by character count"
    );
    assert!(
        value.len() > cfg.metadata_max_value_bytes as usize,
        "and over it by bytes"
    );

    let refused = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({ "entries": { "k": value } }),
    )
    .await;
    assert_eq!(refused.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(error_code(refused).await, "METADATA_LIMIT");
}

/// **An entity that is not there is a 404, not a silent write.**
///
/// The first shape of this door read the head's state, filtered it for
/// terminality and read `None` as "nothing to refuse", which would have
/// landed a metadata row keyed on an id nothing owns.
#[tokio::test]
async fn a_metadata_write_to_no_entity_is_not_found() {
    let h = harness().await;
    let uri = format!("/bss-products/v1/products/{}/metadata", Uuid::now_v7());

    let response = send(app(&h), "PATCH", &uri, &json!({ "entries": { "k": "v" } })).await;

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

// ------------------------------------------------------- category live value

/// **A stale token is `STALE_CATEGORY_TOKEN`, and this slice's own code.**
///
/// Not 01's `STALE_REVISION`, which names an entity head, and not the
/// envelope's `STALE_LIVE_OP`: the door is non-material and its precondition
/// is `products_category.mutation_seq`.
#[tokio::test]
async fn a_stale_category_token_is_refused_with_this_slices_own_code() {
    let h = harness().await;
    let category = create_category(app(&h), "Hardware", None).await;
    let category_id = category["category_id"].as_str().expect("id").to_owned();
    let definition = seed_definition(&h, "displayName").await;
    let uri = format!("/bss-products/v1/categories/{category_id}/attribute-values");

    let stale = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({
            "expected_seq": 99,
            "values": [{ "definition_id": definition, "locale": "", "region": "", "brand": "", "value": "Kit" }]
        }),
    )
    .await;

    // 409 — `design/02` §3.3's status for the door's own precondition. The
    // `Validation` route this rode until 2026-09-03 rendered it 400.
    assert_eq!(stale.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(conflict_code(stale).await, "STALE_CATEGORY_TOKEN");
}

/// **One act, one bump** — `mutation_seq` counts acts and not row writes
/// (P-D-50), so a patch carrying three coordinates moves the token by one.
///
/// A per-row bump would make the number a caller reads back unusable as the
/// next request's precondition, which is the only thing it is for.
#[tokio::test]
async fn three_coordinates_in_one_patch_move_the_token_by_one() {
    let h = harness().await;
    let category = create_category(app(&h), "Hardware", None).await;
    let category_id = category["category_id"].as_str().expect("id").to_owned();
    let definition = seed_definition(&h, "displayName").await;
    let uri = format!("/bss-products/v1/categories/{category_id}/attribute-values");

    let response = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({
            "expected_seq": 0,
            "values": [
                { "definition_id": definition, "locale": "", "region": "", "brand": "", "value": "Kit" },
                { "definition_id": definition, "locale": "de-DE", "region": "", "brand": "", "value": "Zubehor" },
                { "definition_id": definition, "locale": "fr-FR", "region": "", "brand": "", "value": "Materiel" }
            ]
        }),
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["mutation_seq"], 1, "one act, one bump: {body}");
}

/// **A value against a deprecated definition is refused**, which is the
/// defect P-D-107 arm 2 named: the live-value door runs the four value rules,
/// and without `AttributeDefinitionActive` a category value would be admitted
/// against a definition the removal guard counts as live.
#[tokio::test]
async fn a_value_against_a_deprecated_definition_is_refused() {
    let h = harness().await;
    let category = create_category(app(&h), "Hardware", None).await;
    let category_id = category["category_id"].as_str().expect("id").to_owned();
    let definition = seed_definition(&h, "displayName").await;
    {
        let conn = h.db.conn().expect("scoped connection");
        let scope = AccessScope::for_tenant(TENANT);
        repo::flip_definition_state(
            &conn,
            &scope,
            TENANT,
            definition,
            repo::DefinitionFlip {
                expected: DefinitionState::Active,
                to: DefinitionState::Deprecated,
            },
            now(),
        )
        .await
        .expect("deprecate it");
    }
    let uri = format!("/bss-products/v1/categories/{category_id}/attribute-values");

    let response = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({
            "expected_seq": 0,
            "values": [{ "definition_id": definition, "locale": "", "region": "", "brand": "", "value": "Kit" }]
        }),
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(
        error_code(response).await,
        "ATTRIBUTE_DEFINITION_DEPRECATED"
    );
}

/// **A terminal entity's metadata is not writable** — `ENTITY_TERMINAL`, and
/// the terminal roster is `repo::TERMINAL_HEAD_STATES` rather than a second
/// copy of it.
#[tokio::test]
async fn a_terminal_entity_refuses_a_metadata_write() {
    let h = harness().await;
    let product = seed_product(&h).await;
    {
        let conn = h.db.conn().expect("scoped connection");
        let scope = AccessScope::for_tenant(TENANT);
        repo::discard_product_head(&conn, &scope, TENANT, product, 1, now())
            .await
            .expect("discard it");
    }

    let response = send(
        app(&h),
        "PATCH",
        &format!("/bss-products/v1/products/{product}/metadata"),
        &json!({ "entries": { "k": "v" } }),
    )
    .await;

    // 409: the conflict class, where the current state refuses the act.
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(conflict_code(response).await, "ENTITY_TERMINAL");
}

/// **The first write of a definition for a category must carry the global
/// coordinate**, and a later narrower write need not.
///
/// `inst-av-category-branch`'s write-time analogue of the publish-time check,
/// and the one rule this door runs that the entity save door does not
/// (P-D-107 arm 2). The positive control is the second half: without it, a
/// door that refused every locale-scoped write would pass.
#[tokio::test]
async fn the_first_write_of_a_definition_needs_its_global_coordinate() {
    let h = harness().await;
    let category = create_category(app(&h), "Hardware", None).await;
    let category_id = category["category_id"].as_str().expect("id").to_owned();
    let definition = seed_definition(&h, "displayName").await;
    let uri = format!("/bss-products/v1/categories/{category_id}/attribute-values");

    let locale_only = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({
            "expected_seq": 0,
            "values": [{ "definition_id": definition, "locale": "de-DE", "region": "", "brand": "", "value": "Zubehor" }]
        }),
    )
    .await;
    assert_eq!(locale_only.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(error_code(locale_only).await, "DEFAULT_LOCALE_MISSING");

    let with_global = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({
            "expected_seq": 0,
            "values": [
                { "definition_id": definition, "locale": "", "region": "", "brand": "", "value": "Kit" },
                { "definition_id": definition, "locale": "de-DE", "region": "", "brand": "", "value": "Zubehor" }
            ]
        }),
    )
    .await;
    assert_eq!(with_global.status(), axum::http::StatusCode::OK);

    // And now that the definition is known for this category, a narrower
    // write on its own is admitted.
    let later = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({
            "expected_seq": 1,
            "values": [{ "definition_id": definition, "locale": "fr-FR", "region": "", "brand": "", "value": "Materiel" }]
        }),
    )
    .await;
    assert_eq!(later.status(), axum::http::StatusCode::OK);
}

/// **A `null` value removes that coordinate**, leaving the others.
#[tokio::test]
async fn a_null_value_removes_one_coordinate() {
    let h = harness().await;
    let category = create_category(app(&h), "Hardware", None).await;
    let category_id: Uuid = category["category_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("uuid");
    let definition = seed_definition(&h, "displayName").await;
    let uri = format!("/bss-products/v1/categories/{category_id}/attribute-values");

    send(
        app(&h),
        "PATCH",
        &uri,
        &json!({
            "expected_seq": 0,
            "values": [
                { "definition_id": definition, "locale": "", "region": "", "brand": "", "value": "Kit" },
                { "definition_id": definition, "locale": "de-DE", "region": "", "brand": "", "value": "Zubehor" }
            ]
        }),
    )
    .await;

    let removed = send(
        app(&h),
        "PATCH",
        &uri,
        &json!({
            "expected_seq": 1,
            "values": [{ "definition_id": definition, "locale": "de-DE", "region": "", "brand": "", "value": null }]
        }),
    )
    .await;
    assert_eq!(removed.status(), axum::http::StatusCode::OK);

    let conn = h.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let left = repo::attribute_values_of(&conn, &scope, TENANT, "category", category_id)
        .await
        .expect("read back");
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].locale, "", "the global one stands");
}

// ------------------------------------------------------------ category ops

/// **A re-parent that would close a cycle is refused**, judged on a tree read
/// under the writer lock.
#[tokio::test]
async fn a_reparent_that_closes_a_cycle_is_refused() {
    let h = harness().await;
    let root = create_category(app(&h), "Root", None).await;
    let root_id: Uuid = root["category_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("uuid");
    let child = create_category(app(&h), "Child", Some(root_id)).await;
    let child_id: Uuid = child["category_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("uuid");

    let response = send(
        app(&h),
        "POST",
        &format!("/bss-products/v1/categories/{root_id}/operations"),
        &json!({ "op": "reparent", "expected_state": "active", "parent_id": child_id }),
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(error_code(response).await, "TAXONOMY_CYCLE");
}

/// **The envelope's pinned state is re-validated at apply**, a mismatch being
/// `STALE_LIVE_OP` — the world moved between submission and apply.
#[tokio::test]
async fn an_envelope_pinned_to_the_wrong_state_is_stale() {
    let h = harness().await;
    let category = create_category(app(&h), "Hardware", None).await;
    let category_id = category["category_id"].as_str().expect("id").to_owned();

    let response = send(
        app(&h),
        "POST",
        &format!("/bss-products/v1/categories/{category_id}/operations"),
        &json!({ "op": "rename", "expected_state": "retired", "name": "Kit" }),
    )
    .await;

    // 409, not 400: `design/02` §3.3 puts `STALE_LIVE_OP` in the conflict
    // group with `DUPLICATE_CATEGORY_NAME`, `CATEGORY_REFERENCED` and
    // `DEFINITION_IN_USE`, transcribed from its own Problem responses block
    // rather than derived from the 422-architectural class rule.
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(conflict_code(response).await, "STALE_LIVE_OP");
}

/// **A depth past the configured ceiling is refused**, and the ceiling is
/// read from configuration rather than written here.
#[tokio::test]
async fn a_chain_past_the_configured_depth_is_refused() {
    let h = harness().await;
    let depth = ProductsConfig::default().taxonomy_max_depth;
    let mut parent: Option<Uuid> = None;
    // A root sits at depth **0** and `limit_verdict` refuses on
    // `depth > allowed`, so the deepest admitted node is at depth `allowed`
    // and it takes `allowed + 1` creates to reach it. The first shape of this
    // case made `allowed` of them, landed one short of the ceiling and passed
    // 201 -- an off-by-one that would have read as "the limit does not fire".
    for level in 0..=depth {
        let made = create_category(app(&h), &format!("Level {level}"), parent).await;
        parent = Some(
            made["category_id"]
                .as_str()
                .expect("id")
                .parse()
                .expect("uuid"),
        );
    }

    let over = send(
        app(&h),
        "POST",
        "/bss-products/v1/categories",
        &json!({ "name": "One too deep", "parent_id": parent }),
    )
    .await;

    assert_eq!(over.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(error_code(over).await, "TAXONOMY_LIMIT");
}

/// **A re-parent past the configured depth is refused at the leaves of the
/// moved subtree, not only at the moved node.**
///
/// The case `depth_of` alone cannot see: the moved node lands inside the
/// ceiling and its own children do not. A rule reading the landing place
/// admits this, which is why `subtree_height` exists.
#[tokio::test]
async fn a_reparent_is_judged_at_the_leaves_of_what_it_drags() {
    let h = harness().await;
    let caps = crate::api::rest::TaxonomyCaps {
        max_depth: 2,
        ..crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default())
    };

    // A two-level subtree, rooted at depth 0.
    let top = create_category(app(&h), "Top", None).await;
    let top_id: Uuid = top["category_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("uuid");
    let mid = create_category(app(&h), "Mid", Some(top_id)).await;
    let mid_id: Uuid = mid["category_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("uuid");
    create_category(app(&h), "Leaf", Some(mid_id)).await;

    // And a separate chain whose tail sits at depth 1.
    let other = create_category(app(&h), "Other", None).await;
    let other_id: Uuid = other["category_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("uuid");
    let landing = create_category(app(&h), "Landing", Some(other_id)).await;
    let landing_id: Uuid = landing["category_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("uuid");

    // `Top` would land at depth 2 -- inside a ceiling of 2 -- while `Leaf`
    // would land at 4.
    let response = send(
        app_with_caps(&h, caps),
        "POST",
        &format!("/bss-products/v1/categories/{top_id}/operations"),
        &json!({ "op": "reparent", "expected_state": "active", "parent_id": landing_id }),
    )
    .await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(error_code(response).await, "TAXONOMY_LIMIT");
}

/// **The fan-out ceiling fires on the mutation path**, and the refusal names
/// which limit it was.
///
/// Two probes in one case: the sibling set at the ceiling is admitted, one
/// over is refused. Without the first, a rule that refused every create would
/// pass.
#[tokio::test]
async fn the_fan_out_ceiling_admits_the_ceiling_and_refuses_one_over() {
    let h = harness().await;
    let caps = crate::api::rest::TaxonomyCaps {
        max_children_per_node: 2,
        ..crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default())
    };
    let parent = create_category(app(&h), "Parent", None).await;
    let parent_id: Uuid = parent["category_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("uuid");

    for n in 0..2 {
        let made = send(
            app_with_caps(&h, caps),
            "POST",
            "/bss-products/v1/categories",
            &json!({ "name": format!("Child {n}"), "parent_id": parent_id }),
        )
        .await;
        assert_eq!(
            made.status(),
            axum::http::StatusCode::CREATED,
            "the ceiling itself is admitted"
        );
    }

    let over = send(
        app_with_caps(&h, caps),
        "POST",
        "/bss-products/v1/categories",
        &json!({ "name": "Child 3", "parent_id": parent_id }),
    )
    .await;

    assert_eq!(over.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = body_json(over).await;
    assert_eq!(body["context"]["violations"][0]["type"], "TAXONOMY_LIMIT");
    assert!(
        body["context"]["violations"][0]["description"]
            .as_str()
            .unwrap_or_default()
            .contains("max_children"),
        "the refusal names which ceiling: {body}"
    );
}

/// **Both op doors submit their envelope to the `05` gate**, and the
/// submission is a **separate** obligation from the currency check.
///
/// The registered host is `NoMaterialityPolicyGate`, which authorizes and says
/// so, so no case here can be a refusal — a green door test proves the call
/// happens only if something else would change without it. So the assertion
/// is on the **source**: `submit_to_gate` is called once per op door and
/// nowhere else, and it is the only construction of `GateSubject` in this
/// module. A door that dropped the call would move the count, and the day
/// `05` registers a policy that call is the one that starts refusing.
#[test]
fn every_op_door_submits_its_envelope_to_the_gate() {
    let source = include_str!("taxonomy.rs");
    assert_eq!(
        source
            .matches("submit_to_gate(tenant_id, &op.target)")
            .count(),
        2,
        "one per `operations` door -- the category door and the definition \
         door -- and neither the live-value nor the metadata door, which are \
         non-material and carry no envelope"
    );
    // Matched with its open paren: `submit_to_gate`'s own doc names the seam
    // in prose, and a bare-name scan counts that too -- the same over-match
    // that reddened `ContentPiiBlocked`'s privacy assertion on two innocent
    // structs.
    assert_eq!(
        source.matches("GateSubject::governed_live_op(").count(),
        1,
        "one construction, in `submit_to_gate`: a second would be a second \
         door reaching the ceremony its own way"
    );
    assert!(
        !source.contains("GateSubject::entity_publish"),
        "a live op's subject is a string target and not an `EntityRef` -- \
         `EntityKind` is `Product | Sku` and a category is neither"
    );
}

// ---------------------------------------------------------- definition ops

/// **The state machine walks deprecate → remove → re-list**, and each flip
/// refuses from the wrong state.
#[tokio::test]
async fn the_definition_walks_its_three_flips() {
    let h = harness().await;
    let key = "colour";
    let created = send(
        app(&h),
        "POST",
        "/bss-products/v1/attribute-definitions",
        &json!({ "key": key, "value_type": "string", "localized": false, "region_scope": "", "brand_scope": "" }),
    )
    .await;
    assert_eq!(created.status(), axum::http::StatusCode::CREATED);
    let uri = format!("/bss-products/v1/attribute-definitions/{key}/operations");

    // remove before deprecate is refused: the envelope pins `active` and the
    // flip demands `deprecated`.
    let early = send(
        app(&h),
        "POST",
        &uri,
        &json!({ "op": "remove", "expected_state": "active" }),
    )
    .await;
    assert_eq!(early.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(conflict_code(early).await, "STALE_LIVE_OP");

    for (op, from, to) in [
        ("deprecate", "active", "deprecated"),
        ("remove", "deprecated", "removed"),
        ("relist", "removed", "active"),
    ] {
        let response = send(
            app(&h),
            "POST",
            &uri,
            &json!({ "op": op, "expected_state": from }),
        )
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::OK, "{op}");
        assert_eq!(body_json(response).await["state"], to, "{op}");
    }
}

/// **A definition's display label lands as an attribute value on the
/// definition** (P-D-108 arm 2), not in a column the roster does not have.
///
/// Keyed `entity_kind = 'attribute_definition'`, which is one of the four the
/// tightened CHECK admits — the label edit had no target at all before that
/// decision, the op being unspendable.
#[tokio::test]
async fn a_label_edit_writes_a_value_on_the_definition() {
    let h = harness().await;
    let key = "colour";
    send(
        app(&h),
        "POST",
        "/bss-products/v1/attribute-definitions",
        &json!({ "key": key, "value_type": "string", "localized": false, "region_scope": "", "brand_scope": "" }),
    )
    .await;

    let response = send(
        app(&h),
        "POST",
        &format!("/bss-products/v1/attribute-definitions/{key}/operations"),
        &json!({ "op": "label", "expected_state": "active", "display_label": "Colour" }),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let conn = h.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let record = repo::attribute_definition_by_key(&conn, &scope, TENANT, key)
        .await
        .expect("read it back")
        .expect("it exists");
    let values = repo::attribute_values_of(
        &conn,
        &scope,
        TENANT,
        "attribute_definition",
        record.definition_id,
    )
    .await
    .expect("read the definition's own values");

    assert_eq!(values.len(), 1, "{values:?}");
    assert_eq!(values[0].value, "Colour");
    assert_eq!(
        record.state,
        DefinitionState::Active,
        "a label edit is non-material and moves no state"
    );
}

// ------------------------------------------- the guards that judged nothing

/// **A seeded definition deprecates and never removes** (`dod-well-known-seeds`).
///
/// `seeded_edge` existed and judged nothing: the operations door flipped
/// `deprecated → removed` without asking it, so the `MUST NOT` was a doc
/// comment. Now the door asks, and the answer is the Foundation's
/// `ILLEGAL_FIELD_MUTATION` -- a borrowed code, which is what §7 row 17's
/// measurement recorded for this refusal.
#[tokio::test]
async fn a_seeded_definition_deprecates_and_never_removes() {
    let h = harness().await;
    {
        let conn = h.db.conn().expect("scoped connection");
        let scope = AccessScope::for_tenant(TENANT);
        repo::insert_attribute_definition(
            &conn,
            &scope,
            repo::NewAttributeDefinition {
                tenant_id: TENANT,
                definition_id: Uuid::now_v7(),
                key: "imageUri",
                value_type: "uri",
                localized: false,
                region_scope: "",
                brand_scope: "",
                seeded_by: Some(crate::domain::taxonomy::REGISTRY_SEEDED_BY),
            },
            now(),
        )
        .await
        .expect("seed a well-known definition");
    }
    let uri = "/bss-products/v1/attribute-definitions/imageUri/operations";

    let deprecated = send(
        app(&h),
        "POST",
        uri,
        &json!({ "op": "deprecate", "expected_state": "active" }),
    )
    .await;
    assert_eq!(
        deprecated.status(),
        axum::http::StatusCode::OK,
        "a seed is deprecatable"
    );

    let removed = send(
        app(&h),
        "POST",
        uri,
        &json!({ "op": "remove", "expected_state": "deprecated" }),
    )
    .await;
    assert_eq!(removed.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(conflict_code(removed).await, "ILLEGAL_FIELD_MUTATION");
}

/// **A removal is refused while a non-terminal head carries the value, and
/// admitted once the value is gone** -- `DEFINITION_IN_USE`, 409 (P-D-116
/// rows 5 and 11: one operand, and a `draft` head is in it).
#[tokio::test]
async fn a_removal_is_refused_while_a_draft_carries_the_value() {
    let h = harness().await;
    let product = seed_product(&h).await;
    let definition = seed_definition(&h, "colour").await;
    let coordinate = || repo::AttributeCoordinate {
        entity_kind: "product",
        entity_id: product,
        definition_id: definition,
        locale: "",
        region: "",
        brand: "",
    };
    {
        let conn = h.db.conn().expect("scoped connection");
        let scope = AccessScope::for_tenant(TENANT);
        repo::upsert_attribute_value(&conn, &scope, TENANT, coordinate(), "red", now())
            .await
            .expect("the draft carries a value");
    }
    let uri = "/bss-products/v1/attribute-definitions/colour/operations";

    let deprecated = send(
        app(&h),
        "POST",
        uri,
        &json!({ "op": "deprecate", "expected_state": "active" }),
    )
    .await;
    assert_eq!(deprecated.status(), axum::http::StatusCode::OK);

    let held = send(
        app(&h),
        "POST",
        uri,
        &json!({ "op": "remove", "expected_state": "deprecated" }),
    )
    .await;
    assert_eq!(held.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(conflict_code(held).await, "DEFINITION_IN_USE");

    {
        let conn = h.db.conn().expect("scoped connection");
        let scope = AccessScope::for_tenant(TENANT);
        repo::delete_attribute_value(&conn, &scope, TENANT, coordinate())
            .await
            .expect("the value goes");
    }
    let removed = send(
        app(&h),
        "POST",
        uri,
        &json!({ "op": "remove", "expected_state": "deprecated" }),
    )
    .await;
    assert_eq!(
        removed.status(),
        axum::http::StatusCode::OK,
        "the paired admission: no carrier, no hold"
    );
}

/// **A category delete admits only when no link row names it, in any Product
/// state** (P-D-116 row 21) -- and the retire beside it still admits, which is
/// `dod-retire-delete-guard`'s own "discarded draft" case, here through the
/// door. Before this the delete ran no census at all and the engine's foreign
/// key met the act as a 500.
#[tokio::test]
async fn a_category_delete_is_held_by_a_discarded_products_link_row() {
    let h = harness().await;
    let category = create_category(app(&h), "Legacy", None).await;
    let category_id: Uuid = category["category_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    let product = seed_product(&h).await;
    let uri = format!("/bss-products/v1/categories/{category_id}/operations");
    {
        let conn = h.db.conn().expect("scoped connection");
        let scope = AccessScope::for_tenant(TENANT);
        repo::replace_category_assignments(
            &conn,
            &scope,
            TENANT,
            product,
            &[(
                category_id,
                crate::domain::taxonomy::AssignmentRole::Primary,
            )],
            now(),
        )
        .await
        .expect("file the product under the category");
        crate::infra::storage::repo::discard_product_head(&conn, &scope, TENANT, product, 1, now())
            .await
            .expect("discard the draft");
    }

    let retired = send(
        app(&h),
        "POST",
        &uri,
        &json!({ "op": "retire", "expected_state": "active" }),
    )
    .await;
    assert_eq!(
        retired.status(),
        axum::http::StatusCode::OK,
        "a discarded holder does not block the retire"
    );

    let held = send(
        app(&h),
        "POST",
        &uri,
        &json!({ "op": "delete", "expected_state": "retired" }),
    )
    .await;
    assert_eq!(
        held.status(),
        axum::http::StatusCode::CONFLICT,
        "presence, not state"
    );
    assert_eq!(conflict_code(held).await, "CATEGORY_REFERENCED");

    {
        let conn = h.db.conn().expect("scoped connection");
        let scope = AccessScope::for_tenant(TENANT);
        repo::replace_category_assignments(&conn, &scope, TENANT, product, &[], now())
            .await
            .expect("clear the assignment set");
    }
    let deleted = send(
        app(&h),
        "POST",
        &uri,
        &json!({ "op": "delete", "expected_state": "retired" }),
    )
    .await;
    assert_eq!(
        deleted.status(),
        axum::http::StatusCode::OK,
        "no link row and no child: admitted"
    );
}
