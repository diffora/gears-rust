//! The Postgres half of the taxonomy schema oracle
//! (`cpt-cf-bss-products-dod-category-table`: *"A schema-oracle golden
//! **MUST** exist for both engines together with a perturbation case proving
//! the oracle can fail"*), plus the one probe that is engine-specific by
//! nature: **the root-name partial index** (P-D-88 arm 1) exists because the
//! two engines agree NULLs are distinct — so the refusal it produces must be
//! demonstrated on Postgres too, not inferred from `SQLite`.
//!
//! Run under `make test-products-pg`; skipped when no engine is reachable.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-category-table:p1

// The integration-test posture every `tests/` file here takes: a probe that
// cannot reach its engine must panic loudly.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, FromQueryResult as _, Statement};

#[derive(Debug, sea_orm::FromQueryResult)]
struct ColumnRow {
    column_name: String,
    is_nullable: String,
}

const CATEGORY: &[(&str, bool)] = &[
    ("category_id", false),
    ("created_at", false),
    ("mutation_seq", false),
    ("name", false),
    ("name_normalized", false),
    ("parent_id", true),
    ("state", false),
    ("tenant_id", false),
    ("updated_at", false),
];

const ASSIGNMENT: &[(&str, bool)] = &[
    ("assigned_at", false),
    ("category_id", false),
    ("product_id", false),
    ("role", false),
    ("tenant_id", false),
];

async fn roster(conn: &impl ConnectionTrait, table: &str) -> Vec<(String, bool)> {
    let rows = ColumnRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT column_name, is_nullable FROM information_schema.columns \
             WHERE table_schema = 'bss' AND table_name = '{table}' ORDER BY column_name"
        ),
    ))
    .all(conn)
    .await
    .expect("information_schema answers");
    rows.into_iter()
        .map(|row| (row.column_name, row.is_nullable == "YES"))
        .collect()
}

fn golden(rows: &[(&str, bool)]) -> Vec<(String, bool)> {
    rows.iter()
        .map(|(name, nullable)| ((*name).to_owned(), *nullable))
        .collect()
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_taxonomy_rosters_match_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    assert_eq!(roster(&conn, "products_category").await, golden(CATEGORY));
    assert_eq!(
        roster(&conn, "products_product_category").await,
        golden(ASSIGNMENT)
    );

    // The perturbation: the oracle must fail against the wrong roster and
    // report an absent table as empty rather than as a match.
    assert_ne!(roster(&conn, "products_category").await, golden(ASSIGNMENT));
    assert!(
        roster(&conn, "products_category_no_such_table")
            .await
            .is_empty(),
        "the oracle reads the real catalog"
    );
}

/// P-D-88 arm 1 on the engine whose NULL semantics forced it: two roots with
/// one name must collide on the partial index, and the declared in-parent
/// UNIQUE alone would admit them.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_duplicate_root_name_is_refused_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    let seed = |name: &str| {
        format!(
            "INSERT INTO bss.products_category \
             (tenant_id, category_id, parent_id, name, name_normalized, state, \
              created_at, updated_at) \
             VALUES ('00000000-0000-0000-0000-00000000c0de', gen_random_uuid(), NULL, \
              '{name}', '{name}', 'active', now(), now())"
        )
    };
    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        seed("hardware"),
    ))
    .await
    .expect("the first root lands");

    let err = conn
        .execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            seed("hardware"),
        ))
        .await
        .expect_err("the second same-name root must be refused");
    assert!(
        err.to_string().contains("uq_products_category_root_name"),
        "the refusal must be the partial index's, by name: {err}"
    );
}
