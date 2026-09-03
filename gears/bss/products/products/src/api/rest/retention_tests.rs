//! `10-retention-erasure`'s two doors, exercised through the router.
//!
//! # The subject principal is seeded through the repository, not a door
//!
//! No door mints a ref for an arbitrary principal: `resolve_creator_actor_ref`
//! mints for the **caller**, and `authed_ctx` hands every call a fresh
//! `subject_id`, so two requests never share a principal. The subject of an
//! erasure therefore has to be put there directly, through the same
//! `resolve_actor_ref` the shared actor context uses.
//!
//! # Every audit assertion counts rows rather than reading one
//!
//! `raw_string_opt` panics when its query matches no row, so a probe built on
//! it can only ever answer true; a count answers zero, which is what the
//! negative half of *"the access was audited"* needs.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use sea_orm::{ColumnTrait, EntityTrait};
use serde_json::{Value as JsonValue, json};
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::{AccessScope, SecureEntityExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use tower::ServiceExt;
use uuid::Uuid;

use sea_orm_migration::MigratorTrait;

use super::router;
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::infra::events;
use crate::infra::storage::entity::entity_version;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo;
use crate::test_support::{authed_ctx, flat_in_enforcer, raw_i64};

const TENANT: Uuid = Uuid::from_u128(0x7e_43);
const ALICE: &str = "principal:alice";

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
        "bss-products-retention-tests-{}.sqlite3",
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

fn app_for(harness: &TestHarness, tenant: Uuid) -> Router {
    let state = Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: ProductsConfig::default().bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: ProductsConfig::default()
            .bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: ProductsConfig::default().watermark_skew_tolerance(),
    });
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

/// Mint a live map entry for a principal, the way the shared actor context
/// does. See this module's own doc for why a door cannot do it.
async fn seed_principal(harness: &TestHarness, principal_ref: &str) -> Uuid {
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    repo::resolve_actor_ref(
        &conn,
        &scope,
        TENANT,
        principal_ref,
        crate::domain::canonical::write_instant(chrono::Utc::now()),
    )
    .await
    .expect("mint the subject's ref")
}

async fn erase(app: Router, principal_ref: &str, reason: &str) -> axum::http::Response<Body> {
    let body = json!({ "principal_ref": principal_ref, "reason": reason });
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/bss-products/v1/erasure-requests")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(authed_ctx(TENANT))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn export(app: Router, principal_ref: &str) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(format!(
                "/bss-products/v1/compliance/identity-export?principalRef={principal_ref}"
            ))
            .extension(authed_ctx(TENANT))
            .body(Body::empty())
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

/// The wire code of a refusal, read off the **violation** rather than off
/// `context.reason`.
///
/// The sibling door suites read `context.reason`, which is where a
/// `resource_error` builder's denial puts it. This feature's codes reach the
/// wire through `error_mapping`'s `precondition(...)`, which renders each
/// violation's own `code` as its `type` -- a different place in the same
/// envelope, and reading the sibling's path here answered an empty string on
/// a body that carried the code correctly.
async fn error_code(response: axum::http::Response<Body>) -> String {
    let body = body_json(response).await;
    body["context"]["violations"][0]["type"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

async fn audit_rows(dsn: &str, action: &str) -> i64 {
    raw_i64(
        dsn,
        &format!("SELECT COUNT(*) AS v FROM products_audit_log WHERE action = '{action}'"),
    )
    .await
}

/// **The erasure retires the ref, answers it, and writes its evidential row
/// in the same transaction.**
#[tokio::test]
async fn an_erasure_retires_the_ref_and_records_it() {
    let harness = harness().await;
    let seeded = seed_principal(&harness, ALICE).await;

    let response = erase(app_for(&harness, TENANT), ALICE, "dsar-2026-114").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(
        body["actor_ref"].as_str().expect("the retired ref"),
        seeded.to_string(),
        "the door answers the ref it retired"
    );
    assert!(body["tombstoned_at"].is_string());
    assert_eq!(
        audit_rows(&harness.dsn, "erasure.execute").await,
        1,
        "the evidential row committed with the tombstone"
    );
}

/// **An unknown principal is refused `ERASURE_UNKNOWN_ACTOR` and mints
/// nothing.**
///
/// The mint half is the one that matters: the shared actor context would have
/// created a live row for this principal, and a door built on it would report
/// a successful erasure of a principal it had just invented. The export is
/// used as the read-back so the assertion goes through a door rather than
/// around one.
#[tokio::test]
async fn an_unknown_principal_is_refused_and_nothing_is_minted() {
    let harness = harness().await;

    let response = erase(app_for(&harness, TENANT), "principal:nobody", "dsar-x").await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(error_code(response).await, "ERASURE_UNKNOWN_ACTOR");

    let seen = export(app_for(&harness, TENANT), "principal:nobody").await;
    assert_eq!(seen.status(), axum::http::StatusCode::OK);
    let body = body_json(seen).await;
    assert_eq!(
        body["entries"].as_array().expect("entries").len(),
        0,
        "the refusal minted no row: {body}"
    );
}

/// **A blank reason is refused**, because the evidential row is the point of
/// the act and a row with no reason is not evidence. The positive control is
/// every other case here, which supplies one and succeeds.
#[tokio::test]
async fn a_blank_reason_is_refused() {
    let harness = harness().await;
    seed_principal(&harness, ALICE).await;

    let response = erase(app_for(&harness, TENANT), ALICE, "   ").await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(
        audit_rows(&harness.dsn, "erasure.execute").await,
        0,
        "and nothing was erased"
    );
}

/// **The export returns the tombstoned entry and the audit references, and
/// audits the access.**
#[tokio::test]
async fn the_export_returns_the_tombstone_and_audits_the_access() {
    let harness = harness().await;
    let seeded = seed_principal(&harness, ALICE).await;
    let erased = erase(app_for(&harness, TENANT), ALICE, "dsar-2026-114").await;
    assert_eq!(erased.status(), axum::http::StatusCode::OK);

    let response = export(app_for(&harness, TENANT), ALICE).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = body_json(response).await;
    let entries = body["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "{body}");
    assert_eq!(entries[0]["actor_ref"], seeded.to_string());
    assert!(
        entries[0]["tombstoned_at"].is_string(),
        "a DSAR after an erasure must be able to see that the erasure happened: {body}"
    );
    assert_eq!(
        body["audit_references"]
            .as_array()
            .expect("audit references")
            .len(),
        1,
        "the erasure's own row carries the retired ref: {body}"
    );
    assert_eq!(
        audit_rows(&harness.dsn, "compliance.export").await,
        1,
        "and the access itself is audited individually"
    );
}

/// **An export that returns nothing is audited too.**
///
/// The access is the audited event, not the answer. A door that audited only
/// non-empty exports would leave every probe of a principal's presence
/// unrecorded, which is the reconnaissance an individually audited surface
/// exists to catch.
#[tokio::test]
async fn an_empty_export_is_audited_all_the_same() {
    let harness = harness().await;

    let response = export(app_for(&harness, TENANT), "principal:nobody").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["entries"].as_array().expect("entries").len(), 0);
    assert_eq!(
        audit_rows(&harness.dsn, "compliance.export").await,
        1,
        "the access is the audited event, not the answer"
    );
}

/// **C1, the flagship: the erasure moves nothing inside a frozen record.**
///
/// §6 asks for both halves in one probe, *"either half alone passes on a build
/// that got the other wrong"*: the frozen row's digest is byte-identical after
/// the erasure, **and** the map shows the tombstone. The frozen row is stamped
/// with the erased principal's own ref, which is the case that matters -- a
/// version published by the very actor being erased. Erasure is a map-only
/// tombstone precisely so this holds, and nothing else in the crate asserts
/// it.
#[tokio::test]
async fn an_erasure_leaves_a_frozen_record_byte_identical() {
    let harness = harness().await;
    let seeded = seed_principal(&harness, ALICE).await;
    let entity_id = Uuid::from_u128(0xf0_99);
    let digest = vec![0x11_u8, 0x22, 0x33, 0x44];

    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    repo::insert_entity_version(
        &conn,
        &scope,
        repo::NewEntityVersion {
            tenant_id: TENANT,
            entity_kind: repo::VersionedEntityKind::Product,
            entity_id,
            published_version: 1,
            content: "{\"name\":\"Fibre 500\"}".to_owned(),
            content_digest: digest.clone(),
            digest_version: 1,
            approval_ref: None,
            // The frozen row is stamped with the ref about to be erased.
            actor_ref: seeded,
            published_at: crate::domain::canonical::write_instant(chrono::Utc::now()),
        },
    )
    .await
    .expect("freeze a version");

    let erased = erase(app_for(&harness, TENANT), ALICE, "dsar-2026-114").await;
    assert_eq!(erased.status(), axum::http::StatusCode::OK);

    // Half one: the immutable record did not move.
    // Read the row back through the entity, not through raw SQL: `SQLite`
    // stores a uuid as a blob, so `WHERE entity_id = '<hyphenated>'` matches
    // nothing at all -- an equality that answers zero for a reason that has
    // nothing to do with the claim under test.
    let frozen = entity_version::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(
            sea_orm::Condition::all()
                .add(entity_version::Column::TenantId.eq(TENANT))
                .add(entity_version::Column::EntityId.eq(entity_id)),
        )
        .one(&conn)
        .await
        .expect("read the frozen row")
        .expect("it is still there");

    assert_eq!(
        frozen.content_digest, digest,
        "the digest is byte-identical after the erasure"
    );
    assert_eq!(
        frozen.actor_ref, seeded,
        "and the frozen row still carries the erased principal's pseudonym, \
         which is what makes the record readable without re-identifying anyone"
    );

    // Half two: and the map shows the tombstone, so the probe cannot pass by
    // the erasure simply not having happened.
    let export = export(app_for(&harness, TENANT), ALICE).await;
    let body = body_json(export).await;
    assert!(
        body["entries"][0]["tombstoned_at"].is_string(),
        "the erasure did happen: {body}"
    );
}

/// **Two exports are two audit rows** -- *"every access individually"*, which
/// a per-principal or a per-day row would not satisfy.
#[tokio::test]
async fn every_access_is_its_own_audit_row() {
    let harness = harness().await;
    seed_principal(&harness, ALICE).await;

    for _ in 0..2 {
        let response = export(app_for(&harness, TENANT), ALICE).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    assert_eq!(audit_rows(&harness.dsn, "compliance.export").await, 2);
}
