//! Tests for the increment-request door and its in-process binding —
//! `dod-request-door`'s criteria (`features/catalog-version.md` §6:
//! `REQUEST_SOURCE_UNKNOWN`'s both-halves probe) and the P-D-81 contract.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use sea_orm::{ConnectionTrait, Database};
use sea_orm_migration::MigratorTrait;
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use tower::ServiceExt as _;
use uuid::Uuid;

use bss_products_sdk::increments::{IncrementLane, IncrementRequest, IncrementRequests as _};

use super::{InProcessIncrementRequests, router};
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::test_support::{authed_ctx, flat_in_enforcer};

fn unique_sqlite_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "bss-products-tests-{label}-{}.sqlite3",
        Uuid::new_v4()
    ))
}

const TENANT: Uuid = Uuid::from_u128(0xca_01);

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
    let path = unique_sqlite_path("cvdb");
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

fn api_state(harness: &TestHarness) -> Arc<ApiState> {
    Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
    })
}

fn app_for(harness: &TestHarness, tenant: Uuid) -> Router {
    let openapi = OpenApiRegistryImpl::new();
    router(api_state(harness), &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

async fn post_request(
    app: Router,
    tenant: Uuid,
    body: &serde_json::Value,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/bss-products/v1/catalog-version-requests")
            .header("content-type", "application/json")
            .extension(authed_ctx(tenant))
            .body(Body::from(body.to_string()))
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

/// Seed one committed version row and flip the named request onto it, the
/// way the increment transaction will (state + FK stamped together — the
/// shape `CHECK` admits nothing else).
async fn satisfy_request(harness: &TestHarness, source: &str, request_key: &str, version: i64) {
    // An auxiliary connection into the identical file: the production
    // provider is pinned to one connection, and contending with it from a
    // seeding statement is the harness's own documented trap.
    let conn = Database::connect(&harness.dsn)
        .await
        .expect("open an auxiliary connection");
    // The version row's tenant must be byte-identical to the queue rows'
    // stored form (the composite FK compares them), so it is written by
    // copying an existing queue row's own column rather than by guessing
    // the driver's uuid encoding — the harness's own documented trap.
    conn.execute_unprepared(&format!(
        "INSERT INTO products_catalog_version (tenant_id, catalog_version_id, checksum, \
         digest_version, published_at, participant_set_snapshot, freeze_state) SELECT \
         tenant_id, {version}, 'aa', 1, '2026-09-01T10:00:00Z', '[]', 'open' FROM \
         products_catalog_version_request WHERE {} AND source = '{source}' AND \
         request_key = '{request_key}'",
        crate::test_support::id_matches("tenant_id", TENANT),
    ))
    .await
    .expect("seed the version row");
    conn.execute_unprepared(&format!(
        "UPDATE products_catalog_version_request SET state = 'coalesced', \
         satisfied_by_version_id = {version} WHERE {} AND source = \
         '{source}' AND request_key = '{request_key}'",
        crate::test_support::id_matches("tenant_id", TENANT),
    ))
    .await
    .expect("flip the request as the increment transaction will");
}

/// A registered source enqueues and is acknowledged 202/pending; the same
/// key replays the stored state rather than enqueueing a second demand, and
/// once the coalescer satisfies the row the replay answers the version.
#[tokio::test]
async fn a_request_enqueues_once_and_replays_its_state() {
    let harness = harness().await;

    let body = json!({
        "source": "pricing", "lane": "interactive", "request_key": "plan-7",
    });
    let first = post_request(app_for(&harness, TENANT), TENANT, &body).await;
    assert_eq!(
        first.status(),
        StatusCode::ACCEPTED,
        "the door acknowledges"
    );
    let first_view = body_json(first).await;
    assert_eq!(first_view["coalesced"], json!(false));
    assert_eq!(first_view["catalog_version_id"], json!(null));

    let replay = post_request(app_for(&harness, TENANT), TENANT, &body).await;
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    assert_eq!(body_json(replay).await["coalesced"], json!(false));

    satisfy_request(&harness, "pricing", "plan-7", 1).await;
    let after = post_request(app_for(&harness, TENANT), TENANT, &body).await;
    let view = body_json(after).await;
    assert_eq!(
        view["coalesced"],
        json!(true),
        "a replay of a satisfied request answers the committed state"
    );
    assert_eq!(view["catalog_version_id"], json!(1));
}

/// §6's both-halves probe: a source outside the registered set is refused
/// AFTER the grant passes, carrying the `CATALOG_VERSION_REJECTED`
/// precondition violation — and the identical request from a registered
/// source succeeds. A refusal that omitted the violation type would be
/// invisible to the consumer's `Rejected` arm.
#[tokio::test]
async fn an_unregistered_source_is_refused_with_the_discriminator() {
    let harness = harness().await;

    let refused = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "source": "billing", "lane": "interactive", "request_key": "r-1" }),
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "a FailedPrecondition renders 400 on the wire"
    );
    let view = body_json(refused).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("CATALOG_VERSION_REJECTED"),
        "the violation type is the consumer projection's discriminator"
    );

    let admitted = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "source": "pricing", "lane": "interactive", "request_key": "r-1" }),
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::ACCEPTED,
        "the same request from a registered source succeeds"
    );
}

/// The lane's batching operand is judged both ways: a bulk request must
/// name its `operation_key`, an interactive one must not.
#[tokio::test]
async fn the_operation_key_belongs_to_the_bulk_lane() {
    let harness = harness().await;

    let bulk_without = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "source": "pricing", "lane": "bulk", "request_key": "b-1" }),
    )
    .await;
    assert_eq!(bulk_without.status(), StatusCode::BAD_REQUEST);

    let interactive_with = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({
            "source": "pricing", "lane": "interactive", "request_key": "i-1",
            "operation_key": "op-1",
        }),
    )
    .await;
    assert_eq!(interactive_with.status(), StatusCode::BAD_REQUEST);

    let bulk_with = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({
            "source": "pricing", "lane": "bulk", "request_key": "b-2",
            "operation_key": "op-1",
        }),
    )
    .await;
    assert_eq!(bulk_with.status(), StatusCode::ACCEPTED);
}

/// The in-process binding runs the identical gate and core (P-D-15): a
/// request through the SDK trait lands on the same queue the wire door
/// serves, and the poll answers `None` until the coalescer satisfies the
/// row, then the committed version.
#[tokio::test]
async fn the_in_process_binding_shares_the_queue_and_the_poll() {
    let harness = harness().await;
    let binding = InProcessIncrementRequests {
        state: api_state(&harness),
        enforcer: flat_in_enforcer(TENANT),
    };
    let ctx = authed_ctx(TENANT);

    let ack = binding
        .request(
            &ctx,
            TENANT,
            IncrementRequest {
                source: "pricing".to_owned(),
                lane: IncrementLane::Interactive,
                request_key: "sdk-1".to_owned(),
                operation_key: None,
            },
        )
        .await
        .expect("the binding acknowledges");
    assert!(!ack.coalesced);

    let pending = binding
        .committed(&ctx, TENANT, "pricing", "sdk-1")
        .await
        .expect("the poll answers");
    assert_eq!(pending, None, "None while the batch has not committed");

    satisfy_request(&harness, "pricing", "sdk-1", 3).await;
    let committed = binding
        .committed(&ctx, TENANT, "pricing", "sdk-1")
        .await
        .expect("the poll answers")
        .expect("the row is satisfied");
    assert_eq!(committed.catalog_version_id, 3);

    // The wire door replays the SDK-enqueued key: one contract, one queue.
    let via_wire = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "source": "pricing", "lane": "interactive", "request_key": "sdk-1" }),
    )
    .await;
    assert_eq!(body_json(via_wire).await["catalog_version_id"], json!(3));
}

/// P-D-82: every stored instant is truncated to microseconds at the write —
/// the queue's `requested_at` here, asserted at the driver level so neither
/// engine ever holds a digit the other could round.
#[tokio::test]
async fn stored_instants_carry_no_sub_microsecond_digits() {
    let harness = harness().await;
    let response = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "source": "pricing", "lane": "interactive", "request_key": "t-1" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let stored = crate::test_support::raw_string_opt(
        &harness.dsn,
        &format!(
            "SELECT requested_at AS v FROM products_catalog_version_request WHERE \
             {} AND request_key = 't-1'",
            crate::test_support::id_matches("tenant_id", TENANT),
        ),
    )
    .await
    .expect("the row exists");
    let fraction = stored.split('.').nth(1).unwrap_or("");
    let digits: String = fraction.chars().take_while(char::is_ascii_digit).collect();
    assert!(
        digits.len() <= 6 || digits[6..].chars().all(|c| c == '0'),
        "no sub-microsecond digit survives the write: {stored}"
    );
}
