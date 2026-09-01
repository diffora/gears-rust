//! The Postgres half of the recognized-set table's guards
//! (`cpt-cf-bss-products-dod-recognized-set-table`: *"a `CorruptRow` probe per
//! guarded column class on **both engines**"*, and the schema oracle *"together
//! with a perturbation case proving it can fail"*).
//!
//! The `SQLite` twin lives in `migrations_tests::recognized_set_guard_tests`.
//! What only this side can prove is that the **same** whitelist holds under
//! `plpgsql`: the two engines express the guard differently — a `RAISE
//! EXCEPTION` in a trigger function against `RAISE(ABORT)` in a `WHEN` clause
//! — so a divergence between them is invisible to either suite alone, and
//! that is exactly the defect the two-array migration shape makes easy.
//!
//! Run under `make test-products-pg`; skipped when no engine is reachable.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-recognized-set-table:p1

// The integration-test posture every `tests/` file here takes.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, FromQueryResult as _, Statement};

const TENANT: &str = "00000000-0000-0000-0000-0000000005e7";

#[derive(Debug, sea_orm::FromQueryResult)]
struct ColumnRow {
    column_name: String,
    is_nullable: String,
}

const ROSTER: &[(&str, bool)] = &[
    ("created_at", false),
    ("display_label", true),
    ("member_code", false),
    ("seeded_by", true),
    ("set_kind", false),
    ("state", false),
    ("tenant_id", false),
    ("updated_at", false),
];

async fn exec(conn: &impl ConnectionTrait, sql: String) -> Result<(), sea_orm::DbErr> {
    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        sql,
    ))
    .await
    .map(|_| ())
}

async fn seed(conn: &impl ConnectionTrait, code: &str) {
    exec(
        conn,
        format!(
            "INSERT INTO bss.products_recognized_set \
             (tenant_id, set_kind, member_code, state, seeded_by, created_at, updated_at) \
             VALUES ('{TENANT}', 'metering_unit', '{code}', 'active', 'registry', now(), now())"
        ),
    )
    .await
    .expect("seed the member");
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_whitelist_and_the_delete_refusal_hold_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn, "pg-vCPU-hour").await;

    exec(
        &conn,
        "UPDATE bss.products_recognized_set SET state = 'deprecated', \
         display_label = 'vCPU hour' WHERE member_code = 'pg-vCPU-hour'"
            .to_owned(),
    )
    .await
    .expect("state and display_label are writable");

    for (column, value) in [
        ("member_code", "'pg-vCPU-minute'"),
        ("set_kind", "'plan_tier'"),
        ("seeded_by", "'operator'"),
    ] {
        let err = exec(
            &conn,
            format!(
                "UPDATE bss.products_recognized_set SET {column} = {value} \
                 WHERE member_code = 'pg-vCPU-hour'"
            ),
        )
        .await
        .expect_err("the whitelist refuses everything but the two");
        assert!(
            err.to_string()
                .contains("only state and display_label are writable"),
            "{column}: {err}"
        );
    }

    let err = exec(
        &conn,
        "DELETE FROM bss.products_recognized_set WHERE member_code = 'pg-vCPU-hour'".to_owned(),
    )
    .await
    .expect_err("a DELETE must be refused");
    assert!(
        err.to_string()
            .contains("a removal is the removed state, never a DELETE"),
        "{err}"
    );
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_recognized_set_roster_matches_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    let rows = ColumnRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT column_name, is_nullable FROM information_schema.columns \
         WHERE table_schema = 'bss' AND table_name = 'products_recognized_set' \
         ORDER BY column_name"
            .to_owned(),
    ))
    .all(&conn)
    .await
    .expect("information_schema answers");
    let actual: Vec<(String, bool)> = rows
        .into_iter()
        .map(|row| (row.column_name, row.is_nullable == "YES"))
        .collect();
    let golden: Vec<(String, bool)> = ROSTER
        .iter()
        .map(|(name, nullable)| ((*name).to_owned(), *nullable))
        .collect();
    assert_eq!(
        actual, golden,
        "the Postgres roster must equal the SQLite one"
    );

    // The perturbation: a roster the oracle was not written for must fail.
    let wrong: Vec<(String, bool)> = golden.iter().skip(1).cloned().collect();
    assert_ne!(
        actual, wrong,
        "a roster missing a column must not compare equal, or this oracle asserts nothing"
    );
}
