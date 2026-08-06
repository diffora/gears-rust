//! The four Slice-4 taxonomy tables — `pricing_region_taxonomy` and its three
//! siblings — proved by **executing the statement each object must refuse**, on
//! Postgres.
//!
//! # Why this suite exists
//!
//! `tests/sqlite_taxonomy_store.rs` has claimed since it was written that *"the
//! roster is pinned by name in `tests/postgres_migrations.rs` and the refusals
//! are executed in `tests/postgres_schema_taxonomy.rs`"*. **This file did not
//! exist.** The roster half was true — `postgres_migrations.rs` names all eight
//! `CHECK` constraints of the four tables — and the roster proves the constraints
//! were *created*, which is a different claim from what they *refuse*. So the
//! sentence read as correct while nothing on this engine had ever tried to insert
//! a blank value or a third state token.
//!
//! That is the same premise the base commit of this branch closed one table over
//! (*"the third copy of one premise — the Postgres tier's `overlay` refusal,
//! which the fast suite could not show"*), arriving here from the taxonomy side.
//! The file is written rather than the sentence corrected, because the sentence
//! is the right claim: `m20260802_000028`…`000031` emit **different SQL** per
//! backend, so a refusal measured on `SQLite` is not a fact about Postgres.
//!
//! # The one fact the two engines state differently
//!
//! **The uniqueness message.** `SQLite` names the *columns* for a plain
//! composite primary key — `sqlite_taxonomy_store` asserts `UNIQUE constraint
//! failed: <table>.tenant_id, <table>.value` — while Postgres names the
//! **constraint**. So this suite asserts the constraint name, which is the
//! sharper assertion and the one that could not be made over there.
//!
//! # What is deliberately not tested by refusal
//!
//! `tax_category` is a nullable `text` with no `CHECK`: any string is a legal
//! category and the design set names no vocabulary for one (D-01 leaves the
//! universe tenant-declared). It refuses nothing, so there is nothing to execute
//! against it, and its presence is pinned by the column assertions below and by
//! `postgres_migrations.rs`.
//!
//! Ignored by default; they need Docker. Run with
//! `cargo test -p bss-pricing --test postgres_schema_taxonomy -- --ignored`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

const TENANT: &str = "11111111-1111-1111-1111-111111111111";
const OTHER_TENANT: &str = "99999999-9999-9999-9999-999999999999";

/// The four tables, in the order §6 declares them.
///
/// A slice rather than four copies of every case, for
/// `sqlite_taxonomy_store::TAXONOMIES`' reason: the shape is **one** shape, and
/// the whole argument for building the `partner` / `orgTier` pair (D-120) is that
/// it is the region/brand shape with nothing added. A rule proved on one of them
/// and assumed on the other three is exactly the drift this arrangement exists to
/// make impossible.
const TAXONOMIES: &[&str] = &[
    "pricing_region_taxonomy",
    "pricing_brand_taxonomy",
    "pricing_partner_taxonomy",
    "pricing_org_tier_taxonomy",
];

/// The one that carries D-01's two markers.
const REGION: &str = "pricing_region_taxonomy";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A fresh database carrying the applied chain, on the one shared server.
///
/// A **plain** connection: every statement here is raw SQL that deliberately
/// reaches past `TaxonomyRepo`, because the repository is exactly the layer that
/// cannot see a guard stop refusing — it only ever writes values that satisfy
/// them.
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

/// Reject, **and by the named object**.
///
/// The fragment is the constraint name, never the table name: a fragment naming
/// the table is satisfied by `relation does not exist`, which is what a suite
/// written before its migration would assert against forever.
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

fn insert(table: &str, tenant: &str, value: &str, state: &str) -> String {
    format!(
        "INSERT INTO bss.{table} (tenant_id, value, display_name, state) \
         VALUES ('{tenant}', '{value}', 'A label', '{state}')"
    )
}

// ---------------------------------------------------------------------------
// The shape all four share.
// ---------------------------------------------------------------------------

/// Each of the four exists and is keyed `(tenant_id, value)`.
#[tokio::test]
#[ignore = "needs Docker"]
async fn each_taxonomy_is_keyed_by_tenant_and_value() {
    let conn = applied().await;
    for table in TAXONOMIES {
        must_succeed(&conn, &insert(table, TENANT, "eu-west", "active")).await;
        // The same value under a different tenant is a different row: the key is
        // tenant-scoped, which is what makes one operator's `gold` tier
        // independent of another's.
        must_succeed(&conn, &insert(table, OTHER_TENANT, "eu-west", "active")).await;
    }
}

/// One value per tenant, in each of the four — **and Postgres names the
/// constraint**, which `SQLite` does not for a plain composite key.
#[tokio::test]
#[ignore = "needs Docker"]
async fn a_value_is_declared_at_most_once_per_tenant() {
    let conn = applied().await;
    for table in TAXONOMIES {
        must_succeed(&conn, &insert(table, TENANT, "eu-west", "active")).await;
        must_be_rejected(
            &conn,
            &insert(table, TENANT, "eu-west", "retired"),
            &format!("{table}_pkey"),
        )
        .await;
    }
}

/// `state` is `active | retired` and nothing else.
///
/// The pair is the whole state machine §6 gives these tables — retirement is
/// guarded rather than cascading, and re-activation is a legal audited move — so
/// a third token would be a state no rule in the design set describes.
#[tokio::test]
#[ignore = "needs Docker"]
async fn a_state_outside_the_declared_pair_is_refused() {
    let conn = applied().await;
    for table in TAXONOMIES {
        must_be_rejected(
            &conn,
            &insert(table, TENANT, "eu-west", "deprecated"),
            &format!("chk_{table}_state"),
        )
        .await;
    }
}

/// A blank value is refused in each of the four.
///
/// Not decoration: `inst-plv-scope` validates an overlay's `scope_value` against
/// these tables, and `pricing_price_overlay` renders the **absence** of a scope
/// value as the empty string (the `global` class). A blank taxonomy value would
/// make that sentinel forgeable — a `brand` overlay carrying `''` would validate
/// against a declared universe *and* read as classless.
#[tokio::test]
#[ignore = "needs Docker"]
async fn a_blank_value_is_refused() {
    let conn = applied().await;
    for table in TAXONOMIES {
        must_be_rejected(
            &conn,
            &insert(table, TENANT, "", "active"),
            &format!("chk_{table}_value_present"),
        )
        .await;
    }
}

/// A whitespace-only value is **admitted**, and that is measured rather than
/// assumed.
///
/// The constraint is `length(value) > 0`, which `'   '` satisfies. This case
/// began life asserting a refusal — the module doc's own argument seemed to
/// require one, since the `CHECK` exists so *"nothing else may render"* the
/// classless sentinel — and the assertion was **wrong**: the statement lands on
/// both engines. It is pinned as it actually behaves, because a suite asserting
/// a guard the schema does not have is a suite that will be "fixed" by weakening
/// something real.
///
/// **What the gap costs is not a forged sentinel.** `pricing_price_overlay`
/// renders the classless scope as the empty string exactly, and `'   '` is not
/// that, so `inst-plv-scope` is unaffected. The exposure is one level over:
/// `ScopeValue::new` **trims** before it decides and therefore refuses `'   '`,
/// so `TaxonomyRepo::list` maps such a row to `RepoError::CorruptRow` — and a
/// single direct write of a whitespace value makes that class's whole `GET`
/// fail, for every value in it. Register entry `T-6`.
#[tokio::test]
#[ignore = "needs Docker"]
async fn a_whitespace_only_value_is_admitted_by_the_length_check() {
    let conn = applied().await;
    for table in TAXONOMIES {
        must_succeed(&conn, &insert(table, TENANT, "   ", "active")).await;
    }
}

// ---------------------------------------------------------------------------
// The two columns that are the region's alone (D-01).
// ---------------------------------------------------------------------------

/// The region taxonomy carries `tax_category` and `tax_rate_present`.
#[tokio::test]
#[ignore = "needs Docker"]
async fn the_region_taxonomy_carries_the_two_tax_columns() {
    let conn = applied().await;
    must_succeed(
        &conn,
        &format!(
            "INSERT INTO bss.{REGION} (tenant_id, value, display_name, state, \
                                       tax_category, tax_rate_present) \
             VALUES ('{TENANT}', 'eu-west', 'EU West', 'active', 'standard', true)"
        ),
    )
    .await;
}

/// `tax_rate_present` defaults to **false**, which is the fail-closed reading of
/// `RegionTaxReadiness`: a region nobody has declared a rate for is a region with
/// no rate, not a region with an unknown one.
///
/// Asserted by a statement that must be **refused** when the default is anything
/// else, which is the only way to state it in DDL alone: the row is inserted
/// without the column and then required to satisfy a predicate only `false`
/// meets.
#[tokio::test]
#[ignore = "needs Docker"]
async fn tax_rate_present_defaults_to_false() {
    let conn = applied().await;
    must_succeed(&conn, &insert(REGION, TENANT, "eu-west", "active")).await;
    must_succeed(
        &conn,
        &format!(
            "DO $$ BEGIN \
               IF (SELECT tax_rate_present FROM bss.{REGION} \
                   WHERE tenant_id = '{TENANT}' AND value = 'eu-west') <> false \
               THEN RAISE EXCEPTION 'an undeclared region must have no rate'; \
               END IF; \
             END $$"
        ),
    )
    .await;
}

/// The other three do **not** carry them — §6 says "region-only" and this is what
/// that sentence is worth if it is true of the schema.
///
/// Asserted by a statement the parser must refuse outright, which is the only way
/// to prove a column's absence: a `SELECT` of a column that is not there is
/// refused before any constraint is consulted.
#[tokio::test]
#[ignore = "needs Docker"]
async fn the_other_three_taxonomies_carry_no_tax_columns() {
    let conn = applied().await;
    for table in TAXONOMIES.iter().filter(|t| **t != REGION) {
        must_be_rejected(
            &conn,
            &format!("SELECT tax_category FROM bss.{table}"),
            "tax_category",
        )
        .await;
        must_be_rejected(
            &conn,
            &format!("SELECT tax_rate_present FROM bss.{table}"),
            "tax_rate_present",
        )
        .await;
    }
}
