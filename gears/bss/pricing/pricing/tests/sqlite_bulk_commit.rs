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
use bss_pricing::infra::storage::entity::price;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{
    IdempotencyGate, NewBulkOperation, NewPriceDraft, PriceRepo, bulk_repo,
};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureEntityExt};
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
            request_hash: IdempotencyGate::payload_hash(client_key),
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
        BulkState::Validating,
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
        BulkState::Validating,
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
        BulkState::Validating,
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
async fn a_run_that_cannot_take_every_lock_releases_the_ones_it_took() {
    // **The leak the first arrangement hid** (D-294). The earlier case put the
    // contended row FIRST, so the very first insert collided and nothing partial
    // existed. Here it is second: the run takes one lock, collides on the next,
    // and the first must not be left held by an operation that is over — which is
    // the freeze `inst-bs-done`'s "lock released either way" exists to prevent.
    let h = harness().await;
    let (free_row, free_version) = seed_draft(&h, key("us"), 5_000).await;
    let (held_row, held_version) = seed_draft(&h, key("eu"), 9_900).await;

    let neighbour = open_run(&h, "c-12").await;
    let conn = h.provider.conn().expect("conn");
    bulk_repo::advance(
        &conn,
        &scope(),
        TENANT,
        neighbour,
        BulkState::Validating,
        BulkState::Committing,
        serde_json::json!({}),
        at(11),
    )
    .await
    .expect("the neighbour enters committing");
    bulk_repo::take_locks(&conn, &scope(), TENANT, neighbour, &[held_row], at(11))
        .await
        .expect("it holds the second row");

    let run = open_run(&h, "c-13").await;
    let receipt = run_phase_2(
        &h,
        run,
        &[
            row(key("us"), 7_500, Some(free_version)),
            row(key("eu"), 12_500, Some(held_version)),
        ],
    )
    .await;

    assert_eq!(conflicted_rows(&receipt), vec![0, 1]);
    assert_eq!(committed_rows(&receipt), Vec::<usize>::new());
    assert_eq!(
        bulk_repo::lock_holder(&conn, &scope(), TENANT, free_row)
            .await
            .expect("read the lock"),
        None,
        "the row this run did take must be free again"
    );

    // And nothing was written: the seeded amounts stand.
    let rows = bss_pricing::infra::storage::repo::price_repo::load_for_plan(
        &conn,
        &scope(),
        TENANT,
        plan(),
        &[bss_pricing::domain::lifecycle::LifecycleState::Draft],
    )
    .await
    .expect("read the drafts");
    for record in rows {
        assert!(
            record.row.amount_minor.expect("an amount").get() < 9_901,
            "no row moved: {record:?}"
        );
    }
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

#[tokio::test]
async fn a_run_level_failure_keeps_the_rows_that_did_commit_in_the_report() {
    // **The report a failed run leaves has to be true of the store** (D-300).
    // Every row is its own transaction, so on a fault at row 1 the draft row 0
    // wrote is *in* `pricing_price`. The receipt used to be discarded here and
    // replaced wholesale by "every row un-attempted", so the run's stored report
    // denied a row that exists — and an operator who resubmits on that report
    // meets a stale `ETag` on every row it hid.
    //
    // The fault is a real one: `billing_timing` is a free `String` on this plane,
    // Slice 6 owns its vocabulary, and no Phase-1 rule screens it — so the column
    // CHECK is the first thing that sees it.
    let h = harness().await;
    let operation_id = open_run(&h, "the-run-fails-midway").await;

    let mut bad = row(key("us"), 2_500, None);
    bad.content.billing_timing = Some("whenever".to_owned());

    let failure = commit_batch(
        &h.provider,
        &h.prices,
        &scope(),
        TENANT,
        operation_id,
        &[row(key("eu"), 1_500, None), bad],
        stamp(),
        at(11),
    )
    .await
    .expect_err("a fault that is the run's still reaches the caller");
    assert!(
        failure.to_string().contains("billing_timing"),
        "and it names what went wrong: {failure}"
    );

    let conn = h.provider.conn().expect("conn");
    let run = bulk_repo::read(&conn, &scope(), TENANT, operation_id)
        .await
        .expect("read")
        .expect("exists");
    assert!(
        run.state.is_terminal(),
        "the run lands terminal whatever happened: {:?}",
        run.state
    );

    let committed = run.report["committed"]
        .as_array()
        .unwrap_or_else(|| panic!("the stored report's committed arm: {}", run.report));
    assert_eq!(
        committed.len(),
        1,
        "row 0 committed and the report has to say so: {}",
        run.report
    );
    assert_eq!(committed[0]["row"], serde_json::json!(0));

    // And the row it names is really there — which is what makes the old report a
    // false statement rather than a conservative one.
    let landed = committed[0]["price_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("a committed row names its draft");
    let stored = price::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(price::Column::PriceId.eq(landed)))
        .one(&conn)
        .await
        .expect("read the row");
    assert!(
        stored.is_some(),
        "the report's committed row exists in the store: {}",
        run.report
    );
}

// ---------------------------------------------------------------------------
// Z8-8 — the two exits no match arm reaches.
// ---------------------------------------------------------------------------

/// **The RED this case is about.** `commit_batch` covered the `Err` exit
/// meticulously — D-294 took the `?` off `commit_rows` and D-300 took it off
/// `release_locks` — and covered neither of the other two ways the function can
/// end. A panic inside `commit_rows` unwinds *past* a match arm exactly as it
/// unwinds past everything else, and a dropped future (a client disconnect, a
/// shutdown signal, a losing `select!` arm) never runs any of this crate's code
/// again, match arm or not. Neither is rolled back, because `take_locks` requires
/// an autocommit connection **by design**: "Postgres aborts an enclosing
/// transaction on a failed statement".
///
/// What is left behind is the freeze `inst-bs-done`'s "lock released either way"
/// exists to prevent: the rows stay held against every interactive editor and the
/// run stays `committing` with no timeout, no sweeper, and D-37's lease takeover
/// designed and unbuilt.
///
/// Only [`Drop`] is the language's own guarantee across a panic and a
/// cancellation together, and the crate already has the precedent one module over
/// (`infra::repricing::RunLockGuard`, whose own doc names this very finding).
///
/// **Armed at an abnormal exit, not at an `Err`.** The task is genuinely
/// cancelled with `JoinHandle::abort` — `select!`'s own mechanism for a losing arm
/// — and only after the lock has been *observed* taken, because a future dropped
/// before `take_locks` ran would prove nothing about the guard. The polling loop
/// is what stops this case passing that way.
#[tokio::test]
async fn a_commit_future_dropped_mid_flight_releases_its_locks_and_lands_the_run_terminal() {
    let h = harness().await;

    // Enough rows that the per-row loop is still running when the abort lands:
    // every row is its own transaction with its own audit write, so this is the
    // margin the cancellation happens inside. They are **edits**, because only a
    // row whose draft already exists is a row `take_locks` has anything to lock.
    let mut rows = Vec::new();
    let mut price_ids = Vec::new();
    for index in 0..40_u32 {
        let region = format!("r{index}");
        let (price_id, version) = seed_draft(&h, key(&region), 9_900).await;
        price_ids.push(price_id);
        rows.push(row(key(&region), 12_500, Some(version)));
    }
    let run = open_run(&h, "c-drop-1").await;

    let provider = h.provider.clone();
    let prices = h.prices.clone();
    let handle = tokio::spawn(async move {
        commit_batch(
            &provider,
            &prices,
            &scope(),
            TENANT,
            run,
            &rows,
            stamp(),
            at(11),
        )
        .await
    });

    // Polled rather than slept a fixed amount, so this is not a bet on how fast
    // one test box is.
    let take_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let conn = h.provider.conn().expect("conn");
        let held = bulk_repo::lock_holder(&conn, &scope(), TENANT, price_ids[0])
            .await
            .expect("read the lock");
        if held == Some(run) {
            break;
        }
        assert!(
            std::time::Instant::now() < take_deadline,
            "the commit never took its row locks within 10s, and this case cannot say anything \
             about a drop guard without first observing the locks taken"
        );
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }

    handle.abort();
    match handle.await {
        Err(ref e) if e.is_cancelled() => {}
        other => panic!(
            "the task must have been genuinely cancelled mid-flight for this case to prove \
             anything, not merely finished before the abort landed: {other:?}"
        ),
    }

    // The fallback is a detached spawn — `Drop` cannot await — so it is not
    // synchronous with the abort. Polled, for the reason above.
    let settle_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let conn = h.provider.conn().expect("conn");
        let run_now = bulk_repo::read(&conn, &scope(), TENANT, run)
            .await
            .expect("read the run")
            .expect("the run exists");
        let mut held = None;
        for &price_id in &price_ids {
            if let Some(holder) = bulk_repo::lock_holder(&conn, &scope(), TENANT, price_id)
                .await
                .expect("read the lock")
            {
                held = Some((price_id, holder));
                break;
            }
        }
        if held.is_none() && run_now.state != BulkState::Committing {
            assert!(
                run_now.state.is_terminal(),
                "and the run lands on one of the terminal states rather than somewhere no \
                 edge leaves: {run_now:?}"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < settle_deadline,
            "10s after a cancelled commit the run is still {:?} and {held:?} still holds a \
             row: nothing releases the locks a dropped future left, and nothing lands the run",
            run_now.state
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// The **positive control** for the guard: a commit that ends normally must not
/// be swept by it. Without this, a guard that fired unconditionally — landing
/// every run `completed_with_conflicts` over the receipt's own terminal state —
/// would satisfy the case above.
#[tokio::test]
async fn a_commit_that_ends_normally_is_not_touched_by_the_guard() {
    let h = harness().await;
    let (_, version) = seed_draft(&h, key("eu"), 9_900).await;
    let run = open_run(&h, "c-drop-2").await;

    let receipt = run_phase_2(&h, run, &[row(key("eu"), 12_500, Some(version))]).await;
    assert_eq!(committed_rows(&receipt), vec![0]);

    // Long enough that a detached sweep, if one had been spawned, would have run.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let conn = h.provider.conn().expect("conn");
    let stored = bulk_repo::read(&conn, &scope(), TENANT, run)
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(
        stored.state,
        BulkState::Completed,
        "the receipt's own terminal state stands, not the guard's fallback: {stored:?}"
    );
    assert!(
        stored.report.get("aborted").is_none(),
        "and no interruption note was stamped over a run that finished: {}",
        stored.report
    );
}
