//! Tests for the batch worker's stage phase (`dod-stage-phase`; P-D-54,
//! P-D-86).

use std::sync::Arc;

use chrono::Utc;
use sea_orm_migration::MigratorTrait as _;
use serde_json::json;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{StageOutcome, stage_next_batch};
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{self, NewBulkBatch, NewBulkRow};

const TENANT: Uuid = Uuid::from_u128(0x0b_01);
const BRAND: Uuid = Uuid::from_u128(0x0b_02);
const ACTOR: Uuid = Uuid::from_u128(0x0b_03);

struct Harness {
    dsn: String,
    state: Arc<ApiState>,
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

async fn harness() -> Harness {
    let path = std::env::temp_dir().join(format!("bss-products-worker-{}.sqlite3", Uuid::new_v4()));
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
        idempotency_retention_hours: defaults.idempotency_retention_hours,
        bulk_max_rows_per_batch: defaults.bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: defaults.bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: defaults.watermark_skew_tolerance(),
    });
    Harness {
        dsn,
        state,
        outbox_handle,
    }
}

/// Return the pinned connection before the next pass checks one out.
fn return_pinned<T>(conn: T) {
    let _returned = conn;
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

fn product_row(key: &str, name: &str) -> NewBulkRow {
    NewBulkRow {
        row_key: key.to_owned(),
        row_id: Uuid::new_v4(),
        entity_kind: "product".to_owned(),
        entity_id: None,
        pinned_revision: None,
        staged_payload: Some(
            json!({ "name": name, "brand_id": BRAND, "region_scope": "eu" }).to_string(),
        ),
    }
}

async fn seed_batch(harness: &Harness, key: &str, rows: Vec<NewBulkRow>) -> Uuid {
    let conn = harness.state.db.conn().expect("conn");
    let batch_id = Uuid::new_v4();
    repo::insert_bulk_batch(
        &conn,
        &scope(),
        TENANT,
        NewBulkBatch {
            batch_id,
            batch_key: key.to_owned(),
            mode: "import".to_owned(),
            lane: "import".to_owned(),
            operation_key: None,
            created_at: Utc::now(),
        },
        &rows,
    )
    .await
    .expect("seed the batch");
    batch_id
}

/// Every row lands as a draft through the Foundation's own insert path, the
/// ledger records each entity, and the pass that stages the last row flips
/// edge 1 — `staging -> reported` (P-D-54).
#[tokio::test]
async fn a_batch_stages_its_rows_and_reports() {
    let harness = harness().await;
    let batch_id = seed_batch(
        &harness,
        "b-1",
        vec![product_row("r-1", "Alpha"), product_row("r-2", "Beta")],
    )
    .await;

    let outcome = stage_next_batch(&harness.state, TENANT, ACTOR, Utc::now())
        .await
        .expect("stage");
    assert_eq!(
        outcome,
        StageOutcome::Reported {
            batch_id,
            staged: 2,
            failed: 0
        }
    );

    let conn = harness.state.db.conn().expect("conn");
    let batch = repo::find_batch(&conn, &scope(), TENANT, batch_id)
        .await
        .expect("read")
        .expect("the batch");
    assert_eq!(batch.state, "reported", "edge 1 fires with the last row");
    assert_eq!(batch.attempt, 1, "the claim bumped the attempt");

    let rows = repo::find_batch_rows(&conn, &scope(), TENANT, batch_id)
        .await
        .expect("read the ledger");
    for row in &rows {
        assert!(row.entity_id.is_some(), "the row records its draft");
        assert_eq!(
            row.disposition, None,
            "a staged draft is NOT a disposed row: the terminal mix is the commit phase's"
        );
    }

    // The drafts are real Foundation rows, reachable through the ordinary
    // read path.
    let created = repo::find_product(
        &conn,
        &scope(),
        TENANT,
        rows[0].entity_id.expect("the entity"),
    )
    .await
    .expect("read")
    .expect("the product exists");
    assert_eq!(created.lifecycle_state.as_str(), "draft");
}

/// A row the create path refuses fails ALONE, carrying the owning feature's
/// code verbatim — bulk introduces no parallel taxonomy — and its siblings
/// still land.
#[tokio::test]
async fn a_failing_row_fails_alone_with_the_owning_code() {
    let harness = harness().await;
    let mut bad = product_row("r-bad", "Gamma");
    bad.staged_payload = Some(json!({ "brand_id": BRAND }).to_string());
    let batch_id = seed_batch(&harness, "b-2", vec![bad, product_row("r-ok", "Delta")]).await;

    let outcome = stage_next_batch(&harness.state, TENANT, ACTOR, Utc::now())
        .await
        .expect("stage");
    assert_eq!(
        outcome,
        StageOutcome::Reported {
            batch_id,
            staged: 1,
            failed: 1
        }
    );

    let conn = harness.state.db.conn().expect("conn");
    let rows = repo::find_batch_rows(&conn, &scope(), TENANT, batch_id)
        .await
        .expect("read");
    let bad = rows.iter().find(|row| row.row_key == "r-bad").expect("row");
    assert_eq!(bad.disposition.as_deref(), Some("failed"));
    assert_eq!(bad.code.as_deref(), Some("VALIDATION"));
    let good = rows.iter().find(|row| row.row_key == "r-ok").expect("row");
    assert!(good.entity_id.is_some(), "siblings never block");
}

/// A name already reserved is the Foundation's own `DUPLICATE_NAME` inside
/// the ledger, not a bulk-invented code.
#[tokio::test]
async fn a_collision_carries_the_foundations_own_code() {
    let harness = harness().await;
    seed_batch(&harness, "b-3", vec![product_row("r-1", "Epsilon")]).await;
    stage_next_batch(&harness.state, TENANT, ACTOR, Utc::now())
        .await
        .expect("stage");

    let batch_id = seed_batch(&harness, "b-4", vec![product_row("r-2", "Epsilon")]).await;
    stage_next_batch(&harness.state, TENANT, ACTOR, Utc::now())
        .await
        .expect("stage");

    let conn = harness.state.db.conn().expect("conn");
    let rows = repo::find_batch_rows(&conn, &scope(), TENANT, batch_id)
        .await
        .expect("read");
    assert_eq!(rows[0].disposition.as_deref(), Some("failed"));
    assert_eq!(rows[0].code.as_deref(), Some("DUPLICATE_NAME"));
}

/// The resume operand is the ledger: a re-run over a batch whose rows are
/// already staged mints nothing new.
#[tokio::test]
async fn a_resumed_batch_skips_the_rows_it_already_staged() {
    let harness = harness().await;
    let batch_id = seed_batch(&harness, "b-5", vec![product_row("r-1", "Zeta")]).await;
    stage_next_batch(&harness.state, TENANT, ACTOR, Utc::now())
        .await
        .expect("stage");

    let conn = harness.state.db.conn().expect("conn");
    let first = repo::find_batch_rows(&conn, &scope(), TENANT, batch_id)
        .await
        .expect("read")[0]
        .entity_id;
    // Put the batch back in staging as a crashed attempt would have left it.
    repo::move_bulk_batch_state(&conn, &scope(), TENANT, batch_id, "reported", "staging")
        .await
        .expect("rewind");
    let conn2 = harness.state.db.conn().expect("conn");
    return_pinned(conn);

    stage_next_batch(&harness.state, TENANT, ACTOR, Utc::now())
        .await
        .expect("resume");
    let again = repo::find_batch_rows(&conn2, &scope(), TENANT, batch_id)
        .await
        .expect("read")[0]
        .entity_id;
    assert_eq!(again, first, "the resume skipped the staged row");
}

/// A batch already claimed at a later attempt is not re-taken: the claim is
/// a compare-and-swap on `(state, attempt)`.
#[tokio::test]
async fn a_stale_claim_loses() {
    let harness = harness().await;
    let batch_id = seed_batch(&harness, "b-6", vec![product_row("r-1", "Eta")]).await;
    let conn = harness.state.db.conn().expect("conn");
    assert!(
        repo::claim_bulk_batch(&conn, &scope(), TENANT, batch_id, 0, Utc::now())
            .await
            .expect("claim"),
        "the first claim at attempt 0 wins"
    );
    assert!(
        !repo::claim_bulk_batch(&conn, &scope(), TENANT, batch_id, 0, Utc::now())
            .await
            .expect("claim"),
        "a second claim at the same attempt finds the row moved"
    );
}

/// No staging batch is a quiet pass, not an error.
#[tokio::test]
async fn an_empty_queue_is_a_quiet_pass() {
    let harness = harness().await;
    let outcome = stage_next_batch(&harness.state, TENANT, ACTOR, Utc::now())
        .await
        .expect("stage");
    assert_eq!(outcome, StageOutcome::NoBatch);
}
