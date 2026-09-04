//! The Postgres half of `10-retention-erasure`'s allow-list schema oracle
//! (`cpt-cf-bss-products-dod-pii-allowlist`), plus the two probes that are
//! engine-specific by nature: **the partial unique on `state = 'active'`**
//! and the tombstone-inclusive read index **P-D-118** item 18 routed to the
//! same migration.
//!
//! The partial predicate is the whole mechanism — revocation is a state flip
//! and never a `DELETE` (P-D-47's reasoning), so *"at most one entry per
//! value"* has to mean *at most one **active** one* — and a partial index is
//! exactly the kind of DDL that can be accepted by `SQLite` and rejected or
//! silently widened by Postgres. Demonstrated here rather than inferred.
//!
//! Run under `make test-products-pg`; skipped when no engine is reachable.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-pii-allowlist:p1

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

#[derive(Debug, sea_orm::FromQueryResult)]
struct DefRow {
    indexdef: String,
}

/// P-D-117 item 23's roster, transcribed from the decision rather than read
/// from the migration: a column dropped from both at once must still be a
/// red here.
const ALLOWLIST: &[(&str, bool)] = &[
    ("created_at", false),
    ("entry_id", false),
    ("justification", false),
    ("signed_off_at", false),
    ("signed_off_by", false),
    ("state", false),
    ("tenant_id", false),
    ("updated_at", false),
    ("value_normalized", false),
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

async fn indexdef(conn: &impl ConnectionTrait, name: &str) -> String {
    DefRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT indexdef FROM pg_indexes WHERE schemaname = 'bss' AND indexname = '{name}'"
        ),
    ))
    .one(conn)
    .await
    .expect("pg_indexes answers")
    .map(|row| row.indexdef)
    .unwrap_or_default()
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_allowlist_roster_matches_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    let golden: Vec<(String, bool)> = ALLOWLIST
        .iter()
        .map(|(name, nullable)| ((*name).to_owned(), *nullable))
        .collect();
    assert_eq!(roster(&conn, "products_pii_allowlist").await, golden);

    // The perturbation: an absent table must read as empty rather than as a
    // match, or the oracle above proves nothing about a table that failed to
    // create.
    assert!(
        roster(&conn, "products_pii_allowlist_no_such_table")
            .await
            .is_empty(),
        "the oracle reads the real catalog"
    );
}

/// **Both indexes this migration ships carry their predicates on Postgres.**
///
/// Read as text rather than by existence: an index created without its
/// `WHERE` clause exists under the right name and enforces the wrong rule,
/// which is precisely the failure a name check cannot see.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_allowlist_unique_is_partial_and_the_tombstone_index_is_not() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    let unique = indexdef(&conn, "uq_products_pii_allowlist_active").await;
    assert!(
        unique.contains("UNIQUE") && unique.contains("WHERE (state = 'active'"),
        "the uniqueness must be scoped to the active rows, or a revoked entry blocks its own \
         re-sign-off forever: {unique}"
    );

    let tombstone = indexdef(&conn, "idx_products_identity_ref_principal_tombstone").await;
    assert!(
        tombstone.contains("tombstoned_at") && !tombstone.contains("WHERE"),
        "P-D-118 item 18's index must be TOTAL: the compliance export walks a principal's \
         tombstoned refs, and a partial index excluding them is the covering index that \
         already exists: {tombstone}"
    );
}

/// **The partial unique refuses a second active entry and admits one after a
/// revoke — on the engine whose partial-index semantics the rule rides.**
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_second_active_value_is_refused_on_postgres_and_admitted_after_a_revoke() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    let seed = |state: &str| {
        format!(
            "INSERT INTO bss.products_pii_allowlist \
             (tenant_id, entry_id, value_normalized, justification, signed_off_by, \
              signed_off_at, state, created_at, updated_at) \
             VALUES ('00000000-0000-0000-0000-00000000c0de', gen_random_uuid(), \
              'ann fritz', 'founder', 'legal-1', now(), '{state}', now(), now())"
        )
    };
    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        seed("active"),
    ))
    .await
    .expect("the first active entry lands");

    let err = conn
        .execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            seed("active"),
        ))
        .await
        .expect_err("a second ACTIVE entry for one value must be refused");
    assert!(
        err.to_string().contains("uq_products_pii_allowlist_active"),
        "the refusal must be the partial index's, by name: {err}"
    );

    // The other arm: a revoked duplicate is outside the predicate and lands.
    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        seed("revoked"),
    ))
    .await
    .expect("a revoked row for the same value is outside the partial predicate");
}

/// **The `state` CHECK is closed at two, on Postgres.**
///
/// A third value would be a state nothing in the crate reads and every
/// `state = 'active'` predicate would silently exclude.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_state_check_is_closed_at_two_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    let err = conn
        .execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "INSERT INTO bss.products_pii_allowlist \
             (tenant_id, entry_id, value_normalized, justification, signed_off_by, \
              signed_off_at, state, created_at, updated_at) \
             VALUES ('00000000-0000-0000-0000-00000000c0df', gen_random_uuid(), \
              'unrostered', 'x', 'legal-9', now(), 'suspended', now(), now())"
                .to_owned(),
        ))
        .await
        .expect_err("an unrostered state must be refused");
    assert!(
        err.to_string().contains("chk_products_pii_allowlist_state"),
        "the refusal must be the CHECK's, by name: {err}"
    );
}
