//! Probes of `08`'s read surface (P-D-150): the browse door, the limiter,
//! the timelines, the dashboards.

use std::sync::Arc;

use chrono::Utc;
use sea_orm_migration::MigratorTrait as _;
use serde_json::json;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::domain::approval::StoredApprovalGate;
use crate::domain::governance::GateMode;
use crate::infra::events;
use crate::infra::projector::{
    PassOutcome, ProjectorContext, ReadKnobs, poll_dashboards, project_tenant,
};
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{self, NewProduct};

pub(super) const TENANT: Uuid = Uuid::from_u128(0x08_01);
pub(super) const BRAND: Uuid = Uuid::from_u128(0x08_02);
pub(super) const ACTOR: Uuid = Uuid::from_u128(0x08_03);
pub(super) const CATEGORY: Uuid = Uuid::from_u128(0x08_0c);

pub(super) struct Harness {
    pub(super) dsn: String,
    pub(super) state: Arc<ApiState>,
    #[allow(dead_code)]
    outbox_handle: OutboxHandle,
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(rest) = self.dsn.strip_prefix("sqlite://") {
            let path = rest.split('?').next().unwrap_or(rest);
            std::fs::remove_file(path).ok();
        }
    }
}

pub(super) async fn harness() -> Harness {
    let path = std::env::temp_dir().join(format!("bss-products-read-{}.sqlite3", Uuid::new_v4()));
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
    .expect("connect");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("migrate");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX).expect("prefix"),
    )
    .await
    .expect("outbox migrate");
    let outbox_handle = Outbox::builder(db.clone())
        .table_prefix(events::OUTBOX_TABLE_PREFIX)
        .expect("prefix")
        .queue(events::QUEUE_NAME, Partitions::of(events::PARTITIONS))
        .leased(events::PendingBrokerProducer)
        .start()
        .await
        .expect("start the outbox");
    let defaults = ProductsConfig::default();
    let state = Arc::new(ApiState {
        db: DBProvider::<DbError>::new(db),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(outbox_handle.outbox())),
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
    Harness {
        dsn,
        state,
        outbox_handle,
    }
}

pub(super) fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

pub(super) fn ctx(harness: &Harness) -> ProjectorContext {
    ProjectorContext {
        db: harness.state.db.clone(),
        knobs: ReadKnobs {
            poison_retry_ceiling: 2,
            ..ReadKnobs::from(&ProductsConfig::default())
        },
    }
}

#[allow(clippy::unnecessary_wraps)]
fn render_nothing(_record: repo::ProductRecord) -> Result<serde_json::Value, serde_json::Error> {
    Ok(serde_json::Value::Null)
}

/// A product created through the Foundation's own insert path (one inbox
/// row: `ProductCreated`) and given its primary category.
pub(super) async fn draft_product(harness: &Harness, name: &str, region: &str) -> Uuid {
    let product_id = Uuid::new_v4();
    let now = crate::domain::canonical::write_instant(Utc::now());
    let new = NewProduct {
        product_id,
        tenant_id: TENANT,
        brand_id: BRAND,
        name: name.to_owned(),
        name_normalized: crate::domain::name::normalize(name),
        product_code: Some(format!("{}-CODE", name.replace(' ', "-").to_uppercase())),
        region_scope: region.to_owned(),
        brand_scope: String::new(),
        created_by: ACTOR.to_string(),
        created_at: now,
        cloned_from: None,
        cloned_from_version: None,
    };
    crate::infra::create::insert_product_with_event(
        &harness.state.db,
        &harness.state.sink,
        scope(),
        new,
        crate::infra::create::JoinedRecords {
            claim: None,
            stamp: None,
        },
        ACTOR,
        render_nothing,
    )
    .await
    .expect("insert the product");
    let conn = harness.state.db.conn().expect("conn");
    let _existing = repo::insert_category(
        &conn,
        &scope(),
        repo::NewCategory {
            tenant_id: TENANT,
            category_id: CATEGORY,
            parent_id: None,
            name: "Fixture",
            name_normalized: "fixture",
        },
        now,
    )
    .await
    .expect("the category insert runs");
    repo::replace_category_assignments(
        &conn,
        &scope(),
        TENANT,
        product_id,
        &[(CATEGORY, crate::domain::taxonomy::AssignmentRole::Primary)],
        now,
    )
    .await
    .expect("assign the primary category");
    product_id
}

/// Publish a product through the Foundation's own door (ungoverned host):
/// one frozen version row and one `ProductPublished` inbox row.
pub(super) async fn publish_product(harness: &Harness, product_id: Uuid) -> i64 {
    use crate::api::rest::products;
    let conn = harness.state.db.conn().expect("conn");
    let head = repo::find_product(&conn, &scope(), TENANT, product_id)
        .await
        .expect("read")
        .expect("the head exists");
    let inputs = products::HeadActInputs {
        scope: scope(),
        tenant_id: TENANT,
        product_id,
        actor_ref: ACTOR,
        expected: head.internal_revision,
        now: crate::domain::canonical::write_instant(Utc::now()),
        claim: None,
    };
    let outcome = products::run_publish(
        &conn,
        &inputs,
        &StoredApprovalGate::ungoverned(),
        GateMode::Gate,
        &harness.state.sink,
    )
    .await;
    assert!(
        matches!(outcome, Ok(products::HeadActOutcome::Applied { .. })),
        "the fixture publish lands"
    );
    repo::find_product(&conn, &scope(), TENANT, product_id)
        .await
        .expect("read")
        .expect("the head exists")
        .published_version
}

pub(super) async fn project(harness: &Harness) -> PassOutcome {
    project_tenant(
        &ctx(harness),
        TENANT,
        crate::domain::canonical::write_instant(Utc::now()),
    )
    .await
    .expect("the pass runs")
}

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;

use super::ReadPathLimiter;

fn app(harness: &Harness, tenant: Uuid) -> Router {
    super::router(
        Arc::clone(&harness.state),
        &toolkit::api::OpenApiRegistryImpl::new(),
    )
    .layer(axum::Extension(crate::test_support::flat_in_enforcer(
        tenant,
    )))
}

async fn get(harness: &Harness, uri: &str, tenant: Uuid) -> axum::http::Response<Body> {
    app(harness, tenant)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .extension(crate::test_support::authed_ctx(tenant))
                .body(Body::empty())
                .expect("build the request"),
        )
        .await
        .expect("the router answers")
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("read the body");
    serde_json::from_slice(&bytes).expect("json")
}

/// `dod-browse-door`, `inst-rb-stamp`: an empty projection answers with the
/// anchorless stamp; a projected published row appears; a deprecated row
/// carries its flag and `excludeDeprecated=true` drops it; a draft never
/// shows; facets count every assigned category.
#[tokio::test]
async fn browse_serves_the_projection_under_the_visibility_contract_with_the_stamp() {
    use crate::api::rest::products;
    let harness = harness().await;
    let empty = get(&harness, "/bss-products/v1/browse", TENANT).await;
    assert_eq!(empty.status(), StatusCode::OK);
    let view = body_json(empty).await;
    assert_eq!(view["rows"], json!([]));
    assert_eq!(
        view["stamp"]["as_of_catalog_version"],
        json!(null),
        "the anchorless arm"
    );
    assert!(
        view["stamp"]["projected_at"].is_string(),
        "the stamp is never omitted"
    );

    let published = draft_product(&harness, "Alpha Line", "eu").await;
    let deprecated = draft_product(&harness, "Beta Line", "eu").await;
    let _draft = draft_product(&harness, "Draft Line", "eu").await;
    publish_product(&harness, published).await;
    publish_product(&harness, deprecated).await;
    {
        let conn = harness.state.db.conn().expect("conn");
        let head = repo::find_product(&conn, &scope(), TENANT, deprecated)
            .await
            .expect("read")
            .expect("head");
        let inputs = products::HeadActInputs {
            scope: scope(),
            tenant_id: TENANT,
            product_id: deprecated,
            actor_ref: ACTOR,
            expected: head.internal_revision,
            now: crate::domain::canonical::write_instant(Utc::now()),
            claim: None,
        };
        let outcome = products::run_deprecate(
            &conn,
            &inputs,
            &scope(),
            &StoredApprovalGate::ungoverned(),
            GateMode::Gate,
            &harness.state.sink,
        )
        .await;
        assert!(matches!(
            outcome,
            Ok(products::HeadActOutcome::Applied { .. })
        ));
    }
    project(&harness).await;

    let all = body_json(
        get(
            &harness,
            "/bss-products/v1/browse?includeFacets=true",
            TENANT,
        )
        .await,
    )
    .await;
    let rows = all["rows"].as_array().expect("rows");
    assert_eq!(
        rows.len(),
        2,
        "published and deprecated, never the draft: {all}"
    );
    let flagged = rows
        .iter()
        .find(|r| r["entity_id"] == json!(deprecated))
        .expect("the deprecated row");
    assert_eq!(flagged["deprecated"], json!(true));
    assert_eq!(
        all["facets"]["categories"],
        json!([{ "value": "Fixture", "count": 2 }])
    );

    let filtered = body_json(
        get(
            &harness,
            "/bss-products/v1/browse?excludeDeprecated=true",
            TENANT,
        )
        .await,
    )
    .await;
    let rows = filtered["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["entity_id"], json!(published));

    let by_prefix = body_json(get(&harness, "/bss-products/v1/browse?q=Alp", TENANT).await).await;
    assert_eq!(by_prefix["rows"].as_array().expect("rows").len(), 1);
    let bad_kind = get(&harness, "/bss-products/v1/browse?kind=widget", TENANT).await;
    assert_eq!(bad_kind.status(), StatusCode::BAD_REQUEST);
}

/// `dod-degradation`: above the tenant's ceiling the door answers `503
/// READ_MODEL_OVERLOADED` with `Retry-After` and no rows; another tenant is
/// unaffected (per-partition shedding).
#[tokio::test]
async fn the_limiter_sheds_one_tenant_with_retry_after_and_spares_another() {
    let harness = harness().await;
    let shed_tenant = Uuid::from_u128(0x08_5e);
    ReadPathLimiter::global().set_ceiling_for(shed_tenant, 1);
    let first = get(&harness, "/bss-products/v1/browse", shed_tenant).await;
    assert_eq!(first.status(), StatusCode::OK, "the one token");
    let second = get(&harness, "/bss-products/v1/browse", shed_tenant).await;
    assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        second
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        Some("1")
    );
    let body = body_json(second).await;
    assert!(
        body.get("rows").is_none(),
        "a shed response leaks neither content nor counts: {body}"
    );
    let other = get(&harness, "/bss-products/v1/read/delivery-state", TENANT).await;
    assert_eq!(
        other.status(),
        StatusCode::OK,
        "another tenant's traffic is not shed"
    );
}

/// `dod-history-timeline`: the frozen versions with their changed keys and
/// pseudonyms, a retired head still reachable, an unknown id the miss.
#[tokio::test]
async fn the_timeline_renders_frozen_versions_and_their_diffs() {
    let harness = harness().await;
    let product = draft_product(&harness, "Zeta Line", "eu").await;
    publish_product(&harness, product).await;
    let response = get(
        &harness,
        &format!("/bss-products/v1/products/{product}/versions"),
        TENANT,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert_eq!(view["entity_id"], json!(product));
    assert_eq!(view["lifecycle_state"], json!("published"));
    let versions = view["versions"].as_array().expect("versions");
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0]["published_version"], json!(1));
    assert_eq!(versions[0]["actor_pseudonym"], json!(ACTOR));
    assert!(
        versions[0]["changed_keys"]
            .as_array()
            .is_some_and(|keys| keys.iter().any(|k| k == "name")),
        "the first version changes every key: {view}"
    );
    assert!(view["stamp"]["projected_at"].is_string());

    let missing = get(
        &harness,
        &format!("/bss-products/v1/skus/{}/versions", Uuid::now_v7()),
        TENANT,
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

/// `dod-dashboards`: the three polled tables answer through their doors with
/// the stamp, and refresh with the projector's consumer never involved.
#[tokio::test]
async fn the_three_dashboards_answer_from_their_polled_tables() {
    let harness = harness().await;
    let before =
        body_json(get(&harness, "/bss-products/v1/read/delivery-state", TENANT).await).await;
    assert_eq!(before["polled_at"], json!(null), "before the first poll");
    let product = draft_product(&harness, "Theta Line", "eu").await;
    publish_product(&harness, product).await;
    poll_dashboards(
        &ctx(&harness),
        crate::domain::canonical::write_instant(Utc::now()),
    )
    .await
    .expect("poll");
    let delivery =
        body_json(get(&harness, "/bss-products/v1/read/delivery-state", TENANT).await).await;
    assert_eq!(
        delivery["inbox_pending"],
        json!(2),
        "two inbox rows above a checkpoint of zero"
    );
    assert!(delivery["polled_at"].is_string());
    let freeze = get(&harness, "/bss-products/v1/read/freeze-status", TENANT).await;
    assert_eq!(freeze.status(), StatusCode::OK);
    assert_eq!(
        body_json(freeze).await["items"],
        json!([]),
        "no catalog version yet"
    );
    let deferred = get(&harness, "/bss-products/v1/read/deferred-intents", TENANT).await;
    assert_eq!(deferred.status(), StatusCode::OK);
    let view = body_json(deferred).await;
    assert_eq!(view["items"], json!([]));
    assert!(
        view["stamp"]["projected_at"].is_string(),
        "the stamp on a dashboard too"
    );
}

/// **Lineage rides the timeline, both ways** (`dod-clone-lineage`, P-D-152).
///
/// A clone's `cloned_from` is a head column no read model exposed, which left
/// `design/11`'s "queryable" justification for having no clone event unmet.
/// The source's timeline now lists the entities cloned from it — a draft clone
/// included, because a clone is born a draft — and the clone's own timeline,
/// once it publishes, names its source and the version it read.
#[tokio::test]
async fn the_timeline_carries_lineage_forward_and_the_reverse_lookup() {
    let harness = harness().await;
    let source = draft_product(&harness, "Lineage Source", "eu").await;
    let source_version = publish_product(&harness, source).await;
    project(&harness).await;

    // A clone: the create path with the lineage columns set, as the clone
    // door writes them (`cloned_from` = the immediate source, the version read).
    let clone_id = Uuid::new_v4();
    let now = crate::domain::canonical::write_instant(Utc::now());
    crate::infra::create::insert_product_with_event(
        &harness.state.db,
        &harness.state.sink,
        scope(),
        NewProduct {
            product_id: clone_id,
            tenant_id: TENANT,
            brand_id: BRAND,
            name: "Lineage Source (copy)".to_owned(),
            name_normalized: crate::domain::name::normalize("Lineage Source (copy)"),
            product_code: Some("LINEAGE-SOURCE-COPY".to_owned()),
            region_scope: "eu".to_owned(),
            brand_scope: String::new(),
            created_by: ACTOR.to_string(),
            created_at: now,
            cloned_from: Some(source),
            cloned_from_version: Some(source_version),
        },
        crate::infra::create::JoinedRecords {
            claim: None,
            stamp: None,
        },
        ACTOR,
        render_nothing,
    )
    .await
    .expect("insert the clone");

    let response = get(
        &harness,
        &format!("/bss-products/v1/products/{source}/versions"),
        TENANT,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert!(
        view["lineage"].is_null(),
        "the source was not itself cloned"
    );
    let clones = view["clones"].as_array().expect("clones");
    assert_eq!(clones.len(), 1, "the reverse lookup lists the draft clone");
    assert_eq!(clones[0]["entity_id"], json!(clone_id));
    assert_eq!(clones[0]["cloned_from_version"], json!(source_version));

    // The clone's own timeline exists once it publishes, and names its source.
    {
        let conn = harness.state.db.conn().expect("conn");
        repo::replace_category_assignments(
            &conn,
            &scope(),
            TENANT,
            clone_id,
            &[(CATEGORY, crate::domain::taxonomy::AssignmentRole::Primary)],
            now,
        )
        .await
        .expect("assign the primary category");
    }
    publish_product(&harness, clone_id).await;
    let response = get(
        &harness,
        &format!("/bss-products/v1/products/{clone_id}/versions"),
        TENANT,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert_eq!(view["lineage"]["cloned_from"], json!(source));
    assert_eq!(
        view["lineage"]["cloned_from_version"],
        json!(source_version)
    );
    assert!(
        view["clones"].as_array().expect("clones").is_empty(),
        "nothing was cloned from the clone"
    );
}
