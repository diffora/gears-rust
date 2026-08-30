//! The idempotency store's two serialization points, on real Postgres.
//!
//! # The two contested resources this suite covers
//!
//! - **the claim insert** — two duplicates of one request arriving on a key
//!   nobody holds;
//! - **the expired-key takeover** — two duplicates arriving on a key whose
//!   holder has expired (**P-D-49**).
//!
//! One file because they are one row's two lifecycles — and because measuring
//! them together is what showed that they have the **same** serialization
//! point. Both cases were written expecting the second caller to block on the
//! row it was updating; the blocked statement was read out of
//! `pg_stat_activity` in both, and in both it is the `ON CONFLICT` **insert**.
//! The takeover case's own doc has the consequence, which is that
//! `IdempotencyClaim::TakeoverRaceLost` is not what a live race produces here.
//!
//! # What `SQLite` could not do here, in the suite's own words
//!
//! `repo_tests::the_expired_key_takeover_race_admits_exactly_one_winner` says
//! it: *"The interleaving is **simulated directly**"* — it calls the private
//! compare-and-swap twice against one hand-held stamp. That is a faithful model
//! of the race and it is not the race; it cannot fail for the reason a real one
//! would, because no second backend ever exists. `dod-concurrency` asks for
//! "real concurrency probes, not read-then-assert", and this is the file that
//! answers it: two backends, one row, a block observed in `pg_locks` before
//! either is allowed to finish.
//!
//! Both cases go through the **public** `claim_idempotency_key`.
//! `take_over_expired_idempotency_claim` is private, and reaching past the
//! entry point the doors actually call would prove something about a function
//! no door can invoke.
//!
//! Ignored by default; it needs Docker. Run with
//! `cargo test -p cf-gears-bss-products --test postgres_idempotency_race -- --ignored`.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-concurrency:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use std::sync::Arc;
use std::time::Duration;

use bss_products::infra::storage::RepoError;
use bss_products::infra::storage::repo::{self, IdempotencyClaim};
use chrono::{DateTime, TimeZone, Utc};
use pg_support::Pg;
use tokio::sync::Notify;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_42);
const ENDPOINT: &str = "POST /bss-products/v1/products";
const CLIENT_KEY: &str = "client-key-1";

/// The winner's payload digest, and the loser's — **deliberately different**.
///
/// P-D-49 turns on this: the takeover's loser "may even carry a different
/// payload from the winner, and is still refused in-flight rather than for the
/// mismatch, since this transaction never compared the two". Two equal hashes
/// would make the takeover case pass without ever exercising that rule.
const HASH_A: &[u8] = b"hash-a-0123456789abcdef0123456789";
const HASH_B: &[u8] = b"hash-b-0123456789abcdef0123456789";

const RACE_TIMEOUT: Duration = Duration::from_secs(30);

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 30, hour, 0, 0).unwrap()
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

/// One `claim_idempotency_key` call, on its own connection pool.
///
/// `park` is what makes a racer the winner: it holds the transaction open after
/// the claim has written, which is the state the other racer has to meet.
async fn claim(
    pg: &Pg,
    payload_hash: &'static [u8],
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    park: Option<(Arc<Notify>, Arc<Notify>)>,
) -> tokio::task::JoinHandle<Result<IdempotencyClaim, toolkit_db::secure::TxError<RepoError>>> {
    let db = pg.db().await;
    tokio::spawn(async move {
        let (_db, out) = db
            .in_transaction::<IdempotencyClaim, RepoError, _>(move |txn| {
                Box::pin(async move {
                    let outcome = repo::claim_idempotency_key(
                        txn,
                        &scope(),
                        TENANT,
                        ENDPOINT,
                        CLIENT_KEY,
                        payload_hash,
                        now,
                        expires_at,
                    )
                    .await?;
                    if let Some((written, release)) = park {
                        written.notify_one();
                        release.notified().await;
                    }
                    Ok(outcome)
                })
            })
            .await;
        out
    })
}

/// How many rows the key has. One, always — the assertion that catches a
/// takeover which inserted instead of updating.
async fn row_count(conn: &sea_orm::DatabaseConnection) -> i64 {
    use sea_orm::{ConnectionTrait, Statement};

    conn.query_one_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT count(*)::bigint AS v FROM bss.products_idempotency \
             WHERE tenant_id = '{TENANT}' AND client_key = '{CLIENT_KEY}'"
        ),
    ))
    .await
    .expect("count the key's rows")
    .expect("one row")
    .try_get::<i64>("", "v")
    .expect("read the count")
}

/// **Two duplicates claiming one fresh key: exactly one claims it, the other is
/// told in flight.**
///
/// The loser's `INSERT` blocks on the winner's *speculative insertion* — the
/// lock Postgres takes for an `ON CONFLICT` insert that has not committed yet —
/// which is the interleaving `SQLite` cannot produce. Once released, the
/// conflict swallows the loser's insert, it reads the row the winner just
/// committed, finds it live and `claimed`, and answers in flight.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn two_duplicates_claiming_one_fresh_key_admit_exactly_one() {
    let pg = Pg::applied().await;

    let observer = pg.raw().await;
    let written = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    let winner = claim(
        &pg,
        HASH_A,
        at(9),
        at(12),
        Some((Arc::clone(&written), Arc::clone(&release))),
    )
    .await;

    written.notified().await;

    let loser = claim(&pg, HASH_A, at(9), at(12), None).await;

    pg_support::wait_until_a_backend_blocks(&observer).await;
    release.notify_one();

    let winner_outcome = tokio::time::timeout(RACE_TIMEOUT, winner)
        .await
        .expect("the winner must finish once released")
        .expect("its task must not panic")
        .expect("the winner is uncontended and must claim");
    assert!(
        matches!(winner_outcome, IdempotencyClaim::Claimed),
        "the first arrival must hold the key: {winner_outcome:?}"
    );

    let loser_outcome = tokio::time::timeout(RACE_TIMEOUT, loser)
        .await
        .expect("the loser must be released by the winner's commit")
        .expect("its task must not panic")
        .expect("losing the insert is an outcome, not an error");
    match loser_outcome {
        IdempotencyClaim::InFlight { payload_hash } => {
            assert_eq!(
                payload_hash, HASH_A,
                "the in-flight refusal must carry the digest the *winner* recorded, since that \
                 is the row the loser read"
            );
        }
        other => panic!("the loser of the insert must be told in flight: {other:?}"),
    }

    let conn = pg.raw().await;
    assert_eq!(
        row_count(&conn).await,
        1,
        "two duplicates must leave one row, not two"
    );
}

/// **Two duplicates arriving on an expired key: exactly one takes it over, and
/// the loser executes nothing.**
///
/// # The serialization point is the insert, and that was a surprise
///
/// This case was written expecting [`IdempotencyClaim::TakeoverRaceLost`] and
/// it does not happen. The blocked statement was read out of
/// `pg_stat_activity` rather than assumed, and it is the **insert**:
///
/// ```text
/// INSERT INTO "products_idempotency" (...) VALUES (...)
///   ON CONFLICT ("tenant_id", "endpoint", "client_key") DO NOTHING RETURNING ...
/// ```
///
/// Postgres makes an `ON CONFLICT` insert wait on an in-progress transaction
/// that holds the conflicting row, so the second caller is serialized **before
/// it reads anything**. By the time its conflict resolves, the winner has
/// committed, and the row it then reads is the winner's fresh claim — live,
/// unexpired — so it never reaches the takeover path at all and answers in
/// flight.
///
/// # What that means for `TakeoverRaceLost`
///
/// The variant needs both callers to have *read the expired stamp* before
/// either writes. Through `claim_idempotency_key` that window cannot be forced
/// open: the call is atomic from outside, so a probe can park a caller only
/// after its whole claim — by which time the row is already locked and the
/// insert, not the update, is what the other caller meets. The interleaving is
/// reachable in principle (two inserts resolving before either update takes the
/// lock) but not *deterministically*, and a probe that hoped for it would be
/// the coin toss this suite exists to avoid.
///
/// So it is covered where it can be: `repo_tests`'s direct simulation drives
/// the private compare-and-swap twice against one held stamp. That test is not
/// redundant with this one and this one does not replace it — they measure
/// different interleavings, and **this** one is the interleaving a live system
/// actually produces.
///
/// # What is asserted, then
///
/// The property `dod-concurrency` actually asks for, which survives either
/// path: exactly one caller comes away holding the key, the other executed
/// nothing, and one row exists carrying the winner's payload. The loser here
/// carries `HASH_B` against the winner's `HASH_A`, so it is answered against a
/// hash it genuinely read — not P-D-49's fabricated verdict, which is what the
/// `TakeoverRaceLost` variant exists to prevent and which this interleaving
/// never risks.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn two_duplicates_taking_over_one_expired_key_admit_exactly_one() {
    let pg = Pg::applied().await;

    // The expired holder: claimed at 09:00, expiring at 10:00. Committed, so
    // both racers below meet a row that is genuinely stale rather than one held
    // open by a live transaction.
    let seeded = claim(
        &pg,
        b"hash-original-0000000000000000000",
        at(9),
        at(10),
        None,
    )
    .await
    .await
    .expect("the seeding task must not panic")
    .expect("seeding the expired holder must succeed");
    assert!(
        matches!(seeded, IdempotencyClaim::Claimed),
        "the seed must actually hold the key: {seeded:?}"
    );

    let observer = pg.raw().await;
    let written = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    // Both racers run at 11:00, an hour past the holder's expiry.
    let winner = claim(
        &pg,
        HASH_A,
        at(11),
        at(14),
        Some((Arc::clone(&written), Arc::clone(&release))),
    )
    .await;

    written.notified().await;

    let loser = claim(&pg, HASH_B, at(11), at(14), None).await;

    pg_support::wait_until_a_backend_blocks(&observer).await;
    release.notify_one();

    let winner_outcome = tokio::time::timeout(RACE_TIMEOUT, winner)
        .await
        .expect("the winner must finish once released")
        .expect("its task must not panic")
        .expect("the winner is uncontended and must take the key over");
    assert!(
        matches!(winner_outcome, IdempotencyClaim::Claimed),
        "the takeover's winner holds the key: {winner_outcome:?}"
    );

    let loser_outcome = tokio::time::timeout(RACE_TIMEOUT, loser)
        .await
        .expect("the loser must be released by the winner's commit")
        .expect("its task must not panic")
        .expect("losing the takeover is an outcome, not an error");
    // The loser executed nothing and did **not** take the key over a second
    // time. Which refusal it gets is decided by where the engine serialized it
    // — see this case's own doc — so both admissible shapes are named, and a
    // second `Claimed` is what this assertion is really guarding against.
    match loser_outcome {
        IdempotencyClaim::InFlight { payload_hash } => {
            assert_eq!(
                payload_hash, HASH_A,
                "the refusal must be measured against the row the loser actually read, which \
                 is the winner's committed claim"
            );
        }
        IdempotencyClaim::TakeoverRaceLost => {
            // The other admissible interleaving: both callers read the expired
            // stamp before either wrote. See the doc for why it cannot be
            // forced open here.
        }
        other => panic!("the loser must execute nothing and hold nothing: {other:?}"),
    }

    let conn = pg.raw().await;
    assert_eq!(
        row_count(&conn).await,
        1,
        "a takeover updates the held row; it must never insert a second"
    );
    assert_eq!(
        held_hash(&conn).await,
        HASH_A,
        "the surviving row must carry the winner's payload, not the loser's and not the \
         expired holder's"
    );
}

/// The digest the key's single row now holds.
async fn held_hash(conn: &sea_orm::DatabaseConnection) -> Vec<u8> {
    use sea_orm::{ConnectionTrait, Statement};

    conn.query_one_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT payload_hash AS v FROM bss.products_idempotency \
             WHERE tenant_id = '{TENANT}' AND client_key = '{CLIENT_KEY}'"
        ),
    ))
    .await
    .expect("read the held row")
    .expect("the row is there")
    .try_get::<Vec<u8>>("", "v")
    .expect("read the payload hash")
}
