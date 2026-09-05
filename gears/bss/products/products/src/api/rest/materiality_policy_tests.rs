//! The materiality-policy door's tests (**P-D-112**).
//!
//! The load-bearing case is the one with **no setup at all**: a tenant that
//! has never called this door resolves to the default, which is what makes the
//! gear enforceable at launch. Every other case here builds on that.

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
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo;
use crate::test_support::{authed_ctx, flat_in_enforcer};

const TENANT: Uuid = Uuid::from_u128(0x7e_59);

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
        "bss-products-materiality-policy-tests-{}.sqlite3",
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
        reference: crate::api::rest::ReferenceKnobs::from(&ProductsConfig::default()),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        usage_type_resolver: crate::test_support::resolved_usage_types(),
    });
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

/// `PUT` the policy, as the wire does.
/// The governed `PUT` under the stored host (P-D-144): the helper seeds the
/// satisfied record the policy's own mutation needs, then knocks.
async fn put_policy(harness: &TestHarness, body: JsonValue) -> axum::http::Response<Body> {
    crate::test_support::seed_satisfied_approval(
        &harness.db,
        TENANT,
        crate::domain::governance::GateSubject::materiality_policy(TENANT, 0),
        0,
    )
    .await;
    put_policy_via(app_for(harness, TENANT), body).await
}

async fn put_policy_via(app: Router, body: JsonValue) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("PUT")
            .uri("/bss-products/v1/materiality-policy")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(authed_ctx(TENANT))
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

/// **The door writes a policy the resolver then reads back**, and `N = 0` is
/// the value it is armed on.
///
/// Zero is the count **P-D-11** made reachable and the one a
/// `CHECK (approver_count >= 1)` would have refused, so the probe is armed
/// where the schema could be wrong rather than where it is comfortable. The
/// receipt and the store are both asserted: a door that answered correctly and
/// wrote nothing would pass on the receipt alone.
#[tokio::test]
async fn the_door_writes_a_policy_the_resolver_reads_back() {
    let harness = harness().await;
    let response = put_policy(
        &harness,
        json!({
            "field_set": ["tax_category"],
            "affected_entity_trigger": 25,
            "approver_count": 0,
            "reason": "solo tenant, publishing approver-less by policy"
        }),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = body_of(response).await;
    assert_eq!(body["approver_count"], 0);
    assert_eq!(body["affected_entity_trigger"], 25);

    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let resolved = repo::resolve_materiality_policy(&conn, &scope, TENANT)
        .await
        .expect("the read runs");
    match resolved {
        crate::domain::materiality::Resolution::Resolved(policy) => {
            assert_eq!(
                policy.approver_count(),
                0,
                "P-D-11's floor is reachable through the door"
            );
            assert_eq!(policy.affected_entity_trigger(), 25);
            assert!(policy.names_field("tax_category"));
        }
        crate::domain::materiality::Resolution::Unresolvable => {
            panic!("a written policy resolves");
        }
    }
}

/// **A tenant that has never called the door resolves to the default.**
///
/// The case with no setup, and the one P-D-112 arm 2 exists for: every tenant
/// is this tenant at launch. Kept in the door's own suite as well as the
/// store's, because it is the door's absence that has to be survivable.
#[tokio::test]
async fn a_tenant_that_never_called_the_door_resolves_to_the_default() {
    let harness = harness().await;
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    match repo::resolve_materiality_policy(&conn, &scope, TENANT)
        .await
        .expect("the read runs")
    {
        crate::domain::materiality::Resolution::Resolved(policy) => assert_eq!(
            policy,
            crate::domain::materiality::MaterialityPolicy::default(),
            "no row is a resolved default, not an unresolved lookup"
        ),
        crate::domain::materiality::Resolution::Unresolvable => panic!(
            "refusing here refuses every act in every tenant that has never configured anything"
        ),
    }
}

/// A blank reason is refused: the audit row for a governed mutation carries
/// the operator's own words, and a mutation nobody can review is what C4
/// governs this object to prevent.
#[tokio::test]
async fn a_policy_mutation_without_a_reason_is_refused() {
    let harness = harness().await;
    let response = put_policy(
        &harness,
        json!({
            "field_set": [],
            "affected_entity_trigger": 10,
            "approver_count": 2,
            "reason": "   "
        }),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

/// The second `PUT` replaces the first rather than accumulating rows.
#[tokio::test]
async fn a_second_put_replaces_the_policy() {
    let harness = harness().await;
    for count in [3_u32, 1] {
        let response = put_policy(
            &harness,
            json!({
                "field_set": [],
                "affected_entity_trigger": 10,
                "approver_count": count,
                "reason": "tightening then loosening"
            }),
        )
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let crate::domain::materiality::Resolution::Resolved(policy) =
        repo::resolve_materiality_policy(&conn, &scope, TENANT)
            .await
            .expect("the read runs")
    else {
        panic!("resolves");
    };
    assert_eq!(policy.approver_count(), 1, "one row per tenant, replaced");
}
