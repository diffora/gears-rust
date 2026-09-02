//! The Postgres half of the lifecycle stores' schema oracle
//! (`cpt-cf-bss-products-dod-scheduled-transition-store` and
//! `cpt-cf-bss-products-dod-deferred-retirement-store`: *"A schema-oracle
//! golden **MUST** exist on **both engines** with a perturbation case proving
//! it can fail"*).
//!
//! The `SQLite` half lives in
//! `migrations_tests::lifecycle_store_schema_tests`. This file pins the same
//! two rosters — nullability included — against `information_schema`, so a
//! column added to one engine's statement array and not the other fails here.
//!
//! Run under `make test-products-pg`; skipped when no engine is reachable.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-scheduled-transition-store:p1
//! @cpt-dod:cpt-cf-bss-products-dod-deferred-retirement-store:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, FromQueryResult as _, Statement};

#[derive(Debug, sea_orm::FromQueryResult)]
struct ColumnRow {
    column_name: String,
    is_nullable: String,
}

const SCHEDULED: &[(&str, bool)] = &[
    ("approval_ref", false),
    ("at", false),
    ("attempt", false),
    ("claimed_at", true),
    ("created_at", false),
    ("entity_id", false),
    ("entity_kind", false),
    ("kind", false),
    ("outcome_reason", true),
    ("retirement_reason", true),
    ("state", false),
    ("tenant_id", false),
    ("transition_id", false),
    ("updated_at", false),
];

const DEFERRED: &[(&str, bool)] = &[
    ("cascade_ref", false),
    ("children_snapshot", false),
    ("created_at", false),
    ("created_by", false),
    ("product_id", false),
    ("resolution", true),
    ("resolved_at", true),
    ("tenant_id", false),
];

async fn roster(conn: &impl ConnectionTrait, table: &str) -> Vec<(String, bool)> {
    let rows = ColumnRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT column_name, is_nullable \
         FROM information_schema.columns \
         WHERE table_schema = 'bss' AND table_name = $1 \
         ORDER BY column_name",
        [table.into()],
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
async fn the_lifecycle_store_rosters_match_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    assert_eq!(
        roster(&conn, "products_scheduled_transition").await,
        golden(SCHEDULED),
        "scheduled_transition's Postgres roster must equal the SQLite one, nullability included"
    );
    assert_eq!(
        roster(&conn, "products_deferred_retirement").await,
        golden(DEFERRED)
    );

    assert_ne!(
        roster(&conn, "products_scheduled_transition").await,
        golden(DEFERRED),
        "two different rosters must not compare equal, or this oracle asserts nothing"
    );
    assert!(
        roster(&conn, "products_scheduled_transition_nope")
            .await
            .is_empty(),
        "the oracle reads the real catalog: an absent table has no columns"
    );
}

/// The two reason columns stay independent on Postgres too: `retirement_reason`
/// is nullable (publish intents carry none) and `outcome_reason` is nullable
/// (only written on finish), and neither is missing from the roster.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_two_reason_columns_are_nullable_independently_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    let cols = roster(&conn, "products_scheduled_transition").await;
    let retirement = cols
        .iter()
        .find(|(name, _)| name == "retirement_reason")
        .expect("retirement_reason column");
    let outcome = cols
        .iter()
        .find(|(name, _)| name == "outcome_reason")
        .expect("outcome_reason column");
    assert!(
        retirement.1,
        "retirement_reason is nullable (publish carries none)"
    );
    assert!(
        outcome.1,
        "outcome_reason is nullable until the runner finishes"
    );
}
