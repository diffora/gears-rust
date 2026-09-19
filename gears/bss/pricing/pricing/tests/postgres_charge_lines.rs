//! Raw-SQL constraint proofs for the normalized charge-line tables on `PostgreSQL`.
//!
//! The `SQLite` sibling (`sqlite_charge_lines.rs`) proves the same rules against
//! the mirror. This one proves them against the dialect production runs, and it
//! is the sharper of the two: `PostgreSQL` names the constraint it refused with,
//! so each assertion below can demand the specific key rather than "the write
//! was refused somehow".
//!
//! Every statement goes through [`Pg::raw`], deliberately past every repository:
//! a guard that lives only in `charge_line_repo` cannot answer here.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

const TENANT: &str = "00000000-0000-0000-0000-0000000000a1";
const OTHER_TENANT: &str = "00000000-0000-0000-0000-0000000000a2";
const PLAN: &str = "00000000-0000-0000-0000-0000000000b1";
const PHASE: &str = "00000000-0000-0000-0000-0000000000b2";
const SKU: &str = "00000000-0000-0000-0000-0000000000b3";
const LINE: &str = "00000000-0000-0000-0000-0000000000c1";
const OTHER_LINE: &str = "00000000-0000-0000-0000-0000000000c2";
const VERSION: &str = "00000000-0000-0000-0000-0000000000d1";
const OTHER_VERSION: &str = "00000000-0000-0000-0000-0000000000d2";
const MARKET: &str = "00000000-0000-0000-0000-0000000000e1";

async fn exec(conn: &DatabaseConnection, sql: &str) -> Result<(), sea_orm::DbErr> {
    conn.execute_raw(Statement::from_string(
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

/// Refused, and refused by the named constraint.
async fn must_be_rejected(conn: &DatabaseConnection, sql: &str, because: &str) {
    let err = exec(conn, sql)
        .await
        .err()
        .unwrap_or_else(|| panic!("the schema must reject: {sql}"));
    let message = err.to_string();
    assert!(
        message.contains(because),
        "the rejection must be the one under test (`{because}`), got: {message}"
    );
}

/// One line, one draft version and one `EUR`/`eu` market.
async fn seed(conn: &DatabaseConnection) {
    must_succeed(
        conn,
        &format!(
            "INSERT INTO bss.pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, sku_id) VALUES \
             ('{TENANT}','{LINE}','{PLAN}','{PHASE}','recurring','{SKU}')"
        ),
    )
    .await;
    must_succeed(
        conn,
        &format!(
            "INSERT INTO bss.pricing_charge_line_version (tenant_id, line_version_id, \
             charge_line_id, plan_revision, lifecycle_state, model_kind, created_by, \
             created_at_utc, row_version) VALUES \
             ('{TENANT}','{VERSION}','{LINE}',1,'draft','flat','{TENANT}',\
             '2026-01-01T00:00:00Z',0)"
        ),
    )
    .await;
    must_succeed(
        conn,
        &format!(
            "INSERT INTO bss.pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES ('{TENANT}','{MARKET}','{LINE}','EUR','eu')"
        ),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_four_normalized_tables_exist_on_a_fresh_boot() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    let names = pg_support::catalog_strings(
        &conn,
        "SELECT tablename FROM pg_tables WHERE schemaname = 'bss' AND tablename IN \
         ('pricing_charge_line','pricing_charge_line_version','pricing_market_price',\
         'pricing_charge_tier') ORDER BY tablename",
    )
    .await;
    assert_eq!(
        names,
        vec![
            "pricing_charge_line",
            "pricing_charge_line_version",
            "pricing_charge_tier",
            "pricing_market_price",
        ]
    );
}

/// **A fresh `charge_line_id` does not buy a second line on one logical scope.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_new_line_id_cannot_duplicate_a_logical_scope() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, sku_id) VALUES \
             ('{TENANT}','{OTHER_LINE}','{PLAN}','{PHASE}','recurring','{SKU}')"
        ),
        "uq_pricing_charge_line_logical_scope",
    )
    .await;

    // The same logical scope under another tenant is a different line.
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, sku_id) VALUES \
             ('{OTHER_TENANT}','{OTHER_LINE}','{PLAN}','{PHASE}','recurring','{SKU}')"
        ),
    )
    .await;
}

/// **`USD`/`US` and `USD`/`CA` are two markets; one pair twice is one.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn one_currency_in_two_regions_is_two_markets() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn).await;

    must_succeed(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES \
             ('{TENANT}','00000000-0000-0000-0000-0000000000e2','{LINE}','USD','us')"
        ),
    )
    .await;
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES \
             ('{TENANT}','00000000-0000-0000-0000-0000000000e3','{LINE}','USD','ca')"
        ),
    )
    .await;
    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES \
             ('{TENANT}','00000000-0000-0000-0000-0000000000e4','{LINE}','EUR','eu')"
        ),
        "uq_pricing_market_price_scope",
    )
    .await;
}

/// **A market belongs to a line of its own tenant.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_market_cannot_name_a_line_of_another_tenant() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES \
             ('{OTHER_TENANT}','00000000-0000-0000-0000-0000000000e5','{LINE}','EUR','eu')"
        ),
        "fk_pricing_market_price_line",
    )
    .await;
}

/// **A line has one version per plan revision.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_line_holds_one_version_per_revision() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_charge_line_version (tenant_id, line_version_id, \
             charge_line_id, plan_revision, lifecycle_state, model_kind, created_by, \
             created_at_utc, row_version) VALUES \
             ('{TENANT}','{OTHER_VERSION}','{LINE}',1,'draft','flat','{TENANT}',\
             '2026-01-01T00:00:00Z',0)"
        ),
        "uq_pricing_charge_line_version_revision",
    )
    .await;

    // A second revision of the same line is a second version, and must land.
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_charge_line_version (tenant_id, line_version_id, \
             charge_line_id, plan_revision, lifecycle_state, model_kind, created_by, \
             created_at_utc, row_version) VALUES \
             ('{TENANT}','{OTHER_VERSION}','{LINE}',2,'draft','flat','{TENANT}',\
             '2026-01-01T00:00:00Z',0)"
        ),
    )
    .await;
}

/// **Tier geometry hangs off a tiered version, and only off one.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn tier_geometry_requires_a_tiered_version() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_charge_tier (tenant_id, line_version_id, band_ordinal, \
             from_qty, to_qty) VALUES ('{TENANT}','{VERSION}',0,0,100)"
        ),
        "pricing_charge_tier",
    )
    .await;

    must_succeed(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_charge_line_version (tenant_id, line_version_id, \
             charge_line_id, plan_revision, lifecycle_state, model_kind, created_by, \
             created_at_utc, row_version) VALUES \
             ('{TENANT}','{OTHER_VERSION}','{LINE}',2,'draft','graduated','{TENANT}',\
             '2026-01-01T00:00:00Z',0)"
        ),
    )
    .await;
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_charge_tier (tenant_id, line_version_id, band_ordinal, \
             from_qty, to_qty) VALUES ('{TENANT}','{OTHER_VERSION}',0,0,100)"
        ),
    )
    .await;
}

/// **The identity rows are identities.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn identity_rows_refuse_an_in_place_key_move() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "UPDATE bss.pricing_charge_line SET charge_kind = 'usage' \
             WHERE charge_line_id = '{LINE}'"
        ),
        "pricing_charge_line",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE bss.pricing_market_price SET currency = 'USD' \
             WHERE market_price_id = '{MARKET}'"
        ),
        "pricing_market_price",
    )
    .await;
}

/// **The cohort/eligibility biconditional moved with its two columns.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_cohort_without_the_grandfathered_class_is_refused() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, cohort, sku_id) VALUES \
             ('{TENANT}','{LINE}','{PLAN}','{PHASE}','recurring','1893456000000','{SKU}')"
        ),
        "chk_pricing_charge_line_cohort_eligibility",
    )
    .await;

    must_succeed(
        &conn,
        &format!(
            "INSERT INTO bss.pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, cohort, price_eligibility, sku_id) VALUES \
             ('{TENANT}','{LINE}','{PLAN}','{PHASE}','recurring','1893456000000',\
             'existing_grandfathered','{SKU}')"
        ),
    )
    .await;
}
