//! The Postgres half of governance's schema oracle
//! (`cpt-cf-bss-products-dod-approval-store`: *"A schema-oracle golden **MUST**
//! exist on **both engines** with a perturbation case proving it can fail"*).
//!
//! # What the `SQLite` half cannot prove, and this file does
//!
//! `migrations_tests::governance_store_guard_tests` pins the three rosters
//! against `pragma_table_info`, which is `SQLite`'s catalog and reports
//! `SQLite`'s columns. The clause above is about the pair: a column added to
//! one engine's statement array and not the other is exactly the defect the
//! two-array migration shape makes easy, and no `SQLite` test can see it.
//! Here the same three rosters are read from `information_schema` — a
//! **different catalog, on a different engine, against the same literals** —
//! so a divergence fails on this side and names itself.
//!
//! The oracle also pins each column's **nullability**, because the pair that
//! matters most is not the name list: `content_snapshot` and
//! `quorum_descriptor` are `NOT NULL` on purpose (stored at submission, never
//! re-derived), and a nullable copy on one engine would let a row exist that
//! the other engine refuses.
//!
//! Run under `make test-products-pg`; skipped like every other file here when
//! no engine is reachable.
//!
//! Of the three tables this oracle pins, only `dod-breakglass-store` is
//! ticked: `dod-approval-store` waits on §7 rows 9, 11 and 14 and
//! `dod-decision-store` on row 6, so their halves of this file are coverage
//! without a tick.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-breakglass-store:p1

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

/// The three golden rosters, transcribed from the migration's own statement
/// arrays — `(column, nullable)` pairs, ordered by name so the comparison is
/// insensitive to declaration order.
const APPROVAL: &[(&str, bool)] = &[
    ("approval_id", false),
    ("author_override_ack", true),
    ("author_override_ack_at", true),
    ("content_snapshot", false),
    ("diff_basis", true),
    ("finalized_at", true),
    ("internal_revision", false),
    ("quorum_descriptor", false),
    ("state", false),
    ("subject_kind", false),
    ("subject_ref", false),
    ("submitted_at", false),
    ("submitter", false),
    ("tenant_id", false),
];

const DECISION: &[(&str, bool)] = &[
    ("approval_id", false),
    ("approver_principal", false),
    ("decided_at", false),
    ("override_acknowledgments", true),
    ("reason", true),
    ("tenant_id", false),
    ("verdict", false),
];

const SESSION: &[(&str, bool)] = &[
    // P-D-133 row 9: the elevation's two platform approvers, `NULL` on the
    // post-hoc path (strand B, `c1b86fcbb`; the oracle caught up on canon
    // because the strand cannot run the Docker tier).
    ("approver_a", true),
    ("approver_b", true),
    ("expired_emitted", false),
    ("opened_at", false),
    ("posthoc_state", true),
    ("principal", false),
    ("reason", false),
    ("reviewed_at", true),
    ("reviewed_by", true),
    ("session_id", false),
    ("target_tenant", false),
    ("two_person_approval_ref", true),
    ("valid_from", false),
    ("valid_until", false),
];

/// `products_materiality_policy`'s roster (**P-D-112** arm 1).
///
/// Every column is `NOT NULL`, and that is the shape arm 2 needs: *"an absent
/// row resolves to the default"*, so a tenant with no policy has **no row**
/// rather than a row of nulls. A nullable column here would mint a third
/// state between "no row" and "a policy", and the resolver has only two.
const MATERIALITY_POLICY: &[(&str, bool)] = &[
    ("affected_entity_trigger", false),
    ("approver_count", false),
    ("field_set", false),
    ("tenant_id", false),
    ("updated_at", false),
    ("updated_by", false),
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
async fn the_governance_rosters_match_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    assert_eq!(
        roster(&conn, "products_approval").await,
        golden(APPROVAL),
        "products_approval's Postgres roster must equal the SQLite one, nullability included"
    );
    assert_eq!(
        roster(&conn, "products_approval_decision").await,
        golden(DECISION)
    );
    assert_eq!(
        roster(&conn, "products_materiality_policy").await,
        golden(MATERIALITY_POLICY),
        "every column is NOT NULL: P-D-112 arm 2 gives a tenant with no policy no row at all, \
         so a nullable column would be a third state the resolver does not have"
    );
    assert_eq!(
        roster(&conn, "products_breakglass_session").await,
        golden(SESSION)
    );

    // The perturbation, run against the live catalog rather than a mock: the
    // oracle must FAIL when compared with a roster it was not written for,
    // and must report an absent table as empty rather than as a match.
    assert_ne!(
        roster(&conn, "products_approval").await,
        golden(DECISION),
        "two different rosters must not compare equal, or this oracle asserts nothing"
    );
    assert!(
        roster(&conn, "products_approval_no_such_table")
            .await
            .is_empty(),
        "the oracle reads the real catalog: an absent table has no columns"
    );
}

/// The stored-not-derived pair, asserted as a physical fact rather than as a
/// comment: a row missing either snapshot must be refused by the engine.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_stored_snapshots_are_not_nullable_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    for column in ["content_snapshot", "quorum_descriptor"] {
        let sql = format!(
            "INSERT INTO bss.products_approval \
             (tenant_id, approval_id, subject_kind, subject_ref, internal_revision, \
              content_snapshot, quorum_descriptor, state, submitter, submitted_at) \
             VALUES (gen_random_uuid(), gen_random_uuid(), 'entity_publish', 's-1', 1, \
              {}, {}, 'pending', gen_random_uuid(), now())",
            if column == "content_snapshot" {
                "NULL"
            } else {
                "'{}'"
            },
            if column == "quorum_descriptor" {
                "NULL"
            } else {
                "'{}'"
            },
        );
        let err = conn
            .execute_raw(Statement::from_string(
                sea_orm::DatabaseBackend::Postgres,
                sql,
            ))
            .await
            .expect_err("a null snapshot must be refused");
        assert!(
            err.to_string().contains(column),
            "{column} must be the refusal's subject: {err}"
        );
    }
}

/// **The two indexes P-D-110 arm 3 and P-D-111 routed to this migration
/// exist on Postgres**, and the `SQLite` mirror cannot see either.
///
/// Both were created for reads this gear makes and neither is enforced by any
/// constraint, so nothing else in the suite would notice their absence: an
/// index that is missing makes a query slower and never wrong, which is
/// exactly the class of defect a schema oracle exists to catch before
/// production does.
///
/// `idx_products_approval_gate` carries **no state predicate** on purpose —
/// `gate_candidates` is deliberately stateless so `PreAuthorized` can see
/// `consumed` rows, which is why `uq_products_approval_open`'s
/// `WHERE state IN (...)` cannot serve it. The assertion therefore checks the
/// predicate's *absence*, since an index with one would look present and
/// serve nothing.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_routed_indexes_exist_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    let gate = index_definition(&conn, "idx_products_approval_gate").await;
    assert!(
        gate.contains("tenant_id")
            && gate.contains("subject_kind")
            && gate.contains("subject_ref")
            && gate.contains("submitted_at"),
        "P-D-110 arm 3's index must carry all four columns: {gate}"
    );
    assert!(
        !gate.to_ascii_uppercase().contains(" WHERE "),
        "a state predicate would make this index unusable by the stateless read it exists for: \
         {gate}"
    );

    let elevation = index_definition(&conn, "idx_products_breakglass_two_person").await;
    assert!(
        elevation.contains("two_person_approval_ref"),
        "P-D-111's reverse lookup must be indexed: {elevation}"
    );
}

#[derive(Debug, sea_orm::FromQueryResult)]
struct IndexRow {
    indexdef: String,
}

/// One index's definition, or a panic naming the index that is missing.
async fn index_definition(conn: &impl ConnectionTrait, name: &str) -> String {
    IndexRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT indexdef FROM pg_indexes WHERE schemaname = 'bss' AND indexname = '{name}'"
        ),
    ))
    .one(conn)
    .await
    .expect("pg_indexes answers")
    .unwrap_or_else(|| panic!("{name} does not exist on Postgres"))
    .indexdef
}
