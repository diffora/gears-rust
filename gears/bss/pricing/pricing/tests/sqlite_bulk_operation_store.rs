//! What the **schema** refuses of a bulk operation — Slice 12 §4's state
//! machine, driven as raw SQL because what is under test is the store's own
//! guarantee and not any repository's use of it.
//!
//! §4 gives six states and five edges. A caller that walked a sixth edge would
//! re-drive a commit whose rows are already applied, or strand a record in a
//! state nothing can leave, and neither is a mistake a caller should be trusted
//! not to make: the run's report is an operator-facing record of money that
//! moved.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sea_orm::DatabaseConnection;

mod common;

use common::{exec, migrated_db, must_succeed};

const OP: &str = "11111111-1111-1111-1111-111111111111";
const TENANT: &str = "22222222-2222-2222-2222-222222222222";
const ACTOR: &str = "33333333-3333-3333-3333-333333333333";

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

/// A run of `kind` parked in `validating`, the initial state.
fn seed(kind: &str) -> String {
    format!(
        "INSERT INTO pricing_bulk_operation \
         (operation_id, tenant_id, kind, state, client_key, report, submitted_by, submitted_at) \
         VALUES ('{OP}', '{TENANT}', '{kind}', 'validating', 'ck-1', '{{}}', '{ACTOR}', \
         '2026-08-08T00:00:00Z')"
    )
}

fn move_to(state: &str) -> String {
    let completed = matches!(
        state,
        "validation_failed" | "completed" | "completed_with_conflicts"
    )
    .then_some("'2026-08-08T01:00:00Z'")
    .unwrap_or("NULL");
    format!(
        "UPDATE pricing_bulk_operation SET state = '{state}', completed_at = {completed} \
         WHERE operation_id = '{OP}'"
    )
}

/// Every edge §4 draws is walkable, and this is the case that would catch a
/// guard written too tight — a state machine that refuses a legal move is as
/// broken as one that admits an illegal one, and only one of the two is loud.
#[tokio::test]
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
        let conn = migrated_db().await;
        must_succeed(&conn, &seed(kind)).await;
        for state in path {
            must_succeed(&conn, &move_to(state)).await;
        }
    }
}

/// **An import can never park awaiting approval** (D-137): draft-plane authoring
/// is never material, so the record would be waiting for a decision nothing can
/// produce. The one edge of §4 whose violation strands a run forever, and the
/// reason it is a `CHECK` rather than a caller's convention.
#[tokio::test]
async fn an_import_cannot_await_an_approval_that_can_never_come() {
    let conn = migrated_db().await;
    must_succeed(&conn, &seed("import")).await;
    must_be_rejected(&conn, &move_to("awaiting_approval"), "CHECK").await;

    // And the same state is legal for the kind that can actually be material.
    let conn = migrated_db().await;
    must_succeed(&conn, &seed("repricing")).await;
    must_succeed(&conn, &move_to("awaiting_approval")).await;
}

/// The three terminal states are terminal: a record that left one would re-drive
/// a commit whose rows are already applied.
#[tokio::test]
async fn a_terminal_run_never_moves_again() {
    for terminal in ["completed", "completed_with_conflicts"] {
        let conn = migrated_db().await;
        must_succeed(&conn, &seed("repricing")).await;
        must_succeed(&conn, &move_to("committing")).await;
        must_succeed(&conn, &move_to(terminal)).await;
        must_be_rejected(&conn, &move_to("committing"), "not an edge").await;
    }

    let conn = migrated_db().await;
    must_succeed(&conn, &seed("import")).await;
    must_succeed(&conn, &move_to("validation_failed")).await;
    must_be_rejected(&conn, &move_to("committing"), "not an edge").await;
}

/// Skipping the machine is refused, not merely discouraged: `validating`
/// straight to `completed` is a run that reports outcomes for rows it never
/// committed.
#[tokio::test]
async fn a_state_may_not_be_skipped() {
    let conn = migrated_db().await;
    must_succeed(&conn, &seed("repricing")).await;
    must_be_rejected(&conn, &move_to("completed"), "not an edge").await;
}

/// Identity and provenance are frozen; only the run's progress moves.
#[tokio::test]
async fn a_runs_identity_and_provenance_are_frozen() {
    let conn = migrated_db().await;
    must_succeed(&conn, &seed("import")).await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_bulk_operation SET kind = 'repricing' WHERE operation_id = '{OP}'"
        ),
        "frozen",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_bulk_operation SET client_key = 'ck-2' WHERE operation_id = '{OP}'"
        ),
        "frozen",
    )
    .await;
}

/// A run is a record, not a draft: `DELETE` is refused in every state.
#[tokio::test]
async fn a_run_is_never_deleted() {
    let conn = migrated_db().await;
    must_succeed(&conn, &seed("import")).await;
    must_be_rejected(
        &conn,
        &format!("DELETE FROM pricing_bulk_operation WHERE operation_id = '{OP}'"),
        "not permitted",
    )
    .await;
}

/// O4: one operation per client key per tenant.
#[tokio::test]
async fn a_client_key_is_idempotent_within_a_tenant() {
    let conn = migrated_db().await;
    must_succeed(&conn, &seed("import")).await;
    let second = seed("import").replace(OP, "44444444-4444-4444-4444-444444444444");
    must_be_rejected(&conn, &second, "UNIQUE").await;
}

/// **A run is born `validating` and in no other state** — §4's initial state,
/// enforced at `INSERT` rather than assumed of the caller.
///
/// Without this arm the whole machine is a rule about `UPDATE` with a row free to
/// be born past it: born `committing` skips the approval gate transitions 2 and 3
/// exist to impose, and born `completed` reports outcomes for rows it never
/// committed — the very move `a_terminal_run_never_moves_again` proves is refused
/// as an update. `m20260802_000015` carries the same arm for the same reason.
///
/// Found by review; every case above seeded `validating` and so could not see it.
#[tokio::test]
async fn a_run_cannot_be_born_past_the_state_machine() {
    for state in [
        "committing",
        "awaiting_approval",
        "completed",
        "completed_with_conflicts",
        "validation_failed",
    ] {
        let conn = migrated_db().await;
        let born = seed("repricing").replace("'validating'", &format!("'{state}'"));
        must_be_rejected(&conn, &born, "born validating").await;
    }
}
