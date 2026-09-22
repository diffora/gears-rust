//! The P-D-90 membership doors, driven over the wire — the add, the four
//! admitted edges, the refused shortcut, the live-op pin, the seeded rule,
//! and the delist refusal with its holders sample
//! (`dod-recognized-set-mechanics`, `dod-unit-delist`).
//!
//! # Why the delist case seeds its holder over SQL
//!
//! The refusal's operand is a **non-terminal published head declaring the
//! unit**, and reaching that state through the doors alone needs the whole
//! create → save-pair → publish path per case. The fixture writes the head
//! directly instead, in single statements the head guard admits — so the
//! trigger is exercised too, and a fixture the guard would refuse cannot
//! silently exist (`poison columns are the missing guards`, the migrations
//! suite's own rule).

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use sea_orm::{ConnectionTrait, Database};
use serde_json::{Value as JsonValue, json};
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use tower::ServiceExt;
use uuid::Uuid;

use sea_orm_migration::MigratorTrait;

use super::router;
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::test_support::{authed_ctx, enqueued_event_count, flat_in_enforcer, raw_string_opt};

/// The file-backed `SQLite` harness — `skus_tests::TestHarness`'s twin; each
/// door-test module owns one because the struct is test-module-private by
/// that file's own design.
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

const TENANT: Uuid = Uuid::from_u128(0x7e_42);

fn unique_sqlite_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "bss-products-recognized-tests-{label}-{}.sqlite3",
        Uuid::new_v4()
    ))
}

async fn harness() -> TestHarness {
    let path = unique_sqlite_path("db");
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

fn app_for(harness: &TestHarness, tenant: Uuid) -> Router {
    let openapi = OpenApiRegistryImpl::new();
    router(state_for(harness), &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

/// The **approvals** door over the same store — the decide route a
/// two-arm-door case has to drive (**P-D-173**). `super::router` is this
/// module's own, which carries the three member routes and nothing else, so
/// a case that decides a unit has to mount slice 05's router beside it.
fn approvals_app_for(harness: &TestHarness, tenant: Uuid) -> Router {
    let openapi = OpenApiRegistryImpl::new();
    crate::api::rest::approvals::router(state_for(harness), &openapi)
        .layer(axum::Extension(flat_in_enforcer(tenant)))
}

fn state_for(harness: &TestHarness) -> Arc<ApiState> {
    Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: ProductsConfig::default().bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: ProductsConfig::default()
            .bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: ProductsConfig::default().watermark_skew_tolerance(),
        reference: crate::api::rest::ReferenceKnobs::from(&ProductsConfig::default()),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        eol_enabled: false,
        usage_type_catalog: crate::test_support::resolved_usage_types(),
        usage_type_catalog_source: "registry",
    })
}

/// [`post_json`] carrying the `If-Match` the two per-member write doors
/// require (**P-D-174**).
async fn post_json_tagged(
    app: Router,
    uri: &str,
    body: &JsonValue,
    tag: &str,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .header(axum::http::header::IF_MATCH, tag)
            .extension(authed_ctx(TENANT))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

/// The member's current `ETag`, read off the door that hands it out — never
/// recomputed in the test, so a case asserts the round trip rather than this
/// file's idea of the digest.
///
/// A member the read door does not answer has no tag, and the fallback is a
/// **well-formed** one no member can carry: a case about an absent member is
/// asking for its `404`, and a blank header would answer the `If-Match`
/// validation instead and hide the question.
async fn tag_of(harness: &TestHarness, kind: &str, code: &str) -> String {
    let response = get_json(
        app_for(harness, TENANT),
        &format!("/bss-products/v1/config/vocabularies/{kind}/values/{code}"),
    )
    .await;
    response
        .headers()
        .get(axum::http::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map_or_else(|| format!("\"{}\"", "0".repeat(64)), str::to_owned)
}

async fn post_json(app: Router, uri: &str, body: &JsonValue) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(authed_ctx(TENANT))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn get_json(app: Router, uri: &str) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(uri)
            .extension(authed_ctx(TENANT))
            .body(Body::empty())
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

/// The in-test approval double every member op needs under the stored host
/// (`dod-recognized-set-mechanics`, P-D-146): one satisfied record for the
/// exact subject the door presents. A case that seeded its own keeps it.
async fn seed_member_op(harness: &TestHarness, tenant: Uuid, kind: &str, code: &str) {
    // An unknown kind is refused by the door before the host is asked; there
    // is nothing to seed for it.
    let Some(kind) = crate::domain::recognized::SetKind::parse(kind) else {
        return;
    };
    crate::test_support::seed_satisfied_approval(
        &harness.db,
        tenant,
        crate::api::rest::recognized_sets::member_op_subject(tenant, kind, code),
        0,
    )
    .await;
}

async fn add_member(
    harness: &TestHarness,
    tenant: Uuid,
    kind: &str,
    code: &str,
) -> axum::http::Response<Body> {
    seed_member_op(harness, tenant, kind, code).await;
    add_member_via(app_for(harness, tenant), kind, code).await
}

/// [`add_member`] over a caller-built router, **without** the seeding double.
async fn add_member_via(app: Router, kind: &str, code: &str) -> axum::http::Response<Body> {
    post_json(
        app,
        &format!("/bss-products/v1/config/vocabularies/{kind}/values"),
        &json!({ "member_code": code }),
    )
    .await
}

async fn transition(
    harness: &TestHarness,
    tenant: Uuid,
    kind: &str,
    code: &str,
    expected: &str,
    to: &str,
) -> axum::http::Response<Body> {
    seed_member_op(harness, tenant, kind, code).await;
    let tag = tag_of(harness, kind, code).await;
    post_json_tagged(
        app_for(harness, tenant),
        &format!("/bss-products/v1/config/vocabularies/{kind}/values/{code}/transitions"),
        &json!({ "to": to, "expected_state": expected }),
        &tag,
    )
    .await
}

async fn relabel(
    harness: &TestHarness,
    tenant: Uuid,
    kind: &str,
    code: &str,
    label: Option<&str>,
) -> axum::http::Response<Body> {
    seed_member_op(harness, tenant, kind, code).await;
    let tag = tag_of(harness, kind, code).await;
    post_json_tagged(
        app_for(harness, tenant),
        &format!("/bss-products/v1/config/vocabularies/{kind}/values/{code}/label"),
        &json!({ "display_label": label }),
        &tag,
    )
    .await
}

/// `PATCH /skus/{id}` declaring `unit` — the save door, whose recognition
/// check is what a removed member must refuse.
async fn patch_sku_meter(
    harness: &TestHarness,
    sku_id: Uuid,
    etag: &str,
    unit: &str,
) -> axum::http::Response<Body> {
    let state = std::sync::Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: ProductsConfig::default().bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: ProductsConfig::default()
            .bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: ProductsConfig::default().watermark_skew_tolerance(),
        reference: crate::api::rest::ReferenceKnobs::from(&ProductsConfig::default()),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        eol_enabled: false,
        usage_type_catalog: crate::test_support::resolved_usage_types(),
        usage_type_catalog_source: "registry",
    });
    let openapi = OpenApiRegistryImpl::new();
    let app = crate::api::rest::skus::router(state, &openapi)
        .layer(axum::Extension(flat_in_enforcer(TENANT)));
    let body = json!({ "metering_unit": unit, "usage_type_ref": "usage:storage" });
    app.oneshot(
        Request::builder()
            .method("PATCH")
            .uri(format!("/bss-products/v1/skus/{sku_id}"))
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .header(axum::http::header::IF_MATCH, etag)
            .extension(authed_ctx(TENANT))
            .body(Body::from(body.to_string()))
            .expect("build the save request"),
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

async fn error_code(response: axum::http::Response<Body>) -> String {
    let body = body_json(response).await;
    body["context"]["reason"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Seed one published SKU head declaring `unit`, in statements the head
/// guard admits.
async fn seed_holder(harness: &TestHarness, sku_code: &str, unit: &str) -> Uuid {
    let conn = Database::connect(&harness.dsn)
        .await
        .expect("open an auxiliary connection");
    let product_id = Uuid::now_v7();
    let sku_id = Uuid::now_v7();
    let now = "2026-08-29T09:00:00.000000Z";
    for sql in [
        format!(
            "INSERT INTO products_product (product_id, tenant_id, brand_id, name, \
             name_normalized, product_code, lifecycle_state, internal_revision, \
             published_version, region_scope, brand_scope, created_by, created_at, updated_at) \
             VALUES (X'{prod}', X'{tenant}', X'{brand}', 'Holder {sku_code}', \
             'holder {sku_code}', NULL, 'draft', 1, 0, '', '', 'principal:author-1', \
             '{now}', '{now}')",
            prod = product_id.simple(),
            tenant = TENANT.simple(),
            brand = Uuid::from_u128(0xb1).simple(),
        ),
        format!(
            "INSERT INTO products_sku (sku_id, tenant_id, product_id, sku_code, \
             lifecycle_state, internal_revision, published_version, composition_pending, \
             region_scope, brand_scope, created_by, created_at, updated_at, metering_unit, \
             usage_type_ref) \
             VALUES (X'{sku}', X'{tenant}', X'{prod}', '{sku_code}', 'draft', 1, 0, 0, '', '', \
             'principal:author-1', '{now}', '{now}', '{unit}', 'usage:storage')",
            sku = sku_id.simple(),
            tenant = TENANT.simple(),
            prod = product_id.simple(),
        ),
        format!(
            "INSERT INTO products_entity_version (tenant_id, entity_kind, entity_id, \
             published_version, content, content_digest, digest_version, actor_ref, \
             published_at) VALUES (X'{tenant}', 'sku', X'{sku}', 1, \
             '{{\"metering_unit\":\"{unit}\",\"usage_type_ref\":\"usage:storage\"}}', X'00', \
             1, X'{tenant}', '{now}')",
            tenant = TENANT.simple(),
            sku = sku_id.simple(),
        ),
        format!(
            "UPDATE products_sku SET lifecycle_state = 'published', published_version = 1, \
             internal_revision = internal_revision + 1 WHERE sku_id = X'{sku}'",
            sku = sku_id.simple(),
        ),
    ] {
        conn.execute_unprepared(&sql)
            .await
            .expect("the head guard admits this fixture write");
    }
    sku_id
}

/// Move a published holder to `deprecated` — an admitted edge, and the
/// state the `DoD`'s blocked arm names.
async fn deprecate_holder(harness: &TestHarness, sku_id: Uuid) {
    let conn = Database::connect(&harness.dsn)
        .await
        .expect("open an auxiliary connection");
    conn.execute_unprepared(&format!(
        "UPDATE products_sku SET lifecycle_state = 'deprecated', \
         deprecation_provenance = 'direct', internal_revision = internal_revision + 1 \
         WHERE sku_id = X'{sku}'",
        sku = sku_id.simple(),
    ))
    .await
    .expect("the head guard admits the admitted edge");
}

/// A fresh draft SKU and its `ETag`, for the post-removal declaration.
async fn draft_for_declaration(harness: &TestHarness) -> (Uuid, String) {
    let conn = Database::connect(&harness.dsn)
        .await
        .expect("open an auxiliary connection");
    let product_id = Uuid::now_v7();
    let sku_id = Uuid::now_v7();
    let now = "2026-08-29T09:00:00.000000Z";
    for sql in [
        format!(
            "INSERT INTO products_product (product_id, tenant_id, brand_id, name, \
             name_normalized, product_code, lifecycle_state, internal_revision, \
             published_version, region_scope, brand_scope, created_by, created_at, updated_at) \
             VALUES (X'{prod}', X'{tenant}', X'{brand}', 'Decl {prod}', 'decl {prod}', NULL, \
             'draft', 1, 0, '', '', 'principal:author-1', '{now}', '{now}')",
            prod = product_id.simple(),
            tenant = TENANT.simple(),
            brand = Uuid::from_u128(0xb2).simple(),
        ),
        format!(
            "INSERT INTO products_sku (sku_id, tenant_id, product_id, sku_code, \
             lifecycle_state, internal_revision, published_version, composition_pending, \
             region_scope, brand_scope, created_by, created_at, updated_at) \
             VALUES (X'{sku}', X'{tenant}', X'{prod}', 'SKU-DECL-{short}', 'draft', 1, 0, 0, \
             '', '', 'principal:author-1', '{now}', '{now}')",
            sku = sku_id.simple(),
            tenant = TENANT.simple(),
            prod = product_id.simple(),
            short = &sku_id.simple().to_string()[..8],
        ),
    ] {
        conn.execute_unprepared(&sql)
            .await
            .expect("the fixture writes are admitted");
    }
    (sku_id, "\"1\"".to_owned())
}

async fn retire_holder(harness: &TestHarness, sku_id: Uuid) {
    let conn = Database::connect(&harness.dsn)
        .await
        .expect("open an auxiliary connection");
    for sql in [
        format!(
            "UPDATE products_sku SET lifecycle_state = 'deprecated', \
             deprecation_provenance = 'direct', internal_revision = internal_revision + 1 \
             WHERE sku_id = X'{sku}'",
            sku = sku_id.simple(),
        ),
        format!(
            "UPDATE products_sku SET lifecycle_state = 'retired', \
             internal_revision = internal_revision + 1 WHERE sku_id = X'{sku}'",
            sku = sku_id.simple(),
        ),
    ] {
        conn.execute_unprepared(&sql)
            .await
            .expect("the head guard admits the admitted edges");
    }
}

/// **An add lands active and announces the set's own event in the same
/// transaction.**
#[tokio::test]
async fn an_add_lands_active_and_announces() {
    let harness = harness().await;
    let response = add_member(&harness, TENANT, "metering_unit", "gib_month").await;
    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let body = body_json(response).await;
    assert_eq!(body["state"], "active");
    assert_eq!(body["member_code"], "gib_month");
    assert_eq!(body["set_kind"], "metering_unit");

    assert_eq!(
        enqueued_event_count(&harness.dsn, "RecognizedUnitUpdated").await,
        1,
        "the metering-unit set announces through its own token"
    );

    let tier = add_member(&harness, TENANT, "plan_tier", "gold").await;
    assert_eq!(tier.status(), axum::http::StatusCode::CREATED);
    assert_eq!(
        enqueued_event_count(&harness.dsn, "PlanTierUpdated").await,
        1,
        "the tier set has its own event by design"
    );
}

/// **A duplicate add is refused whatever state the standing member is in** —
/// including the removed tombstone, whose PK never frees; re-entry is the
/// transitions door's re-listing.
#[tokio::test]
async fn a_duplicate_add_is_refused_naming_the_relisting_path() {
    let harness = harness().await;
    let first = add_member(&harness, TENANT, "metering_unit", "gib_month").await;
    assert_eq!(first.status(), axum::http::StatusCode::CREATED);

    let again = add_member(&harness, TENANT, "metering_unit", "gib_month").await;
    assert_eq!(again.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(error_code(again).await, "DUPLICATE_CODE");
}

/// **The machine walks deprecate → remove → re-list, refuses the shortcut,
/// and announces every flip.**
#[tokio::test]
async fn the_machine_walks_its_edges_and_refuses_the_shortcut() {
    let harness = harness().await;
    add_member(&harness, TENANT, "metering_unit", "gib_month").await;

    let shortcut = transition(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        "active",
        "removed",
    )
    .await;
    assert_eq!(
        shortcut.status(),
        axum::http::StatusCode::CONFLICT,
        "active -> removed is the refused shortcut: deprecation blocks new declarations first"
    );
    assert_eq!(error_code(shortcut).await, "ILLEGAL_TRANSITION");

    let deprecated = transition(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        "active",
        "deprecated",
    )
    .await;
    assert_eq!(deprecated.status(), axum::http::StatusCode::OK);
    assert_eq!(body_json(deprecated).await["state"], "deprecated");

    let removed = transition(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        "deprecated",
        "removed",
    )
    .await;
    assert_eq!(removed.status(), axum::http::StatusCode::OK);

    let relisted = transition(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        "removed",
        "active",
    )
    .await;
    assert_eq!(
        relisted.status(),
        axum::http::StatusCode::OK,
        "a tombstone re-enters as active: the identity never changed"
    );

    assert_eq!(
        enqueued_event_count(&harness.dsn, "RecognizedUnitUpdated").await,
        4,
        "the add and every admitted flip announce; the refused shortcut does not"
    );
}

/// **The live-op pin**: a caller whose read is stale is told the world
/// moved, not that its edge is illegal.
#[tokio::test]
async fn a_stale_expected_state_is_refused_stale_live_op() {
    let harness = harness().await;
    add_member(&harness, TENANT, "metering_unit", "gib_month").await;
    transition(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        "active",
        "deprecated",
    )
    .await;

    let stale = transition(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        "active",
        "deprecated",
    )
    .await;
    assert_eq!(stale.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(error_code(stale).await, "STALE_LIVE_OP");
}

/// **The delist refusal, armed on every arm the `DoD` names.**
///
/// `dod-recognized-set-mechanics` words the probe precisely — *"removal
/// refused while a `deprecated` head references the member, and removal
/// **admitted** while only frozen version content does — the old snapshot
/// still rendering afterwards, and a new declaration naming the removed
/// member failing `UNRECOGNIZED_UNIT`"* — and an earlier revision of this
/// case armed none of the three as stated: it blocked with a **published**
/// holder, admitted against a head that was merely terminal while its
/// column still named the unit, and never re-declared afterwards. Narrowing
/// the holder filter to `published` alone would have stayed green.
///
/// Now: blocked with the holder `published`, blocked again with it
/// `deprecated` (the `DoD`'s own arm), admitted once the holder is `retired`
/// and only the frozen row names the unit, the frozen bytes re-read
/// afterwards, and a fresh declaration of the removed member refused
/// `UNRECOGNIZED_UNIT` at the save door.
#[tokio::test]
async fn a_removal_is_blocked_by_live_holders_and_admitted_after_them() {
    let harness = harness().await;
    add_member(&harness, TENANT, "metering_unit", "gib_month").await;
    let holder = seed_holder(&harness, "SKU-HOLDER", "gib_month").await;

    transition(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        "active",
        "deprecated",
    )
    .await;

    let blocked = transition(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        "deprecated",
        "removed",
    )
    .await;
    assert_eq!(blocked.status(), axum::http::StatusCode::CONFLICT);
    let body = body_json(blocked).await;
    assert_eq!(body["context"]["reason"], json!("UNIT_DELIST_BLOCKED"));
    assert!(
        body.to_string().contains("SKU-HOLDER"),
        "the refusal samples the holders: {body}"
    );

    // The DoD's own arm: a **deprecated** head still references the member.
    // Without this the holder filter could narrow to `published` alone and
    // every assertion above would still pass.
    deprecate_holder(&harness, holder).await;
    let still_blocked = transition(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        "deprecated",
        "removed",
    )
    .await;
    assert_eq!(
        still_blocked.status(),
        axum::http::StatusCode::CONFLICT,
        "a deprecated head is non-terminal and still references the member"
    );
    assert_eq!(
        body_json(still_blocked).await["context"]["reason"],
        json!("UNIT_DELIST_BLOCKED")
    );

    retire_holder(&harness, holder).await;
    let admitted = transition(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        "deprecated",
        "removed",
    )
    .await;
    assert_eq!(
        admitted.status(),
        axum::http::StatusCode::OK,
        "a retired head is terminal and outside the holder population; frozen version content \
         never blocks a removal"
    );

    let frozen = raw_string_opt(
        &harness.dsn,
        &format!(
            "SELECT content AS v FROM products_entity_version WHERE entity_id = X'{}'",
            holder.simple()
        ),
    )
    .await
    .expect("the frozen row survives its member's removal");
    assert!(
        frozen.contains("gib_month"),
        "the frozen content NAMES the removed unit and still renders byte-for-byte — which is \
         what makes 'only frozen content names it' the admitted case rather than 'the head is \
         terminal': {frozen}"
    );

    // And the third arm: the member is out of the set, so a fresh
    // declaration naming it is refused at the save door.
    let (sku_id, etag) = draft_for_declaration(&harness).await;
    let refused = patch_sku_meter(&harness, sku_id, &etag, "gib_month").await;
    assert_eq!(refused.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(refused).await["context"]["violations"][0]["type"],
        json!("UNRECOGNIZED_UNIT"),
        "a removed member is outside the set, so declaring it is refused"
    );
}

/// **A seeded member is deprecatable and never removed** — and the refusal
/// deliberately carries no delist code, because §7 row 18 has not decided
/// which code refuses it and all three delist codes are predicated on
/// holders a seeded member need not have.
#[tokio::test]
async fn a_seeded_member_deprecates_and_never_removes() {
    let harness = harness().await;
    let conn = Database::connect(&harness.dsn)
        .await
        .expect("open an auxiliary connection");
    conn.execute_unprepared(&format!(
        "INSERT INTO products_recognized_set (tenant_id, set_kind, member_code, display_label, \
         state, seeded_by, created_at, updated_at) VALUES (X'{tenant}', 'metering_unit', \
         'seeded_gib', NULL, 'active', 'platform-seed', '2026-08-29T09:00:00.000000Z', \
         '2026-08-29T09:00:00.000000Z')",
        tenant = TENANT.simple(),
    ))
    .await
    .expect("seed the member");

    let deprecated = transition(
        &harness,
        TENANT,
        "metering_unit",
        "seeded_gib",
        "active",
        "deprecated",
    )
    .await;
    assert_eq!(deprecated.status(), axum::http::StatusCode::OK);

    let removal = transition(
        &harness,
        TENANT,
        "metering_unit",
        "seeded_gib",
        "deprecated",
        "removed",
    )
    .await;
    assert_eq!(removal.status(), axum::http::StatusCode::CONFLICT);
    let body = body_json(removal).await;
    // P-D-131 row 18: not one of the three delist codes — they are predicated
    // on holders and a seeded member is refused for being seeded — but the
    // Foundation's own variant, uniformly with 02's seeded definition, and no
    // sixteenth code (P-D-145 replaced the interim VALIDATION channel).
    assert_eq!(
        body["context"]["reason"],
        json!("ILLEGAL_FIELD_MUTATION"),
        "a seeded member is deprecatable and never removed"
    );
    assert!(
        body["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("seeded by"),
        "and the detail names the seeder: {body}"
    );
}

/// **The class roster is closed at the path**: an unknown `class` is a
/// validation refusal, never a default set.
///
/// The violated **subject** is asserted, not just the status, because
/// **P-D-175** renamed the path parameter from `setKind` to `class` and a
/// refusal that names a parameter the caller did not write is worse than a
/// bare one — the operator goes looking for a field that is not in their
/// request. The subject is the one place in the refusal where the two names
/// could drift apart unnoticed: `OperationBuilder`'s `path_param` and
/// `parse_kind`'s `violate` are edited in different files.
///
/// **The single production change that reddens it**: put `setKind` back as
/// either argument of `parse_kind`'s `report.violate(…)` call.
#[tokio::test]
async fn an_unknown_set_kind_is_refused_closed() {
    let harness = harness().await;
    let response = add_member(&harness, TENANT, "units", "gib_month").await;
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert_eq!(
        body["context"]["violations"][0]["subject"],
        json!("class"),
        "the refusal names the path parameter the caller wrote"
    );
    let detail = body["context"]["violations"][0]["description"]
        .as_str()
        .expect("description")
        .to_owned();
    assert!(
        detail.starts_with("class must be one of"),
        "the detail names the parameter too: {detail}"
    );
    for named in ["metering_unit", "plan_tier"] {
        assert!(
            detail.contains(named),
            "the refusal must name `{named}`: {detail}"
        );
    }
}

/// A transition on a member the set never carried answers the bare 404.
#[tokio::test]
async fn a_transition_on_an_unknown_member_is_not_found() {
    let harness = harness().await;
    let response = transition(
        &harness,
        TENANT,
        "metering_unit",
        "ghost",
        "active",
        "deprecated",
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

/// **The door has a gate, and the gate now opens the unit**
/// (`dod-recognized-set-mechanics`, P-D-146; **P-D-173**): with no record for
/// the member's `GovernedLiveOp` subject, every member op — add, transition,
/// relabel — answers `202` naming the unit it opened, **and nothing is
/// written or announced**.
///
/// # Why the old claim stopped holding, and what this case still proves
///
/// Until P-D-173 these three calls answered `403 APPROVAL_REQUIRED`, and this
/// case asserted that code. The refusal was not wrong — it was
/// *unusable*: the only way past it was to submit through `POST /approvals`
/// first with a `content_snapshot` the caller hand-rendered to match what the
/// door would present (P-D-172), which is a contract no client can hold. The
/// door now opens that unit itself. **The half that mattered is unchanged and
/// is still asserted here**: an unapproved member op writes no row and
/// enqueues no event. `APPROVAL_REQUIRED` keeps its two other paths on this
/// surface — a unit open for a *different* change
/// ([`a_unit_open_for_another_change_is_named_not_superseded`]) and a record
/// bound to another op ([`an_approval_bound_to_one_op_does_not_authorize_another`]).
#[tokio::test]
async fn a_member_op_without_a_record_opens_the_unit_and_writes_nothing() {
    let harness = harness().await;
    // **`N = 2` explicitly, because this test's subject is the discount**
    // (P-D-177): the relabel closes on `min(N, 1)` while the full act spends
    // `N`, and at the default of one those are the same number. The default
    // is no longer a value this distinction can be read off.
    set_quorum(&harness, 2).await;
    let opened = add_member_via(app_for(&harness, TENANT), "metering_unit", "gib_month").await;
    assert_eq!(opened.status(), axum::http::StatusCode::ACCEPTED);
    let unit = body_json(opened).await;
    assert_eq!(unit["state"], "pending");
    assert_eq!(unit["required"], 2, "an add is material: the full N");
    assert_eq!(unit["configured_quorum"], 2);
    assert!(
        unit["approval_id"].as_str().is_some(),
        "the 202 names the unit the caller has to get decided: {unit}"
    );
    assert_eq!(
        enqueued_event_count(&harness.dsn, "RecognizedUnitUpdated").await,
        0,
        "an add that only opened a unit announces nothing"
    );
    let listed = body_json(
        get_json(
            app_for(&harness, TENANT),
            "/bss-products/v1/config/vocabularies/metering_unit",
        )
        .await,
    )
    .await;
    assert!(
        !listed["members"]
            .as_array()
            .expect("the set lists")
            .iter()
            .any(|m| m["member_code"] == "gib_month"),
        "and writes no member: {listed}"
    );

    // A second member, whose add rode a seeded record: the add spent it, so
    // the transitions door finds none and opens its own — over the edge it is
    // about to walk, and without moving the member.
    add_member(&harness, TENANT, "metering_unit", "tib_month").await;
    let tag = tag_of(&harness, "metering_unit", "tib_month").await;
    let stranger = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/metering_unit/values/tib_month/transitions",
        &json!({ "to": "deprecated", "expected_state": "active" }),
        &tag,
    )
    .await;
    assert_eq!(stranger.status(), axum::http::StatusCode::ACCEPTED);
    assert_eq!(
        member_state(&harness, "metering_unit", "tib_month").await,
        "active",
        "the member did not move"
    );
    // One open unit per subject, so the label door meets the transition's and
    // is told so rather than superseding it.
    let tag = tag_of(&harness, "metering_unit", "tib_month").await;
    let relabel_refused = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/metering_unit/values/tib_month/label",
        &json!({ "display_label": "TiB-month" }),
        &tag,
    )
    .await;
    assert_eq!(relabel_refused.status(), axum::http::StatusCode::FORBIDDEN);
    assert_eq!(error_code(relabel_refused).await, "APPROVAL_REQUIRED");
}

/// Set the tenant's `N`, so a case can name the quorum it is asserting under.
async fn set_quorum(harness: &TestHarness, n: u32) {
    let conn = harness.db.conn().expect("scoped connection");
    let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
    crate::infra::storage::repo::write_materiality_policy(
        &conn,
        &scope,
        TENANT,
        &crate::domain::materiality::MaterialityPolicy::new(Vec::new(), 10, n),
        Uuid::from_u128(0x5a_ad),
        crate::test_support::at(9),
    )
    .await
    .expect("write the policy");
}

/// One member's stored state, read back through the by-code door.
async fn member_state(harness: &TestHarness, kind: &str, code: &str) -> String {
    let body = body_json(
        get_json(
            app_for(harness, TENANT),
            &format!("/bss-products/v1/config/vocabularies/{kind}/values/{code}"),
        )
        .await,
    )
    .await;
    body["state"].as_str().unwrap_or_default().to_owned()
}

/// A context carrying a role claim, so a probe can drive the decide door.
fn ctx_with_role(subject: Uuid) -> axum::http::Extensions {
    let mut extensions = axum::http::Extensions::new();
    extensions.insert(
        toolkit_security::SecurityContext::builder()
            .subject_id(subject)
            .subject_tenant_id(TENANT)
            .subject_type(toolkit_gts::gts_id!("cf.core.security.subject_user.v1~"))
            .token_scopes(vec![
                "*".to_owned(),
                crate::domain::approval::ApproverRole::CatalogAdmin
                    .as_str()
                    .to_owned(),
            ])
            .build()
            .expect("authed SecurityContext must build"),
    );
    extensions
}

/// Approve one unit as a `CatalogAdmin` who is not its submitter.
async fn approve(harness: &TestHarness, approval_id: &str, approver: Uuid) -> u16 {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!(
            "/bss-products/v1/approvals/{approval_id}/decisions"
        ))
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "verdict": "approved" }).to_string()))
        .expect("build the request");
    *request.extensions_mut() = ctx_with_role(approver);
    approvals_app_for(harness, TENANT)
        .oneshot(request)
        .await
        .expect("the router answers")
        .status()
        .as_u16()
}

/// **The by-code read hands out a tag its write doors assert, and a stale one
/// is refused** (**P-D-174**).
///
/// The tag is the member's whole row, not its state: the label arm is the one
/// that matters, because a relabel had **no** staleness pin at all before
/// this entry and `expected_state` — the transitions body's — would not have
/// caught it. The sequence is one operator's read, another operator's
/// relabel, and the first operator's write refused on the tag they read.
#[tokio::test]
async fn a_stale_member_tag_is_refused_and_a_current_one_writes() {
    let harness = harness().await;
    add_member(&harness, TENANT, "plan_tier", "gold").await;
    let stale = tag_of(&harness, "plan_tier", "gold").await;
    assert!(
        stale.starts_with('"') && stale.len() == 66,
        "the tag is one strong entity tag quoting a SHA-256: {stale}"
    );

    // A peer relabels, so the row — and its tag — move.
    assert_eq!(
        relabel(&harness, TENANT, "plan_tier", "gold", Some("Gold"))
            .await
            .status(),
        axum::http::StatusCode::OK
    );
    let current = tag_of(&harness, "plan_tier", "gold").await;
    assert_ne!(stale, current, "a changed label changes the tag");

    seed_bound_op(
        &harness,
        TENANT,
        "plan_tier",
        "gold",
        &json!({ "op": "recognized_set.label", "display_label": "Gold tier" }),
    )
    .await;
    let refused = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
        &json!({ "display_label": "Gold tier" }),
        &stale,
    )
    .await;
    assert_eq!(refused.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(
        error_code(refused).await,
        "STALE_LIVE_OP",
        "a live row's staleness has one voice, whatever operand caught it"
    );
    assert_eq!(
        body_json(
            get_json(
                app_for(&harness, TENANT),
                "/bss-products/v1/config/vocabularies/plan_tier/values/gold",
            )
            .await,
        )
        .await["display_label"],
        "Gold",
        "the refused relabel wrote nothing"
    );

    let landed = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
        &json!({ "display_label": "Gold tier" }),
        &current,
    )
    .await;
    assert_eq!(
        landed.status(),
        axum::http::StatusCode::OK,
        "the current tag writes, which is what keeps the refusal above about staleness"
    );
}

/// **Every shape that is not one strong member tag is refused `VALIDATION`,
/// and an absent header is one of them** (**P-D-174**).
///
/// The wildcard is the case with a reason rather than a convention behind it:
/// `If-Match: *` means *overwrite whichever row is current*, which is exactly
/// the unconditional write the precondition exists to make unreachable —
/// `domain::concurrency`'s own doc records the gear taking the opposite
/// position from `gears/file-storage` on it, and this door reads that rule
/// off the same function rather than restating it.
#[tokio::test]
async fn a_member_tag_that_is_not_one_strong_digest_is_refused() {
    let harness = harness().await;
    add_member(&harness, TENANT, "plan_tier", "gold").await;
    let good = tag_of(&harness, "plan_tier", "gold").await;

    let absent = post_json(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
        &json!({ "display_label": "Gold" }),
    )
    .await;
    assert_eq!(
        absent.status(),
        axum::http::StatusCode::BAD_REQUEST,
        "a member op without the header is the unconditional write the precondition forbids"
    );

    for (label, tag) in [
        ("the wildcard", "*".to_owned()),
        ("a weak validator", format!("W/{good}")),
        ("a list", format!("{good}, {good}")),
        ("an unquoted body", good.trim_matches('"').to_owned()),
        ("a revision tag from an entity head", "\"7\"".to_owned()),
        ("uppercase hex", format!("\"{}\"", "A".repeat(64))),
        ("a short digest", format!("\"{}\"", "a".repeat(63))),
    ] {
        let refused = post_json_tagged(
            app_for(&harness, TENANT),
            "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
            &json!({ "display_label": "Gold" }),
            &tag,
        )
        .await;
        assert_eq!(
            refused.status(),
            axum::http::StatusCode::BAD_REQUEST,
            "{label} was admitted"
        );
    }
    // The positive control: the same door, the same body, the tag this gear
    // minted — without it every refusal above would pass against a door that
    // rejected every header. The record is seeded, so the answer is the
    // door's write rather than P-D-173's `202`.
    seed_member_op(&harness, TENANT, "plan_tier", "gold").await;
    assert_eq!(
        post_json_tagged(
            app_for(&harness, TENANT),
            "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
            &json!({ "display_label": "Gold" }),
            &good,
        )
        .await
        .status(),
        axum::http::StatusCode::OK
    );
}

/// **The add door takes no `If-Match`, and that is a decision rather than an
/// omission** (**P-D-174**): a create names no member to have read, and the
/// wildcard — the only tag a creating caller could send — is refused
/// everywhere in this gear. Its concurrency guard is the primary key, which
/// the duplicate refusal already proves; this asserts the header is not
/// demanded.
#[tokio::test]
async fn the_add_door_demands_no_tag() {
    let harness = harness().await;
    seed_member_op(&harness, TENANT, "plan_tier", "gold").await;
    let created = add_member_via(app_for(&harness, TENANT), "plan_tier", "gold").await;
    assert_eq!(created.status(), axum::http::StatusCode::CREATED);
}

/// **The two arms are one door** (**P-D-173**): the first call opens the unit
/// and answers `202`, and the **identical** request after two principals have
/// approved it answers the door's ordinary `201` and writes the member.
///
/// This is the whole point of the entry, so it is driven end to end rather
/// than by flipping a row: the unit is decided through
/// `POST /approvals/{id}/decisions` by two `CatalogAdmin`s, neither of whom
/// is the submitter — the door's own actor is (`decision_admitted` refuses a
/// self-approval at `required >= 1`).
#[tokio::test]
async fn the_first_call_opens_the_unit_and_the_approved_re_send_applies_it() {
    let harness = harness().await;
    let opened = add_member_via(app_for(&harness, TENANT), "plan_tier", "gold").await;
    assert_eq!(opened.status(), axum::http::StatusCode::ACCEPTED);
    let unit = body_json(opened).await;
    let approval_id = unit["approval_id"]
        .as_str()
        .expect("the 202 names the unit")
        .to_owned();

    assert_eq!(
        approve(&harness, &approval_id, Uuid::from_u128(0xa9_01)).await,
        200
    );
    assert_eq!(
        approve(&harness, &approval_id, Uuid::from_u128(0xa9_02)).await,
        200
    );

    let applied = add_member_via(app_for(&harness, TENANT), "plan_tier", "gold").await;
    assert_eq!(
        applied.status(),
        axum::http::StatusCode::CREATED,
        "the re-send of the approved change is the door's ordinary answer"
    );
    assert_eq!(body_json(applied).await["member_code"], "gold");
    assert_eq!(
        enqueued_event_count(&harness.dsn, "PlanTierUpdated").await,
        1,
        "the announcement rides the write, not the proposal"
    );
}

/// **A re-send before approval answers the same unit and supersedes nothing**
/// (**P-D-173**).
///
/// The hazard this guards is specific: `repo::submit_approval` supersedes
/// whatever open record the subject held (L-4), so a door that submitted on
/// every unauthorized call would discard the approvals already collected on
/// the pending unit — on the caller's own retry. The second `202` must name
/// the **same** `approval_id`, and the approval cast in between must still
/// count.
#[tokio::test]
async fn a_re_send_before_approval_answers_the_same_unit() {
    let harness = harness().await;
    // **`N = 2` explicitly, because this test's subject is the discount**
    // (P-D-177): the relabel closes on `min(N, 1)` while the full act spends
    // `N`, and at the default of one those are the same number. The default
    // is no longer a value this distinction can be read off.
    set_quorum(&harness, 2).await;
    let first =
        body_json(add_member_via(app_for(&harness, TENANT), "plan_tier", "gold").await).await;
    let approval_id = first["approval_id"].as_str().expect("a unit").to_owned();
    assert_eq!(
        approve(&harness, &approval_id, Uuid::from_u128(0xa9_11)).await,
        200,
        "one of two principals decides"
    );

    let again = add_member_via(app_for(&harness, TENANT), "plan_tier", "gold").await;
    assert_eq!(again.status(), axum::http::StatusCode::ACCEPTED);
    let second = body_json(again).await;
    assert_eq!(
        second["approval_id"], first["approval_id"],
        "the retry names the standing unit, not a fresh one"
    );

    // The decision cast against the first `202` still counts: the second
    // principal closes it, and the third call applies.
    assert_eq!(
        approve(&harness, &approval_id, Uuid::from_u128(0xa9_12)).await,
        200
    );
    assert_eq!(
        add_member_via(app_for(&harness, TENANT), "plan_tier", "gold")
            .await
            .status(),
        axum::http::StatusCode::CREATED,
        "two principals decided one unit across two retries"
    );
}

/// **A unit open for another change is named, never superseded**
/// (**P-D-173**): `design/05` §4 admits one open record per subject, so the
/// second proposal is refused `APPROVAL_REQUIRED` naming the standing unit
/// rather than replacing it.
#[tokio::test]
async fn a_unit_open_for_another_change_is_named_not_superseded() {
    let harness = harness().await;
    add_member(&harness, TENANT, "plan_tier", "gold").await;
    let tag = tag_of(&harness, "plan_tier", "gold").await;
    let opened = body_json(
        post_json_tagged(
            app_for(&harness, TENANT),
            "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
            &json!({ "display_label": "Gold tier" }),
            &tag,
        )
        .await,
    )
    .await;
    let standing = opened["approval_id"].as_str().expect("a unit").to_owned();

    let tag = tag_of(&harness, "plan_tier", "gold").await;
    let other = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold/transitions",
        &json!({ "to": "deprecated", "expected_state": "active" }),
        &tag,
    )
    .await;
    assert_eq!(other.status(), axum::http::StatusCode::FORBIDDEN);
    assert_eq!(error_code(other).await, "APPROVAL_REQUIRED");

    // The standing unit survived and still decides its own change.
    assert_eq!(
        approve(&harness, &standing, Uuid::from_u128(0xa9_21)).await,
        200
    );
    assert_eq!(
        approve(&harness, &standing, Uuid::from_u128(0xa9_22)).await,
        200
    );
    let tag = tag_of(&harness, "plan_tier", "gold").await;
    let relabelled = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
        &json!({ "display_label": "Gold tier" }),
        &tag,
    )
    .await;
    assert_eq!(relabelled.status(), axum::http::StatusCode::OK);
    assert_eq!(body_json(relabelled).await["display_label"], "Gold tier");
}

/// **At `N = 0` there is nobody to wait for, so the first call applies**
/// (**P-D-173**): the unit is born `satisfied` (P-D-119 row 31) and `202
/// Accepted` would be a lie about a record that waits on no principal.
#[tokio::test]
async fn at_quorum_zero_the_first_call_opens_and_applies_in_one_request() {
    let harness = harness().await;
    set_quorum(&harness, 0).await;
    let landed = add_member_via(app_for(&harness, TENANT), "plan_tier", "gold").await;
    assert_eq!(
        landed.status(),
        axum::http::StatusCode::CREATED,
        "a tenant at N = 0 writes approver-less by policy, in one call"
    );
    assert_eq!(body_json(landed).await["member_code"], "gold");
}

/// **A rename touches the display label and nothing else**
/// (`dod-plantier-governance`, `dod-unit-immutable`): the code and the state
/// survive, the set announces, and a member that does not exist is `404`.
#[tokio::test]
async fn a_relabel_changes_the_display_label_only_and_announces() {
    let harness = harness().await;
    add_member(&harness, TENANT, "plan_tier", "gold").await;
    let renamed = relabel(&harness, TENANT, "plan_tier", "gold", Some("Gold")).await;
    assert_eq!(renamed.status(), axum::http::StatusCode::OK);
    let body = body_json(renamed).await;
    assert_eq!(body["display_label"], "Gold");
    assert_eq!(body["member_code"], "gold", "the code is the identity");
    assert_eq!(body["state"], "active", "a relabel is not a transition");
    assert_eq!(
        enqueued_event_count(&harness.dsn, "PlanTierUpdated").await,
        2,
        "the add and the relabel each announce through the tier token"
    );

    let cleared = relabel(&harness, TENANT, "plan_tier", "gold", None).await;
    assert_eq!(cleared.status(), axum::http::StatusCode::OK);
    assert!(body_json(cleared).await["display_label"].is_null());

    let missing = relabel(&harness, TENANT, "plan_tier", "platinum", Some("Platinum")).await;
    assert_eq!(missing.status(), axum::http::StatusCode::NOT_FOUND);
}

/// Seed a satisfied record whose snapshot **is** the op declaration the door
/// will present — the double a binding case needs (**P-D-172**).
async fn seed_bound_op(
    harness: &TestHarness,
    tenant: Uuid,
    kind: &str,
    code: &str,
    op: &JsonValue,
) {
    let kind = crate::domain::recognized::SetKind::parse(kind).expect("a roster kind");
    crate::test_support::seed_satisfied_approval_with_snapshot(
        &harness.db,
        tenant,
        crate::api::rest::recognized_sets::member_op_subject(tenant, kind, code),
        0,
        op,
    )
    .await;
}

/// **An approval submitted for one op does not authorize another on the same
/// member** (**P-D-172**).
///
/// The subject is the member, so before this entry a satisfied record for
/// `recognized_set/plan_tier/gold` authorized *any* of the three doors on
/// `gold` — two principals agreed to a relabel and a deprecate went through
/// on it. Each arm here seeds exactly one record, declaring one op, and
/// drives a **different** door: the refusal is `APPROVAL_REQUIRED`, the same
/// code an absent record earns, because from the door's side there is no
/// record for this change.
///
/// The positive control is the fourth arm — the same record, the door it was
/// submitted for, `200` — without which every assertion above would pass
/// against a door that had simply stopped authorizing anything.
#[tokio::test]
async fn an_approval_bound_to_one_op_does_not_authorize_another() {
    let harness = harness().await;
    add_member(&harness, TENANT, "plan_tier", "gold").await;

    // A relabel's record, spent at the transitions door.
    seed_bound_op(
        &harness,
        TENANT,
        "plan_tier",
        "gold",
        &json!({ "op": "recognized_set.label", "display_label": "Gold tier" }),
    )
    .await;
    let tag = tag_of(&harness, "plan_tier", "gold").await;
    let deprecate = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold/transitions",
        &json!({ "to": "deprecated", "expected_state": "active" }),
        &tag,
    )
    .await;
    assert_eq!(
        error_code(deprecate).await,
        "APPROVAL_REQUIRED",
        "an agreed relabel is not an agreed deprecation"
    );

    // A transition's record, spent at the label door. The member has to
    // exist for the label door to have a tag to assert, so it is added first
    // — on its own record, which its add spends.
    add_member(&harness, TENANT, "metering_unit", "gib_month").await;
    seed_bound_op(
        &harness,
        TENANT,
        "metering_unit",
        "gib_month",
        &json!({ "op": "recognized_set.transition", "to": "deprecated", "expected_state": "active" }),
    )
    .await;
    let tag = tag_of(&harness, "metering_unit", "gib_month").await;
    let relabelled = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/metering_unit/values/gib_month/label",
        &json!({ "display_label": "GiB-hours" }),
        &tag,
    )
    .await;
    assert_eq!(
        error_code(relabelled).await,
        "APPROVAL_REQUIRED",
        "an agreed deprecation is not an agreed relabel"
    );

    // An add's record, spent at the transitions door.
    seed_bound_op(
        &harness,
        TENANT,
        "plan_tier",
        "silver",
        &json!({ "op": "recognized_set.add", "member_code": "silver", "display_label": null }),
    )
    .await;
    let tag = tag_of(&harness, "plan_tier", "silver").await;
    let silver = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/silver/transitions",
        &json!({ "to": "deprecated", "expected_state": "active" }),
        &tag,
    )
    .await;
    assert_eq!(
        error_code(silver).await,
        "APPROVAL_REQUIRED",
        "an agreed add is not an agreed deprecation"
    );

    // The positive control: the add's own door, on the record it names.
    let added = add_member_via(app_for(&harness, TENANT), "plan_tier", "silver").await;
    assert_eq!(
        added.status(),
        axum::http::StatusCode::CREATED,
        "the record authorizes the change it was submitted for"
    );
}

/// **The binding is the whole proposal, not the op token** (**P-D-172**): an
/// approval for the label `Gold tier` does not authorize a relabel to
/// `Platinum`, and neither authorizes a **clear**.
///
/// The clear arm is what makes `canonical::Absence::Omit` the right mode
/// rather than an arbitrary one: it carries an explicit `null` as `null`, so
/// *set the label to nothing* and *set it to a value* render differently and
/// cannot satisfy one another's record (P-D-34).
#[tokio::test]
async fn an_approval_bound_to_one_label_does_not_authorize_a_different_one() {
    let harness = harness().await;
    add_member(&harness, TENANT, "plan_tier", "gold").await;
    seed_bound_op(
        &harness,
        TENANT,
        "plan_tier",
        "gold",
        &json!({ "op": "recognized_set.label", "display_label": "Gold tier" }),
    )
    .await;

    let tag = tag_of(&harness, "plan_tier", "gold").await;
    let other = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
        &json!({ "display_label": "Platinum" }),
        &tag,
    )
    .await;
    assert_eq!(
        error_code(other).await,
        "APPROVAL_REQUIRED",
        "the approved label is part of what was agreed"
    );

    let tag = tag_of(&harness, "plan_tier", "gold").await;
    let cleared = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
        &json!({ "display_label": null }),
        &tag,
    )
    .await;
    assert_eq!(
        error_code(cleared).await,
        "APPROVAL_REQUIRED",
        "clearing a label is not the relabel that was agreed"
    );

    let tag = tag_of(&harness, "plan_tier", "gold").await;
    let agreed = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
        &json!({ "display_label": "Gold tier" }),
        &tag,
    )
    .await;
    assert_eq!(
        agreed.status(),
        axum::http::StatusCode::OK,
        "the agreed label lands"
    );
}

/// **A record whose snapshot declares no op of this slice's authorizes every
/// op, exactly as before P-D-172** — the compatibility arm, asserted rather
/// than left to be inferred from the other probes passing.
///
/// `{}` is what `test_support::seed_satisfied_approval` writes and what every
/// other case here rides; `{"subject": …}` is what `vhp-core`'s e2e library
/// sends for all eight of its live-op subjects. Both must keep authorizing
/// all three doors, and the day they stop is the day that e2e goes red — which
/// is why P-D-172 owes those lines rather than leaving the arm undocumented.
#[tokio::test]
async fn a_record_declaring_no_op_authorizes_every_door() {
    let harness = harness().await;
    add_member(&harness, TENANT, "plan_tier", "gold").await;

    seed_bound_op(
        &harness,
        TENANT,
        "plan_tier",
        "gold",
        &json!({ "subject": "recognized_set/plan_tier/gold" }),
    )
    .await;
    let tag = tag_of(&harness, "plan_tier", "gold").await;
    let relabelled = post_json_tagged(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold/label",
        &json!({ "display_label": "Gold" }),
        &tag,
    )
    .await;
    assert_eq!(
        relabelled.status(),
        axum::http::StatusCode::OK,
        "an undeclared record authorizes the label door as it always did"
    );

    let deprecated = transition(
        &harness,
        TENANT,
        "plan_tier",
        "gold",
        "active",
        "deprecated",
    )
    .await;
    assert_eq!(
        deprecated.status(),
        axum::http::StatusCode::OK,
        "and the transitions door, on the empty-object snapshot the shared double writes"
    );
}

/// A published head carrying a **tier** blocks that member's removal exactly
/// as a metering unit's does — the guard is uniform across the kinds
/// (`dod-recognized-set-mechanics`, `dod-plantier-governance`; P-D-146) — and
/// the positive control removes a member nobody carries.
///
/// **The accounting half of this case went with P-D-169**: `tax_category` and
/// `gl_code` are no longer set kinds, so `ACCOUNTING_CODE_DELIST_BLOCKED` has
/// no reachable path and the uniformity claim now spans two kinds, not four.
/// The refusal a retired path segment earns is
/// [`the_kind_roster_and_its_refusal_codes`]'s to prove.
#[tokio::test]
async fn a_tier_retire_is_blocked_by_a_published_carrier() {
    let harness = harness().await;
    add_member(&harness, TENANT, "plan_tier", "gold").await;
    add_member(&harness, TENANT, "plan_tier", "silver").await;
    seed_carrier(&harness, "SKU-GOLD", "plan_tier", "gold").await;

    let deprecated = transition(
        &harness,
        TENANT,
        "plan_tier",
        "gold",
        "active",
        "deprecated",
    )
    .await;
    assert_eq!(deprecated.status(), axum::http::StatusCode::OK);
    let blocked = transition(
        &harness,
        TENANT,
        "plan_tier",
        "gold",
        "deprecated",
        "removed",
    )
    .await;
    assert_eq!(blocked.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(error_code(blocked).await, "PLAN_TIER_RETIRE_BLOCKED");

    let deprecated = transition(
        &harness,
        TENANT,
        "plan_tier",
        "silver",
        "active",
        "deprecated",
    )
    .await;
    assert_eq!(deprecated.status(), axum::http::StatusCode::OK);
    let removed = transition(
        &harness,
        TENANT,
        "plan_tier",
        "silver",
        "deprecated",
        "removed",
    )
    .await;
    assert_eq!(
        removed.status(),
        axum::http::StatusCode::OK,
        "a member no published head carries removes - the positive control"
    );
}

/// A published SKU carrying `value` in `column` — the fixture the tier and
/// code guards count. Written the way [`seed_holder`] writes a metered one:
/// through the head guard, with a frozen version row naming the column.
async fn seed_carrier(harness: &TestHarness, sku_code: &str, column: &str, value: &str) -> Uuid {
    let conn = Database::connect(&harness.dsn)
        .await
        .expect("open an auxiliary connection");
    let product_id = Uuid::now_v7();
    let sku_id = Uuid::now_v7();
    let now = "2026-08-29T09:00:00.000000Z";
    for sql in [
        format!(
            "INSERT INTO products_product (product_id, tenant_id, brand_id, name, \
             name_normalized, product_code, lifecycle_state, internal_revision, \
             published_version, region_scope, brand_scope, created_by, created_at, updated_at) \
             VALUES (X'{prod}', X'{tenant}', X'{brand}', 'Carrier {sku_code}', \
             'carrier {sku_code}', NULL, 'draft', 1, 0, '', '', 'principal:author-1', \
             '{now}', '{now}')",
            prod = product_id.simple(),
            tenant = TENANT.simple(),
            brand = Uuid::from_u128(0xb1).simple(),
        ),
        format!(
            "INSERT INTO products_sku (sku_id, tenant_id, product_id, sku_code, \
             lifecycle_state, internal_revision, published_version, composition_pending, \
             region_scope, brand_scope, created_by, created_at, updated_at, {column}) \
             VALUES (X'{sku}', X'{tenant}', X'{prod}', '{sku_code}', 'draft', 1, 0, 0, '', '', \
             'principal:author-1', '{now}', '{now}', '{value}')",
            sku = sku_id.simple(),
            tenant = TENANT.simple(),
            prod = product_id.simple(),
        ),
        format!(
            "INSERT INTO products_entity_version (tenant_id, entity_kind, entity_id, \
             published_version, content, content_digest, digest_version, actor_ref, \
             published_at) VALUES (X'{tenant}', 'sku', X'{sku}', 1, \
             '{{\"{column}\":\"{value}\"}}', X'00', 1, X'{tenant}', '{now}')",
            tenant = TENANT.simple(),
            sku = sku_id.simple(),
        ),
        format!(
            "UPDATE products_sku SET lifecycle_state = 'published', published_version = 1, \
             internal_revision = internal_revision + 1 WHERE sku_id = X'{sku}'",
            sku = sku_id.simple(),
        ),
    ] {
        conn.execute_unprepared(&sql)
            .await
            .expect("the head guard admits this fixture write");
    }
    sku_id
}

// ── The read surface (P-D-170) ───────────────────────────────────────────────

/// **The set can be enumerated, and the platform baseline is there before the
/// first write.** This is the whole of what the gear could not do until this
/// door: the repository's only read was a single-member lookup, so no caller
/// could learn which codes exist. The seed assertion is the half that would
/// rot silently — a list that answered `[]` on a fresh tenant and a create
/// door that then found four units would be two answers from one gear.
#[tokio::test]
async fn a_set_lists_its_members_and_seeds_the_baseline_on_the_first_read() {
    let harness = harness().await;

    let units = body_json(
        get_json(
            app_for(&harness, TENANT),
            "/bss-products/v1/config/vocabularies/metering_unit",
        )
        .await,
    )
    .await;
    assert_eq!(units["set_kind"], json!("metering_unit"));
    let codes: Vec<&str> = units["members"]
        .as_array()
        .expect("members is an array")
        .iter()
        .map(|m| m["member_code"].as_str().expect("a code"))
        .collect();
    assert_eq!(
        codes,
        vec!["GB-egress", "GB-storage", "request-count", "vCPU-hours"],
        "the four seeded units, `member_code`-ordered and seeded by this read"
    );
    assert!(
        units["members"]
            .as_array()
            .expect("array")
            .iter()
            .all(|m| m["state"] == json!("active") && m["seeded_by"] == json!("platform")),
        "the baseline arrives `active` and marked platform-seeded: {units}"
    );

    let tiers = body_json(
        get_json(
            app_for(&harness, TENANT),
            "/bss-products/v1/config/vocabularies/plan_tier",
        )
        .await,
    )
    .await;
    assert_eq!(tiers["set_kind"], json!("plan_tier"));
    assert_eq!(tiers["members"][0]["member_code"], json!("standard"));
    assert_eq!(
        tiers["members"][0]["display_label"],
        json!("Standard"),
        "the tier set is the one that carries a label"
    );
}

/// **A tombstone stays in the list, carrying its state.** Hiding it would put
/// the list at odds with the add door one call later, which refuses a
/// `removed` code `DUPLICATE_CODE` because its primary key never frees. The
/// probe walks a member all the way to `removed` through the doors, so the
/// claim rests on the real edge rather than on a hand-written row.
#[tokio::test]
async fn a_removed_member_is_listed_with_its_state_not_filtered_out() {
    let harness = harness().await;
    add_member(&harness, TENANT, "plan_tier", "bronze").await;
    transition(
        &harness,
        TENANT,
        "plan_tier",
        "bronze",
        "active",
        "deprecated",
    )
    .await;
    let removed = transition(
        &harness,
        TENANT,
        "plan_tier",
        "bronze",
        "deprecated",
        "removed",
    )
    .await;
    assert_eq!(removed.status(), axum::http::StatusCode::OK);

    let set = body_json(
        get_json(
            app_for(&harness, TENANT),
            "/bss-products/v1/config/vocabularies/plan_tier",
        )
        .await,
    )
    .await;
    let bronze = set["members"]
        .as_array()
        .expect("array")
        .iter()
        .find(|m| m["member_code"] == json!("bronze"))
        .expect("the tombstone is listed, not filtered");
    assert_eq!(bronze["state"], json!("removed"));

    // The claim this case exists for: the door the caller meets next agrees.
    // The record is seeded again because each member op spends its own
    // (P-D-144's one-shot), and a `403` here would prove the gate, not the
    // tombstone.
    seed_member_op(&harness, TENANT, "plan_tier", "bronze").await;
    let refused = add_member_via(app_for(&harness, TENANT), "plan_tier", "bronze").await;
    assert_eq!(refused.status(), axum::http::StatusCode::CONFLICT);
}

/// One member reads back by code, and a code the set does not carry is a bare
/// `404` — the same miss shape every other read on this gear answers with, so
/// absent and out-of-scope stay indistinguishable.
#[tokio::test]
async fn one_member_reads_by_code_and_an_unknown_one_is_a_bare_miss() {
    let harness = harness().await;
    add_member(&harness, TENANT, "plan_tier", "gold").await;

    let hit = get_json(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/gold",
    )
    .await;
    assert_eq!(hit.status(), axum::http::StatusCode::OK);
    let member = body_json(hit).await;
    assert_eq!(member["member_code"], json!("gold"));
    assert_eq!(member["state"], json!("active"));
    assert_eq!(
        member["seeded_by"],
        JsonValue::Null,
        "an operator-added member carries no seed provenance"
    );

    let miss = get_json(
        app_for(&harness, TENANT),
        "/bss-products/v1/config/vocabularies/plan_tier/values/platinum",
    )
    .await;
    assert_eq!(miss.status(), axum::http::StatusCode::NOT_FOUND);
}

/// A retired path segment is refused on the **read** doors too. Without this
/// the roster's fail-closed parse would be proved only on the write half, and
/// a read admitting `tax_category` would answer an empty set rather than a
/// refusal — telling a caller the vocabulary exists and is empty.
#[tokio::test]
async fn the_read_doors_refuse_a_kind_outside_the_roster() {
    let harness = harness().await;
    for uri in [
        "/bss-products/v1/config/vocabularies/tax_category",
        "/bss-products/v1/config/vocabularies/gl_code/values/GL-4000",
        "/bss-products/v1/config/vocabularies/units",
    ] {
        let refused = get_json(app_for(&harness, TENANT), uri).await;
        assert_eq!(
            refused.status(),
            axum::http::StatusCode::BAD_REQUEST,
            "{uri} must be refused, never answered empty"
        );
    }
}
