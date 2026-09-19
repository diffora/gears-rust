//! Fresh-install charge-line schema and raw-SQL constraint proofs on `SQLite`.
//!
//! Repository prechecks cannot hide a missing constraint: every refusal below
//! is issued as SQL against a migrated database, so a guard that lives only in
//! `charge_line_repo` fails here rather than passing on the strength of the
//! precheck that was about to be deleted.
//!
//! The pairing is deliberate throughout — each refusal has a sibling that must
//! *land*. A suite that only proved bans would pass against a table nobody can
//! write to at all, and the four normalized tables exist to be written.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::{exec, must_succeed};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

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

/// Reject, and reject for the reason under test.
///
/// The sharp fragment is the table name plus the constraint or trigger
/// sentence: a bare "some error happened" would pass with the guard it means to
/// prove switched off, which is the failure mode the sibling schema suites
/// record in their own copies of this helper.
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

/// One line, one draft version and one `EUR`/`eu` market, by raw SQL.
async fn seed(conn: &DatabaseConnection) {
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, sku_id) VALUES \
             ('{TENANT}','{LINE}','{PLAN}','{PHASE}','recurring','{SKU}')"
        ),
    )
    .await;
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_charge_line_version (tenant_id, line_version_id, \
             charge_line_id, plan_revision, lifecycle_state, model_kind, created_by, \
             created_at_utc, row_version) VALUES \
             ('{TENANT}','{VERSION}','{LINE}',1,'draft','flat','{TENANT}','2026-01-01 00:00:00',0)"
        ),
    )
    .await;
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES ('{TENANT}','{MARKET}','{LINE}','EUR','eu')"
        ),
    )
    .await;
}

#[tokio::test]
async fn fresh_schema_contains_normalized_charge_tables() {
    let db = common::migrated_db().await;
    let rows = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type='table' AND name IN \
        ('pricing_charge_line','pricing_charge_line_version',\
        'pricing_market_price','pricing_charge_tier')"
                .to_owned(),
        ))
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
}

/// **A fresh `charge_line_id` does not buy a second line on one logical scope.**
///
/// This is the whole point of `uq_pricing_charge_line_logical_scope`: the
/// surrogate id is an identity, not a licence. A schema that keyed uniqueness on
/// `charge_line_id` alone would admit two active charges on the same plan,
/// phase, SKU and kind, and every coverage rule downstream counts them both.
#[tokio::test]
async fn a_new_line_id_cannot_duplicate_a_logical_scope() {
    let conn = common::migrated_db().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, sku_id) VALUES \
             ('{TENANT}','{OTHER_LINE}','{PLAN}','{PHASE}','recurring','{SKU}')"
        ),
        "pricing_charge_line",
    )
    .await;

    // The same logical scope under another tenant is a different line, and
    // landing it is what says the unique key is tenant-scoped rather than global.
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, sku_id) VALUES \
             ('{OTHER_TENANT}','{OTHER_LINE}','{PLAN}','{PHASE}','recurring','{SKU}')"
        ),
    )
    .await;

    // One axis apart is a different line, not a duplicate.
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, sku_id) VALUES \
             ('{TENANT}','{OTHER_LINE}','{PLAN}','{PHASE}','usage','{SKU}')"
        ),
    )
    .await;
}

/// **`USD`/`US` and `USD`/`CA` are two markets; `EUR`/`eu` twice is one.**
///
/// Region left the logical line for this table, and the design's own example
/// turns on the pair staying distinct while the currency is identical.
#[tokio::test]
async fn one_currency_in_two_regions_is_two_markets() {
    let conn = common::migrated_db().await;
    seed(&conn).await;

    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES \
             ('{TENANT}','00000000-0000-0000-0000-0000000000e2','{LINE}','USD','us')"
        ),
    )
    .await;
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES \
             ('{TENANT}','00000000-0000-0000-0000-0000000000e3','{LINE}','USD','ca')"
        ),
    )
    .await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES \
             ('{TENANT}','00000000-0000-0000-0000-0000000000e4','{LINE}','EUR','eu')"
        ),
        "pricing_market_price",
    )
    .await;
}

/// **A market belongs to a line that exists, in its own tenant.**
#[tokio::test]
async fn a_market_cannot_name_a_line_of_another_tenant() {
    let conn = common::migrated_db().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO pricing_market_price (tenant_id, market_price_id, charge_line_id, \
             currency, region) VALUES \
             ('{OTHER_TENANT}','00000000-0000-0000-0000-0000000000e5','{LINE}','EUR','eu')"
        ),
        "FOREIGN KEY",
    )
    .await;
}

/// **A version belongs to a line, and a line has one version per revision.**
#[tokio::test]
async fn a_line_version_is_bound_to_its_line_and_its_revision() {
    let conn = common::migrated_db().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_line_version (tenant_id, line_version_id, \
             charge_line_id, plan_revision, lifecycle_state, model_kind, created_by, \
             created_at_utc, row_version) VALUES \
             ('{TENANT}','{OTHER_VERSION}','00000000-0000-0000-0000-0000000000cf',1,'draft',\
             'flat','{TENANT}','2026-01-01 00:00:00',0)"
        ),
        "FOREIGN KEY",
    )
    .await;

    // A second revision of the same line is a second version, and must land.
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_line_version (tenant_id, line_version_id, \
             charge_line_id, plan_revision, lifecycle_state, model_kind, created_by, \
             created_at_utc, row_version) VALUES \
             ('{TENANT}','{OTHER_VERSION}','{LINE}',2,'draft','flat','{TENANT}',\
             '2026-01-01 00:00:00',0)"
        ),
    )
    .await;
}

/// **Tier geometry hangs off a version, and only off a tiered one.**
///
/// The kind trigger is the half a foreign key cannot state: a `flat` version has
/// no ladder to carry, so a band under one is not a dangling reference but a
/// contradiction.
#[tokio::test]
async fn tier_geometry_requires_a_tiered_version() {
    let conn = common::migrated_db().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_tier (tenant_id, line_version_id, band_ordinal, \
             from_qty, to_qty) VALUES ('{TENANT}','{VERSION}',0,0,100)"
        ),
        "pricing_charge_tier",
    )
    .await;

    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_line_version (tenant_id, line_version_id, \
             charge_line_id, plan_revision, lifecycle_state, model_kind, created_by, \
             created_at_utc, row_version) VALUES \
             ('{TENANT}','{OTHER_VERSION}','{LINE}',2,'draft','graduated','{TENANT}',\
             '2026-01-01 00:00:00',0)"
        ),
    )
    .await;
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_tier (tenant_id, line_version_id, band_ordinal, \
             from_qty, to_qty) VALUES ('{TENANT}','{OTHER_VERSION}',0,0,100)"
        ),
    )
    .await;

    // A band on a version of nobody is refused by the key, not by a precheck.
    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_tier (tenant_id, line_version_id, band_ordinal, \
             from_qty, to_qty) VALUES \
             ('{TENANT}','00000000-0000-0000-0000-0000000000df',0,0,100)"
        ),
        "pricing_charge_tier",
    )
    .await;
}

/// **The identity rows are identities.** Neither a line's logical scope nor a
/// market's currency/region may be moved in place: a key that could be edited is
/// a key two published versions could come to share.
#[tokio::test]
async fn identity_rows_refuse_an_in_place_key_move() {
    let conn = common::migrated_db().await;
    seed(&conn).await;

    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_charge_line SET charge_kind = 'usage' WHERE charge_line_id = '{LINE}'"
        ),
        "pricing_charge_line",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_market_price SET currency = 'USD' WHERE market_price_id = '{MARKET}'"
        ),
        "pricing_market_price",
    )
    .await;
}

/// **The cohort/eligibility biconditional reaches the line.**
///
/// It was a `pricing_price` CHECK; the two columns it relates moved together, so
/// the rule moved with them rather than being left behind on a table that no
/// longer holds either.
#[tokio::test]
async fn a_cohort_without_the_grandfathered_class_is_refused() {
    let conn = common::migrated_db().await;

    must_be_rejected(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, cohort, sku_id) VALUES \
             ('{TENANT}','{LINE}','{PLAN}','{PHASE}','recurring','1893456000000','{SKU}')"
        ),
        "pricing_charge_line",
    )
    .await;

    must_succeed(
        &conn,
        &format!(
            "INSERT INTO pricing_charge_line (tenant_id, charge_line_id, plan_id, phase, \
             charge_kind, cohort, price_eligibility, sku_id) VALUES \
             ('{TENANT}','{LINE}','{PLAN}','{PHASE}','recurring','1893456000000',\
             'existing_grandfathered','{SKU}')"
        ),
    )
    .await;
}
