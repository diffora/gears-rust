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
