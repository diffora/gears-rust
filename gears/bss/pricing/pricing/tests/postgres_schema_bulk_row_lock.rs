//! Slice 12's bulk lock **on the engine that runs in production**
//! (`design/12-operator-efficiency.md` §6, `inst-bk-lock`, `inst-bs-commit`,
//! `inst-bs-approval`, D-37).
//!
//! `sqlite_bulk_row_lock_store` proves the same rules against the mirror, and
//! that is not the same thing: the two arms are written separately — one PL/pgSQL
//! function against three `RAISE(ABORT, …)` triggers — and only the `SQLite` side
//! carries a trigger-**body** digest census. D-260 paid this debt twice after
//! finding a lost disjunct on the shipping engine invisible to every gate; a new
//! table owes a behavioural Postgres suite from the start.
//!
//! Postgres also names its constraints in the error, which the mirror cannot: a
//! key violation there is the bare `FOREIGN KEY constraint failed`. So the two
//! key cases below assert by name where the mirror has to assert by which
//! referent it removed.
//!
//! Run with:
//! `cargo test -p bss-pricing --test postgres_schema_bulk_row_lock -- --ignored`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

const RUN: &str = "11111111-1111-1111-1111-111111111111";
const TENANT: &str = "22222222-2222-2222-2222-222222222222";
const ACTOR: &str = "33333333-3333-3333-3333-333333333333";
const PRICE: &str = "44444444-4444-4444-4444-444444444444";
const OTHER_RUN: &str = "55555555-5555-5555-5555-555555555555";
const OTHER_TENANT: &str = "66666666-6666-6666-6666-666666666666";
const PLAN: &str = "77777777-7777-7777-7777-777777777777";
const PHASE: &str = "88888888-8888-8888-8888-888888888888";

async fn applied() -> DatabaseConnection {
    Pg::applied().await.raw().await
}

async fn exec(conn: &DatabaseConnection, sql: &str) -> Result<(), sea_orm::DbErr> {
    conn.execute(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        sql.to_owned(),
    ))
    .await
    .map(|_| ())
}

async fn must_succeed(conn: &DatabaseConnection, sql: &str) {
    exec(conn, sql)
        .await
        .unwrap_or_else(|e| panic!("statement must succeed: {sql}\n{e}"));
}

/// How many locks the tenant holds on the fixture's row.
///
/// Load-bearing, and it was missing: a `BEFORE DELETE` trigger returning `NULL`
/// **silently cancels** the delete, so a release that never happened raises no
/// error and a case asserting only "the statement did not fail" cannot tell the
/// two apart. Found by a probe that reddened nothing -- binding this table's
/// trigger to `DELETE` as well left every case green while no lock was ever
/// released, which is the exact freeze D-37's release path exists to prevent.
async fn locks_held(conn: &DatabaseConnection) -> i64 {
    conn.query_one(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT count(*) AS n FROM bss.pricing_bulk_row_lock \
             WHERE tenant_id = '{TENANT}' AND price_id = '{PRICE}'"
        ),
    ))
    .await
    .expect("run the count")
    .expect("the count returns a row")
    .try_get::<i64>("", "n")
    .expect("read the count")
}

async fn must_be_rejected(conn: &DatabaseConnection, sql: &str, by: &str) {
    let Err(err) = exec(conn, sql).await else {
        panic!("this statement must be rejected: {sql}");
    };
    let message = err.to_string();
    assert!(
        message.contains(by),
        "the rejection must be the one under test (`{by}`), got: {message}"
    );
}

fn seed_run(op: &str, tenant: &str) -> String {
    format!(
        "INSERT INTO bss.pricing_bulk_operation \
         (operation_id, tenant_id, kind, state, client_key, report, submitted_by, submitted_at) \
         VALUES ('{op}', '{tenant}', 'repricing', 'validating', 'ck-{op}', '{{}}'::jsonb, \
         '{ACTOR}', now())"
    )
}

fn advance(op: &str, state: &str) -> String {
    let completed = if matches!(
        state,
        "validation_failed" | "completed" | "completed_with_conflicts"
    ) {
        "now()"
    } else {
        "NULL"
    };
    format!(
        "UPDATE bss.pricing_bulk_operation SET state = '{state}', completed_at = {completed} \
         WHERE operation_id = '{op}'"
    )
}

/// The held row, as a real `pricing_price` row — the lock keys it.
fn seed_price(tenant: &str) -> String {
    format!(
        "INSERT INTO bss.pricing_price ( \
             price_id, tenant_id, plan_id, currency, region, phase, \
             charge_kind, amount_minor, model_kind, lifecycle_state, \
             created_by, created_at_utc) \
         VALUES ('{PRICE}', '{tenant}', '{PLAN}', 'EUR', 'eu', '{PHASE}', \
             'recurring', 1000, 'flat', 'published', '{ACTOR}', now())"
    )
}

fn take_lock(op: &str, tenant: &str) -> String {
    format!(
        "INSERT INTO bss.pricing_bulk_row_lock \
         (tenant_id, price_id, bulk_operation_id, locked_at) \
         VALUES ('{tenant}', '{PRICE}', '{op}', now())"
    )
}

/// A run in `committing` and the row it is about to hold.
async fn committing_run() -> DatabaseConnection {
    let conn = applied().await;
    must_succeed(&conn, &seed_price(TENANT)).await;
    must_succeed(&conn, &seed_run(RUN, TENANT)).await;
    must_succeed(&conn, &advance(RUN, "committing")).await;
    conn
}

/// A lock is taken while the run commits and released when it is done — the
/// whitelist half, and the shape every other case here departs from.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_committing_run_takes_and_releases_a_lock() {
    let conn = committing_run().await;
    must_succeed(&conn, &take_lock(RUN, TENANT)).await;
    assert_eq!(locks_held(&conn).await, 1, "the lock must have been taken");
    must_succeed(
        &conn,
        &format!(
            "DELETE FROM bss.pricing_bulk_row_lock \
             WHERE tenant_id = '{TENANT}' AND bulk_operation_id = '{RUN}'"
        ),
    )
    .await;
    assert_eq!(
        locks_held(&conn).await,
        0,
        "and the release must have taken effect, not merely not errored"
    );
}

/// **The bulk lock takes effect only on entry to `committing`**, on the engine
/// where the arm's loss would ship. `awaiting_approval` is the state that
/// matters: `inst-bs-approval` says rows are *not* locked while a run awaits its
/// approval, so a lock taken there bounces interactive supersessions on keys §4
/// leaves free.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_lock_may_only_be_taken_while_its_run_commits() {
    for state in [
        "validating",
        "awaiting_approval",
        "completed",
        "completed_with_conflicts",
    ] {
        let conn = applied().await;
        must_succeed(&conn, &seed_price(TENANT)).await;
        must_succeed(&conn, &seed_run(RUN, TENANT)).await;
        // The two terminal states are only reachable through `committing`, and a
        // run must not keep its lock across that edge either.
        if matches!(state, "completed" | "completed_with_conflicts") {
            must_succeed(&conn, &advance(RUN, "committing")).await;
        }
        if state != "validating" {
            must_succeed(&conn, &advance(RUN, state)).await;
        }
        must_be_rejected(
            &conn,
            &take_lock(RUN, TENANT),
            "only on entry to committing",
        )
        .await;
    }
}

/// The lock's tenant is its run's tenant. The operation key covers the run alone,
/// so without this arm one tenant's run could mark another tenant's row — and the
/// concurrent-edit check, scoped to that second tenant, would refuse an edit
/// naming an operation the tenant cannot see.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_run_may_not_lock_another_tenants_row() {
    let conn = applied().await;
    // The price row belongs to the other tenant, so its key is satisfied and this
    // arm is what answers rather than the key.
    must_succeed(&conn, &seed_price(OTHER_TENANT)).await;
    must_succeed(&conn, &seed_run(RUN, TENANT)).await;
    must_succeed(&conn, &advance(RUN, "committing")).await;
    must_be_rejected(
        &conn,
        &take_lock(RUN, OTHER_TENANT),
        "belongs to another tenant",
    )
    .await;
}

/// One lock per row: the key **is** the mutual exclusion, so a second run cannot
/// take a row the first is holding.
#[tokio::test]
#[ignore = "requires Docker"]
async fn two_runs_cannot_hold_one_row() {
    let conn = committing_run().await;
    must_succeed(&conn, &take_lock(RUN, TENANT)).await;
    must_succeed(&conn, &seed_run(OTHER_RUN, TENANT)).await;
    must_succeed(&conn, &advance(OTHER_RUN, "committing")).await;
    must_be_rejected(
        &conn,
        &take_lock(OTHER_RUN, TENANT),
        "pricing_bulk_row_lock_pkey",
    )
    .await;
}

/// A lock is taken or released, never edited: an in-place hand-off would tell the
/// interactive editor that an operation which never took the row holds it, and
/// leave the release path unable to find what it owns.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_lock_is_never_edited() {
    let conn = committing_run().await;
    must_succeed(&conn, &take_lock(RUN, TENANT)).await;
    must_succeed(&conn, &seed_run(OTHER_RUN, TENANT)).await;
    must_succeed(&conn, &advance(OTHER_RUN, "committing")).await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE bss.pricing_bulk_row_lock SET bulk_operation_id = '{OTHER_RUN}' \
             WHERE tenant_id = '{TENANT}' AND price_id = '{PRICE}'"
        ),
        "never edited",
    )
    .await;
}

/// **A lock is releasable whatever its run's state, and that is the rule rather
/// than a gap.**
///
/// Both sibling tables of this slice refuse `DELETE` outright and copying that
/// here would be precisely wrong: a crashed run is terminal *and* still holding
/// rows, which is exactly when release matters most. D-37 puts release on lease
/// takeover and on the operator's abort, and a crashed import must never freeze
/// interactive authoring indefinitely. The trigger is bound to `INSERT OR UPDATE`
/// and not to `DELETE`, so the absence is structural rather than a fall-through.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_lock_is_releasable_whatever_state_its_run_reached() {
    for state in ["committing", "completed", "completed_with_conflicts"] {
        let conn = committing_run().await;
        must_succeed(&conn, &take_lock(RUN, TENANT)).await;
        if state != "committing" {
            must_succeed(&conn, &advance(RUN, state)).await;
        }
        must_succeed(
            &conn,
            &format!(
                "DELETE FROM bss.pricing_bulk_row_lock \
                 WHERE tenant_id = '{TENANT}' AND price_id = '{PRICE}'"
            ),
        )
        .await;
        assert_eq!(
            locks_held(&conn).await,
            0,
            "the release must have taken effect under a run in {state}, not \
             merely not errored"
        );
    }
}

/// Both keys, each by **name**, and each asserted by removing only its own
/// referent.
///
/// The run key is observable because both `INSERT` arms defer when the run is
/// absent: a `BEFORE` trigger answers ahead of a key on this engine, so an arm
/// that raised there would shadow the key and report a fault the caller does not
/// have.
#[tokio::test]
#[ignore = "requires Docker"]
async fn both_keys_name_a_row_that_exists() {
    let conn = applied().await;
    must_succeed(&conn, &seed_price(TENANT)).await;
    must_be_rejected(
        &conn,
        &take_lock(OTHER_RUN, TENANT),
        "fk_pricing_bulk_row_lock_operation",
    )
    .await;

    let conn = applied().await;
    must_succeed(&conn, &seed_run(RUN, TENANT)).await;
    must_succeed(&conn, &advance(RUN, "committing")).await;
    must_be_rejected(
        &conn,
        &take_lock(RUN, TENANT),
        "fk_pricing_bulk_row_lock_price",
    )
    .await;

    let conn = committing_run().await;
    must_succeed(&conn, &take_lock(RUN, TENANT)).await;
}
