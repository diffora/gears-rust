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
    let state = Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: ProductsConfig::default().bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: ProductsConfig::default()
            .bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: ProductsConfig::default().watermark_skew_tolerance(),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        usage_type_resolver: crate::test_support::resolved_usage_types(),
    });
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
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

async fn add_member(app: Router, kind: &str, code: &str) -> axum::http::Response<Body> {
    post_json(
        app,
        &format!("/bss-products/v1/recognized-sets/{kind}/members"),
        &json!({ "member_code": code }),
    )
    .await
}

async fn transition(
    app: Router,
    kind: &str,
    code: &str,
    expected: &str,
    to: &str,
) -> axum::http::Response<Body> {
    post_json(
        app,
        &format!("/bss-products/v1/recognized-sets/{kind}/members/{code}/transitions"),
        &json!({ "to": to, "expected_state": expected }),
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
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        usage_type_resolver: crate::test_support::resolved_usage_types(),
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
    let now = "2026-08-29 09:00:00.000000 +00:00";
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
    let now = "2026-08-29 09:00:00.000000 +00:00";
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
    let response = add_member(app_for(&harness, TENANT), "metering_unit", "gib_month").await;
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

    let tier = add_member(app_for(&harness, TENANT), "plan_tier", "gold").await;
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
    let first = add_member(app_for(&harness, TENANT), "metering_unit", "gib_month").await;
    assert_eq!(first.status(), axum::http::StatusCode::CREATED);

    let again = add_member(app_for(&harness, TENANT), "metering_unit", "gib_month").await;
    assert_eq!(again.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(error_code(again).await, "DUPLICATE_CODE");
}

/// **The machine walks deprecate → remove → re-list, refuses the shortcut,
/// and announces every flip.**
#[tokio::test]
async fn the_machine_walks_its_edges_and_refuses_the_shortcut() {
    let harness = harness().await;
    add_member(app_for(&harness, TENANT), "metering_unit", "gib_month").await;

    let shortcut = transition(
        app_for(&harness, TENANT),
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
        app_for(&harness, TENANT),
        "metering_unit",
        "gib_month",
        "active",
        "deprecated",
    )
    .await;
    assert_eq!(deprecated.status(), axum::http::StatusCode::OK);
    assert_eq!(body_json(deprecated).await["state"], "deprecated");

    let removed = transition(
        app_for(&harness, TENANT),
        "metering_unit",
        "gib_month",
        "deprecated",
        "removed",
    )
    .await;
    assert_eq!(removed.status(), axum::http::StatusCode::OK);

    let relisted = transition(
        app_for(&harness, TENANT),
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
    add_member(app_for(&harness, TENANT), "metering_unit", "gib_month").await;
    transition(
        app_for(&harness, TENANT),
        "metering_unit",
        "gib_month",
        "active",
        "deprecated",
    )
    .await;

    let stale = transition(
        app_for(&harness, TENANT),
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
    add_member(app_for(&harness, TENANT), "metering_unit", "gib_month").await;
    let holder = seed_holder(&harness, "SKU-HOLDER", "gib_month").await;

    transition(
        app_for(&harness, TENANT),
        "metering_unit",
        "gib_month",
        "active",
        "deprecated",
    )
    .await;

    let blocked = transition(
        app_for(&harness, TENANT),
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
        app_for(&harness, TENANT),
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
        app_for(&harness, TENANT),
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
         'seeded_gib', NULL, 'active', 'platform-seed', '2026-08-29 09:00:00.000000 +00:00', \
         '2026-08-29 09:00:00.000000 +00:00')",
        tenant = TENANT.simple(),
    ))
    .await
    .expect("seed the member");

    let deprecated = transition(
        app_for(&harness, TENANT),
        "metering_unit",
        "seeded_gib",
        "active",
        "deprecated",
    )
    .await;
    assert_eq!(deprecated.status(), axum::http::StatusCode::OK);

    let removal = transition(
        app_for(&harness, TENANT),
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

/// **The kind roster is closed at the path**: an unknown `setKind` is a
/// validation refusal, never a default set.
#[tokio::test]
async fn an_unknown_set_kind_is_refused_closed() {
    let harness = harness().await;
    let response = add_member(app_for(&harness, TENANT), "units", "gib_month").await;
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

/// A transition on a member the set never carried answers the bare 404.
#[tokio::test]
async fn a_transition_on_an_unknown_member_is_not_found() {
    let harness = harness().await;
    let response = transition(
        app_for(&harness, TENANT),
        "metering_unit",
        "ghost",
        "active",
        "deprecated",
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}
