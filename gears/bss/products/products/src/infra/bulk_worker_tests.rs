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
