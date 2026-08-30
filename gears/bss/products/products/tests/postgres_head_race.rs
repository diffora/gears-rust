//! The head row's serialization point, on real Postgres: two writers meeting
//! on one `products_product` row.
//!
//! # The two contested resources this suite covers
//!
//! `dod-concurrency` names six. Two of them are this row:
//!
//! - **the `If-Match` draft race** — two saves presenting the same
//!   `internal_revision`;
//! - **publish-versus-edit** — a publish and a save presenting the same one.
//!
//! They share a file because they share the invariant: every mutating door on
//! this entity funnels through one guarded `UPDATE` whose filter pins
//! `internal_revision = <the caller's If-Match>`, and what both cases measure
//! is that the filter is re-evaluated **after** the winner commits rather than
//! against the snapshot the loser started from. Splitting them would let one
//! be repaired and the other left, which is exactly how the Product and SKU
//! doors drifted apart six times in Phase 6.
//!
//! # Why `SQLite` cannot host either
//!
//! The in-crate suites prove the guard refuses a *stale* revision — one already
//! known to be stale when the call is made. That is a different claim. The one
//! under test here is about two writers holding the **same, currently valid**
//! revision at the same instant, which `sqlite::memory:` cannot produce: it is
//! private to its connection and the suites pin `max_conns: Some(1)`, so the
//! second writer waits on the pool and by the time it runs the first has
//! finished. What that measures is one writer twice.
//!
//! # The choreography, and the step that makes it a race
//!
//! The winner performs its write and **parks inside its transaction**, holding
//! the row lock. The loser's `UPDATE` then blocks on that lock — it has already
//! matched its filter against the pre-commit snapshot at this point, which is
//! precisely the dangerous interleaving. A third connection observes the block
//! in `pg_locks`; only then is the winner released. Postgres re-evaluates the
//! loser's filter against the committed row under READ COMMITTED, the
//! `internal_revision` predicate no longer matches, and the write affects zero
//! rows — [`HeadWrite::Unmatched`], which the door renders `STALE_REVISION`.
//!
//! Without the observed block the loser could simply start after the winner
//! committed and be refused by an ordinary, uncontended staleness check: green,
//! and about nothing `dod-concurrency` asks.
//!
//! Ignored by default; it needs Docker. Run with
//! `cargo test -p cf-gears-bss-products --test postgres_head_race -- --ignored`.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-concurrency:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use std::sync::Arc;
use std::time::Duration;

use bss_products::infra::storage::RepoError;
use bss_products::infra::storage::repo::{
    self, HeadWrite, NewEntityVersion, NewProduct, ProductHeadSave, SavedName, VersionedEntityKind,
};
use chrono::{DateTime, TimeZone, Utc};
use pg_support::Pg;
use tokio::sync::Notify;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_42);
const BRAND: Uuid = Uuid::from_u128(0xb2_a0);
const SUBJECT: Uuid = Uuid::from_u128(0x_5b_1e);
const ACTOR: Uuid = Uuid::from_u128(0x_ac70);

/// The revision both racers present. The head carries it at the instant each
/// of them reads, which is what makes them a race rather than a retry.
const CONTESTED_REVISION: i64 = 1;

const RACE_TIMEOUT: Duration = Duration::from_secs(30);

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 30, hour, 0, 0).unwrap()
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

/// A save of the name alone.
///
/// **Bucket iii on purpose.** A save carrying a bucket-i column makes
/// `save_product_head` pin `published_version = 0` in its filter as well, and
/// the publish race below moves exactly that column — so a bucket-i save would
/// leave two reasons for the loser's filter to miss and the assertion could not
/// say which one fired. With only the name in play, `internal_revision` is the
/// single operand under test.
fn rename(to: &str, normalized: &str) -> ProductHeadSave {
    ProductHeadSave {
        brand_id: None,
        product_code: None,
        name: Some(SavedName {
            value: to.to_owned(),
            normalized: normalized.to_owned(),
        }),
        region_scope: None,
        brand_scope: None,
    }
}

/// Commit one `draft` Product at [`CONTESTED_REVISION`] for the racers to meet
/// on.
async fn seed_draft(pg: &Pg) {
    let db = pg.db().await;
    let (_db, out) = db
        .in_transaction::<(), RepoError, _>(move |txn| {
            Box::pin(async move {
                repo::insert_product(
                    txn,
                    &scope(),
                    NewProduct {
                        product_id: SUBJECT,
                        tenant_id: TENANT,
                        brand_id: BRAND,
                        name: "Fibre 500".to_owned(),
                        name_normalized: "fibre 500".to_owned(),
                        product_code: Some("FIBRE-500".to_owned()),
                        region_scope: "eu,apac".to_owned(),
                        brand_scope: String::new(),
                        created_by: "principal:author-1".to_owned(),
                        created_at: at(9),
                    },
                )
                .await
                .map(|_| ())
            })
        })
        .await;
    out.expect("the draft must commit before anything races for it");
}

/// The version row a publish's own head-row guard looks for.
///
/// The guard admits a `published_version` bump **only where the matching frozen
/// row already exists**, so this is written on the publisher's own transaction,
/// one statement ahead of the bump.
fn frozen(version: i64) -> NewEntityVersion {
    NewEntityVersion {
        tenant_id: TENANT,
        entity_kind: VersionedEntityKind::Product,
        entity_id: SUBJECT,
        published_version: version,
        content: r#"{"name":"Fibre 500","productCode":"FIBRE-500"}"#.to_owned(),
        content_digest: (1..=32_u8).collect(),
        digest_version: 1,
        approval_ref: None,
        actor_ref: ACTOR,
        published_at: at(10),
    }
}

/// The head's `internal_revision` as the database now holds it.
async fn head_revision(conn: &sea_orm::DatabaseConnection) -> i64 {
    use sea_orm::{ConnectionTrait, Statement};

    conn.query_one_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT internal_revision AS v FROM bss.products_product \
             WHERE product_id = '{SUBJECT}'"
        ),
    ))
    .await
    .expect("read the head")
    .expect("the head is there")
    .try_get::<i64>("", "v")
    .expect("read the revision")
}

/// **Two saves presenting one `If-Match`: exactly one lands.**
///
/// The loser is answered [`HeadWrite::Unmatched`] — the door's `STALE_REVISION`
/// — rather than overwriting the winner's name, and the head moves by exactly
/// one revision for the two attempts.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn two_saves_presenting_one_if_match_serialize_and_the_loser_is_refused() {
    let pg = Pg::applied().await;
    seed_draft(&pg).await;

    let observer = pg.raw().await;
    let written = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    let winner = {
        let db = pg.db().await;
        let (written, release) = (Arc::clone(&written), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<HeadWrite, RepoError, _>(move |txn| {
                    Box::pin(async move {
                        let outcome = repo::save_product_head(
                            txn,
                            &scope(),
                            TENANT,
                            SUBJECT,
                            CONTESTED_REVISION,
                            &rename("Fibre 500 Pro", "fibre 500 pro"),
                            at(11),
                        )
                        .await?;
                        written.notify_one();
                        release.notified().await;
                        Ok(outcome)
                    })
                })
                .await;
            out
        })
    };

    written.notified().await;

    let loser = {
        let db = pg.db().await;
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<HeadWrite, RepoError, _>(move |txn| {
                    Box::pin(async move {
                        repo::save_product_head(
                            txn,
                            &scope(),
                            TENANT,
                            SUBJECT,
                            CONTESTED_REVISION,
                            &rename("Fibre 500 Lite", "fibre 500 lite"),
                            at(11),
                        )
                        .await
                    })
                })
                .await;
            out
        })
    };

    pg_support::wait_until_a_backend_blocks(&observer).await;
    release.notify_one();

    let winner_outcome = tokio::time::timeout(RACE_TIMEOUT, winner)
        .await
        .expect("the winner must finish once released")
        .expect("its task must not panic")
        .expect("the winner is uncontended and must commit");
    assert_eq!(
        winner_outcome,
        HeadWrite::Applied,
        "the uncontended writer's own save must land"
    );

    let loser_outcome = tokio::time::timeout(RACE_TIMEOUT, loser)
        .await
        .expect("the loser must be released by the winner's commit")
        .expect("its task must not panic")
        .expect("a lost race is not an error at this layer, it is an unmatched write");
    assert_eq!(
        loser_outcome,
        HeadWrite::Unmatched,
        "the loser's filter must miss after the winner's commit, which the door renders \
         STALE_REVISION"
    );

    let conn = pg.raw().await;
    assert_eq!(
        head_revision(&conn).await,
        CONTESTED_REVISION + 1,
        "two attempts under one If-Match must move the head by exactly one revision"
    );
    // And the surviving name is the winner's: an `Unmatched` that had
    // nonetheless written would show up here and nowhere else.
    let name = surviving_name(&conn).await;
    assert_eq!(
        name, "Fibre 500 Pro",
        "the loser must not have written its own name under an unmatched filter"
    );
}

/// **A publish and a save presenting one `If-Match`: the publish lands and the
/// edit is refused.**
///
/// This is the case where the two writers are not symmetric — one bumps
/// `published_version` and freezes a version row, the other only renames — and
/// the guard is the same guard. What it proves is that an edit cannot slip in
/// against a revision the publish has already consumed, which is the whole of
/// "publish-versus-edit".
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_publish_and_an_edit_presenting_one_if_match_serialize_and_the_edit_is_refused() {
    let pg = Pg::applied().await;
    seed_draft(&pg).await;

    let observer = pg.raw().await;
    let written = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    let publisher = {
        let db = pg.db().await;
        let (written, release) = (Arc::clone(&written), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<HeadWrite, RepoError, _>(move |txn| {
                    Box::pin(async move {
                        // Freeze first, bump second: the head-row guard admits
                        // the bump only where the matching frozen row is
                        // already visible to it.
                        repo::insert_entity_version(txn, &scope(), frozen(1)).await?;
                        let outcome = repo::publish_product_head(
                            txn,
                            &scope(),
                            TENANT,
                            SUBJECT,
                            CONTESTED_REVISION,
                            at(11),
                        )
                        .await?;
                        written.notify_one();
                        release.notified().await;
                        Ok(outcome)
                    })
                })
                .await;
            out
        })
    };

    written.notified().await;

    let editor = {
        let db = pg.db().await;
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<HeadWrite, RepoError, _>(move |txn| {
                    Box::pin(async move {
                        repo::save_product_head(
                            txn,
                            &scope(),
                            TENANT,
                            SUBJECT,
                            CONTESTED_REVISION,
                            &rename("Fibre 500 Lite", "fibre 500 lite"),
                            at(11),
                        )
                        .await
                    })
                })
                .await;
            out
        })
    };

    pg_support::wait_until_a_backend_blocks(&observer).await;
    release.notify_one();

    let publish_outcome = tokio::time::timeout(RACE_TIMEOUT, publisher)
        .await
        .expect("the publisher must finish once released")
        .expect("its task must not panic")
        .expect("the publish is uncontended and must commit");
    assert_eq!(publish_outcome, HeadWrite::Applied, "the publish must land");

    let edited = tokio::time::timeout(RACE_TIMEOUT, editor)
        .await
        .expect("the edit must be released by the publish's commit")
        .expect("its task must not panic")
        .expect("a lost race is an unmatched write, not an error");
    assert_eq!(
        edited,
        HeadWrite::Unmatched,
        "an edit may not land against a revision the publish already consumed"
    );

    let conn = pg.raw().await;
    assert_eq!(
        head_revision(&conn).await,
        CONTESTED_REVISION + 1,
        "the publish moved the head once and the edit moved it not at all"
    );
    assert_eq!(
        surviving_name(&conn).await,
        "Fibre 500",
        "the edit's name must not be on a row it never matched"
    );
    assert_eq!(
        published_version(&conn).await,
        1,
        "the publish's own column must carry the version it froze"
    );
}

async fn surviving_name(conn: &sea_orm::DatabaseConnection) -> String {
    use sea_orm::{ConnectionTrait, Statement};

    conn.query_one_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!("SELECT name AS v FROM bss.products_product WHERE product_id = '{SUBJECT}'"),
    ))
    .await
    .expect("read the head")
    .expect("the head is there")
    .try_get::<String>("", "v")
    .expect("read the name")
}

async fn published_version(conn: &sea_orm::DatabaseConnection) -> i64 {
    use sea_orm::{ConnectionTrait, Statement};

    conn.query_one_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT published_version AS v FROM bss.products_product \
             WHERE product_id = '{SUBJECT}'"
        ),
    ))
    .await
    .expect("read the head")
    .expect("the head is there")
    .try_get::<i64>("", "v")
    .expect("read the published version")
}
