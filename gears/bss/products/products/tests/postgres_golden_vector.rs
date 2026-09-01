//! The cross-engine half of the canonical-serialization golden vector
//! (`cpt-cf-bss-products-dod-version-history-table`: the rendering *"MUST be
//! pinned by a golden vector asserted byte-identical across engines under the
//! `digest_version` it was computed with"*).
//!
//! # What the in-crate vector cannot prove, and this file does
//!
//! `domain::canonical_tests`' golden vector pins the rendering and the digest
//! as literals, in pure `Rust`, and is engine-independent by construction —
//! so it proves the *renderer* stable and nothing about *storage*. The clause
//! above is about the round trip: the same bytes written into
//! `products_entity_version` on Postgres must read back exactly as they do on
//! `SQLite`. The two halves are the same literals, transcribed once in each
//! suite from `canonical_tests`' own vector, so a drift on either engine
//! fails on that engine and names itself.
//!
//! This was **unassertable until P-D-82**: `Utc::now()` carries nanoseconds,
//! `SQLite` stores nine digits and Postgres `timestamptz` **rounds** to six,
//! so the same logical entity could freeze under two `content` strings. The
//! instants below are literal microsecond values inside the content string —
//! they never pass through either engine's timestamp type — and the head
//! rows' own instants now truncate at the write, which is what closed the
//! hazard for the columns that do.
//!
//! Run under `make test-products-pg`; skipped like every other file here when
//! no engine is reachable.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-version-history-table:p1

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, FromQueryResult as _, Statement};

const TENANT: &str = "00000000-0000-0000-0000-0000000067a1";
const ENTITY: &str = "3f8f6a1e-0000-4000-8000-000000000001";
const ACTOR: &str = "00000000-0000-0000-0000-00000000ac70";

/// The golden vector, transcribed from
/// `domain::canonical_tests::the_golden_vector_pins_the_rendering_and_the_digest_independently`.
/// Not imported: this suite asserts that the *stored* bytes equal the vector,
/// and importing the constant from the code under test would let one drift
/// carry both sides.
const GOLDEN_CONTENT: &str = "{\"brand_id\":\"3f8f6a1e-0000-4000-8000-000000000001\",\
     \"name\":\"Fibre 500\",\"product_code\":null,\
     \"published_at\":\"2026-08-27T11:04:05.123456Z\",\"weight_kg\":1.5}";

/// Its `SHA-256`, hex — the same value the in-crate vector pins and the same
/// one `printf … | sha256sum` reproduces outside `Rust`.
const GOLDEN_DIGEST_HEX: &str = "e252632893610a1207b4844a24a1aec1682c8a4b7b5242bd7a26b082b1e77c35";

/// The digest scheme the vector was computed under (`canonical::DIGEST_VERSION`).
const GOLDEN_DIGEST_VERSION: i32 = 1;

/// The three columns the round trip reads back.
#[derive(Debug, sea_orm::FromQueryResult)]
struct Row {
    content: String,
    digest_hex: String,
    digest_version: i32,
}

/// The golden vector survives a Postgres round trip byte for byte: the
/// `content` string, the `content_digest` bytes and the `digest_version` that
/// governs both come back exactly as written.
///
/// Written through raw SQL rather than the repository, deliberately: the
/// claim is about what the **engine** stores, and a probe routed through the
/// gear's own encoder would pass on an engine that mangled the bytes as long
/// as the encoder mangled them symmetrically.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_golden_vector_survives_a_postgres_round_trip() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;

    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "INSERT INTO bss.products_entity_version (tenant_id, entity_kind, entity_id, \
             published_version, content, content_digest, digest_version, approval_ref, \
             actor_ref, published_at) VALUES ('{TENANT}', 'product', '{ENTITY}', 1, \
             $${GOLDEN_CONTENT}$$, decode('{GOLDEN_DIGEST_HEX}', 'hex'), \
             {GOLDEN_DIGEST_VERSION}, NULL, '{ACTOR}', '2026-08-27T11:04:05.123456Z')"
        ),
    ))
    .await
    .expect("freeze the golden vector");

    let row = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT content, encode(content_digest, 'hex') AS digest_hex, digest_version \
             FROM bss.products_entity_version WHERE tenant_id = '{TENANT}' AND entity_id = \
             '{ENTITY}'"
        ),
    ))
    .one(&conn)
    .await
    .expect("read the frozen row")
    .expect("the row this test just wrote must exist");

    assert_eq!(
        row.content, GOLDEN_CONTENT,
        "Postgres must store the canonical rendering byte for byte; a difference here is a \
         cross-engine digest divergence, which is what the flagship exists to catch"
    );
    assert_eq!(
        row.digest_hex, GOLDEN_DIGEST_HEX,
        "and the digest bytes with it"
    );
    assert_eq!(
        row.digest_version, GOLDEN_DIGEST_VERSION,
        "under the scheme the vector was computed with, which is the clause's own qualifier"
    );
}
