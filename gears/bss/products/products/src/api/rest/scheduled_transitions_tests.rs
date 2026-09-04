//! Scheduled-transition door tests (**P-D-134**): the GET surface and the
//! governed cancel.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{TimeZone, Utc};
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
use crate::domain::activation::{ClaimLease, DeferralPopulation, RunFinish};
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{
    self, NewScheduledTransition, claim_due_transition, finish_scheduled_transition,
    insert_scheduled_transition, list_due_transitions,
};
use crate::test_support::{authed_ctx, flat_in_enforcer};

const TENANT: Uuid = Uuid::from_u128(0x00_cc_00_22);
const OTHER: Uuid = Uuid::from_u128(0x00_cc_00_23);
const ENTITY: Uuid = Uuid::from_u128(0x00_cc_00_24);
const APPROVAL: Uuid = Uuid::from_u128(0x00_cc_00_25);
const TRANSITION: Uuid = Uuid::from_u128(0x00_cc_00_26);
const OTHER_TRANSITION: Uuid = Uuid::from_u128(0x00_cc_00_27);

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
        "bss-products-scheduled-transitions-tests-{}.sqlite3",
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
        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: ProductsConfig::default().bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: ProductsConfig::default()
            .bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: ProductsConfig::default().watermark_skew_tolerance(),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
    });
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

async fn get_list(app: Router, tenant: Uuid, query: &str) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(format!("/bss-products/v1/scheduled-transitions{query}"))
            .extension(authed_ctx(tenant))
            .body(Body::empty())
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn post_op(
    app: Router,
    tenant: Uuid,
    id: Uuid,
    body: JsonValue,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(format!(
                "/bss-products/v1/scheduled-transitions/{id}/operations"
            ))
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(authed_ctx(tenant))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn body_of(response: axum::http::Response<Body>) -> JsonValue {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the body");
    serde_json::from_slice(&bytes).expect("the body is JSON")
}

fn seed_row(transition_id: Uuid, tenant_id: Uuid) -> NewScheduledTransition {
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 10, 0, 0).unwrap();
    NewScheduledTransition {
        transition_id,
        tenant_id,
        entity_kind: "sku".to_owned(),
        entity_id: ENTITY,
        kind: "retire".to_owned(),
        at: now - chrono::Duration::hours(1),
        approval_ref: APPROVAL,
        retirement_reason: Some("operator text".to_owned()),
        now,
    }
}

/// GET lists a deferred row with `outcome_reason`, filters by state, and
/// stays tenant-scoped.
#[tokio::test]
async fn get_lists_deferred_rows_with_outcome_reason_and_filters_by_state() {
    let harness = harness().await;
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let other_scope = AccessScope::for_tenant(OTHER);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 11, 0, 0).unwrap();

    insert_scheduled_transition(&conn, &scope, &seed_row(TRANSITION, TENANT))
        .await
        .expect("insert the tenant row");
    insert_scheduled_transition(&conn, &other_scope, &seed_row(OTHER_TRANSITION, OTHER))
        .await
        .expect("insert the other-tenant row");

    assert!(
        claim_due_transition(&conn, &scope, TENANT, TRANSITION, now)
            .await
            .expect("claim")
    );
    assert!(
        finish_scheduled_transition(
            &conn,
            &scope,
            TENANT,
            TRANSITION,
            &RunFinish::Deferred {
                population: DeferralPopulation::FlipGuard,
                reason: "retention_orphan_blocked".to_owned(),
            },
            now,
        )
        .await
        .expect("defer")
    );

    let listed = get_list(app_for(&harness, TENANT), TENANT, "").await;
    assert_eq!(listed.status(), StatusCode::OK);
    let body = body_of(listed).await;
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "the other tenant's row is not visible");
    assert_eq!(items[0]["transition_id"], TRANSITION.to_string());
    assert_eq!(items[0]["state"], "deferred");
    assert_eq!(items[0]["outcome_reason"], "retention_orphan_blocked");

    let filtered = get_list(app_for(&harness, TENANT), TENANT, "?state=pending").await;
    assert_eq!(filtered.status(), StatusCode::OK);
    let filtered_body = body_of(filtered).await;
    assert_eq!(
        filtered_body["items"].as_array().expect("items").len(),
        0,
        "state=pending excludes the deferred row"
    );

    let deferred = get_list(app_for(&harness, TENANT), TENANT, "?state=deferred").await;
    assert_eq!(deferred.status(), StatusCode::OK);
    let deferred_body = body_of(deferred).await;
    assert_eq!(deferred_body["items"].as_array().expect("items").len(), 1);
}

/// Cancel supersedes the live row; the runner's due list no longer sees it.
#[tokio::test]
async fn cancel_supersedes_the_row_and_the_runner_never_claims_it() {
    let harness = harness().await;
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 4, 11, 0, 0).unwrap();

    insert_scheduled_transition(&conn, &scope, &seed_row(TRANSITION, TENANT))
        .await
        .expect("insert");

    let cancelled = post_op(
        app_for(&harness, TENANT),
        TENANT,
        TRANSITION,
        json!({ "op": "cancel" }),
    )
    .await;
    assert_eq!(cancelled.status(), StatusCode::ACCEPTED);

    let row = repo::find_scheduled_transition(&conn, &scope, TENANT, TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.state, "superseded");

    let due = list_due_transitions(
        &conn,
        &scope,
        TENANT,
        now,
        ClaimLease {
            ttl: chrono::Duration::seconds(60),
        },
    )
    .await
    .expect("due list");
    assert!(
        due.is_empty(),
        "a superseded row is one more state the runner never claims"
    );
}
