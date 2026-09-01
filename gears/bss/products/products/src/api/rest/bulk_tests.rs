//! Tests for the import door and the `RowLedger` reader (`dod-import-door`,
//! the reporting half of `dod-bulk-errors`, and `dod-idempotency-lane`'s
//! constant).

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use sea_orm_migration::MigratorTrait as _;
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use tower::ServiceExt as _;
use uuid::Uuid;

use super::{INTERNAL_BULK_ROW_LANE, router};
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::test_support::{authed_ctx, flat_in_enforcer};

const TENANT: Uuid = Uuid::from_u128(0xb0_01);

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
    let path = std::env::temp_dir().join(format!("bss-products-bulk-{}.sqlite3", Uuid::new_v4()));
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

fn app_for(harness: &TestHarness, tenant: Uuid, max_rows: u32, max_batches: u32) -> Router {
    let state = Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: max_rows,
        bulk_max_concurrent_batches_per_tenant: max_batches,
    });
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

fn default_app(harness: &TestHarness) -> Router {
    let defaults = ProductsConfig::default();
    app_for(
        harness,
        TENANT,
        defaults.bulk_max_rows_per_batch,
        defaults.bulk_max_concurrent_batches_per_tenant,
    )
}

async fn post_import(app: Router, body: &serde_json::Value) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/bss-products/v1/bulk/imports")
            .header("content-type", "application/json")
            .extension(authed_ctx(TENANT))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn get_batch(app: Router, batch_id: &str) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(format!("/bss-products/v1/bulk/batches/{batch_id}"))
            .extension(authed_ctx(TENANT))
            .body(Body::empty())
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("read the response body");
    serde_json::from_slice(&bytes).expect("the response body is JSON")
}

fn two_rows() -> serde_json::Value {
    json!([
        { "row_key": "r-1", "entity_kind": "product" },
        { "row_key": "r-2", "entity_kind": "sku" },
    ])
}

/// A batch lands 202 with its whole ledger, `staging`, mode defaulting to
/// the strict `import`; the reader answers one entry per row, each in
/// flight (no disposition) and each carrying its own lane client key.
#[tokio::test]
async fn a_batch_lands_with_its_ledger_and_the_reader_reports_it() {
    let harness = harness().await;
    let response = post_import(
        default_app(&harness),
        &json!({ "batch_key": "onboard-1", "rows": two_rows() }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "the door answers 202"
    );
    let view = body_json(response).await;
    assert_eq!(view["state"], json!("staging"));
    assert_eq!(
        view["mode"],
        json!("import"),
        "the strict mode is the default"
    );
    assert_eq!(view["row_count"], json!(2));
    assert_eq!(view["replayed"], json!(false));

    let batch_id = view["batch_id"].as_str().expect("batch_id").to_owned();
    let ledger = get_batch(default_app(&harness), &batch_id).await;
    assert_eq!(ledger.status(), StatusCode::OK);
    let ledger = body_json(ledger).await;
    let rows = ledger["rows"].as_array().expect("the ledger");
    assert_eq!(
        rows.len(),
        2,
        "one entry per row: the partial-failure surface"
    );
    assert_eq!(rows[0]["row_key"], json!("r-1"));
    assert_eq!(
        rows[0]["disposition"],
        json!(null),
        "in flight until the worker runs"
    );
    assert!(
        rows[0]["row_id"]
            .as_str()
            .is_some_and(|id| id != rows[1]["row_id"]),
        "each row carries its own lane client key"
    );
}

/// The batch key is the door's idempotency: a replay answers the existing
/// batch rather than minting a second, and the ledger is not duplicated.
#[tokio::test]
async fn a_replayed_batch_key_answers_the_existing_batch() {
    let harness = harness().await;
    let body = json!({ "batch_key": "onboard-2", "rows": two_rows() });

    let first = body_json(post_import(default_app(&harness), &body).await).await;
    let replay = post_import(default_app(&harness), &body).await;
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    let replay = body_json(replay).await;
    assert_eq!(replay["batch_id"], first["batch_id"], "the same batch");
    assert_eq!(replay["replayed"], json!(true));
    assert_eq!(replay["row_count"], json!(2), "the ledger was not doubled");
}

/// `promote` is the mode a caller must ask for; anything else is the
/// ordinary shape refusal.
#[tokio::test]
async fn the_mode_is_carried_and_junk_is_refused() {
    let harness = harness().await;
    let promote = body_json(
        post_import(
            default_app(&harness),
            &json!({ "batch_key": "promote-1", "mode": "promote", "rows": two_rows() }),
        )
        .await,
    )
    .await;
    assert_eq!(promote["mode"], json!("promote"));

    let junk = post_import(
        default_app(&harness),
        &json!({ "batch_key": "promote-2", "mode": "overwrite", "rows": two_rows() }),
    )
    .await;
    assert_eq!(junk.status(), StatusCode::BAD_REQUEST);
    let view = body_json(junk).await;
    assert_eq!(
        view["context"]["violations"][0]["subject"],
        json!("mode"),
        "the refusal names the field"
    );
}

/// Both of `inst-bm-limits`' operands refuse under the one code, and the
/// row-count bound is the door's own.
#[tokio::test]
async fn both_bounds_refuse_bulk_limit() {
    let harness = harness().await;

    let too_many = post_import(
        app_for(&harness, TENANT, 1, 5),
        &json!({ "batch_key": "big-1", "rows": two_rows() }),
    )
    .await;
    assert_eq!(too_many.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(too_many).await["context"]["reason"],
        json!("BULK_LIMIT")
    );

    // The ceiling: one live batch admitted, the second refused.
    let app = app_for(&harness, TENANT, 100, 1);
    let first = post_import(app, &json!({ "batch_key": "c-1", "rows": two_rows() })).await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let second = post_import(
        app_for(&harness, TENANT, 100, 1),
        &json!({ "batch_key": "c-2", "rows": two_rows() }),
    )
    .await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let view = body_json(second).await;
    assert_eq!(view["context"]["reason"], json!("BULK_LIMIT"));
    assert!(
        view["detail"]
            .as_str()
            .is_some_and(|d| d.contains("ceiling")),
        "the two operands are told apart in the sentence: {view}"
    );
}

/// Row keys are batch-scoped and must be unique within the batch; a live
/// entity kind this gear cannot stage yet is refused rather than silently
/// queued.
#[tokio::test]
async fn the_row_shape_is_judged_in_one_report() {
    let harness = harness().await;
    let refused = post_import(
        default_app(&harness),
        &json!({
            "batch_key": "shape-1",
            "rows": [
                { "row_key": "dup", "entity_kind": "product" },
                { "row_key": "dup", "entity_kind": "product" },
                { "row_key": "cat", "entity_kind": "category" },
            ]
        }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let violations = body_json(refused).await["context"]["violations"]
        .as_array()
        .expect("violations")
        .len();
    assert!(
        violations >= 2,
        "one collected report carries both the duplicate key and the unstageable kind"
    );
}

/// An unknown batch is a 404 under the reader's own grant.
#[tokio::test]
async fn an_unknown_batch_is_a_404() {
    let harness = harness().await;
    let missing = get_batch(default_app(&harness), &Uuid::now_v7().to_string()).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

/// The reserved lane's constant now exists in code, which is what
/// `dod-idempotency-lane` obliges beyond a caller: a build that minted a
/// new name would leave the reserved one dead.
#[test]
fn the_reserved_lane_carries_its_declared_name() {
    assert_eq!(INTERNAL_BULK_ROW_LANE, "internal:bulk-row");
}
