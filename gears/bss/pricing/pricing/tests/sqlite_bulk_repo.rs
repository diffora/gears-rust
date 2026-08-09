//! The bulk run's store surface, executed
//! (`design/12-operator-efficiency.md` §4, §6; D-260, D-262, D-267, D-290).
//!
//! **The edges are asserted through the store, not against a table written here.**
//! The state machine lives in `pricing_bulk_operation`'s trigger on both engines,
//! and the repository deliberately does not restate it — so the only way to say
//! what the edges are is to try them. A fixture listing them would be the second
//! spelling the repository refuses to be, and D-283 is what a rule judged against
//! a private copy of the schema costs.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::bulk::{BulkKind, BulkState};
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewBulkOperation, NewPriceDraft, PriceRepo, bulk_repo};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_41);
const ACTOR: Uuid = Uuid::from_u128(0xac_40);

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 9, hour, 0, 0).unwrap()
}
fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

async fn provider() -> DBProvider<DbError> {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    DBProvider::<DbError>::new(db)
}

fn new_run(kind: BulkKind, client_key: &str) -> NewBulkOperation {
    NewBulkOperation {
        operation_id: Uuid::now_v7(),
        tenant_id: TENANT,
        kind,
        client_key: client_key.to_owned(),
        report: serde_json::json!({ "rows": [] }),
        submitted_by: ACTOR,
        submitted_at: at(10),
    }
}

/// Author a real draft price row and answer its id.
///
/// **The lock's `price_id` is a foreign key**, so a lock cannot be taken on a row
/// that does not exist — which is the store saying that `inst-bk-lock` is about
/// *rows an edit could target*, not about a set of ids a run declares.
async fn a_price_row(p: &DBProvider<DbError>, region: &str) -> Uuid {
    let price_id = Uuid::now_v7();
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    PriceRepo::new(p.clone())
        .create_draft(
            &scope(),
            TENANT,
            NewPriceDraft {
                price_id,
                scope_key: ScopeKey::new(
                    PlanId::new(Uuid::from_u128(0x50_c5)),
                    CurrencyCode::new("EUR").expect("three letters"),
                    Region::new(region).expect("a non-blank region"),
                    PhaseId::new(Uuid::from_u128(0xfa_80)),
                    PriceEligibility::AllSubscriptions,
                    ChargeKind::Recurring,
                    Cohort::None,
                )
                .expect("the class pairs with the cohort"),
                content: PriceContent {
                    row,
                    tax_inclusive: false,
                    tax_category_ref: None,
                    billing_timing: Some("advance".to_owned()),
                    proration_contract: None,
                    rounding_policy_ref: Some("half_up".to_owned()),
                    grandfather_until: None,
                    supersedes_price_id: None,
                },
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: Uuid::from_u128(0xc0_41),
            },
        )
        .await
        .expect("author the row");
    price_id
}

/// Open a run and move it to `committing`, which is the only state its locks may
/// be taken in — the lock table's own trigger says so ("the bulk lock takes
/// effect only on entry to committing"), and it is right: a lock belongs to a run
/// that is actually committing, not to one still validating.
async fn committing_run(conn: &toolkit_db::secure::DbConn<'_>, client_key: &str) -> Uuid {
    let run = bulk_repo::open(conn, &scope(), new_run(BulkKind::Import, client_key))
        .await
        .expect("open");
    bulk_repo::advance(
        conn,
        &scope(),
        TENANT,
        run.operation_id,
        BulkState::Committing,
        serde_json::json!({}),
        at(11),
    )
    .await
    .expect("enter committing");
    run.operation_id
}

#[tokio::test]
async fn a_run_is_born_validating_whatever_the_caller_wanted() {
    // `NewBulkOperation` carries no state, and this is why: the insert trigger
    // refuses any birth state but `validating`, so a field would offer a choice
    // the store takes back.
    let p = provider().await;
    let conn = p.conn().expect("conn");
    let run = bulk_repo::open(&conn, &scope(), new_run(BulkKind::Import, "k-1"))
        .await
        .expect("open the run");

    assert_eq!(run.state, BulkState::Validating);
    assert_eq!(run.kind, BulkKind::Import);
    assert!(run.completed_at.is_none(), "a run at birth is not over");
    assert_eq!(
        bulk_repo::read(&conn, &scope(), TENANT, run.operation_id)
            .await
            .expect("read it back")
            .expect("it exists"),
        run,
        "what `open` answers is what the store holds"
    );
}

#[tokio::test]
async fn one_client_key_opens_one_run() {
    // O4's key is unique per tenant, which is what makes the replay possible at
    // all: a second run under one key would leave two reports to answer with.
    let p = provider().await;
    let conn = p.conn().expect("conn");
    bulk_repo::open(&conn, &scope(), new_run(BulkKind::Import, "k-2"))
        .await
        .expect("the first");

    let second = bulk_repo::open(&conn, &scope(), new_run(BulkKind::Import, "k-2")).await;
    assert!(second.is_err(), "the second is refused by the unique key");

    let found = bulk_repo::find_by_client_key(&conn, &scope(), TENANT, "k-2")
        .await
        .expect("read by key")
        .expect("the first run");
    assert_eq!(found.client_key, "k-2");
}

#[tokio::test]
async fn an_imports_happy_path_is_validating_then_committing_then_completed() {
    let p = provider().await;
    let conn = p.conn().expect("conn");
    let run = bulk_repo::open(&conn, &scope(), new_run(BulkKind::Import, "k-3"))
        .await
        .expect("open");

    let committing = bulk_repo::advance(
        &conn,
        &scope(),
        TENANT,
        run.operation_id,
        BulkState::Committing,
        serde_json::json!({ "phase": 2 }),
        at(11),
    )
    .await
    .expect("validating -> committing");
    assert_eq!(committing.state, BulkState::Committing);
    assert!(
        committing.completed_at.is_none(),
        "a run mid-commit is not over, and the CHECK agrees"
    );

    let done = bulk_repo::advance(
        &conn,
        &scope(),
        TENANT,
        run.operation_id,
        BulkState::Completed,
        serde_json::json!({ "committed": 3 }),
        at(12),
    )
    .await
    .expect("committing -> completed");
    assert_eq!(done.state, BulkState::Completed);
    assert_eq!(
        done.completed_at,
        Some(at(12)),
        "a terminal state is stamped"
    );
    assert_eq!(done.report, serde_json::json!({ "committed": 3 }));
}

#[tokio::test]
async fn a_move_that_is_not_an_edge_is_refused_by_the_store() {
    // The repository names a state and writes it; the trigger decides. So this
    // asserts the trigger, which is the only place the edges live.
    let p = provider().await;
    let conn = p.conn().expect("conn");
    let run = bulk_repo::open(&conn, &scope(), new_run(BulkKind::Import, "k-4"))
        .await
        .expect("open");

    let skipped = bulk_repo::advance(
        &conn,
        &scope(),
        TENANT,
        run.operation_id,
        BulkState::Completed,
        serde_json::json!({}),
        at(11),
    )
    .await;
    assert!(
        skipped.is_err(),
        "validating -> completed skips the commit and is not an edge"
    );
    assert_eq!(
        bulk_repo::read(&conn, &scope(), TENANT, run.operation_id)
            .await
            .expect("read")
            .expect("exists")
            .state,
        BulkState::Validating,
        "and the refusal left the run where it was"
    );
}

#[tokio::test]
async fn an_import_cannot_wait_for_an_approval_it_never_needs() {
    // D-137: a bulk import is never material, so the `CHECK` forbids the pair
    // outright. The state exists for mass repricing, whose rows are published.
    let p = provider().await;
    let conn = p.conn().expect("conn");
    let import = bulk_repo::open(&conn, &scope(), new_run(BulkKind::Import, "k-5"))
        .await
        .expect("open");
    assert!(
        bulk_repo::advance(
            &conn,
            &scope(),
            TENANT,
            import.operation_id,
            BulkState::AwaitingApproval,
            serde_json::json!({}),
            at(11),
        )
        .await
        .is_err()
    );

    // The same move on a repricing run is legal — otherwise this case would pass
    // for a store that refused the state to everybody.
    let repricing = bulk_repo::open(&conn, &scope(), new_run(BulkKind::Repricing, "k-6"))
        .await
        .expect("open");
    let waiting = bulk_repo::advance(
        &conn,
        &scope(),
        TENANT,
        repricing.operation_id,
        BulkState::AwaitingApproval,
        serde_json::json!({}),
        at(11),
    )
    .await
    .expect("a repricing run may wait");
    assert_eq!(waiting.state, BulkState::AwaitingApproval);
}

#[tokio::test]
async fn a_rejected_run_leaves_awaiting_approval_and_is_over() {
    // D-267's whole content: before it, a refused batch approval stranded the run
    // in `awaiting_approval` with no exit.
    let p = provider().await;
    let conn = p.conn().expect("conn");
    let run = bulk_repo::open(&conn, &scope(), new_run(BulkKind::Repricing, "k-7"))
        .await
        .expect("open");
    bulk_repo::advance(
        &conn,
        &scope(),
        TENANT,
        run.operation_id,
        BulkState::AwaitingApproval,
        serde_json::json!({}),
        at(11),
    )
    .await
    .expect("waiting");

    let rejected = bulk_repo::advance(
        &conn,
        &scope(),
        TENANT,
        run.operation_id,
        BulkState::Rejected,
        serde_json::json!({ "reason": "refused" }),
        at(12),
    )
    .await
    .expect("awaiting_approval -> rejected");
    assert_eq!(rejected.state, BulkState::Rejected);
    assert_eq!(
        rejected.completed_at,
        Some(at(12)),
        "rejected is terminal and the CHECK stamps it"
    );

    assert!(
        bulk_repo::advance(
            &conn,
            &scope(),
            TENANT,
            run.operation_id,
            BulkState::Committing,
            serde_json::json!({}),
            at(13),
        )
        .await
        .is_err(),
        "and nothing leaves it"
    );
}

#[tokio::test]
async fn two_runs_cannot_hold_one_row_and_the_refusal_names_the_holder() {
    // `inst-bk-lock` and `fr-concurrent-edit`: the mutual exclusion is the key,
    // and the point of reading the holder back is that the conflict can name it.
    let p = provider().await;
    let conn = p.conn().expect("conn");
    let first = committing_run(&conn, "k-8").await;
    let second = committing_run(&conn, "k-9").await;
    let row = a_price_row(&p, "eu").await;

    bulk_repo::take_locks(&conn, &scope(), TENANT, first, &[row], at(11))
        .await
        .expect("the first run takes it");

    let clash = bulk_repo::take_locks(&conn, &scope(), TENANT, second, &[row], at(11))
        .await
        .expect_err("the second cannot");
    match clash {
        RepoError::BulkRowLocked {
            price_id,
            bulk_operation_id,
        } => {
            assert_eq!(price_id, row.to_string());
            assert_eq!(
                bulk_operation_id,
                first.to_string(),
                "the refusal names the run that holds it, not merely that one does"
            );
        }
        other => panic!("expected a bulk-row-lock refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn releasing_frees_the_row_for_the_next_run() {
    let p = provider().await;
    let conn = p.conn().expect("conn");
    let first = committing_run(&conn, "k-10").await;
    let second = committing_run(&conn, "k-11").await;
    let rows = [a_price_row(&p, "eu").await, a_price_row(&p, "us").await];

    bulk_repo::take_locks(&conn, &scope(), TENANT, first, &rows, at(11))
        .await
        .expect("take both");
    assert_eq!(
        bulk_repo::lock_holder(&conn, &scope(), TENANT, rows[0])
            .await
            .expect("read"),
        Some(first)
    );

    let freed = bulk_repo::release_locks(&conn, &scope(), TENANT, first)
        .await
        .expect("release");
    assert_eq!(freed, 2, "both, and only this run's");
    assert_eq!(
        bulk_repo::lock_holder(&conn, &scope(), TENANT, rows[0])
            .await
            .expect("read"),
        None
    );

    bulk_repo::take_locks(&conn, &scope(), TENANT, second, &rows, at(12))
        .await
        .expect("the next run may take them");
}

#[tokio::test]
async fn a_release_takes_only_its_own_runs_locks() {
    // Without this the release sweep is a `DELETE` nobody has bounded, and one
    // run finishing would free every row every other run is holding.
    let p = provider().await;
    let conn = p.conn().expect("conn");
    let mine = committing_run(&conn, "k-12").await;
    let theirs = committing_run(&conn, "k-13").await;
    let my_row = a_price_row(&p, "eu").await;
    let their_row = a_price_row(&p, "us").await;

    bulk_repo::take_locks(&conn, &scope(), TENANT, mine, &[my_row], at(11))
        .await
        .expect("mine");
    bulk_repo::take_locks(&conn, &scope(), TENANT, theirs, &[their_row], at(11))
        .await
        .expect("theirs");

    assert_eq!(
        bulk_repo::release_locks(&conn, &scope(), TENANT, mine)
            .await
            .expect("release"),
        1
    );
    assert_eq!(
        bulk_repo::lock_holder(&conn, &scope(), TENANT, their_row)
            .await
            .expect("read"),
        Some(theirs),
        "the other run still holds its own row"
    );
}
