//! The freeze ledger's guard, **executed on Postgres** — the engine that
//! serves it (`m20260901_000012`; P-D-60, P-D-67).
//!
//! # The hole this closes
//!
//! `SQLite` carries the ledger's rules as four `WHEN`-guarded triggers;
//! Postgres carries them as one `plpgsql` function with four sequential `IF`
//! arms. `migrations_tests` executes the `SQLite` arm. Until this file
//! nothing executed the Postgres one, and none of the four `RAISE` arms —
//! the delete refusal, the immutable key, the write-once `released_at`, the
//! six-edge list — had been reached on the engine that runs them. The
//! head-row guards had exactly this gap, and it is how a Phase 6 defect
//! reached the tree: correct on the mirror, inoperative on the engine.
//!
//! # Raw SQL, on purpose
//!
//! The claim under test is that the **database** refuses, whatever reaches
//! it. A probe through `infra::storage::repo` would pass with the trigger
//! dropped, because the repository would never form the forbidden statement.
//! Seeding is raw for the same reason: an `INSERT` is not guarded, so a row
//! can be placed in any state the edge list would otherwise take several
//! steps to reach.
//!
//! # Every case names the arm it expects
//!
//! The function returns on its first failing `IF`, so a refusal is only
//! evidence for the arm whose message it carries. Each assertion below reads
//! the message back, and the positive controls prove the trigger admits the
//! six edges it lists — a suite of refusals alone would pass against a
//! trigger that refuses everything.
//!
//! Ignored by default; it needs Docker. Run with
//! `cargo test -p cf-gears-bss-products --test postgres_freeze_ledger_guards -- --ignored`.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-freeze-ledger-tables:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

const TENANT: &str = "00000000-0000-0000-0000-00000000f4e2";
const VERSION: i64 = 7;

/// The ledger's four states.
const STATES: [&str; 4] = ["pending", "acked", "released", "not_frozen(forced)"];

/// P-D-60's six edges, as `(from, to)`.
const ADMITTED: [(&str, &str); 6] = [
    ("pending", "acked"),
    ("pending", "released"),
    ("pending", "not_frozen(forced)"),
    ("acked", "released"),
    ("not_frozen(forced)", "acked"),
    ("not_frozen(forced)", "released"),
];

/// Run one statement, expecting the guard to refuse it, and hand back the
/// engine's message. A statement that **succeeds** is the failure.
async fn refusal(conn: &DatabaseConnection, sql: &str) -> String {
    match conn
        .execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            sql.to_owned(),
        ))
        .await
    {
        Ok(_) => panic!("the guard admitted a write it must refuse:\n{sql}"),
        Err(e) => e.to_string(),
    }
}

/// Run one statement the guard must **admit**.
async fn admitted(conn: &DatabaseConnection, sql: &str) {
    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        sql.to_owned(),
    ))
    .await
    .unwrap_or_else(|e| panic!("the guard refused a write it must admit:\n{sql}\n{e}"));
}

/// The version every ledger row hangs from — the ack table's foreign key.
async fn seed_version(conn: &DatabaseConnection) {
    admitted(
        conn,
        &format!(
            "INSERT INTO bss.products_catalog_version
               (tenant_id, catalog_version_id, checksum, digest_version, published_at,
                participant_set_snapshot, freeze_state)
             VALUES ('{TENANT}', {VERSION}, 'c0ffee', 1, now(), '[\"pricing\"]', 'open')"
        ),
    )
    .await;
}

/// One ledger row for `participant` in `state`, shaped to satisfy the two
/// `CHECK`s: `acked` carries `acked_at`, `not_frozen(forced)` carries
/// `forced_at`, `ceremony_ref` and `released_at`.
async fn seed_row(conn: &DatabaseConnection, participant: &str, state: &str) {
    let (acked_at, released_at, forced_at, ceremony_ref) = match state {
        "acked" => ("now()", "NULL", "NULL", "NULL"),
        "not_frozen(forced)" => ("NULL", "now()", "now()", "gen_random_uuid()"),
        _ => ("NULL", "NULL", "NULL", "NULL"),
    };
    admitted(
        conn,
        &format!(
            "INSERT INTO bss.products_freeze_ack
               (tenant_id, catalog_version_id, participant, state, acked_at, released_at,
                forced_at, ceremony_ref)
             VALUES ('{TENANT}', {VERSION}, '{participant}', '{state}', {acked_at},
                {released_at}, {forced_at}, {ceremony_ref})"
        ),
    )
    .await;
}

/// The `UPDATE` that moves `participant` to `state`, shaped like
/// [`seed_row`] so a refusal is the trigger's and never a `CHECK`'s.
fn move_to(participant: &str, state: &str) -> String {
    let set = match state {
        "acked" => "state = 'acked', acked_at = now(), forced_at = NULL, ceremony_ref = NULL",
        "not_frozen(forced)" => {
            "state = 'not_frozen(forced)', forced_at = now(), ceremony_ref = gen_random_uuid(), \
             released_at = COALESCE(released_at, now())"
        }
        "released" => "state = 'released', forced_at = NULL, ceremony_ref = NULL",
        _ => "state = 'pending', forced_at = NULL, ceremony_ref = NULL",
    };
    format!(
        "UPDATE bss.products_freeze_ack SET {set}
         WHERE tenant_id = '{TENANT}' AND catalog_version_id = {VERSION}
           AND participant = '{participant}'"
    )
}

/// **A ledger row is never deleted while its version exists** (AC #44).
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_ledger_row_is_never_deleted() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed_version(&conn).await;
    seed_row(&conn, "pricing", "pending").await;

    let message = refusal(
        &conn,
        &format!(
            "DELETE FROM bss.products_freeze_ack WHERE tenant_id = '{TENANT}' \
             AND participant = 'pricing'"
        ),
    )
    .await;
    assert!(
        message.contains("never deleted"),
        "the delete arm answers by name: {message}"
    );
}

/// **The key columns are immutable**: a row cannot be re-pointed at another
/// version or participant, which would silently move an ack.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_key_columns_are_immutable() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed_version(&conn).await;
    seed_row(&conn, "pricing", "pending").await;

    let message = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_freeze_ack SET participant = 'billing' \
             WHERE tenant_id = '{TENANT}' AND participant = 'pricing'"
        ),
    )
    .await;
    assert!(
        message.contains("key columns are immutable"),
        "the key arm answers by name: {message}"
    );
}

/// **`released_at` is write-once** (P-D-67): stamped by force-completion, it
/// is never moved — while the state beside it may still move, which is the
/// recovered participant's ack.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn released_at_is_write_once_and_the_state_beside_it_still_moves() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed_version(&conn).await;
    seed_row(&conn, "pricing", "not_frozen(forced)").await;

    let message = refusal(
        &conn,
        &format!(
            "UPDATE bss.products_freeze_ack SET released_at = now() + interval '1 hour' \
             WHERE tenant_id = '{TENANT}' AND participant = 'pricing'"
        ),
    )
    .await;
    assert!(
        message.contains("released_at is write-once"),
        "the write-once arm answers by name: {message}"
    );

    // The recovered participant's ack: state moves, the stamp stays.
    admitted(&conn, &move_to("pricing", "acked")).await;
}

/// **The six admitted edges are admitted, and nothing else is.** Every
/// listed edge is driven as a positive control; every unlisted edge between
/// the four states is refused by the edge arm, by name.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn exactly_the_six_edges_are_admitted() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed_version(&conn).await;

    let mut n = 0;
    for from in STATES {
        for to in STATES {
            if from == to {
                continue;
            }
            n += 1;
            let participant = format!("p{n}");
            seed_row(&conn, &participant, from).await;
            let sql = move_to(&participant, to);
            if ADMITTED.contains(&(from, to)) {
                admitted(&conn, &sql).await;
            } else {
                let message = refusal(&conn, &sql).await;
                assert!(
                    message.contains("not one of the six admitted edges"),
                    "{from} -> {to} must be refused by the edge arm: {message}"
                );
            }
        }
    }
    assert_eq!(n, 12, "every ordered pair of distinct states was driven");
}
