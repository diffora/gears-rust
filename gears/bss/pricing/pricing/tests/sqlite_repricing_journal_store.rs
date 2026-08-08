//! What the **schema** refuses of a repricing journal row — Slice 12's
//! idempotency spine (`design/12-operator-efficiency.md` §6, `inst-mr-journal`,
//! `inst-mp-journal`, O3), driven as raw SQL because what is under test is the
//! store's own guarantee and not any repository's use of it.
//!
//! The journal is what makes a crashed run safe to re-drive, so every rule here
//! is about one of the two ways a re-drive can go wrong: applying a row twice,
//! or skipping a row that was never applied. The second is the quiet one — a
//! skipped row leaves a completed run, an untouched price, and no state anywhere
//! that disagrees.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sea_orm::DatabaseConnection;

mod common;

use common::{exec, migrated_db, must_succeed};

const RUN: &str = "11111111-1111-1111-1111-111111111111";
const TENANT: &str = "22222222-2222-2222-2222-222222222222";
const ACTOR: &str = "33333333-3333-3333-3333-333333333333";
const PRICE: &str = "44444444-4444-4444-4444-444444444444";
const SUCCESSOR: &str = "55555555-5555-5555-5555-555555555555";

async fn must_be_rejected(conn: &DatabaseConnection, sql: &str, because: &str) {
    let err = exec(conn, sql)
        .await
        .expect_err(&format!("this statement must be rejected: {sql}"));
    let message = err.to_string();
    assert!(
        message.contains(because),
        "the rejection must be the one under test (`{because}`), got: {message}"
    );
}

/// The run the journal rows belong to, in its own initial state.
fn seed_run() -> String {
    format!(
        "INSERT INTO pricing_bulk_operation \
         (operation_id, tenant_id, kind, state, client_key, report, submitted_by, submitted_at) \
         VALUES ('{RUN}', '{TENANT}', 'repricing', 'validating', 'ck-1', '{{}}', '{ACTOR}', \
         '2026-08-09T00:00:00Z')"
    )
}

/// One selected row, `pending` — the only state a journal row may be born in.
fn seed_row() -> String {
    format!(
        "INSERT INTO pricing_repricing_journal (run_id, price_id, tenant_id, state) \
         VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'pending')"
    )
}

/// A journalled outcome, with the columns that state owes.
fn decide(state: &str) -> String {
    let columns = match state {
        "applied" => {
            format!("applied_price_id = '{SUCCESSOR}', applied_at = '2026-08-09T01:00:00Z'")
        }
        "failed" => "failure_reason = 'the plan-level pass refused this plan'".to_owned(),
        _ => "applied_price_id = NULL, applied_at = NULL, failure_reason = NULL".to_owned(),
    };
    format!(
        "UPDATE pricing_repricing_journal SET state = '{state}', {columns} \
         WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
    )
}

async fn journalled(conn: &DatabaseConnection) {
    must_succeed(conn, &seed_run()).await;
    must_succeed(conn, &seed_row()).await;
}

/// Both edges out of `pending` are walkable — the whitelist half, and the case
/// that would catch a guard written too tight. A journal that refused `failed`
/// would leave the run's own state machine unable to reach either terminal state.
#[tokio::test]
async fn both_sanctioned_outcomes_are_reachable() {
    for state in ["applied", "failed"] {
        let conn = migrated_db().await;
        journalled(&conn).await;
        must_succeed(&conn, &decide(state)).await;
    }
}

/// **A journal row is born `pending` and in no other state.**
///
/// This is the arm whose absence is silent. `m20260802_000047` shipped without
/// its `INSERT` arm for one commit and a run could be born past its own machine;
/// here the same hole means a row born `applied` is a row the re-drive **skips** —
/// the operator sees a completed run, the price was never repriced, and no state
/// anywhere disagrees. Every other case in this file seeds `pending`, so without
/// this one the whole green suite could not see it.
#[tokio::test]
async fn a_journal_row_cannot_be_born_decided() {
    for born in [
        format!(
            "INSERT INTO pricing_repricing_journal \
             (run_id, price_id, tenant_id, state, applied_price_id, applied_at) \
             VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'applied', '{SUCCESSOR}', \
             '2026-08-09T01:00:00Z')"
        ),
        format!(
            "INSERT INTO pricing_repricing_journal \
             (run_id, price_id, tenant_id, state, failure_reason) \
             VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'failed', 'never actually tried')"
        ),
    ] {
        let conn = migrated_db().await;
        must_succeed(&conn, &seed_run()).await;
        must_be_rejected(&conn, &born, "born pending").await;
    }
}

/// A decided row never moves again — and that one guard also freezes the outcome
/// columns, so an `applied` row cannot later be made to name a different
/// successor than the one that actually committed.
#[tokio::test]
async fn a_decided_row_never_moves_again() {
    for (first, second) in [("applied", "failed"), ("failed", "applied")] {
        let conn = migrated_db().await;
        journalled(&conn).await;
        must_succeed(&conn, &decide(first)).await;
        must_be_rejected(&conn, &decide(second), "never moves again").await;
    }

    // Back to `pending` is the re-drive's own nightmare: a second apply against a
    // row that already has a successor.
    let conn = migrated_db().await;
    journalled(&conn).await;
    must_succeed(&conn, &decide("applied")).await;
    must_be_rejected(&conn, &decide("pending"), "never moves again").await;

    // And the successor cannot be rewritten in place while the state stands.
    let conn = migrated_db().await;
    journalled(&conn).await;
    must_succeed(&conn, &decide("applied")).await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_repricing_journal SET applied_price_id = '{TENANT}' \
             WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "never moves again",
    )
    .await;
}

/// The key and its tenant are what the row is *about*, so they are frozen.
#[tokio::test]
async fn the_journal_key_is_frozen() {
    let conn = migrated_db().await;
    journalled(&conn).await;
    for column in ["run_id", "price_id", "tenant_id"] {
        must_be_rejected(
            &conn,
            &format!(
                "UPDATE pricing_repricing_journal SET {column} = '{SUCCESSOR}' \
                 WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
            ),
            "key is frozen",
        )
        .await;
    }
}

/// `DELETE` is refused: a missing journal row is a row the re-drive re-applies,
/// which is the one thing O3 promises cannot happen.
#[tokio::test]
async fn a_journal_row_is_never_deleted() {
    let conn = migrated_db().await;
    journalled(&conn).await;
    must_be_rejected(
        &conn,
        &format!(
            "DELETE FROM pricing_repricing_journal WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "not permitted",
    )
    .await;
}

/// The outcome columns and the state agree in both directions — a `pending` row
/// carrying a successor would be re-driven into a second one, and an `applied`
/// row without one records an apply nothing can point at.
#[tokio::test]
async fn the_outcome_columns_agree_with_the_state() {
    let conn = migrated_db().await;
    must_succeed(&conn, &seed_run()).await;

    for born in [
        // Pending, but already naming a successor.
        format!(
            "INSERT INTO pricing_repricing_journal \
             (run_id, price_id, tenant_id, state, applied_price_id, applied_at) \
             VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'pending', '{SUCCESSOR}', \
             '2026-08-09T01:00:00Z')"
        ),
        // Pending, naming a successor, with **no instant** — the half-set shape
        // the pair-wise spelling of this `CHECK` would have admitted, and the
        // one the re-drive would apply a second time.
        format!(
            "INSERT INTO pricing_repricing_journal \
             (run_id, price_id, tenant_id, state, applied_price_id) \
             VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'pending', '{SUCCESSOR}')"
        ),
        // Pending, but already carrying a reason it failed.
        format!(
            "INSERT INTO pricing_repricing_journal \
             (run_id, price_id, tenant_id, state, failure_reason) \
             VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'pending', 'refused before it began')"
        ),
    ] {
        must_be_rejected(&conn, &born, "CHECK").await;
    }

    // The same disagreements arriving as an update of a legitimately pending row.
    let conn = migrated_db().await;
    journalled(&conn).await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_repricing_journal SET state = 'applied' \
             WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "CHECK",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_repricing_journal SET state = 'failed' \
             WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "CHECK",
    )
    .await;
    // Applied, with a successor but no instant — the other half-set shape.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_repricing_journal \
             SET state = 'applied', applied_price_id = '{SUCCESSOR}' \
             WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "CHECK",
    )
    .await;
    // And the vocabulary is closed. This one is asserted **only** as an update:
    // at `INSERT` the born-pending trigger is strictly narrower than the `CHECK`
    // and answers first, so the constraint is shadowed there and a case
    // expecting it would be reading the trigger's refusal instead — the same
    // ordering `postgres_schema_composite_meter` records for its parent key.
    // `not-attempted` is the word `inst-bs-abort` uses for an aborted run's
    // uncommitted rows, and it is deliberately not a state this table holds.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_repricing_journal SET state = 'not-attempted' \
             WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "CHECK",
    )
    .await;
}

/// `inst-mp-standard`: an applied row is a **new** immutable row through the
/// Foundation path, because bulk never mutates in place. A successor wearing the
/// selected row's own id would be that mutation, journalled as a supersession.
#[tokio::test]
async fn a_successor_may_not_wear_the_selected_rows_own_id() {
    let conn = migrated_db().await;
    journalled(&conn).await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_repricing_journal \
             SET state = 'applied', applied_price_id = '{PRICE}', \
                 applied_at = '2026-08-09T01:00:00Z' \
             WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "CHECK",
    )
    .await;
}

/// A journal row belongs to a run, and the foreign key says so.
///
/// `PRAGMA foreign_keys` is off by default on a bare `SQLite` connection and on
/// in production (`toolkit_db` turns it on), so the pragma is set here for the
/// same reason `sqlite_bundle_store` sets it: without it this case would assert
/// a key the mirror never enforces and would pass whether or not it existed.
#[tokio::test]
async fn a_journal_row_belongs_to_a_run() {
    let conn = migrated_db().await;
    must_succeed(&conn, "PRAGMA foreign_keys = ON").await;
    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO pricing_repricing_journal (run_id, price_id, tenant_id, state) \
             VALUES ('{SUCCESSOR}', '{PRICE}', '{TENANT}', 'pending')"
        ),
        "FOREIGN KEY",
    )
    .await;

    // And the same row lands once its run exists, so the refusal above is the
    // key and not something else about the statement.
    must_succeed(&conn, &seed_run()).await;
    must_succeed(&conn, &seed_row()).await;
}
