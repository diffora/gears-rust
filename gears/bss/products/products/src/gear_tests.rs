#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;

#[test]
fn default_leaves_the_runtime_slot_empty() {
    let gear = BssProductsGear::default();
    assert!(gear.runtime.load_full().is_none());
}

/// `register_rest`'s empty-runtime branch returns a router and does not
/// error — the behaviour that distinguishes this gear from
/// `simple-user-settings`, whose `register_rest` errors out of an
/// uninitialised `service` slot.
///
/// Calling `register_rest` itself needs a `GearCtx` and a
/// `dyn OpenApiRegistry`; the former needs a
/// `tokio_util::sync::CancellationToken`, which this slice's dependency
/// delta does not carry. What is exercised directly, without either, is
/// [`crate::api::rest::router`] — the helper both of `register_rest`'s
/// branches call, and the only place the nesting happens. It is
/// infallible (`Router -> Router`, no `Result`), which is what makes
/// `register_rest`'s `Ok(...)` around it unconditional in both branches.
/// A request under the reserved prefix is answered by **this** gear with
/// a `404`, and a path outside it is untouched by the nest.
///
/// The earlier version of this test built the router and dropped it,
/// which asserted nothing: it passed just as well if `router` returned
/// `host_router` unnested, or nested under the wrong prefix, or swapped
/// its arguments. The prefix reservation is the one behaviour this
/// module exists to deliver, so it is asserted where it is observable —
/// through a request — rather than by trusting the type.
#[tokio::test]
async fn a_request_under_the_reserved_prefix_is_answered_by_this_gear() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt as _;

    let host = Router::new().route("/elsewhere", get(|| async { "host" }));
    let mounted = crate::api::rest::router(host);

    let under_prefix = mounted
        .clone()
        .oneshot(
            Request::builder()
                .uri("/bss-products/v1/anything")
                .body(Body::empty())
                .expect("build the probe request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(
        under_prefix.status(),
        StatusCode::NOT_FOUND,
        "the prefix is claimed, so an unmounted path under it is this gear's 404"
    );

    let outside = mounted
        .oneshot(
            Request::builder()
                .uri("/elsewhere")
                .body(Body::empty())
                .expect("build the control request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(
        outside.status(),
        StatusCode::OK,
        "nesting under the prefix must not shadow the host router's own paths"
    );
}
/// A configured gear registers every implemented operation.
#[tokio::test]
async fn configured_gear_registers_implemented_routes() -> anyhow::Result<()> {
    use sea_orm_migration::MigratorTrait;
    use toolkit::api::{OpenApiInfo, OpenApiRegistryImpl};
    let (gear, ctx) = skeleton_harness().await?;
    assert!(gear.runtime.load_full().is_some());
    assert_eq!(
        crate::infra::storage::migrations::Migrator::migrations().len(),
        8,
        "the schema guard, coordination and the six PriceBook migrations"
    );
    let openapi = OpenApiRegistryImpl::new();
    let router = gear.register_rest(&ctx, Router::new(), &openapi)?;
    assert!(router.has_routes());
    let api = serde_json::to_value(openapi.build_openapi(&OpenApiInfo::default())?)?;
    let mut actual: Vec<&str> = api["paths"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|p| p.as_object().unwrap().values())
        .filter_map(|op| op["operationId"].as_str())
        .collect();
    actual.sort_unstable();
    let mut expected = vec![
        "bss_products.create_category",
        "bss_products.list_categories",
        "bss_products.update_category",
        "bss_products.retire_category",
        "bss_products.create_sku",
        "bss_products.list_skus",
        "bss_products.get_sku",
        "bss_products.update_sku_draft",
        "bss_products.sku_versions",
        "bss_products.sku_references",
        "bss_products.submit_sku",
        "bss_products.change_sku",
        "bss_products.retire_sku",
        "bss_products.unfence_sku",
        "bss_products.list_approval_units",
        "bss_products.get_approval_unit",
        "bss_products.approve_unit",
        "bss_products.reject_unit",
        "bss_products.withdraw_unit",
        "bss_products.get_approval_policy",
        "bss_products.put_approval_policy",
        "bss_products.reserve_reference",
        "bss_products.confirm_reference",
        "bss_products.release_reference",
        "bss_products.browse",
    ];
    expected.sort_unstable();
    assert_eq!(actual, expected);
    // The actual lifecycle entry must honor the retained cancellation token.
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        Arc::new(gear).serve(cancel),
    )
    .await??;
    Ok(())
}

async fn skeleton_harness() -> anyhow::Result<(BssProductsGear, GearCtx)> {
    use crate::infra::events::{PARTITIONS, PendingBrokerProducer, QUEUE_NAME};
    use toolkit::contracts::DatabaseCapability;
    use toolkit_db::{ConnectOpts, DBProvider, connect_db};

    struct NoConfig;
    impl toolkit::config::ConfigProvider for NoConfig {
        fn get_gear_config(&self, _gear: &str) -> Option<&serde_json::Value> {
            None
        }
    }
    let gear = BssProductsGear::default();
    let db = connect_db(
        "sqlite::memory:",
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..ConnectOpts::default()
        },
    )
    .await?;
    toolkit_db::migration_runner::run_migrations_for_testing(&db, gear.migrations()).await?;
    let pipeline = toolkit_db::outbox::Outbox::builder(db.clone())
        .table_prefix(OUTBOX_TABLE_PREFIX)?
        .queue(QUEUE_NAME, toolkit_db::outbox::Partitions::of(PARTITIONS))
        .leased(PendingBrokerProducer)
        .start()
        .await?;
    let db = DBProvider::new(db);
    let api_state = Arc::new(crate::api::rest::ApiState {
        db: db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(pipeline.outbox())),
        usage_type_catalog: Arc::new(crate::infra::usage_types::UnconfiguredUsageTypes),
        usage_type_catalog_source: USAGE_TYPE_SOURCE_UNCONFIGURED,
        idempotency_retention_hours: ProductsConfig::default()
            .resolved_idempotency_retention_hours(),
        fence_ttl_minutes: 30,
        reference_principals: std::collections::BTreeMap::new(),
    });
    gear.runtime.store(Some(Arc::new(ProductsRuntime {
        enforcer: Arc::new(crate::test_support::flat_in_enforcer(uuid::Uuid::new_v4())),
        api_state,
        _pipeline: OutboxLifetime::Interim(pipeline),
    })));
    let ctx = GearCtx::new(
        "bss-products",
        uuid::Uuid::new_v4(),
        Arc::new(NoConfig),
        Arc::new(toolkit::ClientHub::new()),
        tokio_util::sync::CancellationToken::new(),
    )
    .with_db(db);
    Ok((gear, ctx))
}

#[tokio::test]
async fn registered_products_client_reads_drafts_and_hides_foreign_rows() {
    use crate::infra::storage::repo;
    use crate::test_support::*;
    let (db, scope, tenant, _) = test_db().await;
    let conn = db.conn().unwrap();
    let cat = repo::insert_category(
        &conn,
        &scope,
        tenant,
        crate::domain::category::NewCategory {
            code: "C".into(),
            name: "Category".into(),
            is_default: false,
            sort_order: 0,
        },
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    let sku = seed_rest_sku(&conn, &scope, tenant, cat.id, "DRAFT").await;
    let hub = toolkit::ClientHub::new();
    register_products_client(&hub, db.db(), Arc::new(flat_in_enforcer(tenant)));
    let client = hub.get::<dyn bss_products_sdk::ProductsClient>().unwrap();
    let ctx = authed_ctx(tenant);
    let found = client.get_sku(&ctx, tenant, sku.id).await.unwrap();
    assert_eq!(found, sku);
    assert!(matches!(
        client.get_sku(&ctx, uuid::Uuid::new_v4(), sku.id).await,
        Err(toolkit_canonical_errors::CanonicalError::NotFound { .. })
    ));
    assert!(matches!(
        client.get_sku(&ctx, tenant, uuid::Uuid::new_v4()).await,
        Err(toolkit_canonical_errors::CanonicalError::NotFound { .. })
    ));
}
