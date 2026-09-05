//! Tests for the watermark door, the two membership ops and the reference
//! predicate (`dod-watermark-door`, `dod-producer-table`,
//! `dod-reference-predicate`; P-D-71, P-D-87).

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration as ChronoDuration, Utc};
use sea_orm_migration::MigratorTrait as _;
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use tower::ServiceExt as _;
use uuid::Uuid;

use super::{ProducerVerdict, evaluate_reference, router};
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::test_support::{authed_ctx, flat_in_enforcer};

const TENANT: Uuid = Uuid::from_u128(0xfe_01);
const SKU: Uuid = Uuid::from_u128(0xfe_02);
const OTHER_SKU: Uuid = Uuid::from_u128(0xfe_03);

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
    let path = std::env::temp_dir().join(format!("bss-products-ref-{}.sqlite3", Uuid::new_v4()));
    let dsn = format!("sqlite://{}?mode=rwc", path.display());
    let db = connect_db(
        &dsn,
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
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
    let defaults = ProductsConfig::default();
    let state = Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
        idempotency_retention_hours: defaults.idempotency_retention_hours,
        bulk_max_rows_per_batch: defaults.bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: defaults.bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: defaults.watermark_skew_tolerance(),
        reference: crate::api::rest::ReferenceKnobs::from(&defaults),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        eol_enabled: false,
        usage_type_resolver: crate::test_support::resolved_usage_types(),
    });
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(TENANT)))
}

/// Return the pinned connection before the next door call checks one out.
fn return_pinned<T>(conn: T) {
    let _returned = conn;
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

/// An anchor in the recent past, captured once per test.
///
/// **Not a fixed calendar instant**: the door compares `watermark_at`
/// against the *receiving clock* plus the configured skew, so a literal
/// date is refused `WATERMARK_FUTURE` whenever the suite runs before it —
/// which is what a fixed 12:00 UTC anchor did on the first run of this
/// file.
fn anchor() -> chrono::DateTime<Utc> {
    Utc::now() - ChronoDuration::minutes(1)
}

async fn post_json(app: Router, uri: &str, body: &serde_json::Value) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .extension(authed_ctx(TENANT))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn post_empty(app: Router, uri: &str) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(uri)
            .extension(authed_ctx(TENANT))
            .body(Body::empty())
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read the response body");
    serde_json::from_slice(&bytes).expect("the response body is JSON")
}

/// A published SKU row this registry knows — the referent an override row's
/// foreign key needs. Written in statements the head guard admits, the way
/// the sets suite seeds its holders.
async fn seed_sku_row(harness: &TestHarness, sku_code: &str) -> Uuid {
    use sea_orm::ConnectionTrait as _;
    let conn = sea_orm::Database::connect(&harness.dsn)
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
             region_scope, brand_scope, created_by, created_at, updated_at) \
             VALUES (X'{sku}', X'{tenant}', X'{prod}', '{sku_code}', 'draft', 1, 0, 0, '', '', \
             'principal:author-1', '{now}', '{now}')",
            sku = sku_id.simple(),
            tenant = TENANT.simple(),
            prod = product_id.simple(),
        ),
        format!(
            "INSERT INTO products_entity_version (tenant_id, entity_kind, entity_id, \
             published_version, content, content_digest, digest_version, actor_ref, \
             published_at) VALUES (X'{tenant}', 'sku', X'{sku}', 1, '{{}}', X'00', 1, \
             X'{tenant}', '{now}')",
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

/// The in-test approval double every producer op needs under the stored host
/// (`dod-producer-registration`, P-D-147): one satisfied record for the exact
/// subject the door presents.
async fn seed_producer_op(harness: &TestHarness, producer: &str) {
    crate::test_support::seed_satisfied_approval(
        &harness.db,
        TENANT,
        crate::api::rest::reference::producer_op_subject(TENANT, producer),
        0,
    )
    .await;
}

async fn register(harness: &TestHarness, producer: &str) {
    seed_producer_op(harness, producer).await;
    let response = post_json(
        app(harness),
        "/bss-products/v1/reference-producers",
        &json!({ "producer": producer }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "premise: registered"
    );
}

/// Retire through the door with the double seeded; `justification` selects
/// the dead-producer lane.
async fn retire(
    harness: &TestHarness,
    producer: &str,
    justification: Option<&str>,
) -> axum::http::Response<Body> {
    seed_producer_op(harness, producer).await;
    let uri = format!("/bss-products/v1/reference-producers/{producer}/retirements");
    match justification {
        Some(text) => post_json(app(harness), &uri, &json!({ "justification": text })).await,
        None => post_empty(app(harness), &uri).await,
    }
}

fn watermark_body(producer: &str, at: chrono::DateTime<Utc>, skus: &[Uuid]) -> serde_json::Value {
    json!({
        "producer": producer,
        "watermark_at": at,
        "sku_ids": skus,
    })
}

/// A registered producer's post lands, and the identical re-post is the
/// admitted idempotent replay — told apart from a conflict by the stored
/// set hash (P-D-71).
#[tokio::test]
async fn a_post_lands_and_an_identical_repost_replays() {
    let harness = harness().await;
    register(&harness, "pricing").await;

    let at = anchor();
    let first = post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", at, &[SKU]),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let view = body_json(first).await;
    assert_eq!(view["member_count"], json!(1));
    assert_eq!(view["replayed"], json!(false));

    let replay = post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", at, &[SKU]),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(body_json(replay).await["replayed"], json!(true));
}

/// An unregistered poster is refused 403 — the identity is the subject.
#[tokio::test]
async fn an_unregistered_poster_is_refused() {
    let harness = harness().await;
    let refused = post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("billing", anchor(), &[SKU]),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_json(refused).await["context"]["reason"],
        json!("PRODUCER_UNREGISTERED")
    );
}

/// The three timestamp verdicts: an older instant regresses, an equal one
/// with a different set conflicts, and a newer one replaces the set whole.
#[tokio::test]
async fn the_timestamp_verdicts_are_told_apart() {
    let harness = harness().await;
    register(&harness, "pricing").await;
    let at = anchor();
    post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", at, &[SKU]),
    )
    .await;

    let older = post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", at - ChronoDuration::minutes(1), &[SKU]),
    )
    .await;
    assert_eq!(older.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(older).await["context"]["reason"],
        json!("WATERMARK_REGRESSION")
    );

    let conflict = post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", at, &[SKU, OTHER_SKU]),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(conflict).await["context"]["reason"],
        json!("WATERMARK_CONFLICT")
    );

    let newer = post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", at + ChronoDuration::seconds(30), &[OTHER_SKU]),
    )
    .await;
    assert_eq!(
        newer.status(),
        StatusCode::OK,
        "a newer post replaces the set"
    );
}

/// The future bound: a post above the receiving clock plus the configured
/// skew is refused. The bound is p1 because one accepted future post
/// inverts the never-falsely-free invariant.
#[tokio::test]
async fn a_future_dated_post_is_refused() {
    let harness = harness().await;
    register(&harness, "pricing").await;
    let far_future = Utc::now() + ChronoDuration::hours(1);
    let refused = post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", far_future, &[SKU]),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(refused).await["context"]["violations"][0]["type"],
        json!("WATERMARK_FUTURE")
    );
}

/// Retirement clears the watermark and its members, so a re-registered
/// producer starts never-received (P-D-87 arm 2) — onboarding can only
/// tighten, never free.
#[tokio::test]
async fn retirement_clears_the_watermark_and_re_registration_starts_never_received() {
    let harness = harness().await;
    register(&harness, "pricing").await;
    post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", Utc::now(), &[SKU]),
    )
    .await;

    // A second producer, so the retirement is not the set's last
    // (`PRODUCER_SET_EMPTY_FORBIDDEN`); the one under test is fresh.
    register(&harness, "billing").await;
    let retired = retire(&harness, "pricing", None).await;
    assert_eq!(retired.status(), StatusCode::OK);
    assert_eq!(body_json(retired).await["state"], json!("retired"));

    let conn = harness.db.conn().expect("conn");
    assert_eq!(
        crate::infra::storage::repo::find_reference_watermark(&conn, &scope(), TENANT, "pricing")
            .await
            .expect("read"),
        None,
        "the retirement cleared the watermark"
    );
    assert!(
        !crate::infra::storage::repo::reference_member_exists(
            &conn,
            &scope(),
            TENANT,
            "pricing",
            SKU
        )
        .await
        .expect("read"),
        "and its member rows"
    );
    return_pinned(conn);

    register(&harness, "pricing").await;
    let conn = harness.db.conn().expect("conn");
    let verdict = evaluate_reference(
        &conn,
        &scope(),
        TENANT,
        SKU,
        Utc::now(),
        ProductsConfig::default().reference_freshness(),
    )
    .await
    .expect("evaluate");
    assert_eq!(
        verdict.per_producer[0].1,
        ProducerVerdict::ConservativelyReferencedNeverReceived,
        "a re-registered producer starts never-received, so onboarding cannot free"
    );
    assert!(verdict.referenced, "and the SKU stays held");
}

/// The predicate's four verdicts, and the OR that is never a sum.
#[tokio::test]
async fn the_predicate_answers_four_verdicts() {
    let harness = harness().await;
    let conn = harness.db.conn().expect("conn");

    // Fourth verdict: no registered producer at all — conservative, and
    // distinct from a fresh zero.
    let empty = evaluate_reference(
        &conn,
        &scope(),
        TENANT,
        SKU,
        Utc::now(),
        ProductsConfig::default().reference_freshness(),
    )
    .await
    .expect("evaluate");
    assert!(
        empty.no_producers && empty.referenced,
        "an empty set frees nothing"
    );
    return_pinned(conn);

    register(&harness, "pricing").await;
    register(&harness, "contracts").await;
    post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", Utc::now(), &[SKU]),
    )
    .await;

    let conn = harness.db.conn().expect("conn");
    let mixed = evaluate_reference(
        &conn,
        &scope(),
        TENANT,
        SKU,
        Utc::now(),
        ProductsConfig::default().reference_freshness(),
    )
    .await
    .expect("evaluate");
    assert!(mixed.referenced);
    let verdicts: std::collections::BTreeMap<_, _> = mixed.per_producer.into_iter().collect();
    assert_eq!(verdicts["pricing"], ProducerVerdict::Referenced);
    assert_eq!(
        verdicts["contracts"],
        ProducerVerdict::ConservativelyReferencedNeverReceived,
        "a silent producer holds the SKU under its own distinct flag"
    );

    // Fresh-zero requires EVERY registered producer fresh and omitting.
    let other = evaluate_reference(
        &conn,
        &scope(),
        TENANT,
        OTHER_SKU,
        Utc::now(),
        ProductsConfig::default().reference_freshness(),
    )
    .await
    .expect("evaluate");
    assert!(
        other.referenced,
        "pricing omits it but contracts never posted, so it is not free"
    );
    return_pinned(conn);

    post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("contracts", Utc::now(), &[]),
    )
    .await;
    let conn = harness.db.conn().expect("conn");
    let freed = evaluate_reference(
        &conn,
        &scope(),
        TENANT,
        OTHER_SKU,
        Utc::now(),
        ProductsConfig::default().reference_freshness(),
    )
    .await
    .expect("evaluate");
    assert!(
        !freed.referenced,
        "with every producer fresh and omitting, and only then, the SKU is free"
    );
}

/// The in-process binding is the default deployment mode (P-D-15), so this
/// is the case that proves "identical gate and core" is more than the
/// module doc's claim: a post through the trait lands in the store the
/// wire door replays from, a duplicated id is deduplicated by the shared
/// core, the configured set bound refuses through the same path, and an
/// unregistered producer is refused by the same gate.
#[tokio::test]
async fn the_in_process_binding_shares_the_gate_and_the_store() {
    use bss_products_sdk::watermarks::{WatermarkPost, WatermarkPosts as _};

    let harness = harness().await;
    register(&harness, "pricing").await;

    let defaults = ProductsConfig::default();
    let binding = super::InProcessWatermarkPosts {
        state: Arc::new(ApiState {
            db: harness.db.clone(),
            sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
            taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
            idempotency_retention_hours: defaults.idempotency_retention_hours,
            // Small on purpose: the set bound must be provable through the
            // binding, which no HTTP body limit covers.
            bulk_max_rows_per_batch: 2,
            bulk_max_concurrent_batches_per_tenant: defaults.bulk_max_concurrent_batches_per_tenant,
            watermark_skew_tolerance: defaults.watermark_skew_tolerance(),
            reference: crate::api::rest::ReferenceKnobs::from(&defaults),
            breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
            breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
            eol_enabled: false,
            usage_type_resolver: crate::test_support::resolved_usage_types(),
        }),
        enforcer: flat_in_enforcer(TENANT),
    };
    let ctx = authed_ctx(TENANT);

    // A duplicated id is deduplicated at the shared core, so the hash, the
    // stored rows and the acked member_count all describe the same set.
    let at = anchor();
    let ack = binding
        .post(
            &ctx,
            TENANT,
            WatermarkPost {
                producer: "pricing".to_owned(),
                watermark_at: at,
                sku_ids: vec![SKU, SKU],
            },
        )
        .await
        .expect("the binding's post lands");
    assert_eq!(ack.member_count, 1, "the duplicated id is one member");
    assert!(!ack.replayed);

    // One contract, one store: the WIRE door's re-post of the binding's
    // watermark is the admitted idempotent replay.
    let replay = post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", at, &[SKU]),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(body_json(replay).await["replayed"], json!(true));

    // The set bound refuses through the binding — the path an HTTP-layer
    // body limit never sees.
    let oversized = binding
        .post(
            &ctx,
            TENANT,
            WatermarkPost {
                producer: "pricing".to_owned(),
                watermark_at: at + ChronoDuration::seconds(30),
                sku_ids: vec![SKU, OTHER_SKU, Uuid::from_u128(0xfe_04)],
            },
        )
        .await
        .expect_err("three ids are above the ceiling of two");
    assert_eq!(oversized.status_code(), 400);

    // The gate is spent on the binding's path too.
    let unregistered = binding
        .post(
            &ctx,
            TENANT,
            WatermarkPost {
                producer: "billing".to_owned(),
                watermark_at: at,
                sku_ids: vec![SKU],
            },
        )
        .await
        .expect_err("an unregistered producer is refused");
    assert_eq!(unregistered.status_code(), 403);
}

/// A stale watermark holds the SKU under the stale flag — the condition
/// the alarm fires on, this evaluation emitting nothing (P-D-59).
#[tokio::test]
async fn a_stale_watermark_holds_the_sku_conservatively() {
    let harness = harness().await;
    register(&harness, "pricing").await;
    let posted_at = Utc::now() - ChronoDuration::minutes(30);
    post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", posted_at, &[]),
    )
    .await;

    let conn = harness.db.conn().expect("conn");
    let verdict = evaluate_reference(
        &conn,
        &scope(),
        TENANT,
        SKU,
        Utc::now(),
        ProductsConfig::default().reference_freshness(),
    )
    .await
    .expect("evaluate");
    assert_eq!(
        verdict.per_producer[0].1,
        ProducerVerdict::ConservativelyReferencedStale,
        "30 minutes is past the 15-minute interim threshold"
    );
    assert!(verdict.referenced, "a stale producer never frees");
}

/// The accepted half of `dod-reference-audit`: every accepted act of this
/// surface writes an audit row, not only every refusal.
mod accepted_audit_tests {
    use super::*;

    /// How many audit rows the plane carries for `action`.
    ///
    /// Asked per action rather than by reading the whole plane: the surface
    /// writes several rows per case and a list assertion would pin an
    /// ordering nothing promises.
    ///
    /// **A count, not `raw_string_opt`.** That helper's `Option` is the
    /// *column's* nullability and it panics on a missing row, so a
    /// presence probe built on it can only answer `true` or panic — which
    /// makes every assertion message below unreachable and a negative
    /// control unwritable.
    async fn audit_rows(dsn: &str, action: &str) -> i64 {
        crate::test_support::raw_i64(
            dsn,
            &format!("SELECT COUNT(*) AS v FROM products_audit_log WHERE action = '{action}'"),
        )
        .await
    }

    /// **A registration, a watermark post and a retirement each leave a
    /// row.**
    ///
    /// The accepted half is the one the investigation needs: ingestion emits
    /// no broker event by design and the watermark row is overwritten in
    /// place, so this row is the only record of what the producer claimed.
    /// Before this change the surface audited refusals only, and every
    /// assertion about it would have passed.
    #[tokio::test]
    async fn every_accepted_act_of_this_surface_is_audited() {
        let harness = harness().await;
        // Two producers, because the retirement below must not be the
        // last one: `PRODUCER_SET_EMPTY_FORBIDDEN` is declared and unbuilt,
        // and a one-producer fixture would redden this *audit* probe the
        // day that guard lands — a failure that reads as an audit
        // regression rather than as a fixture needing a second producer.
        for producer in ["pricing", "rating"] {
            seed_producer_op(&harness, producer).await;
            let response = post_json(
                app(&harness),
                "/bss-products/v1/reference-producers",
                &serde_json::json!({ "producer": producer }),
            )
            .await;
            assert_eq!(response.status(), axum::http::StatusCode::CREATED);
        }
        assert_eq!(
            audit_rows(&harness.dsn, "reference_producer_register").await,
            2,
            "each accepted registration leaves its own row"
        );

        // Bound once: `anchor()` is `now - 1min` and answers a NEW value on
        // every call, so re-posting `anchor()` would be a *later* instant
        // and take the write path instead of the replay's.
        let at = anchor();
        let posted = post_json(
            app(&harness),
            "/bss-products/v1/reference-watermarks",
            &watermark_body("pricing", at, &[SKU]),
        )
        .await;
        assert_eq!(posted.status(), axum::http::StatusCode::OK);
        assert_eq!(
            audit_rows(&harness.dsn, "reference_watermark_post").await,
            1,
            "an accepted post leaves the only record of what the producer claimed"
        );
        // The negative control the count makes writable: nothing has
        // replayed yet, so the replay action carries no row. A presence
        // probe that could only answer `true` could not assert this.
        assert_eq!(
            audit_rows(&harness.dsn, "reference_watermark_replay").await,
            0,
            "no replay has happened yet"
        );

        // **An admitted replay is an accepted post and owes its own row.**
        // The same instant and the same set: the door answers 200 and
        // changes nothing, and that is exactly the act a producer's retry
        // loop repeats.
        let replayed = post_json(
            app(&harness),
            "/bss-products/v1/reference-watermarks",
            &watermark_body("pricing", at, &[SKU]),
        )
        .await;
        assert_eq!(replayed.status(), axum::http::StatusCode::OK);
        assert_eq!(
            audit_rows(&harness.dsn, "reference_watermark_replay").await,
            1,
            "the replay is audited under its own action, so a producer that \
             posted once is distinguishable from one that posted fifty times"
        );
        assert_eq!(
            audit_rows(&harness.dsn, "reference_watermark_post").await,
            1,
            "and the replay did not file itself as a write"
        );

        register(&harness, "billing").await;
        let retired = retire(&harness, "pricing", None).await;
        assert_eq!(retired.status(), axum::http::StatusCode::OK);
        assert_eq!(
            audit_rows(&harness.dsn, "reference_producer_retire").await,
            1,
            "an accepted retirement leaves a row"
        );
    }
}

/// **The producer doors have a gate** (`dod-producer-registration`, P-D-147):
/// without a satisfied record for the producer's `GovernedLiveOp` subject a
/// registration answers `APPROVAL_REQUIRED` and writes nothing; with it the
/// set moves and `ReferenceProducerSetChanged` announces per tenant.
#[tokio::test]
async fn a_producer_op_without_a_satisfied_record_is_refused_approval_required() {
    let harness = harness().await;
    let refused = post_json(
        app(&harness),
        "/bss-products/v1/reference-producers",
        &json!({ "producer": "pricing" }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_json(refused).await["context"]["reason"],
        json!("APPROVAL_REQUIRED")
    );
    assert_eq!(
        crate::test_support::enqueued_event_count(&harness.dsn, "ReferenceProducerSetChanged")
            .await,
        0
    );

    register(&harness, "pricing").await;
    assert_eq!(
        crate::test_support::enqueued_event_count(&harness.dsn, "ReferenceProducerSetChanged")
            .await,
        1,
        "a registration announces the set change in its own transaction"
    );
}

/// **The retirement rule, P-D-129 rows 2 and 5** (`dod-producer-registration`):
/// the last registered producer never retires; a stale or never-received one
/// retires only with the break-glass justification, which passes the PII gate
/// and leaves one `producer_unavailable` override row per SKU it held plus an
/// audit row carrying the ceremony reference.
#[tokio::test]
async fn retiring_the_last_or_a_dead_producer_is_refused_unless_break_glass_justifies_it() {
    let harness = harness().await;
    register(&harness, "pricing").await;

    let last = retire(&harness, "pricing", None).await;
    assert_eq!(last.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(last).await["context"]["reason"],
        json!("PRODUCER_SET_EMPTY_FORBIDDEN")
    );

    register(&harness, "billing").await;
    // pricing posts a stale watermark naming one real SKU: it now holds that
    // SKU conservatively, and its retirement would free it.
    let held = seed_sku_row(&harness, "HELD-1").await;
    let stale_at = Utc::now() - chrono::Duration::hours(2);
    let posted = post_json(
        app(&harness),
        "/bss-products/v1/reference-watermarks",
        &watermark_body("pricing", stale_at, &[held]),
    )
    .await;
    assert_eq!(
        posted.status(),
        StatusCode::OK,
        "premise: the stale watermark lands"
    );

    let would_free = retire(&harness, "pricing", None).await;
    assert_eq!(would_free.status(), StatusCode::CONFLICT);
    let body = body_json(would_free).await;
    assert_eq!(
        body["context"]["reason"],
        json!("PRODUCER_RETIREMENT_WOULD_FREE")
    );
    assert!(
        body.to_string().contains("stale"),
        "the refusal names the standing: {body}"
    );

    let pii = retire(&harness, "pricing", Some("requested by Ann Fritz")).await;
    assert_eq!(
        pii.status(),
        StatusCode::BAD_REQUEST,
        "a person-shaped justification fails 02's gate"
    );
    assert_eq!(
        body_json(pii).await["context"]["violations"][0]["type"],
        json!("CONTENT_PII_BLOCKED")
    );

    let retired = retire(
        &harness,
        "pricing",
        Some("producer decommissioned, host gone"),
    )
    .await;
    assert_eq!(
        retired.status(),
        StatusCode::OK,
        "{}",
        body_json(retired).await
    );
    assert_eq!(
        crate::test_support::raw_i64(
            &harness.dsn,
            "SELECT count(*) AS v FROM products_correction_override \
             WHERE admitting_arm = 'producer_unavailable' AND field = 'producer_retirement'"
        )
        .await,
        1,
        "one override row per SKU the dead producer held"
    );
    assert_eq!(
        crate::test_support::raw_i64(
            &harness.dsn,
            "SELECT count(*) AS v FROM products_audit_log \
             WHERE action = 'reference_producer_retire_breakglass' AND ceremony_ref IS NOT NULL"
        )
        .await,
        1,
        "the audit row carries the ceremony reference"
    );
    assert_eq!(
        crate::test_support::raw_i64(
            &harness.dsn,
            "SELECT count(*) AS v FROM products_audit_log a JOIN products_correction_override o \
             ON a.ceremony_ref = o.ceremony_ref"
        )
        .await,
        1,
        "the ceremony and the evidence are joinable from either side"
    );
    assert_eq!(
        crate::test_support::enqueued_event_count(&harness.dsn, "ReferenceProducerSetChanged")
            .await,
        3,
        "two registrations and one retirement"
    );
}

/// **The tripwire is a windowed count over the override table**
/// (`dod-tripwire`, P-D-129 rows 8 and 9): the sixth `producer_unavailable`
/// row in thirty days trips it and raises the derived
/// `signal_delivery_release_blocker`; the `unresolvable_target` arm counts
/// separately and never raises the blocker.
#[tokio::test]
async fn the_tripwire_trips_on_the_sixth_override_in_the_window() {
    let harness = harness().await;
    let conn = harness.db.conn().expect("conn");
    let knobs = crate::api::rest::ReferenceKnobs::from(&ProductsConfig::default());
    let now = Utc::now();
    let mut last = None;
    for n in 0..6_u32 {
        crate::infra::storage::repo::record_correction_override(
            &conn,
            &scope(),
            TENANT,
            crate::infra::storage::repo::NewCorrectionOverride {
                override_id: Uuid::now_v7(),
                sku_id: seed_sku_row(&harness, &format!("TRIP-{n}")).await,
                field: "meter".to_owned(),
                reason: "signal unavailable".to_owned(),
                evidence: crate::infra::storage::repo::OverrideEvidence::ProducerUnavailable {
                    snapshot: "{}".to_owned(),
                },
                ceremony_ref: Uuid::now_v7(),
                recorded_at: now - chrono::Duration::days(i64::from(n)),
            },
        )
        .await
        .expect("record the override");
        last = Some(
            crate::api::rest::reference::tripwire_after_override(
                &conn,
                &scope(),
                TENANT,
                knobs,
                "producer_unavailable",
                now,
            )
            .await
            .expect("count the window"),
        );
        if n < 5 {
            assert!(
                !last.expect("verdict").tripped,
                "{} rows do not trip",
                n + 1
            );
        }
    }
    let verdict = last.expect("verdict");
    assert_eq!(verdict.count, 6);
    assert!(verdict.tripped, "the sixth within the window trips");
    assert!(
        crate::api::rest::reference::signal_delivery_release_blocker(
            &conn,
            &scope(),
            TENANT,
            knobs,
            now
        )
        .await
        .expect("derive the blocker"),
        "the blocker is derived from the same window"
    );
    // The other arm's rows are its own population.
    let other = crate::api::rest::reference::tripwire_after_override(
        &conn,
        &scope(),
        TENANT,
        knobs,
        "unresolvable_target",
        now,
    )
    .await
    .expect("count the other arm");
    assert_eq!(other.count, 0);
    assert!(!other.tripped);
    // Past the window the rows fall out and the blocker clears.
    assert!(
        !crate::api::rest::reference::signal_delivery_release_blocker(
            &conn,
            &scope(),
            TENANT,
            knobs,
            now + chrono::Duration::days(31)
        )
        .await
        .expect("derive the blocker later"),
        "a rolling window, not stored state"
    );
    return_pinned(conn);
}
