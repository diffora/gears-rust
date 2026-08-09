//! Phase 2's per-row optimistic commit, executed against a real store
//! (`design/12-operator-efficiency.md` §3 `inst-bk-phase2`, §4 `inst-bi-commit`;
//! D-141, D-291).
//!
//! **What this suite is about is the partial result.** Phase 2's product is
//! `{committed, conflicted}`: a conflict fails one row and the rest stand. So
//! every case here asserts *both* halves — which rows landed and which did not —
//! because a receipt that named only the failures would be satisfied by a run
//! that committed nothing, and one that named only the successes by a run that
//! silently dropped the rest.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::bulk::{BulkKind, BulkState};
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::import::ImportRow;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::bulk::{BULK_ROW_CONFLICT, CommitReceipt, commit_batch};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewBulkOperation, NewPriceDraft, PriceRepo, bulk_repo};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_51);
const ACTOR: Uuid = Uuid::from_u128(0xac_50);
const CORRELATION: Uuid = Uuid::from_u128(0xc0_51);

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x50_c6))
}
fn phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(0xfa_90))
}
fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 9, hour, 0, 0).unwrap()
}
fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}
fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: ACTOR,
        recorded_at: at(10),
        correlation_id: CORRELATION,
    }
}

fn key(region: &str) -> ScopeKey {
    ScopeKey::new(
        plan(),
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new(region).expect("a non-blank region"),
        phase(),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("the class pairs with the cohort")
}

fn content(amount: i64) -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(amount).expect("a non-negative amount"));
    PriceContent {
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: Some("advance".to_owned()),
        proration_contract: None,
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

fn row(scope_key: ScopeKey, amount: i64, if_match: Option<RowVersion>) -> ImportRow {
    ImportRow {
        scope_key,
        content: content(amount),
        if_match,
    }
}

struct Harness {
    provider: DBProvider<DbError>,
    prices: PriceRepo,
}

async fn harness() -> Harness {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    let prices = PriceRepo::new(provider.clone());
    Harness { provider, prices }
}

/// Author a draft row and answer `(price_id, its version)`.
async fn seed_draft(h: &Harness, scope_key: ScopeKey, amount: i64) -> (Uuid, RowVersion) {
    let price_id = Uuid::now_v7();
    let record = h
        .prices
        .create_draft(
            &scope(),
            TENANT,
            NewPriceDraft {
                price_id,
                scope_key,
                content: content(amount),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author the draft");
    (price_id, record.row_version)
}

async fn open_run(h: &Harness, client_key: &str) -> Uuid {
    let conn = h.provider.conn().expect("conn");
    bulk_repo::open(
        &conn,
        &scope(),
        NewBulkOperation {
            operation_id: Uuid::now_v7(),
            tenant_id: TENANT,
            kind: BulkKind::Import,
            client_key: client_key.to_owned(),
            report: serde_json::json!({}),
            submitted_by: ACTOR,
            submitted_at: at(10),
        },
    )
    .await
    .expect("open the run")
    .operation_id
}

async fn run_phase_2(h: &Harness, operation_id: Uuid, rows: &[ImportRow]) -> CommitReceipt {
    commit_batch(
        &h.provider,
        &h.prices,
        &scope(),
        TENANT,
        operation_id,
        rows,
        stamp(),
        at(11),
    )
    .await
    .expect("the run itself does not fail")
}

async fn state_of(h: &Harness, operation_id: Uuid) -> BulkState {
    let conn = h.provider.conn().expect("conn");
    bulk_repo::read(&conn, &scope(), TENANT, operation_id)
        .await
        .expect("read")
        .expect("exists")
        .state
}

fn committed_rows(receipt: &CommitReceipt) -> Vec<usize> {
    receipt.committed.iter().map(|row| row.row).collect()
}

fn conflicted_rows(receipt: &CommitReceipt) -> Vec<usize> {
    receipt.conflicted.iter().map(|row| row.row).collect()
}

#[tokio::test]
async fn new_keys_are_authored_and_the_run_completes() {
    let h = harness().await;
    let run = open_run(&h, "c-1").await;

    let receipt = run_phase_2(
        &h,
        run,
        &[row(key("eu"), 9_900, None), row(key("us"), 12_500, None)],
    )
    .await;

    assert_eq!(committed_rows(&receipt), vec![0, 1]);
    assert_eq!(conflicted_rows(&receipt), Vec::<usize>::new());
    assert_eq!(state_of(&h, run).await, BulkState::Completed);
    assert_ne!(
        receipt.committed[0].price_id, receipt.committed[1].price_id,
        "two keys are two rows"
    );
}

#[tokio::test]
async fn an_edit_lands_under_its_etag_and_moves_the_row() {
    let h = harness().await;
    let (price_id, version) = seed_draft(&h, key("eu"), 9_900).await;
    let run = open_run(&h, "c-2").await;

    let receipt = run_phase_2(&h, run, &[row(key("eu"), 12_500, Some(version))]).await;

    assert_eq!(committed_rows(&receipt), vec![0]);
    assert_eq!(
        receipt.committed[0].price_id, price_id,
        "an edit writes the row that already held the key, not a new one"
    );
    assert_eq!(state_of(&h, run).await, BulkState::Completed);
}

#[tokio::test]
async fn a_stale_etag_fails_only_its_own_row_and_the_rest_stand() {
    // **`inst-bk-phase2`'s whole content.** The conflicted row is named for
    // retry; the others are committed and stay committed.
    let h = harness().await;
    let (_, version) = seed_draft(&h, key("eu"), 9_900).await;
    let run = open_run(&h, "c-3").await;

    let receipt = run_phase_2(
        &h,
        run,
        &[
            row(key("eu"), 12_500, Some(RowVersion::new(version.get() + 7))),
            row(key("us"), 7_500, None),
        ],
    )
    .await;

    assert_eq!(
        conflicted_rows(&receipt),
        vec![0],
        "the stale row, and only it"
    );
    assert_eq!(
        receipt.conflicted[0].violations[0].code, BULK_ROW_CONFLICT,
        "section 5's per-row code"
    );
    assert_eq!(
        committed_rows(&receipt),
        vec![1],
        "and its neighbour stands: committed rows stand"
    );
    assert_eq!(
        state_of(&h, run).await,
        BulkState::CompletedWithConflicts,
        "which is a success, not a failure"
    );
}

#[tokio::test]
async fn a_row_asserting_no_version_over_an_existing_draft_conflicts() {
    // Silent overwrite never happens in either direction: without a version there
    // is nothing to compare, so the row is refused rather than written over
    // somebody's edit.
    let h = harness().await;
    seed_draft(&h, key("eu"), 9_900).await;
    let run = open_run(&h, "c-4").await;

    let receipt = run_phase_2(&h, run, &[row(key("eu"), 12_500, None)]).await;

    assert_eq!(conflicted_rows(&receipt), vec![0]);
    assert!(
        receipt.conflicted[0].violations[0].detail.contains("ETag"),
        "the refusal says what to do: {}",
        receipt.conflicted[0].violations[0].detail
    );
    assert_eq!(committed_rows(&receipt), Vec::<usize>::new());
}

#[tokio::test]
async fn a_row_asserting_a_version_over_nothing_conflicts_too() {
    // The mirror fault: the row it meant to edit is gone. Reported rather than
    // silently turned into a create, which would resurrect a row an operator
    // deliberately abandoned.
    let h = harness().await;
    let run = open_run(&h, "c-5").await;

    let receipt = run_phase_2(&h, run, &[row(key("eu"), 12_500, Some(RowVersion::new(0)))]).await;

    assert_eq!(conflicted_rows(&receipt), vec![0]);
    assert_eq!(committed_rows(&receipt), Vec::<usize>::new());
    assert_eq!(state_of(&h, run).await, BulkState::CompletedWithConflicts);
}

#[tokio::test]
async fn the_locks_are_released_when_the_run_ends() {
    // `inst-bk-lock` holds rows only while the run is committing. A lock left
    // behind is an interactive editor refused forever by a run that finished.
    let h = harness().await;
    let (price_id, version) = seed_draft(&h, key("eu"), 9_900).await;
    let run = open_run(&h, "c-6").await;

    run_phase_2(&h, run, &[row(key("eu"), 12_500, Some(version))]).await;

    let conn = h.provider.conn().expect("conn");
    assert_eq!(
        bulk_repo::lock_holder(&conn, &scope(), TENANT, price_id)
            .await
            .expect("read the lock"),
        None,
        "the run released what it held"
    );
}

#[tokio::test]
async fn a_row_another_run_holds_conflicts_the_whole_batch_and_commits_nothing() {
    // The one run-level conflict there is. §4 offers no failure edge out of
    // `committing`, so the honest terminal state is `completed_with_conflicts`
    // with every row named — and, critically, nothing written.
    let h = harness().await;
    let (price_id, version) = seed_draft(&h, key("eu"), 9_900).await;
    let (_, other_version) = seed_draft(&h, key("us"), 5_000).await;

    // A neighbour run holds the row this one wants.
    let neighbour = open_run(&h, "c-7").await;
    let conn = h.provider.conn().expect("conn");
    bulk_repo::advance(
        &conn,
        &scope(),
        TENANT,
        neighbour,
        BulkState::Committing,
        serde_json::json!({}),
        at(11),
    )
    .await
    .expect("the neighbour enters committing");
    bulk_repo::take_locks(&conn, &scope(), TENANT, neighbour, &[price_id], at(11))
        .await
        .expect("and takes the row");

    let run = open_run(&h, "c-8").await;
    let receipt = run_phase_2(
        &h,
        run,
        &[
            row(key("eu"), 12_500, Some(version)),
            row(key("us"), 7_500, Some(other_version)),
        ],
    )
    .await;

    assert_eq!(conflicted_rows(&receipt), vec![0, 1], "every row");
    assert_eq!(committed_rows(&receipt), Vec::<usize>::new());
    assert_eq!(state_of(&h, run).await, BulkState::CompletedWithConflicts);
    assert!(
        receipt.conflicted[0].violations[0]
            .detail
            .contains(&neighbour.to_string()),
        "and the refusal names the run holding it: {}",
        receipt.conflicted[0].violations[0].detail
    );
}

#[tokio::test]
async fn an_interactive_edit_is_refused_by_the_lock_and_told_which_run() {
    // `fr-concurrent-edit`'s other end, in the door rather than on a surface: a
    // rule that lives on one authoring path is not a rule, and this is the second
    // path onto the same rows.
    let h = harness().await;
    let (price_id, version) = seed_draft(&h, key("eu"), 9_900).await;
    let run = open_run(&h, "c-10").await;
    let conn = h.provider.conn().expect("conn");
    bulk_repo::advance(
        &conn,
        &scope(),
        TENANT,
        run,
        BulkState::Committing,
        serde_json::json!({}),
        at(11),
    )
    .await
    .expect("enter committing");
    bulk_repo::take_locks(&conn, &scope(), TENANT, run, &[price_id], at(11))
        .await
        .expect("hold the row");

    let refused = h
        .prices
        .update_draft(
            &scope(),
            TENANT,
            price_id,
            version,
            content(12_500),
            stamp(),
            None,
        )
        .await
        .expect_err("an interactive edit belongs to no run");
    let rendered = format!("{refused:?}");
    assert!(
        rendered.contains(&run.to_string()),
        "the conflict names the run holding it: {rendered}"
    );
}

#[tokio::test]
async fn the_run_holding_the_row_edits_it_freely() {
    // **The distinction the guard exists for, and the easy mistake.** Phase 2
    // edits the very rows it locked, so a guard refusing every locked row would
    // make the commit refuse its own batch. What the lock excludes is somebody
    // else.
    let h = harness().await;
    let (price_id, version) = seed_draft(&h, key("eu"), 9_900).await;
    let run = open_run(&h, "c-11").await;
    let conn = h.provider.conn().expect("conn");
    bulk_repo::advance(
        &conn,
        &scope(),
        TENANT,
        run,
        BulkState::Committing,
        serde_json::json!({}),
        at(11),
    )
    .await
    .expect("enter committing");
    bulk_repo::take_locks(&conn, &scope(), TENANT, run, &[price_id], at(11))
        .await
        .expect("hold the row");

    h.prices
        .update_draft(
            &scope(),
            TENANT,
            price_id,
            version,
            content(12_500),
            stamp(),
            Some(run),
        )
        .await
        .expect("its own holder passes");
}

#[tokio::test]
async fn the_stored_report_carries_both_halves() {
    // `inst-bk-idem` replays this report to a retry, so what the run *stores* is
    // the contract — not merely what `commit_batch` returned to its caller.
    let h = harness().await;
    let (_, version) = seed_draft(&h, key("eu"), 9_900).await;
    let run = open_run(&h, "c-9").await;

    run_phase_2(
        &h,
        run,
        &[
            row(key("eu"), 12_500, Some(RowVersion::new(version.get() + 7))),
            row(key("us"), 7_500, None),
        ],
    )
    .await;

    let conn = h.provider.conn().expect("conn");
    let stored = bulk_repo::read(&conn, &scope(), TENANT, run)
        .await
        .expect("read")
        .expect("exists")
        .report;
    assert_eq!(
        stored["committed"].as_array().expect("an array").len(),
        1,
        "the committed half survives the round trip: {stored}"
    );
    assert_eq!(
        stored["conflicted"][0]["violations"][0]["code"],
        serde_json::json!(BULK_ROW_CONFLICT),
        "and so does the conflicted half, code included: {stored}"
    );
}
