//! Probes of the `ReadProjector` (P-D-150): the inbox as source, the row from
//! the frozen version, the stamp's floor, the head-field flips, poison,
//! rebuild, and the dashboards.

use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
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

pub(super) async fn read_row(
    harness: &Harness,
    kind: &str,
    id: Uuid,
) -> Option<crate::infra::storage::entity::read_entity::Model> {
    let conn = harness.state.db.conn().expect("conn");
    repo::find_read_entity(&conn, &scope(), TENANT, kind, id)
        .await
        .expect("read")
}

pub(super) async fn stamp(harness: &Harness) -> Option<crate::domain::read_model::StalenessStamp> {
    let conn = harness.state.db.conn().expect("conn");
    repo::load_read_stamp(&conn, &scope(), TENANT)
        .await
        .expect("read the stamp")
}

pub(super) async fn inbox_count(harness: &Harness) -> i64 {
    crate::test_support::raw_i64(
        &harness.dsn,
        "SELECT COUNT(*) AS v FROM products_read_inbox",
    )
    .await
}

/// Write a synthetic inbox row as an event family would.
pub(super) async fn synthetic_inbox(
    harness: &Harness,
    payload_type: &str,
    payload: serde_json::Value,
) {
    let conn = harness.state.db.conn().expect("conn");
    repo::record_read_inbox(
        &conn,
        TENANT,
        0,
        TENANT,
        payload_type,
        &payload.to_string(),
        ACTOR,
        crate::domain::canonical::write_instant(Utc::now()),
    )
    .await
    .expect("record the inbox row");
}

/// `dod-projector`, `dod-frozen-read-path`: a publish writes its event to
/// the inbox in the same transaction; the pass projects the row from the
/// **frozen** version (the head's later edit is not read), stamps the tenant
/// anchorless, and advances the checkpoint; a second pass is idle.
#[tokio::test]
async fn a_publish_reaches_the_inbox_and_projects_from_the_frozen_row() {
    let harness = harness().await;
    let product = draft_product(&harness, "Alpha Line", "eu").await;
    assert_eq!(
        inbox_count(&harness).await,
        1,
        "ProductCreated rides the inbox"
    );
    let version = publish_product(&harness, product).await;
    assert_eq!(version, 1);
    assert_eq!(
        inbox_count(&harness).await,
        2,
        "ProductPublished rides the inbox"
    );

    let outcome = project(&harness).await;
    assert_eq!(
        outcome,
        PassOutcome::Projected {
            applied: 2,
            parked: 0
        }
    );
    let row = read_row(&harness, "product", product)
        .await
        .expect("projected");
    assert_eq!(row.name, "Alpha Line");
    assert_eq!(row.entity_code.as_deref(), Some("ALPHA-LINE-CODE"));
    assert_eq!(row.lifecycle_state, "published");
    assert_eq!(row.published_version, 1);
    assert_eq!(row.region_scope, "eu");
    assert_eq!(
        row.category_paths.as_deref(),
        Some("[\"Fixture\"]"),
        "every assigned category's path"
    );
    assert!(!row.deprecated);
    let stamp = stamp(&harness).await.expect("the stamp exists");
    assert_eq!(
        stamp.as_of_catalog_version, None,
        "no catalog version yet: anchorless"
    );
    assert_eq!(project(&harness).await, PassOutcome::Idle);

    // The head edited after the publish: the row follows the frozen version,
    // not the head (the carve-out reads three columns from the head, nothing
    // else).
    let conn = sea_orm::Database::connect(&harness.dsn).await.expect("aux");
    {
        use sea_orm::ConnectionTrait as _;
        conn.execute_unprepared(&format!(
            "UPDATE products_product SET name = 'Renamed Locally', internal_revision = \
             internal_revision + 1 WHERE product_id = X'{}'",
            product.simple()
        ))
        .await
        .expect("edit the head");
    }
    synthetic_inbox(
        &harness,
        events::PRODUCT_PUBLISHED_PAYLOAD_TYPE,
        json!({ "tenantId": TENANT, "entityKind": "product", "entityId": product,
                "internalRevision": 3, "lifecycleState": "published", "publishedVersion": 1 }),
    )
    .await;
    project(&harness).await;
    assert_eq!(
        read_row(&harness, "product", product)
            .await
            .expect("row")
            .name,
        "Alpha Line",
        "frozen content, not the head's unpublished edit"
    );
}

/// `inst-rp-stamp` (P-D-07): a `CatalogVersionPublished` naming projected
/// entities advances the stamp to that version; one naming an entity not
/// yet projected leaves it where it was.
#[tokio::test]
async fn the_stamp_is_a_floor_over_projected_entities() {
    let harness = harness().await;
    let product = draft_product(&harness, "Beta Line", "eu").await;
    publish_product(&harness, product).await;
    project(&harness).await;

    synthetic_inbox(
        &harness,
        events::CATALOG_VERSION_PUBLISHED_PAYLOAD_TYPE,
        json!({ "tenantId": TENANT, "catalogVersionId": 7, "act": "published", "participants": [],
                "changedEntities": [{ "entityKind": "product", "entityId": product, "publishedVersion": 1 }],
                "satisfiedRequests": 1 }),
    )
    .await;
    project(&harness).await;
    assert_eq!(
        stamp(&harness).await.expect("stamp").as_of_catalog_version,
        Some(7)
    );

    synthetic_inbox(
        &harness,
        events::CATALOG_VERSION_PUBLISHED_PAYLOAD_TYPE,
        json!({ "tenantId": TENANT, "catalogVersionId": 8, "act": "published", "participants": [],
                "changedEntities": [{ "entityKind": "product", "entityId": Uuid::from_u128(0xdead), "publishedVersion": 1 }],
                "satisfiedRequests": 1 }),
    )
    .await;
    project(&harness).await;
    assert_eq!(
        stamp(&harness).await.expect("stamp").as_of_catalog_version,
        Some(7),
        "an entity the projection has not reflected holds the floor"
    );
}

/// The `04` flips project the three head-read columns: a deprecation sets
/// the flag and the provenance from the head, without a new frozen row.
#[tokio::test]
async fn a_deprecation_flips_the_head_fields_on_the_row() {
    use crate::api::rest::products;
    let harness = harness().await;
    let product = draft_product(&harness, "Gamma Line", "eu").await;
    publish_product(&harness, product).await;
    project(&harness).await;

    let conn = harness.state.db.conn().expect("conn");
    let head = repo::find_product(&conn, &scope(), TENANT, product)
        .await
        .expect("read")
        .expect("head");
    let inputs = products::HeadActInputs {
        scope: scope(),
        tenant_id: TENANT,
        product_id: product,
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
    assert!(
        matches!(outcome, Ok(products::HeadActOutcome::Applied { .. })),
        "the deprecation lands"
    );

    project(&harness).await;
    let row = read_row(&harness, "product", product).await.expect("row");
    assert_eq!(row.lifecycle_state, "deprecated");
    assert!(row.deprecated, "the machine-readable flag (C2)");
    assert_eq!(row.deprecation_provenance.as_deref(), Some("direct"));
    assert_eq!(row.published_version, 1, "no frozen row moved");
}

/// P-D-126 rows 9 and 12: a publish whose frozen row is gone is parked and
/// retried, then skipped past the ceiling with the checkpoint moving on; the
/// delivery-state dashboard shows the park.
#[tokio::test]
async fn a_poison_row_is_parked_retried_then_skipped_and_surfaced() {
    let harness = harness().await;
    let ghost = Uuid::from_u128(0x0b_ad);
    synthetic_inbox(
        &harness,
        events::PRODUCT_PUBLISHED_PAYLOAD_TYPE,
        json!({ "tenantId": TENANT, "entityKind": "product", "entityId": ghost,
                "internalRevision": 1, "lifecycleState": "published", "publishedVersion": 1 }),
    )
    .await;
    let alive = draft_product(&harness, "Delta Line", "eu").await;
    publish_product(&harness, alive).await;

    // Pass 1: parked (attempt 1 of a ceiling of 2), the pass stops there so
    // the tenant's order holds; the later publish is not yet projected.
    assert_eq!(
        project(&harness).await,
        PassOutcome::Projected {
            applied: 0,
            parked: 1
        }
    );
    assert!(read_row(&harness, "product", alive).await.is_none());
    // Pass 2: attempt 2 reaches the ceiling — skipped and alarmed, the rest
    // of the inbox projects.
    let second = project(&harness).await;
    assert!(
        matches!(
            second,
            PassOutcome::Projected {
                applied: 2,
                parked: 1
            }
        ),
        "{second:?}"
    );
    assert!(read_row(&harness, "product", alive).await.is_some());
    let conn = harness.state.db.conn().expect("conn");
    let parked = repo::parked_poison(&conn, &scope(), TENANT)
        .await
        .expect("read");
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0].attempts, 2);

    poll_dashboards(
        &ctx(&harness),
        crate::domain::canonical::write_instant(Utc::now()),
    )
    .await
    .expect("poll");
    let conn = harness.state.db.conn().expect("conn");
    let state = repo::read_delivery_state(&conn, &scope(), TENANT)
        .await
        .expect("read")
        .expect("polled");
    assert_eq!(state.parked, 1);
    assert_eq!(state.inbox_pending, 0);
}

/// `inst-rp-bootstrap` (P-D-126 row 8): a checkpoint the swept tail ran past
/// rebuilds into a new generation from the latest catalog version and swaps;
/// with no version the rebuild is anchorless and the tail projects on.
#[tokio::test]
async fn a_checkpoint_behind_the_swept_tail_rebuilds_and_swaps() {
    let harness = harness().await;
    let product = draft_product(&harness, "Epsilon Line", "eu").await;
    publish_product(&harness, product).await;
    project(&harness).await;
    let conn = harness.state.db.conn().expect("conn");
    let (checkpoint, generation) = repo::load_read_checkpoint(&conn, &scope(), TENANT)
        .await
        .expect("read")
        .expect("a checkpoint");
    assert_eq!((checkpoint, generation), (2, 0));
    // Sweep the consumed tail and leave a gap: the next inbox id is 3 while
    // the checkpoint is forced back to 1.
    let swept = repo::sweep_inbox(
        &conn,
        &scope(),
        TENANT,
        checkpoint,
        Utc::now() + ChronoDuration::hours(1),
    )
    .await
    .expect("sweep");
    assert_eq!(swept, 2);
    repo::write_read_checkpoint(&conn, &scope(), TENANT, 1, 0, Utc::now())
        .await
        .expect("rewind");
    synthetic_inbox(
        &harness,
        events::PRODUCT_PUBLISHED_PAYLOAD_TYPE,
        json!({ "tenantId": TENANT, "entityKind": "product", "entityId": product,
                "internalRevision": 2, "lifecycleState": "published", "publishedVersion": 1 }),
    )
    .await;

    let outcome = project(&harness).await;
    assert_eq!(
        outcome,
        PassOutcome::Rebuilt {
            rows: 0,
            generation: 1
        },
        "no catalog version: anchorless"
    );
    let conn = harness.state.db.conn().expect("conn");
    let (checkpoint, generation) = repo::load_read_checkpoint(&conn, &scope(), TENANT)
        .await
        .expect("read")
        .expect("a checkpoint");
    assert_eq!(generation, 1, "the swap moved the serving generation");
    assert_eq!(checkpoint, 3, "the tail is the new resume position");
    assert!(
        read_row(&harness, "product", product).await.is_none(),
        "the old generation's rows were dropped with the swap"
    );
}
