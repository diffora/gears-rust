//! The append-only column whitelist on `pricing_price`, proven against a real
//! database.
//!
//! Postgres carries the whitelist as one PL/pgSQL trigger; `SQLite` mirrors it
//! as four `RAISE(ABORT, ...)` triggers (see the migration's module doc), so the
//! guard is exercisable without Docker and this suite does not need a
//! testcontainers test to know the rule holds.
//!
//! Four cases, one per branch of the whitelist: a forbidden price mutation, a
//! forbidden lifecycle transition, a forbidden loosening of `grandfather_until`,
//! and a forbidden DELETE — plus the two moves that are *supposed* to work, so
//! the test proves a whitelist rather than a blanket ban. Without the guard an
//! ad-hoc UPDATE would silently change a frozen `CatalogVersion`'s content at
//! the next warm re-drive, because the projector reads truth rows.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::{MigrationTrait, MigratorTrait, SchemaManager};

use bss_pricing::infra::storage::migrations::Migrator;

const TENANT: &str = "11111111-1111-1111-1111-111111111111";
const PLAN: &str = "22222222-2222-2222-2222-222222222222";
const PHASE: &str = "33333333-3333-3333-3333-333333333333";
const ACTOR: &str = "44444444-4444-4444-4444-444444444444";
const PUBLISHED: &str = "55555555-5555-5555-5555-555555555555";
const DRAFT: &str = "66666666-6666-6666-6666-666666666666";

async fn migrated_db() -> DatabaseConnection {
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    let mut chain: Vec<Box<dyn MigrationTrait>> = Migrator::migrations();
    chain.sort_by(|a, b| a.name().cmp(b.name()));
    for migration in &chain {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }
    conn
}

async fn exec(conn: &DatabaseConnection, sql: &str) -> Result<(), sea_orm::DbErr> {
    conn.execute(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
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

async fn must_be_rejected(conn: &DatabaseConnection, sql: &str) {
    let err = exec(conn, sql)
        .await
        .err()
        .unwrap_or_else(|| panic!("the append-only guard must reject: {sql}"));
    let message = err.to_string();
    assert!(
        message.contains("pricing_price"),
        "the rejection must name the guard it came from, got: {message}"
    );
}

async fn scalar(conn: &DatabaseConnection, sql: &str) -> String {
    let row = conn
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .expect("query")
        .expect("one row");
    row.try_get::<String>("", "v").expect("read value")
}

/// Insert one published `all_subscriptions` row and one draft row on the same
/// plan (different charge kinds, so the scope-key unique index is satisfied).
async fn seed(conn: &DatabaseConnection) {
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_price (
                price_id, tenant_id, plan_id, currency, region, phase,
                charge_kind, amount_minor, model_kind, lifecycle_state,
                created_by, created_at_utc)
             VALUES ('{PUBLISHED}', '{TENANT}', '{PLAN}', 'USD', 'EU', '{PHASE}',
                'recurring', 1000, 'flat', 'published', '{ACTOR}', '2026-08-02 10:00:00 +00:00')"
        ),
    )
    .await;
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_price (
                price_id, tenant_id, plan_id, currency, region, phase,
                charge_kind, amount_minor, model_kind, lifecycle_state,
                created_by, created_at_utc)
             VALUES ('{DRAFT}', '{TENANT}', '{PLAN}', 'USD', 'EU', '{PHASE}',
                'one_time', 500, 'flat', 'draft', '{ACTOR}', '2026-08-02 10:00:00 +00:00')"
        ),
    )
    .await;
}

#[tokio::test]
async fn a_published_price_row_is_immutable_in_content() {
    let conn = migrated_db().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_price SET amount_minor = 1 WHERE price_id = '{PUBLISHED}'"),
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_price SET currency = 'EUR' WHERE price_id = '{PUBLISHED}'"),
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_price SET model_kind = 'volume' WHERE price_id = '{PUBLISHED}'"),
    )
    .await;

    let amount = scalar(
        &conn,
        &format!("SELECT CAST(amount_minor AS TEXT) AS v FROM pricing_price WHERE price_id = '{PUBLISHED}'"),
    )
    .await;
    assert_eq!(amount, "1000", "no rejected UPDATE may have landed");
}

#[tokio::test]
async fn only_the_sanctioned_lifecycle_transition_is_permitted() {
    let conn = migrated_db().await;
    seed(&conn).await;

    // `published -> retired` is a plan-revision flip, not a price-row one.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price SET lifecycle_state = 'retired' WHERE price_id = '{PUBLISHED}'"
        ),
    )
    .await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price SET lifecycle_state = 'draft' WHERE price_id = '{PUBLISHED}'"
        ),
    )
    .await;

    // `published -> superseded` is the one the state machine sanctions.
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price SET lifecycle_state = 'superseded' WHERE price_id = '{PUBLISHED}'"
        ),
    )
    .await;
    let state = scalar(
        &conn,
        &format!("SELECT lifecycle_state AS v FROM pricing_price WHERE price_id = '{PUBLISHED}'"),
    )
    .await;
    assert_eq!(state, "superseded");
}

#[tokio::test]
async fn grandfather_until_may_only_be_tightened() {
    let conn = migrated_db().await;
    let row = "77777777-7777-7777-7777-777777777777";
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_price (
                price_id, tenant_id, plan_id, currency, region, phase,
                price_eligibility, charge_kind, cohort, amount_minor, model_kind,
                lifecycle_state, created_by, created_at_utc)
             VALUES ('{row}', '{TENANT}', '{PLAN}', 'USD', 'EU', '{PHASE}',
                'existing_grandfathered', 'recurring', '1780000000000', 900, 'flat',
                'published', '{ACTOR}', '2026-08-02 10:00:00 +00:00')"
        ),
    )
    .await;

    // Setting it when null is a tightening.
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price SET grandfather_until = '2027-01-01 00:00:00 +00:00' \
             WHERE price_id = '{row}'"
        ),
    )
    .await;
    // Moving it earlier is a tightening.
    must_succeed(
        &conn,
        &format!(
            "UPDATE pricing_price SET grandfather_until = '2026-10-01 00:00:00 +00:00' \
             WHERE price_id = '{row}'"
        ),
    )
    .await;
    // Moving it later is a loosening.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_price SET grandfather_until = '2028-01-01 00:00:00 +00:00' \
             WHERE price_id = '{row}'"
        ),
    )
    .await;
    // Clearing it is a loosening too.
    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_price SET grandfather_until = NULL WHERE price_id = '{row}'"),
    )
    .await;

    let horizon = scalar(
        &conn,
        &format!("SELECT grandfather_until AS v FROM pricing_price WHERE price_id = '{row}'"),
    )
    .await;
    assert_eq!(horizon, "2026-10-01 00:00:00 +00:00");
}

#[tokio::test]
async fn published_rows_never_delete_and_drafts_stay_mutable() {
    let conn = migrated_db().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!("DELETE FROM pricing_price WHERE price_id = '{PUBLISHED}'"),
    )
    .await;

    // A never-published draft is freely mutable and deletable — the whitelist
    // guards frozen rows, it does not freeze authoring.
    must_succeed(
        &conn,
        &format!("UPDATE pricing_price SET amount_minor = 750 WHERE price_id = '{DRAFT}'"),
    )
    .await;
    must_succeed(
        &conn,
        &format!("DELETE FROM pricing_price WHERE price_id = '{DRAFT}'"),
    )
    .await;

    let remaining = scalar(
        &conn,
        "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price",
    )
    .await;
    assert_eq!(remaining, "1", "only the published row survives");
}
