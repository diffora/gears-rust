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

use super::{
    AbandonOutcome, BulkWorkerContext, CompleteOutcome, StageOutcome, abandon_batch,
    complete_batch, stage_next_batch,
};
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::domain::states::BatchState;
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
        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
        idempotency_retention_hours: defaults.idempotency_retention_hours,
        bulk_max_rows_per_batch: defaults.bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: defaults.bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: defaults.watermark_skew_tolerance(),
        reference: crate::api::rest::ReferenceKnobs::from(&defaults),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        usage_type_resolver: crate::test_support::resolved_usage_types(),
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

/// The worker's own context, built from the harness state exactly as
/// `gear.rs`'s composition root builds it at boot.
fn worker_ctx(harness: &Harness) -> BulkWorkerContext {
    BulkWorkerContext {
        db: harness.state.db.clone(),
        sink: harness.state.sink.clone(),
        bulk_max_concurrent_batches_per_tenant: harness
            .state
            .bulk_max_concurrent_batches_per_tenant,
        idempotency_retention_hours: harness.state.idempotency_retention_hours,
        batch_ttl_hours: crate::config::BULK_BATCH_TTL_HOURS_DEFAULT,
    }
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
        governed_live_op: None,
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

    let outcome = stage_next_batch(
        &worker_ctx(&harness),
        TENANT,
        ACTOR,
        Utc::now(),
        &tokio_util::sync::CancellationToken::new(),
    )
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
    assert_eq!(
        batch.state,
        crate::domain::states::BatchState::Reported,
        "edge 1 fires with the last row"
    );
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

    let outcome = stage_next_batch(
        &worker_ctx(&harness),
        TENANT,
        ACTOR,
        Utc::now(),
        &tokio_util::sync::CancellationToken::new(),
    )
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
    stage_next_batch(
        &worker_ctx(&harness),
        TENANT,
        ACTOR,
        Utc::now(),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("stage");

    let batch_id = seed_batch(&harness, "b-4", vec![product_row("r-2", "Epsilon")]).await;
    stage_next_batch(
        &worker_ctx(&harness),
        TENANT,
        ACTOR,
        Utc::now(),
        &tokio_util::sync::CancellationToken::new(),
    )
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
    stage_next_batch(
        &worker_ctx(&harness),
        TENANT,
        ACTOR,
        Utc::now(),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("stage");

    let conn = harness.state.db.conn().expect("conn");
    let first = repo::find_batch_rows(&conn, &scope(), TENANT, batch_id)
        .await
        .expect("read")[0]
        .entity_id;
    // Put the batch back in staging as a crashed attempt would have left it.
    repo::move_bulk_batch_state(
        &conn,
        &scope(),
        TENANT,
        batch_id,
        crate::domain::states::BatchState::Reported,
        crate::domain::states::BatchState::Staging,
        Utc::now(),
    )
    .await
    .expect("rewind");
    let conn2 = harness.state.db.conn().expect("conn");
    return_pinned(conn);

    stage_next_batch(
        &worker_ctx(&harness),
        TENANT,
        ACTOR,
        Utc::now(),
        &tokio_util::sync::CancellationToken::new(),
    )
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
        repo::claim_bulk_batch(
            &conn,
            &scope(),
            TENANT,
            batch_id,
            BatchState::Staging,
            0,
            Utc::now(),
            chrono::Duration::minutes(10),
        )
        .await
        .expect("claim"),
        "the first claim at attempt 0 wins"
    );
    assert!(
        !repo::claim_bulk_batch(
            &conn,
            &scope(),
            TENANT,
            batch_id,
            BatchState::Staging,
            0,
            Utc::now(),
            chrono::Duration::minutes(10),
        )
        .await
        .expect("claim"),
        "a second claim at the same attempt finds the row moved"
    );
}

/// No staging batch is a quiet pass, not an error.
#[tokio::test]
async fn an_empty_queue_is_a_quiet_pass() {
    let harness = harness().await;
    let outcome = stage_next_batch(
        &worker_ctx(&harness),
        TENANT,
        ACTOR,
        Utc::now(),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("stage");
    assert_eq!(outcome, StageOutcome::NoBatch);
}

/// The abandon procedure and the completion edge — `dod-resume-abandon` and
/// `dod-coalesced-event`, the two halves of the batch machine's terminal
/// ends.
mod terminal_edges_tests {
    use super::*;

    async fn move_state(harness: &Harness, batch_id: Uuid, from: BatchState, to: BatchState) {
        let conn = harness.state.db.conn().expect("conn");
        assert!(
            repo::move_bulk_batch_state(&conn, &scope(), TENANT, batch_id, from, to, Utc::now())
                .await
                .expect("the CAS runs"),
            "the fixture's own edge {} -> {} must land",
            from.as_str(),
            to.as_str()
        );
    }

    async fn state_of(harness: &Harness, batch_id: Uuid) -> BatchState {
        let conn = harness.state.db.conn().expect("conn");
        repo::find_batch(&conn, &scope(), TENANT, batch_id)
            .await
            .expect("read the batch")
            .expect("the batch exists")
            .state
    }

    async fn close_row(harness: &Harness, batch_id: Uuid, row_key: &str, disposition: &str) {
        let conn = harness.state.db.conn().expect("conn");
        repo::record_bulk_row_outcome(
            &conn,
            &scope(),
            TENANT,
            batch_id,
            row_key,
            repo::BulkRowOutcome {
                entity_id: None,
                disposition: Some(disposition),
                code: None,
                reason: None,
                now: Utc::now(),
            },
        )
        .await
        .expect("close the row");
    }

    /// **A reported batch abandons: the edge fires, the staged draft is
    /// discarded, and every touched row records the pinned literal.**
    #[tokio::test]
    async fn a_reported_batch_abandons_and_discards_its_staged_drafts() {
        let harness = harness().await;
        let batch_id =
            seed_batch(&harness, "b-abandon", vec![product_row("r1", "Fibre 500")]).await;
        let ctx = worker_ctx(&harness);
        let staged = stage_next_batch(
            &ctx,
            TENANT,
            ACTOR,
            Utc::now(),
            &tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("staging runs");
        assert!(
            matches!(staged, StageOutcome::Reported { .. }),
            "{staged:?}"
        );

        let outcome = abandon_batch(&ctx, TENANT, batch_id, Utc::now())
            .await
            .expect("abandon runs");
        assert_eq!(
            outcome,
            AbandonOutcome::Abandoned {
                discarded: 1,
                dropped: 0,
                untouched: 0
            }
        );
        assert_eq!(state_of(&harness, batch_id).await, BatchState::Abandoned);

        let conn = harness.state.db.conn().expect("conn");
        let rows = repo::find_batch_rows(&conn, &scope(), TENANT, batch_id)
            .await
            .expect("read the ledger");
        assert_eq!(rows[0].disposition.as_deref(), Some("no_op"));
        assert_eq!(
            rows[0].reason.as_deref(),
            Some("batch-abandoned"),
            "the reason is the literal P-D-50 pins, and the migration's CHECK admits only it"
        );
    }

    /// **Abandon has one entry.** A batch in any other state is left
    /// untouched — the guard that keeps a committing batch from being
    /// abandoned out from under its own row publishes.
    #[tokio::test]
    async fn a_batch_outside_reported_is_not_abandoned() {
        let harness = harness().await;
        let batch_id = seed_batch(&harness, "b-guard", vec![product_row("r1", "Fibre 900")]).await;
        let ctx = worker_ctx(&harness);
        stage_next_batch(
            &ctx,
            TENANT,
            ACTOR,
            Utc::now(),
            &tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("staging runs");
        move_state(
            &harness,
            batch_id,
            BatchState::Reported,
            BatchState::Approved,
        )
        .await;
        move_state(
            &harness,
            batch_id,
            BatchState::Approved,
            BatchState::Committing,
        )
        .await;

        let outcome = abandon_batch(&ctx, TENANT, batch_id, Utc::now())
            .await
            .expect("abandon runs");
        assert_eq!(outcome, AbandonOutcome::NotReported);
        assert_eq!(state_of(&harness, batch_id).await, BatchState::Committing);
    }

    /// **The completion edge emits exactly one summary, and the second
    /// caller emits nothing** — the CAS is the mechanism, not a convention.
    #[tokio::test]
    async fn completion_emits_exactly_one_summary() {
        let harness = harness().await;
        let batch_id = seed_batch(
            &harness,
            "b-complete",
            vec![product_row("r1", "Fibre A"), product_row("r2", "Fibre B")],
        )
        .await;
        let ctx = worker_ctx(&harness);
        stage_next_batch(
            &ctx,
            TENANT,
            ACTOR,
            Utc::now(),
            &tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("staging runs");
        move_state(
            &harness,
            batch_id,
            BatchState::Reported,
            BatchState::Approved,
        )
        .await;
        move_state(
            &harness,
            batch_id,
            BatchState::Approved,
            BatchState::Committing,
        )
        .await;

        // One row in flight: the batch stays open and nothing is emitted.
        close_row(&harness, batch_id, "r1", "published").await;
        assert_eq!(
            complete_batch(&ctx, TENANT, batch_id, ACTOR, Utc::now())
                .await
                .expect("completion runs"),
            CompleteOutcome::RowsInFlight
        );
        assert_eq!(
            crate::test_support::enqueued_event_count(
                &harness.dsn,
                "CatalogBulkOperationCompleted"
            )
            .await,
            0,
            "an open batch announces nothing"
        );

        // A FAILED row still completes the batch: parts-succeeded is the
        // honest end state, not an error.
        close_row(&harness, batch_id, "r2", "failed").await;
        let first = complete_batch(&ctx, TENANT, batch_id, ACTOR, Utc::now())
            .await
            .expect("completion runs");
        let CompleteOutcome::Completed { ledger_digest } = first else {
            panic!("a batch whose rows are all terminal completes: {first:?}");
        };
        assert!(!ledger_digest.is_empty());
        assert_eq!(state_of(&harness, batch_id).await, BatchState::Completed);

        // The re-claim, through the CAS itself rather than through the
        // caller's state pre-check: a probe that only exercised the
        // pre-check would go green against a build that emitted outside the
        // CAS, and a first revision of this case did exactly that — the
        // falsification passed until this call replaced it.
        assert!(
            !super::super::flip_and_announce(
                &ctx,
                TENANT,
                batch_id,
                super::super::CompletionSummary {
                    batch_key: "b-complete",
                    ledger_digest: &ledger_digest,
                    counts: crate::infra::events::BulkCompletedRows {
                        published: 1,
                        applied: 0,
                        no_op: 0,
                        failed: 1,
                    },
                },
                ACTOR,
                Utc::now(),
            )
            .await
            .expect("the CAS runs"),
            "a second caller's CAS matches no row, so it did not flip"
        );
        assert_eq!(
            crate::test_support::enqueued_event_count(
                &harness.dsn,
                "CatalogBulkOperationCompleted"
            )
            .await,
            1,
            "exactly one, and the losing CAS emitting nothing is what makes it exactly one"
        );

        // And the caller's own fast path still answers honestly.
        assert_eq!(
            complete_batch(&ctx, TENANT, batch_id, ACTOR, Utc::now())
                .await
                .expect("completion runs"),
            CompleteOutcome::NotCommitting
        );
    }
}

/// Group 7 (P-D-149): the batch machine past the report edge — the record,
/// the commit, the reaper, the ceremony, the resolver, the lifecycle lane and
/// the export.
mod batch_machine_tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use chrono::Duration as ChronoDuration;
    use sea_orm::ConnectionTrait as _;
    use tower::ServiceExt as _;

    use super::*;
    use crate::domain::governance::ApprovalId;
    use crate::domain::materiality::MaterialityPolicy;
    use crate::infra::bulk_worker::{
        BULK_REQUEST_SOURCE, BUNDLE_OVERRIDE_CONDITION, advance_batches, sweep,
    };
    use crate::infra::storage::repo::NewBulkBatch;

    fn cancel() -> tokio_util::sync::CancellationToken {
        tokio_util::sync::CancellationToken::new()
    }

    async fn set_quorum(harness: &Harness, approver_count: u32) {
        let conn = harness.state.db.conn().expect("conn");
        repo::write_materiality_policy(
            &conn,
            &scope(),
            TENANT,
            &MaterialityPolicy::new(Vec::new(), 1, approver_count),
            ACTOR,
            Utc::now(),
        )
        .await
        .expect("write the policy");
    }

    async fn batch_of(harness: &Harness, batch_id: Uuid) -> repo::BulkBatchRecord {
        let conn = harness.state.db.conn().expect("conn");
        repo::find_batch(&conn, &scope(), TENANT, batch_id)
            .await
            .expect("read the batch")
            .expect("the batch exists")
    }

    async fn rows_of(harness: &Harness, batch_id: Uuid) -> Vec<repo::BulkRowRecord> {
        let conn = harness.state.db.conn().expect("conn");
        repo::find_batch_rows(&conn, &scope(), TENANT, batch_id)
            .await
            .expect("read the ledger")
    }

    async fn row(harness: &Harness, batch_id: Uuid, key: &str) -> repo::BulkRowRecord {
        rows_of(harness, batch_id)
            .await
            .into_iter()
            .find(|row| row.row_key == key)
            .expect("the row exists")
    }

    async fn approval_state(harness: &Harness, batch_id: Uuid) -> (Uuid, String, String) {
        let batch = batch_of(harness, batch_id).await;
        let approval_ref = batch.approval_ref.expect("the report edge pinned a record");
        let conn = harness.state.db.conn().expect("conn");
        let record = repo::read_approval(&conn, &scope(), TENANT, ApprovalId::new(approval_ref))
            .await
            .expect("read the record")
            .expect("the record exists");
        (approval_ref, record.state, record.quorum_descriptor)
    }

    async fn stage_only(harness: &Harness) {
        stage_next_batch(&worker_ctx(harness), TENANT, ACTOR, Utc::now(), &cancel())
            .await
            .expect("staging runs");
    }

    async fn advance(harness: &Harness) -> crate::infra::bulk_worker::AdvanceOutcome {
        advance_batches(&worker_ctx(harness), TENANT, ACTOR, Utc::now(), &cancel())
            .await
            .expect("the advance pass runs")
    }

    async fn full_sweep(harness: &Harness) {
        sweep(&worker_ctx(harness), ACTOR, Utc::now(), &cancel())
            .await
            .expect("the sweep runs");
    }

    async fn seed_batch_with(
        harness: &Harness,
        key: &str,
        mode: &str,
        lane: &str,
        created_at: chrono::DateTime<Utc>,
        rows: Vec<NewBulkRow>,
    ) -> Uuid {
        let conn = harness.state.db.conn().expect("conn");
        let batch_id = Uuid::new_v4();
        repo::insert_bulk_batch(
            &conn,
            &scope(),
            TENANT,
            NewBulkBatch {
                batch_id,
                batch_key: key.to_owned(),
                mode: mode.to_owned(),
                lane: lane.to_owned(),
                operation_key: Some(batch_id.to_string()),
                created_at,
            },
            &rows,
        )
        .await
        .expect("seed the batch");
        batch_id
    }

    fn sku_row(key: &str, parent: Uuid, code: &str, sku_type: &str) -> NewBulkRow {
        NewBulkRow {
            row_key: key.to_owned(),
            row_id: Uuid::new_v4(),
            entity_kind: "sku".to_owned(),
            entity_id: None,
            pinned_revision: None,
            staged_payload: Some(
                json!({
                    "product_id": parent, "sku_code": code, "sku_type": sku_type,
                    "region_scope": "eu",
                })
                .to_string(),
            ),
            governed_live_op: None,
        }
    }

    fn product_row_in(key: &str, name: &str, region: &str) -> NewBulkRow {
        NewBulkRow {
            row_key: key.to_owned(),
            row_id: Uuid::new_v4(),
            entity_kind: "product".to_owned(),
            entity_id: None,
            pinned_revision: None,
            staged_payload: Some(
                json!({ "name": name, "brand_id": BRAND, "region_scope": region }).to_string(),
            ),
            governed_live_op: None,
        }
    }

    /// A product created and published by its own batch at quorum zero: the
    /// fixture every later probe builds on.
    async fn published_product(harness: &Harness, key: &str, name: &str) -> Uuid {
        let batch_id = seed_batch(harness, key, vec![product_row(key, name)]).await;
        stage_assign_advance(harness).await;
        let row = row(harness, batch_id, key).await;
        assert_eq!(row.disposition.as_deref(), Some("published"), "{row:?}");
        row.entity_id.expect("the row minted its head")
    }

    const CATEGORY: Uuid = Uuid::from_u128(0x0b_0c);

    /// A product publish needs a primary category (`PRIMARY_CATEGORY_REQUIRED`):
    /// the fixture assigns one to a staged product before the commit walks it.
    async fn assign_primary(harness: &Harness, product_id: Uuid) {
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
            Utc::now(),
        )
        .await
        .expect("the category insert runs");
        repo::replace_category_assignments(
            &conn,
            &scope(),
            TENANT,
            product_id,
            &[(CATEGORY, crate::domain::taxonomy::AssignmentRole::Primary)],
            Utc::now(),
        )
        .await
        .expect("assign the primary category");
    }

    /// Stage the next batch, give every staged product its primary category,
    /// and walk the machine on.
    async fn stage_assign_advance(harness: &Harness) {
        stage_only(harness).await;
        let conn = harness.state.db.conn().expect("conn");
        let reported = repo::batches_in_state(&conn, &scope(), TENANT, BatchState::Reported)
            .await
            .expect("read");
        return_pinned(conn);
        for batch in reported {
            for row in rows_of(harness, batch.batch_id).await {
                if row.entity_kind == "product"
                    && row.disposition.is_none()
                    && row.governed_live_op.is_none()
                    && let Some(id) = row.entity_id
                {
                    assign_primary(harness, id).await;
                }
            }
        }
        advance(harness).await;
    }

    async fn product(harness: &Harness, id: Uuid) -> repo::ProductRecord {
        let conn = harness.state.db.conn().expect("conn");
        repo::find_product(&conn, &scope(), TENANT, id)
            .await
            .expect("read")
            .expect("the head exists")
    }

    async fn sku(harness: &Harness, id: Uuid) -> repo::SkuRecord {
        let conn = harness.state.db.conn().expect("conn");
        repo::find_sku(&conn, &scope(), TENANT, id)
            .await
            .expect("read")
            .expect("the head exists")
    }

    /// Raw SQL over an auxiliary connection; uuid columns are blobs on
    /// `SQLite`, so callers spell ids as `X'..'` hex.
    async fn exec(harness: &Harness, sql: &str) {
        let conn = sea_orm::Database::connect(&harness.dsn)
            .await
            .expect("open an auxiliary connection");
        conn.execute_unprepared(sql)
            .await
            .expect("the statement runs");
    }

    fn approvals_app(harness: &Harness) -> Router {
        crate::api::rest::approvals::router(
            Arc::clone(&harness.state),
            &toolkit::api::OpenApiRegistryImpl::new(),
        )
        .layer(axum::Extension(crate::test_support::flat_in_enforcer(
            TENANT,
        )))
    }

    async fn decide(
        harness: &Harness,
        approval: Uuid,
        subject: u128,
        body: &serde_json::Value,
    ) -> (u16, String) {
        let ctx = toolkit_security::SecurityContext::builder()
            .subject_id(Uuid::from_u128(subject))
            .subject_tenant_id(TENANT)
            .subject_type(toolkit_gts::gts_id!("cf.core.security.subject_user.v1~"))
            .token_scopes(vec![
                "*".to_owned(),
                crate::domain::approval::ApproverRole::CatalogAdmin
                    .as_str()
                    .to_owned(),
            ])
            .build()
            .expect("ctx");
        let response = approvals_app(harness)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/bss-products/v1/approvals/{approval}/decisions"))
                    .header("content-type", "application/json")
                    .extension(ctx)
                    .body(Body::from(body.to_string()))
                    .expect("build the request"),
            )
            .await
            .expect("the router answers");
        let status = response.status().as_u16();
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("read the body");
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    /// `dod-batch-state-machine`, `dod-commit-phase`, `dod-change-report`,
    /// `dod-operation-key`, `dod-coalesced-event`: at quorum zero one sweep
    /// reports, consumes the record once, publishes every row in
    /// `PreAuthorized`, enqueues the one bulk-lane request and completes; a
    /// second sweep replays nothing.
    #[tokio::test]
    async fn a_batch_reports_commits_under_one_consumed_record_and_completes() {
        let harness = harness().await;
        set_quorum(&harness, 0).await;
        let batch_id = seed_batch(
            &harness,
            "b-machine",
            vec![product_row("r1", "Fibre A"), product_row("r2", "Fibre B")],
        )
        .await;

        stage_assign_advance(&harness).await;
        let batch = batch_of(&harness, batch_id).await;
        assert_eq!(batch.state, BatchState::Completed, "{batch:?}");
        let (_, state, descriptor) = approval_state(&harness, batch_id).await;
        assert_eq!(
            state, "consumed",
            "the record is spent at the approved -> committing flip"
        );
        assert!(
            descriptor.contains("\"overrideConditions\":[]"),
            "{descriptor}"
        );
        let rows = rows_of(&harness, batch_id).await;
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row.disposition.as_deref(), Some("published"), "{row:?}");
            assert!(row.pinned_revision.is_some(), "the report pinned the row");
            let head = product(&harness, row.entity_id.expect("minted")).await;
            assert_eq!(head.published_version, 1);
        }
        assert_eq!(
            crate::test_support::enqueued_event_count(
                &harness.dsn,
                "CatalogBulkOperationCompleted"
            )
            .await,
            1
        );
        let request_key = batch_id.to_string();
        let conn = harness.state.db.conn().expect("conn");
        let request = repo::find_increment_request(
            &conn,
            &scope(),
            TENANT,
            BULK_REQUEST_SOURCE,
            &request_key,
        )
        .await
        .expect("read")
        .expect("the commit enqueued the batch's one request");
        assert_eq!(request.state, crate::domain::states::RequestState::Pending);
        assert_eq!(
            crate::test_support::raw_string_opt(
                &harness.dsn,
                &format!(
                    "SELECT lane || '/' || operation_key AS v FROM products_catalog_version_request \
                     WHERE source = '{BULK_REQUEST_SOURCE}' AND request_key = '{request_key}'"
                ),
            )
            .await
            .as_deref(),
            Some(format!("bulk/{request_key}").as_str()),
            "the bulk lane and the batch's operation key (dod-operation-key)"
        );
        return_pinned(conn);

        full_sweep(&harness).await;
        for row in rows_of(&harness, batch_id).await {
            assert_eq!(
                product(&harness, row.entity_id.expect("minted"))
                    .await
                    .published_version,
                1,
                "a replayed sweep publishes nothing twice"
            );
        }
        assert_eq!(
            crate::test_support::enqueued_event_count(
                &harness.dsn,
                "CatalogBulkOperationCompleted"
            )
            .await,
            1
        );
    }

    /// The no-hidden-partial-failure criterion: a head edited after the report
    /// fails `STALE_REVISION` alone at commit, its sibling publishes, and the
    /// batch reaches `completed`, not `failed`.
    #[tokio::test]
    async fn a_row_edited_after_the_report_fails_stale_revision_alone() {
        let harness = harness().await;
        set_quorum(&harness, 0).await;
        let batch_id = seed_batch(
            &harness,
            "b-stale",
            vec![product_row("r1", "Fibre A"), product_row("r2", "Fibre B")],
        )
        .await;
        stage_only(&harness).await;
        assert_eq!(
            batch_of(&harness, batch_id).await.state,
            BatchState::Reported
        );
        for key in ["r1", "r2"] {
            let id = row(&harness, batch_id, key)
                .await
                .entity_id
                .expect("minted");
            assign_primary(&harness, id).await;
        }
        let edited = row(&harness, batch_id, "r1")
            .await
            .entity_id
            .expect("minted");
        exec(
            &harness,
            &format!(
                "UPDATE products_product SET internal_revision = internal_revision + 1 WHERE \
                 product_id = X'{}'",
                edited.simple()
            ),
        )
        .await;

        advance(&harness).await;
        let batch = batch_of(&harness, batch_id).await;
        assert_eq!(
            batch.state,
            BatchState::Completed,
            "parts-succeeded is an end state"
        );
        let r1 = row(&harness, batch_id, "r1").await;
        assert_eq!(r1.disposition.as_deref(), Some("failed"));
        assert_eq!(r1.code.as_deref(), Some("STALE_REVISION"));
        let r2 = row(&harness, batch_id, "r2").await;
        assert_eq!(
            r2.disposition.as_deref(),
            Some("published"),
            "the sibling proceeds"
        );
    }

    /// `dod-resume-abandon` and P-D-127 row 6: a rejected record abandons the
    /// batch (its drafts discarded, the rows `no_op` under the literal reason),
    /// and a `reported` batch past `bulk_batch_ttl_hours` is reaped.
    #[tokio::test]
    async fn a_rejected_record_abandons_and_the_reaper_takes_a_stale_report() {
        let harness = harness().await;
        set_quorum(&harness, 2).await;
        let batch_id = seed_batch(&harness, "b-reject", vec![product_row("r1", "Fibre A")]).await;
        stage_only(&harness).await;
        let (approval, state, _) = approval_state(&harness, batch_id).await;
        assert_eq!(
            state, "pending",
            "at N = 2 the record waits for its approvers"
        );
        assert_eq!(
            advance(&harness).await.abandoned,
            0,
            "a young pending report is left alone"
        );

        let status = decide(
            &harness,
            approval,
            0x7a_01,
            &json!({ "verdict": "rejected", "reason": "not this quarter" }),
        )
        .await;
        assert_eq!(status.0, 200, "{}", status.1);
        let outcome = advance(&harness).await;
        assert_eq!(outcome.abandoned, 1);
        assert_eq!(
            batch_of(&harness, batch_id).await.state,
            BatchState::Abandoned
        );
        let r1 = row(&harness, batch_id, "r1").await;
        assert_eq!(r1.disposition.as_deref(), Some("no_op"));
        assert_eq!(
            r1.reason.as_deref(),
            Some(crate::domain::batch::ABANDON_REASON)
        );

        let old = seed_batch_with(
            &harness,
            "b-old",
            "import",
            "import",
            Utc::now() - ChronoDuration::hours(200),
            vec![product_row("o1", "Fibre Old")],
        )
        .await;
        stage_only(&harness).await;
        assert_eq!(batch_of(&harness, old).await.state, BatchState::Reported);
        assert_eq!(
            advance(&harness).await.abandoned,
            1,
            "the reaper's TTL is the other exit"
        );
        assert_eq!(batch_of(&harness, old).await.state, BatchState::Abandoned);
    }

    /// `dod-bulk-override-ceremony`: the report itemises the uncomposed-bundle
    /// rows by `skuCode` on the record, both approvers acknowledge them by
    /// name, the itemised row publishes with its flag raised under the one
    /// batch ceremony, and a bundle that appeared after the report fails
    /// `BULK_OVERRIDE_UNACKNOWLEDGED` alone.
    #[tokio::test]
    async fn the_ceremony_itemises_bundles_and_a_late_bundle_fails_alone() {
        let harness = harness().await;
        set_quorum(&harness, 0).await;
        let parent = published_product(&harness, "p-parent", "Parent Line").await;
        set_quorum(&harness, 2).await;
        let batch_id = seed_batch(
            &harness,
            "b-bundles",
            vec![
                sku_row("rb", parent, "BNDL-1", "bundle"),
                sku_row("rp", parent, "PLAIN-1", "product"),
            ],
        )
        .await;
        stage_only(&harness).await;
        let (approval, state, descriptor) = approval_state(&harness, batch_id).await;
        assert_eq!(state, "pending");
        let condition = format!("{BUNDLE_OVERRIDE_CONDITION}/BNDL-1");
        assert!(
            descriptor.contains(&format!("\"overrideConditions\":[\"{condition}\"]")),
            "the ceremony's conditions name the row by skuCode: {descriptor}"
        );
        assert!(row(&harness, batch_id, "rb").await.override_acknowledged);
        assert!(!row(&harness, batch_id, "rp").await.override_acknowledged);

        let late = row(&harness, batch_id, "rp")
            .await
            .entity_id
            .expect("minted");
        exec(
            &harness,
            &format!(
                "UPDATE products_sku SET sku_type = 'bundle', internal_revision = \
                 internal_revision + 1 WHERE sku_id = X'{}'",
                late.simple()
            ),
        )
        .await;

        let refused = decide(
            &harness,
            approval,
            0x7b_01,
            &json!({ "verdict": "approved" }),
        )
        .await;
        assert_eq!(
            refused.0, 400,
            "an approval that does not name the condition is refused: {}",
            refused.1
        );
        for approver in [0x7b_02_u128, 0x7b_03] {
            let (status, body) = decide(
                &harness,
                approval,
                approver,
                &json!({ "verdict": "approved", "override_acknowledgments": condition }),
            )
            .await;
            assert_eq!(status, 200, "{body}");
        }
        assert_eq!(approval_state(&harness, batch_id).await.1, "satisfied");

        advance(&harness).await;
        assert_eq!(
            batch_of(&harness, batch_id).await.state,
            BatchState::Completed
        );
        let rb = row(&harness, batch_id, "rb").await;
        assert_eq!(rb.disposition.as_deref(), Some("published"), "{rb:?}");
        assert!(
            sku(&harness, rb.entity_id.expect("minted"))
                .await
                .composition_pending
        );
        let rp = row(&harness, batch_id, "rp").await;
        assert_eq!(rp.disposition.as_deref(), Some("failed"));
        assert_eq!(rp.code.as_deref(), Some("BULK_OVERRIDE_UNACKNOWLEDGED"));
        assert_eq!(approval_state(&harness, batch_id).await.1, "consumed");
    }

    /// `dod-promotion-resolver`: over a published head the same content is a
    /// `no_op`, different promotable content saves as a draft pinned to the
    /// new revision, and a head with unpublished edits is a conflict; the
    /// abandon procedure reverts the draft through the save door.
    #[tokio::test]
    async fn a_promote_batch_classifies_no_op_update_as_draft_and_conflict_then_reverts() {
        let harness = harness().await;
        set_quorum(&harness, 0).await;
        let alpha = published_product(&harness, "p-alpha", "Alpha Line").await;
        let before = product(&harness, alpha).await;
        assert_eq!(before.region_scope, "eu");
        // A draft that never published: unpublished local edits by definition.
        seed_batch(&harness, "b-beta", vec![product_row("beta", "Beta Line")]).await;
        stage_only(&harness).await;

        let promote = seed_batch_with(
            &harness,
            "b-promote",
            "promote",
            "import",
            Utc::now(),
            vec![
                product_row_in("same", "Alpha Line", "eu"),
                product_row_in("upd", "Alpha Line", "us"),
                product_row_in("dirty", "Beta Line", "eu"),
                product_row_in("new", "Gamma Line", "eu"),
            ],
        )
        .await;
        // The reported beta batch would commit under this sweep; stage the
        // promote batch alone.
        loop {
            let outcome =
                stage_next_batch(&worker_ctx(&harness), TENANT, ACTOR, Utc::now(), &cancel())
                    .await
                    .expect("staging runs");
            if matches!(outcome, StageOutcome::NoBatch) {
                break;
            }
        }
        let same = row(&harness, promote, "same").await;
        assert_eq!(same.disposition.as_deref(), Some("no_op"));
        assert_eq!(same.entity_id, Some(alpha));
        let upd = row(&harness, promote, "upd").await;
        assert_eq!(upd.disposition, None, "in flight for the commit: {upd:?}");
        assert_eq!(
            upd.entity_id,
            Some(alpha),
            "bound to the existing head, no second create"
        );
        assert!(
            upd.governed_live_op
                .as_deref()
                .is_some_and(|m| m.contains("update_as_draft"))
        );
        let drafted = product(&harness, alpha).await;
        assert_eq!(
            drafted.region_scope, "us",
            "the save landed on the head as a draft"
        );
        assert!(drafted.internal_revision > before.internal_revision);
        assert_eq!(upd.pinned_revision, Some(drafted.internal_revision));
        let dirty = row(&harness, promote, "dirty").await;
        assert_eq!(dirty.disposition.as_deref(), Some("failed"));
        assert_eq!(dirty.code.as_deref(), Some("PROMOTION_DIRTY_HEAD"));
        let created = row(&harness, promote, "new").await;
        assert!(
            created.entity_id.is_some_and(|id| id != alpha),
            "an unknown identity creates"
        );

        let outcome = abandon_batch(&worker_ctx(&harness), TENANT, promote, Utc::now())
            .await
            .expect("abandon runs");
        assert!(
            matches!(outcome, AbandonOutcome::Abandoned { .. }),
            "{outcome:?}"
        );
        let reverted = product(&harness, alpha).await;
        assert_eq!(
            reverted.region_scope, "eu",
            "the draft reverts to the frozen content"
        );
        assert!(
            reverted.internal_revision > drafted.internal_revision,
            "with a revision bump"
        );
    }

    /// `dod-bulk-lifecycle`: the lane deprecates published heads through the
    /// ordinary `04` door in `PreAuthorized` mode, a draft refuses under the
    /// ordinary guard alone, and the rows read `applied`.
    #[tokio::test]
    async fn a_lifecycle_batch_deprecates_through_the_ordinary_door() {
        let harness = harness().await;
        set_quorum(&harness, 0).await;
        let a = published_product(&harness, "l-a", "Line A").await;
        let b = published_product(&harness, "l-b", "Line B").await;
        seed_batch(
            &harness,
            "l-draft",
            vec![product_row("draft", "Draft Line")],
        )
        .await;
        stage_only(&harness).await;
        let draft = row(&harness, batch_of_key(&harness, "l-draft").await, "draft")
            .await
            .entity_id
            .expect("minted");
        let op = json!({ "op": "deprecate" }).to_string();
        let lifecycle_row = |id: Uuid| NewBulkRow {
            row_key: id.to_string(),
            row_id: Uuid::new_v4(),
            entity_kind: "product".to_owned(),
            entity_id: Some(id),
            pinned_revision: None,
            staged_payload: Some(op.clone()),
            governed_live_op: Some(op.clone()),
        };
        let batch_id = seed_batch_with(
            &harness,
            "l-batch",
            "import",
            "lifecycle",
            Utc::now(),
            vec![lifecycle_row(a), lifecycle_row(b), lifecycle_row(draft)],
        )
        .await;
        full_sweep(&harness).await;
        full_sweep(&harness).await;
        full_sweep(&harness).await;
        let batch = batch_of(&harness, batch_id).await;
        assert_eq!(batch.state, BatchState::Completed, "{batch:?}");
        for id in [a, b] {
            let row = row(&harness, batch_id, &id.to_string()).await;
            assert_eq!(row.disposition.as_deref(), Some("applied"), "{row:?}");
            assert_eq!(
                product(&harness, id).await.lifecycle_state,
                bss_products_sdk::models::LifecycleState::Deprecated
            );
        }
        let refused = row(&harness, batch_id, &draft.to_string()).await;
        assert_eq!(
            refused.disposition.as_deref(),
            Some("failed"),
            "{refused:?}"
        );
        assert_eq!(
            product(&harness, draft).await.lifecycle_state,
            bss_products_sdk::models::LifecycleState::Draft,
            "the ordinary guard holds: a draft is not deprecated"
        );
    }

    async fn batch_of_key(harness: &Harness, key: &str) -> Uuid {
        let conn = harness.state.db.conn().expect("conn");
        repo::find_batch_by_key(&conn, &scope(), TENANT, key)
            .await
            .expect("read")
            .expect("the batch exists")
            .batch_id
    }

    /// `dod-export`: two exports of one version are byte-identical, the header
    /// carries the format version, every entry carries its frozen content and
    /// identity; an unknown version is 404 and a missing id is 400.
    #[tokio::test]
    async fn the_export_is_byte_identical_for_a_version() {
        let harness = harness().await;
        set_quorum(&harness, 0).await;
        let alpha = published_product(&harness, "e-alpha", "Alpha Line").await;
        let outcome = crate::infra::increment::drain_tenant(
            &harness.state.db,
            &harness.state.sink,
            TENANT,
            Utc::now() + ChronoDuration::minutes(6),
        )
        .await
        .expect("drain");
        let crate::infra::increment::DrainOutcome::Committed {
            catalog_version_id, ..
        } = outcome
        else {
            panic!("the batch's bulk-lane request closes into one version: {outcome:?}");
        };
        let app = || {
            crate::api::rest::bulk::router(
                Arc::clone(&harness.state),
                &toolkit::api::OpenApiRegistryImpl::new(),
            )
            .layer(axum::Extension(crate::test_support::flat_in_enforcer(
                TENANT,
            )))
        };
        let get = |uri: String| async move {
            app()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(uri)
                        .extension(crate::test_support::authed_ctx(TENANT))
                        .body(Body::empty())
                        .expect("build the request"),
                )
                .await
                .expect("the router answers")
        };
        let uri = format!("/bss-products/v1/bulk/exports?catalogVersionId={catalog_version_id}");
        let first = get(uri.clone()).await;
        assert_eq!(first.status().as_u16(), 200);
        let first = axum::body::to_bytes(first.into_body(), 1 << 20)
            .await
            .expect("read");
        let second = get(uri.clone()).await;
        let second = axum::body::to_bytes(second.into_body(), 1 << 20)
            .await
            .expect("read");
        assert_eq!(first, second, "byte-identical for a given version (C4)");
        let artifact: serde_json::Value = serde_json::from_slice(&first).expect("json");
        assert_eq!(artifact["format_version"], json!(1));
        assert_eq!(artifact["catalog_version_id"], json!(catalog_version_id));
        let entries = artifact["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["entity_id"], json!(alpha));
        assert!(
            entries[0]["identity"]
                .as_str()
                .is_some_and(|s| s.contains("Alpha Line"))
        );
        assert!(
            entries[0]["content"]
                .as_str()
                .is_some_and(|s| s.contains("Alpha Line"))
        );
        assert!(
            !artifact["captures"]
                .as_array()
                .expect("captures")
                .is_empty()
        );

        let unknown = get("/bss-products/v1/bulk/exports?catalogVersionId=999".to_owned()).await;
        assert_eq!(unknown.status().as_u16(), 404);
        let missing = get("/bss-products/v1/bulk/exports".to_owned()).await;
        assert_eq!(missing.status().as_u16(), 400);
    }
}
