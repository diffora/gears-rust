//! Slice 12's bulk-operation state machine **on the engine that runs in
//! production** (`design/12-operator-efficiency.md` §4, §6).
//!
//! `sqlite_bulk_operation_store` proves the same rules against the mirror. This
//! suite exists because that is not the same thing: the two arms are written
//! separately — one PL/pgSQL function against four `RAISE(ABORT, …)` triggers —
//! and the `SQLite` side is additionally covered by a trigger-**body** digest
//! census that Postgres has no equivalent of. Until this file, dropping a
//! disjunct from the PL/pgSQL edge list kept every gate green while production
//! refused every conflicted run.
//!
//! Run with:
//! `cargo test -p bss-pricing --test postgres_schema_bulk_operation -- --ignored`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

const OP: &str = "11111111-1111-1111-1111-111111111111";
const TENANT: &str = "22222222-2222-2222-2222-222222222222";
const ACTOR: &str = "33333333-3333-3333-3333-333333333333";

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

fn seed(kind: &str, state: &str) -> String {
    format!(
        "INSERT INTO bss.pricing_bulk_operation \
         (operation_id, tenant_id, kind, state, client_key, report, submitted_by, submitted_at) \
         VALUES ('{OP}', '{TENANT}', '{kind}', '{state}', 'ck-1', '{{}}'::jsonb, '{ACTOR}', \
         now())"
    )
}

fn move_to(state: &str) -> String {
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
         WHERE operation_id = '{OP}'"
    )
}

/// Every edge §4 draws is walkable on Postgres too — including
/// `committing → completed_with_conflicts`, which carries both `inst-bs-done`'s
/// conflict outcome and `inst-bs-abort`. A guard written too tight here would
/// leave every conflicted run unclosable in production and nothing else would
/// say so.
#[tokio::test]
#[ignore = "requires Docker"]
async fn every_sanctioned_edge_is_walkable() {
    for (kind, path) in [
        ("import", vec!["committing", "completed"]),
        (
            "repricing",
            vec![
                "awaiting_approval",
                "committing",
                "completed_with_conflicts",
            ],
        ),
        ("import", vec!["validation_failed"]),
    ] {
        // A fresh database per path: the run is undeletable by design, so the
        // three paths cannot share one.
        let conn = applied().await;
        must_succeed(&conn, &seed(kind, "validating")).await;
        for state in path {
            must_succeed(&conn, &move_to(state)).await;
        }
    }
}

/// A run is born `validating` and in no other state — the arm that was missing
/// for one commit, here on the engine where its absence would have shipped.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_run_cannot_be_born_past_the_state_machine() {
    let conn = applied().await;
    for state in [
        "committing",
        "awaiting_approval",
        "completed",
        "completed_with_conflicts",
        "validation_failed",
    ] {
        must_be_rejected(&conn, &seed("repricing", state), "born validating").await;
    }
}

/// D-137: an import can never park awaiting an approval nothing can grant.
#[tokio::test]
#[ignore = "requires Docker"]
async fn an_import_cannot_await_an_approval_that_can_never_come() {
    let conn = applied().await;
    must_succeed(&conn, &seed("import", "validating")).await;
    must_be_rejected(
        &conn,
        &move_to("awaiting_approval"),
        "chk_pricing_bulk_operation_import_never_awaits",
    )
    .await;
}

/// The three terminal states are terminal, and a state may not be skipped.
#[tokio::test]
#[ignore = "requires Docker"]
async fn the_machine_admits_no_edge_section_four_does_not_draw() {
    let conn = applied().await;
    must_succeed(&conn, &seed("repricing", "validating")).await;
    // Skipping straight to a terminal state.
    must_be_rejected(&conn, &move_to("completed"), "not an edge").await;
    // And a terminal record never moves again.
    must_succeed(&conn, &move_to("committing")).await;
    must_succeed(&conn, &move_to("completed")).await;
    must_be_rejected(&conn, &move_to("committing"), "not an edge").await;
}

/// Identity and provenance are frozen; `DELETE` is refused in every state.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_run_is_frozen_and_undeletable() {
    let conn = applied().await;
    must_succeed(&conn, &seed("import", "validating")).await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE bss.pricing_bulk_operation SET kind = 'repricing' WHERE operation_id = '{OP}'"
        ),
        "is frozen",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("DELETE FROM bss.pricing_bulk_operation WHERE operation_id = '{OP}'"),
        "not permitted",
    )
    .await;
}
