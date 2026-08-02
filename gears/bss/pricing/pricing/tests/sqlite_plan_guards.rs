//! The physical guards on `pricing_plan`, proven against a real database: the
//! append-only whitelist, the two partial `UNIQUE` indexes, and the
//! `lifecycle_state` CHECK.
//!
//! They are one suite because one rule rests on all three. A `revision` number
//! is an **identity** — minted once for a plan and never again (D-145) — and
//! the mechanism that keeps it true is a discarded draft becoming an
//! `abandoned` tombstone instead of disappearing. That needs the CHECK to admit
//! the token, the trigger to keep the tombstone frozen and undeletable, and
//! **both** partial predicates to exclude it, so a plan can accumulate
//! tombstones while still holding exactly one open draft and one current
//! revision. A repository-level suite cannot see any of it: these are properties
//! of the schema, and the engine is not the only thing that can reach the table.
//!
//! Postgres carries the whitelist as one PL/pgSQL trigger and `SQLite` mirrors
//! it as three `RAISE(ABORT, ...)` triggers (see the migration's module doc), so
//! the guards are exercisable without Docker.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sea_orm::DatabaseConnection;

mod common;

use common::{exec, migrated_db, must_succeed, scalar};

const TENANT: &str = "11111111-1111-1111-1111-111111111111";
const PLAN: &str = "22222222-2222-2222-2222-222222222222";
const ACTOR: &str = "44444444-4444-4444-4444-444444444444";
const AUTHORED: &str = "2026-08-02 10:00:00 +00:00";

/// Reject, **and** for the stated reason.
///
/// The fragment is not decoration: `pricing_plan` carries three whitelist
/// triggers, three `CHECK` constraints and two partial `UNIQUE` indexes, and
/// every one of those names contains the string `pricing_plan` — as does the
/// column list `SQLite` reports for a unique violation. A test that accepted any
/// error naming the table would pass with the guard it means to prove switched
/// off, refused instead by a constraint it never intended to trip.
async fn must_be_rejected(conn: &DatabaseConnection, sql: &str, because: &str) {
    let err = exec(conn, sql)
        .await
        .err()
        .unwrap_or_else(|| panic!("the guard must reject: {sql}"));
    let message = err.to_string();
    assert!(
        message.contains("pricing_plan"),
        "the rejection must name the guard it came from, got: {message}"
    );
    assert!(
        message.contains(because),
        "the rejection must be the one under test (`{because}`), got: {message}"
    );
}

/// Rejected by the append-only whitelist — by **either** of its two arms.
///
/// On a tombstone the arms overlap by construction: `abandoned` is not
/// `published`, so the flip arm refuses every UPDATE of the row, and a statement
/// that also moves a content column trips the frozen arm as well. Which one
/// answers is a trigger-ordering detail `SQLite` does not define and Postgres
/// settles the other way — its single function checks the frozen columns first.
/// Pinning one message would pin the mirror's incidental order instead of the
/// rule both backends enforce, which is that nothing about this row moves.
async fn must_be_frozen(conn: &DatabaseConnection, sql: &str) {
    let err = exec(conn, sql)
        .await
        .err()
        .unwrap_or_else(|| panic!("the guard must reject: {sql}"));
    let message = err.to_string();
    assert!(
        message.contains("is frozen") || message.contains("not a sanctioned flip"),
        "the rejection must come from the append-only whitelist, got: {message}"
    );
}

/// One revision row of `PLAN`, in whatever state the case needs.
fn insert(plan: &str, revision: u32, state: &str) -> String {
    format!(
        "INSERT INTO pricing_plan (
            plan_id, revision, tenant_id, plan_tier, lifecycle_state,
            created_by, created_at_utc)
         VALUES ('{plan}', {revision}, '{TENANT}', 'gold', '{state}',
            '{ACTOR}', '{AUTHORED}')"
    )
}

async fn state_of(conn: &DatabaseConnection, revision: u32) -> String {
    scalar(
        conn,
        &format!(
            "SELECT lifecycle_state AS v FROM pricing_plan \
             WHERE plan_id = '{PLAN}' AND revision = {revision}"
        ),
    )
    .await
}

#[tokio::test]
async fn a_discarded_draft_flips_to_abandoned_and_the_tombstone_freezes() {
    let conn = migrated_db().await;
    must_succeed(&conn, &insert(PLAN, 0, "draft")).await;

    // The discard itself. It rides the draft plane, which is unguarded because
    // that is where content is supposed to move.
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_plan SET lifecycle_state = 'abandoned', \
             row_version = row_version + 1 WHERE plan_id = '{PLAN}' AND revision = 0"
        ),
    )
    .await;
    assert_eq!(state_of(&conn, 0).await, "abandoned");

    // From here the row is a tombstone, and the number it holds may never be
    // attached to a different shape: its content is frozen exactly as a
    // published revision's is, and so is the entity tag that names it.
    must_be_frozen(
        &conn,
        &format!("UPDATE pricing_plan SET plan_tier = 'silver' WHERE plan_id = '{PLAN}'"),
    )
    .await;
    must_be_frozen(
        &conn,
        &format!("UPDATE pricing_plan SET row_version = row_version + 1 WHERE plan_id = '{PLAN}'"),
    )
    .await;

    // And it is terminal. An edge out of it would put a live revision back at a
    // number already handed out as a durable name — which is the state the
    // tombstone exists to make unreachable.
    for state in ["draft", "published", "superseded", "retired"] {
        must_be_rejected(
            &conn,
            &format!(
                "UPDATE pricing_plan SET lifecycle_state = '{state}' WHERE plan_id = '{PLAN}'"
            ),
            "not a sanctioned flip",
        )
        .await;
    }
    assert_eq!(
        state_of(&conn, 0).await,
        "abandoned",
        "no rejected move may have landed"
    );
}

#[tokio::test]
async fn no_revision_row_is_ever_deleted_not_even_a_draft() {
    let conn = migrated_db().await;
    must_succeed(&conn, &insert(PLAN, 0, "published")).await;
    must_succeed(&conn, &insert(PLAN, 1, "draft")).await;

    // The draft is the case that matters. Deleting one was the old discard, and
    // it returned `max(revision)` to its previous value: the next opened draft
    // minted the same number, `(plan_id, revision)` named two rows over the
    // plan's life, and a stale entity tag passed its precondition against the
    // wrong one. There is no verb here that frees a number.
    must_be_rejected(
        &conn,
        &format!("DELETE FROM pricing_plan WHERE plan_id = '{PLAN}' AND revision = 1"),
        "DELETE of a revision is not permitted",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("DELETE FROM pricing_plan WHERE plan_id = '{PLAN}' AND revision = 0"),
        "DELETE of a revision is not permitted",
    )
    .await;

    let rows = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_plan",
    )
    .await;
    assert_eq!(rows, "2", "both revisions survive");
}

#[tokio::test]
async fn a_plan_holds_many_tombstones_beside_one_draft_and_one_current() {
    let conn = migrated_db().await;

    // Two discarded revisions, the plan's current one, and the draft that
    // replaced them — the ordinary shape of a plan somebody has edited. It is
    // legal only because `abandoned` falls outside **both** partial predicates:
    // outside `WHERE lifecycle_state = 'draft'`, so the replacement draft opens
    // immediately, and outside `IN ('published','retired')`, so the current
    // revision is untouched.
    must_succeed(&conn, &insert(PLAN, 0, "published")).await;
    must_succeed(&conn, &insert(PLAN, 1, "abandoned")).await;
    must_succeed(&conn, &insert(PLAN, 2, "abandoned")).await;
    must_succeed(&conn, &insert(PLAN, 3, "draft")).await;

    // Both indexes are still live over the states they do cover, so the
    // exclusion above widened nothing else.
    must_be_rejected(
        &conn,
        &insert(PLAN, 4, "draft"),
        "UNIQUE constraint failed: pricing_plan.plan_id",
    )
    .await;
    must_be_rejected(
        &conn,
        &insert(PLAN, 4, "published"),
        "UNIQUE constraint failed: pricing_plan.plan_id",
    )
    .await;
    // Including the state D-128 widened the current-revision predicate to hold.
    must_be_rejected(
        &conn,
        &insert(PLAN, 4, "retired"),
        "UNIQUE constraint failed: pricing_plan.plan_id",
    )
    .await;

    // A third tombstone, on the other hand, is always admissible — a plan may
    // discard as many drafts as its author cares to.
    must_succeed(&conn, &insert(PLAN, 4, "abandoned")).await;

    let tombstones = scalar(
        &conn,
        &format!(
            "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_plan \
             WHERE plan_id = '{PLAN}' AND lifecycle_state = 'abandoned'"
        ),
    )
    .await;
    assert_eq!(tombstones, "3");
}

#[tokio::test]
async fn the_lifecycle_check_admits_the_five_states_and_nothing_else() {
    let conn = migrated_db().await;

    // One plan per token, because two of these five collide on a single plan by
    // design and this case is about the CHECK, not about the indexes.
    for (i, state) in ["draft", "abandoned", "published", "superseded", "retired"]
        .into_iter()
        .enumerate()
    {
        let plan = format!("2222222{i}-2222-2222-2222-222222222222");
        must_succeed(&conn, &insert(&plan, 0, state)).await;
    }

    // The token the state has to be spelled with is the one the domain machine
    // renders. A near-miss stored here would read back as a corrupt row, and the
    // revision would be unreachable through every typed path in the gear.
    must_be_rejected(
        &conn,
        &insert(PLAN, 0, "discarded"),
        "chk_pricing_plan_lifecycle_state",
    )
    .await;
}
