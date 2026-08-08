//! Slice 12's repricing journal **on the engine that runs in production**
//! (`design/12-operator-efficiency.md` §6, `inst-mr-journal`, O3).
//!
//! `sqlite_repricing_journal_store` proves the same rules against the mirror,
//! and that is not the same thing: the two arms are written separately — one
//! PL/pgSQL function against four `RAISE(ABORT, …)` triggers — and only the
//! `SQLite` side carries a trigger-**body** digest census. D-260 paid this debt
//! for `pricing_bulk_operation` and `pricing_composite_meter` after finding that
//! a lost disjunct on the shipping engine was invisible to every gate; a new
//! table owes a behavioural Postgres suite from the start, not four roster lines.
//!
//! Run with:
//! `cargo test -p bss-pricing --test postgres_schema_repricing_journal -- --ignored`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

const RUN: &str = "11111111-1111-1111-1111-111111111111";
const TENANT: &str = "22222222-2222-2222-2222-222222222222";
const ACTOR: &str = "33333333-3333-3333-3333-333333333333";
const PRICE: &str = "44444444-4444-4444-4444-444444444444";
const SUCCESSOR: &str = "55555555-5555-5555-5555-555555555555";

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

fn seed_run() -> String {
    format!(
        "INSERT INTO bss.pricing_bulk_operation \
         (operation_id, tenant_id, kind, state, client_key, report, submitted_by, submitted_at) \
         VALUES ('{RUN}', '{TENANT}', 'repricing', 'validating', 'ck-1', '{{}}'::jsonb, \
         '{ACTOR}', now())"
    )
}

fn seed_row() -> String {
    format!(
        "INSERT INTO bss.pricing_repricing_journal (run_id, price_id, tenant_id, state) \
         VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'pending')"
    )
}

fn decide(state: &str) -> String {
    let columns = match state {
        "applied" => format!("applied_price_id = '{SUCCESSOR}', applied_at = now()"),
        "failed" => "failure_reason = 'the plan-level pass refused this plan'".to_owned(),
        _ => "applied_price_id = NULL, applied_at = NULL, failure_reason = NULL".to_owned(),
    };
    format!(
        "UPDATE bss.pricing_repricing_journal SET state = '{state}', {columns} \
         WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
    )
}

async fn journalled() -> DatabaseConnection {
    let conn = applied().await;
    must_succeed(&conn, &seed_run()).await;
    must_succeed(&conn, &seed_row()).await;
    conn
}

/// Both edges out of `pending` are walkable on Postgres too. A guard written too
/// tight here would leave a run unable to journal one of its two outcomes, and —
/// the journal being what the run's completion predicate reads — unable to finish.
#[tokio::test]
#[ignore = "requires Docker"]
async fn both_sanctioned_outcomes_are_reachable() {
    for state in ["applied", "failed"] {
        // A fresh database per outcome: the row is undeletable and final once
        // decided, so the two cannot share one.
        let conn = journalled().await;
        must_succeed(&conn, &decide(state)).await;
    }
}

/// A journal row is born `pending` and in no other state — the arm whose absence
/// is silent, here on the engine where that silence would ship. A row born
/// `applied` is a row the re-drive skips: a completed run, an untouched price,
/// and nothing anywhere that disagrees.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_journal_row_cannot_be_born_decided() {
    let conn = applied().await;
    must_succeed(&conn, &seed_run()).await;
    for born in [
        format!(
            "INSERT INTO bss.pricing_repricing_journal \
             (run_id, price_id, tenant_id, state, applied_price_id, applied_at) \
             VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'applied', '{SUCCESSOR}', now())"
        ),
        format!(
            "INSERT INTO bss.pricing_repricing_journal \
             (run_id, price_id, tenant_id, state, failure_reason) \
             VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'failed', 'never actually tried')"
        ),
    ] {
        must_be_rejected(&conn, &born, "born pending").await;
    }
}

/// A decided row never moves again, outcome columns included — so an `applied`
/// row cannot be made to name a successor other than the one that committed.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_decided_row_never_moves_again() {
    let conn = journalled().await;
    must_succeed(&conn, &decide("applied")).await;
    must_be_rejected(&conn, &decide("failed"), "never moves again").await;
    must_be_rejected(&conn, &decide("pending"), "never moves again").await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE bss.pricing_repricing_journal SET applied_price_id = '{TENANT}' \
             WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "never moves again",
    )
    .await;

    let conn = journalled().await;
    must_succeed(&conn, &decide("failed")).await;
    must_be_rejected(&conn, &decide("applied"), "never moves again").await;
}

/// The key and its tenant are frozen, and `DELETE` is refused: a missing journal
/// row is a row the re-drive re-applies.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_journal_row_is_keyed_frozen_and_undeletable() {
    let conn = journalled().await;
    for column in ["run_id", "price_id", "tenant_id"] {
        must_be_rejected(
            &conn,
            &format!(
                "UPDATE bss.pricing_repricing_journal SET {column} = '{SUCCESSOR}' \
                 WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
            ),
            "key is frozen",
        )
        .await;
    }
    must_be_rejected(
        &conn,
        &format!(
            "DELETE FROM bss.pricing_repricing_journal \
             WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "not permitted",
    )
    .await;
}

/// The outcome columns agree with the state in both directions, the vocabulary
/// is closed, and a successor may not wear the selected row's own id
/// (`inst-mp-standard`: bulk never mutates in place). Asserted by constraint
/// **name**, which is what a `CHECK` census cannot do and what tells a lost
/// constraint from a differently-worded one.
#[tokio::test]
#[ignore = "requires Docker"]
async fn the_outcome_columns_agree_with_the_state() {
    let conn = applied().await;
    must_succeed(&conn, &seed_run()).await;

    for (born, constraint) in [
        (
            format!(
                "INSERT INTO bss.pricing_repricing_journal \
                 (run_id, price_id, tenant_id, state, applied_price_id, applied_at) \
                 VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'pending', '{SUCCESSOR}', now())"
            ),
            "chk_pricing_repricing_journal_applied",
        ),
        (
            // Naming a successor with **no instant** — the half-set shape the
            // pair-wise spelling of this `CHECK` would have admitted, and the one
            // the re-drive would apply a second time.
            format!(
                "INSERT INTO bss.pricing_repricing_journal \
                 (run_id, price_id, tenant_id, state, applied_price_id) \
                 VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'pending', '{SUCCESSOR}')"
            ),
            "chk_pricing_repricing_journal_applied",
        ),
        (
            format!(
                "INSERT INTO bss.pricing_repricing_journal \
                 (run_id, price_id, tenant_id, state, failure_reason) \
                 VALUES ('{RUN}', '{PRICE}', '{TENANT}', 'pending', 'refused before it began')"
            ),
            "chk_pricing_repricing_journal_failed",
        ),
    ] {
        must_be_rejected(&conn, &born, constraint).await;
    }

    must_succeed(&conn, &seed_row()).await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE bss.pricing_repricing_journal \
             SET state = 'applied', applied_price_id = '{PRICE}', applied_at = now() \
             WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "chk_pricing_repricing_journal_successor_is_new",
    )
    .await;
    // The vocabulary is closed — asserted **only** as an update, because at
    // `INSERT` the born-pending arm is strictly narrower than this `CHECK` and
    // raises first, so a case expecting the constraint there would be reading the
    // trigger's refusal instead. `not-attempted` is the word `inst-bs-abort` uses
    // for an aborted run's uncommitted rows and is deliberately not a state this
    // table holds.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE bss.pricing_repricing_journal SET state = 'not-attempted' \
             WHERE run_id = '{RUN}' AND price_id = '{PRICE}'"
        ),
        "chk_pricing_repricing_journal_state",
    )
    .await;
}

/// A journal row belongs to a run, by key.
///
/// Observable here, unlike `pricing_composite_meter`'s parent key: this table's
/// `BEFORE INSERT` arm reads `NEW.state` alone, so a row with a good state and a
/// run that does not exist reaches the key rather than being shadowed by the
/// trigger.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_journal_row_belongs_to_a_run() {
    let conn = applied().await;
    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_repricing_journal (run_id, price_id, tenant_id, state) \
             VALUES ('{SUCCESSOR}', '{PRICE}', '{TENANT}', 'pending')"
        ),
        "fk_pricing_repricing_journal_run",
    )
    .await;

    must_succeed(&conn, &seed_run()).await;
    must_succeed(&conn, &seed_row()).await;
}
