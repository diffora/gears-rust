//! Slice 10's composite meter **on the engine that runs in production**
//! (`design/10-advanced-primitives.md` §6, D-256).
//!
//! `sqlite_migrations` proves the objects exist by name and digest on the
//! mirror; `postgres_migrations` proves they exist by name here. Neither runs
//! the guard. The two arms are written separately — one PL/pgSQL function
//! against three `RAISE(ABORT, …)` triggers — and only the `SQLite` side carries
//! a trigger-**body** digest census, so until this file a lost disjunct in the
//! PL/pgSQL append-only function was invisible on the engine that ships.
//!
//! This is the standing half of the debt D-260 records, closed for this table.
//!
//! Run with:
//! `cargo test -p bss-pricing --test postgres_schema_composite_meter -- --ignored`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

const PLAN: &str = "aaaaaaaa-0000-0000-0000-000000000001";
const TENANT: &str = "22222222-2222-2222-2222-222222222222";
const ACTOR: &str = "33333333-3333-3333-3333-333333333333";
const COMPOSITE: &str = "cccccccc-0000-0000-0000-000000000001";

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

/// A plan revision in `state`, the parent every composite row hangs off.
async fn seed_plan(conn: &DatabaseConnection, revision: i64, state: &str) {
    must_succeed(
        conn,
        &format!(
            "INSERT INTO bss.pricing_plan \
             (plan_id, revision, tenant_id, lifecycle_state, created_by, created_at_utc) \
             VALUES ('{PLAN}', {revision}, '{TENANT}', '{state}', '{ACTOR}', now())"
        ),
    )
    .await;
}

fn insert_composite(composite: &str, revision: i64, output: &str) -> String {
    format!(
        "INSERT INTO bss.pricing_composite_meter \
         (composite_id, plan_revision, tenant_id, plan_id, output_unit, constituent_units, formula) \
         VALUES ('{composite}', {revision}, '{TENANT}', '{PLAN}', '{output}', \
         '[\"vcpu\",\"ram\"]'::jsonb, '{{\"op\":\"weighted_sum\"}}'::jsonb)"
    )
}

/// A composite is authorable under a **draft** revision and under no other — the
/// parent's `lifecycle_state` is the row's, which is the whole append-only
/// arrangement.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_composite_is_authorable_only_under_a_draft_revision() {
    let conn = applied().await;
    seed_plan(&conn, 0, "draft").await;
    must_succeed(&conn, &insert_composite(COMPOSITE, 0, "vm-hour")).await;

    // The parent publishes; the child is frozen with it.
    must_succeed(
        &conn,
        &format!(
            "UPDATE bss.pricing_plan SET lifecycle_state = 'published' \
             WHERE plan_id = '{PLAN}' AND revision = 0"
        ),
    )
    .await;
    must_be_rejected(
        &conn,
        &insert_composite("cccccccc-0000-0000-0000-000000000002", 0, "pod-hour"),
        "is not permitted",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE bss.pricing_composite_meter SET output_unit = 'moved' \
             WHERE composite_id = '{COMPOSITE}'"
        ),
        "is not permitted",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("DELETE FROM bss.pricing_composite_meter WHERE composite_id = '{COMPOSITE}'"),
        "is not permitted",
    )
    .await;
}

/// One output unit per revision — §3 step 3's injectivity as far as the schema
/// carries it. Two composites rating into the same unit would produce two priced
/// lines on one `(meter, dimensionKey)`.
#[tokio::test]
#[ignore = "requires Docker"]
async fn one_output_unit_per_revision() {
    let conn = applied().await;
    seed_plan(&conn, 0, "draft").await;
    must_succeed(&conn, &insert_composite(COMPOSITE, 0, "vm-hour")).await;
    must_be_rejected(
        &conn,
        &insert_composite("cccccccc-0000-0000-0000-000000000002", 0, "vm-hour"),
        "uq_pricing_composite_meter_output",
    )
    .await;
}

/// An empty output unit publishes a composite nothing can rate.
#[tokio::test]
#[ignore = "requires Docker"]
async fn the_output_unit_is_not_empty() {
    let conn = applied().await;
    seed_plan(&conn, 0, "draft").await;
    must_be_rejected(
        &conn,
        &insert_composite(COMPOSITE, 0, ""),
        "chk_pricing_composite_meter_output_unit",
    )
    .await;
}

/// A composite naming a revision that does not exist is refused — **by the
/// trigger, and the foreign key behind it is shadowed.**
///
/// The append-only function reads the parent's `lifecycle_state` and renders a
/// missing parent as `missing`, so it answers `BEFORE INSERT`, ahead of
/// `fk_pricing_composite_meter_revision`. The key is not removed: it still
/// declares the relationship and would answer if the trigger were ever
/// narrowed. What no test can assert is the key *acting* — the same shadowing
/// D-231 recorded for `pricing_price_overlay_line`'s composite key, stated here
/// rather than discovered again.
#[tokio::test]
#[ignore = "requires Docker"]
async fn a_composite_names_a_revision_that_exists() {
    let conn = applied().await;
    seed_plan(&conn, 0, "draft").await;
    must_be_rejected(
        &conn,
        &insert_composite(COMPOSITE, 7, "vm-hour"),
        "under a missing plan revision is not permitted",
    )
    .await;
}
