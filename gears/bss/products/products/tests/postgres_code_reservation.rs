//! The code reservations' serialization points, on real Postgres — the
//! `product_code` index and its `sku_code` twin.
//!
//! Both halves live here because the plan counts them as one item ("the
//! reservation index on **both** code columns") and because the two doors have
//! a history of diverging when their cases were split: Phase 6 sliced them
//! apart and they parted ways six times. Whatever one door's loser is told,
//! the case for the other is on the same screen.
//!
//! # Why this cannot be a `SQLite` suite
//!
//! `sqlite::memory:` is private to its connection, and the in-crate suites pin
//! `max_conns: Some(1)` because of it. A second writer therefore queues on the
//! *pool* and is handed the connection only after the first has finished — so
//! what those suites measure is one writer twice, never two at once. The
//! interleaving under test can be neither confirmed nor refuted there.
//!
//! `repo_tests`'s create cases prove that a duplicate `product_code` is refused
//! when the holder is already committed. What they cannot show is what happens
//! when two inserts of one code are **in flight at the same time**, which is the
//! case `dod-code-reservation` names: *"refusing the loser of a concurrent race
//! ... with an audited reason"*.
//!
//! # The choreography, and why each step is there
//!
//! A concurrency test that starts two tasks and asserts on the outcome is a coin
//! toss with a green side. This one is driven by observable events only, in the
//! idiom `gears/bss/pricing`'s `postgres_approval_race.rs` established:
//!
//! 1. the winner inserts its row and then **parks**, holding the index entry for
//!    the contested code on an uncommitted transaction;
//! 2. the loser starts; its `INSERT` reaches `uq_products_product_code`, finds
//!    the pending entry, and blocks on the winner's transaction id;
//! 3. a third connection **observes the block** in `pg_locks`, which is what
//!    proves the loser's insert actually reached the index;
//! 4. only then is the winner released to commit, and the loser's insert
//!    re-evaluates, finds a committed duplicate, and is refused.
//!
//! Step 3 is the load-bearing one. Without it the loser could start after the
//! winner had already committed and be refused by an ordinary, uncontended
//! duplicate check — green, and about nothing that `dod-concurrency` asks.
//!
//! # Why the two rows differ in everything except the code
//!
//! `products_product` carries **two** partial unique indexes, and a naive pair
//! of identical rows would collide on `uq_products_product_name` as well. Which
//! of the two Postgres names in the failure is then its own choice, and the
//! assertion below would be measuring that choice rather than the reservation.
//! The two rows here share `tenant_id` and `product_code` and differ in
//! `product_id`, `name` and `name_normalized`, so the code index is the only one
//! that can refuse either of them.
//!
//! # What this proves that no existing test does
//!
//! `api::rest::products::classify_insert_conflict` tells `DUPLICATE_CODE` from
//! `DUPLICATE_NAME` by substring-matching the driver's own error text, and its
//! own doc records that the suite exercising it runs on `SQLite` alone. The two
//! engines word this failure differently — Postgres names the constraint
//! (`uq_products_product_code`), `SQLite` lists the covered columns — so the
//! Postgres half of that classifier has never been executed. This suite executes
//! it, against the exact string the create door classifies: the door calls
//! `classify_insert_conflict(&db_error.to_string())`, and what is asserted below
//! is the rendering of the error `insert_product` returns for the same failure.
//!
//! The classifier itself is private to its module and unreachable from an
//! integration test, so what is asserted here is its **premise**: that the
//! Postgres text carries the two substrings it keys off, and does not carry the
//! one that would send this failure down the `DUPLICATE_NAME` arm.
//!
//! # What the engine actually says, measured
//!
//! The rendering this suite judges, recorded so a reader need not stand a
//! container up to see it:
//!
//! ```text
//! products repo db error: insert product <uuid>: query error: error returned
//! from database: duplicate key value violates unique constraint
//! "uq_products_product_code"
//! ```
//!
//! Traced through the classifier: `unique constraint` opens its
//! unique-violation gate, and `product_code` matches — **but only as a substring
//! of the constraint's name**, since Postgres names the index and never the
//! columns. That is precisely the fragility the classifier's own doc warns
//! about, now measured rather than reasoned about: renaming
//! `uq_products_product_code`, or adding any index to this table whose name
//! contains `product_code`, moves this answer with no test but this one to
//! notice.
//!
//! Ignored by default; it needs Docker. Run with
//! `cargo test -p cf-gears-bss-products --test postgres_code_reservation -- --ignored`.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-concurrency:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use std::sync::Arc;
use std::time::Duration;

use bss_products::infra::storage::RepoError;
use bss_products::infra::storage::repo::{self, NewProduct, NewSku};
use chrono::{DateTime, TimeZone, Utc};
use pg_support::Pg;
use tokio::sync::Notify;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_42);
const BRAND: Uuid = Uuid::from_u128(0xb2_a0);
const WINNER: Uuid = Uuid::from_u128(0x_1111);
const LOSER: Uuid = Uuid::from_u128(0x_2222);
const PARENT: Uuid = Uuid::from_u128(0x_3333);
const SKU_WINNER: Uuid = Uuid::from_u128(0x_4444);
const SKU_LOSER: Uuid = Uuid::from_u128(0x_5555);

/// The contested reservation. One value, both racers.
const CODE: &str = "FIBRE-500";

/// The SKU half's contested reservation, on `uq_products_sku_code`.
const SKU_CODE: &str = "FIBRE-500-STD";

/// Long enough that a loaded machine is not a failure, short enough that a
/// genuine deadlock is not a hung suite.
const RACE_TIMEOUT: Duration = Duration::from_secs(30);

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 30, hour, 0, 0).unwrap()
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

/// A Product carrying the contested code, distinguishable from its rival in
/// every other indexed column. See the module doc for why that matters.
fn contender(product_id: Uuid, name: &str, name_normalized: &str) -> NewProduct {
    NewProduct {
        product_id,
        tenant_id: TENANT,
        brand_id: BRAND,
        name: name.to_owned(),
        name_normalized: name_normalized.to_owned(),
        product_code: Some(CODE.to_owned()),
        region_scope: "eu,apac".to_owned(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: at(9),
        cloned_from: None,
        cloned_from_version: None,
    }
}

/// Two creates of one `product_code`, in flight at once: exactly one row
/// survives, and the loser's refusal is the code index's, not the name index's
/// and not a fault.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn two_creates_of_one_code_contend_on_the_reservation_index_and_one_is_refused() {
    let pg = Pg::applied().await;

    let observer = pg.raw().await;
    let written = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    // The winner: insert, then park holding the pending index entry for `CODE`.
    let winner = {
        let db = pg.db().await;
        let (written, release) = (Arc::clone(&written), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        repo::insert_product(
                            txn,
                            &scope(),
                            contender(WINNER, "Fibre 500", "fibre 500"),
                        )
                        .await?;
                        written.notify_one();
                        release.notified().await;
                        Ok(())
                    })
                })
                .await;
            out
        })
    };

    written.notified().await;

    // The loser: a different Product by every measure except the reserved code.
    // Its `INSERT` blocks on the winner's transaction id inside
    // `uq_products_product_code`.
    let loser = {
        let db = pg.db().await;
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        repo::insert_product(
                            txn,
                            &scope(),
                            contender(LOSER, "Copper 100", "copper 100"),
                        )
                        .await
                        .map(|_| ())
                    })
                })
                .await;
            out
        })
    };

    // The loser's insert has reached the index and is waiting. Only now is the
    // winner allowed to commit. Without this the loser could arrive after the
    // commit and never contend at all.
    pg_support::wait_until_a_backend_blocks(&observer).await;
    release.notify_one();

    tokio::time::timeout(RACE_TIMEOUT, winner)
        .await
        .expect("the winner must finish once released")
        .expect("its task must not panic")
        .expect("the winner is uncontended and must commit");

    let refusal = tokio::time::timeout(RACE_TIMEOUT, loser)
        .await
        .expect("the loser must be released by the winner's commit")
        .expect("its task must not panic")
        .expect_err("the code was reserved under it");

    // `in_transaction` wraps the body's error, so the repository's own error is
    // unwrapped before it is judged: an `Infra` here would be the store failing
    // rather than the index refusing, which is the distinction this assertion is
    // about.
    let refusal = refusal.into_domain(|infra| RepoError::Db(format!("race transaction: {infra}")));

    // The premise `classify_insert_conflict` rests on, measured on the engine
    // whose wording it has never been run against. The door classifies
    // `db_error.to_string()`; this is the same rendering, of the same failure.
    let rendered = refusal.to_string().to_ascii_lowercase();
    assert!(
        rendered.contains("unique constraint") || rendered.contains("duplicate key"),
        "the classifier's unique-violation gate would not open on this text: {rendered}"
    );
    assert!(
        rendered.contains("product_code"),
        "the classifier reads DUPLICATE_CODE off this substring and it is absent: {rendered}"
    );
    assert!(
        !rendered.contains("name_normalized"),
        "the name index's token appears in a code collision's text, so the two arms are \
         not separable on this engine: {rendered}"
    );

    // And exactly one holder of the reservation survives: the winner's.
    let conn = pg.raw().await;
    let holders = surviving_holders(&conn).await;
    assert_eq!(
        holders,
        vec![WINNER.to_string()],
        "the reservation admitted other than exactly one winner"
    );
}

/// The `product_id`s that hold [`CODE`] under the index's own predicate.
///
/// Read past the repository on purpose: what is being checked is the state of
/// the table the index guards, and a repository read could only report what its
/// own scoping let it see.
async fn surviving_holders(conn: &sea_orm::DatabaseConnection) -> Vec<String> {
    use sea_orm::{ConnectionTrait, Statement};

    conn.query_all_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT product_id::text AS id
               FROM bss.products_product
              WHERE product_code = '{CODE}' AND lifecycle_state <> 'discarded'
              ORDER BY id"
        ),
    ))
    .await
    .expect("read the surviving holders")
    .iter()
    .map(|row| row.try_get::<String>("", "id").expect("read the id"))
    .collect()
}

/// A SKU carrying the contested code under the one parent.
///
/// Unlike its Product twin the two contenders need differ in nothing but their
/// own id: `products_sku` carries **one** partial unique index, so `sku_code`
/// is the only thing either row can collide on and there is no second index to
/// confuse the failure with.
fn sku_contender(sku_id: Uuid) -> NewSku {
    NewSku {
        sku_type: "bundle".to_owned(),
        sellable: true,
        plan_tier: "standard".to_owned(),
        tax_category_ref: None,
        gl_code_ref: None,
        sku_id,
        tenant_id: TENANT,
        product_id: PARENT,
        sku_code: SKU_CODE.to_owned(),
        region_scope: "eu".to_owned(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: at(9),
        cloned_from: None,
        cloned_from_version: None,
    }
}

/// Commit the parent every SKU below hangs from, before any racing starts.
///
/// Committed rather than raced: a parent inserted inside one of the racing
/// transactions would make the *other* racer's foreign key the thing that
/// blocked, and the probe would be measuring the wrong lock.
async fn seed_parent(pg: &Pg) {
    let db = pg.db().await;
    let (_db, out) = db
        .in_transaction::<(), RepoError, _>(move |txn| {
            Box::pin(async move {
                // Deliberately **code-less**: a parent holding the contested
                // `CODE` would put an irrelevant reservation in the same
                // database and leave a reader wondering which index this test
                // is about.
                let parent = NewProduct {
                    product_code: None,
                    ..contender(PARENT, "Parent", "parent")
                };
                repo::insert_product(txn, &scope(), parent)
                    .await
                    .map(|_| ())
            })
        })
        .await;
    out.expect("the parent Product must commit before the SKUs race for a code");
}

/// **The SKU half of the same reservation invariant.**
///
/// The Product and SKU doors were built by parallel slices and diverged six
/// times in Phase 6. `uq_products_sku_code` is a second physical index with its
/// own name, its own predicate and its own driver text, so a probe on the
/// Product side proves nothing about it: the classifier reads a **substring**,
/// and `uq_products_sku_code` contains neither `product_code` nor
/// `name_normalized`. This is the case that says what the SKU door's loser is
/// actually told.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn two_creates_of_one_sku_code_contend_on_the_reservation_index_and_one_is_refused() {
    let pg = Pg::applied().await;
    seed_parent(&pg).await;

    let observer = pg.raw().await;
    let written = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    let winner = {
        let db = pg.db().await;
        let (written, release) = (Arc::clone(&written), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        repo::insert_sku(txn, &scope(), sku_contender(SKU_WINNER)).await?;
                        written.notify_one();
                        release.notified().await;
                        Ok(())
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
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        repo::insert_sku(txn, &scope(), sku_contender(SKU_LOSER))
                            .await
                            .map(|_| ())
                    })
                })
                .await;
            out
        })
    };

    pg_support::wait_until_a_backend_blocks(&observer).await;
    release.notify_one();

    tokio::time::timeout(RACE_TIMEOUT, winner)
        .await
        .expect("the winner must finish once released")
        .expect("its task must not panic")
        .expect("the winner is uncontended and must commit");

    let refusal = tokio::time::timeout(RACE_TIMEOUT, loser)
        .await
        .expect("the loser must be released by the winner's commit")
        .expect("its task must not panic")
        .expect_err("the code was reserved under it");

    let refusal = refusal.into_domain(|infra| RepoError::Db(format!("race transaction: {infra}")));
    let rendered = refusal.to_string().to_ascii_lowercase();

    assert!(
        rendered.contains("unique constraint") || rendered.contains("duplicate key"),
        "the classifier's unique-violation gate would not open on this text: {rendered}"
    );
    // The SKU door tells its own index apart by `sku_code`, the way the Product
    // door does by `product_code`. Postgres names the constraint, so the
    // substring is present through `uq_products_sku_code`.
    assert!(
        rendered.contains("sku_code"),
        "the SKU door reads DUPLICATE_CODE off this substring and it is absent: {rendered}"
    );

    let conn = pg.raw().await;
    let holders = surviving_sku_holders(&conn).await;
    assert_eq!(
        holders,
        vec![SKU_WINNER.to_string()],
        "the SKU reservation admitted other than exactly one winner"
    );
}

/// The `sku_id`s holding [`SKU_CODE`] under `uq_products_sku_code`'s own
/// predicate.
async fn surviving_sku_holders(conn: &sea_orm::DatabaseConnection) -> Vec<String> {
    use sea_orm::{ConnectionTrait, Statement};

    conn.query_all_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT sku_id::text AS id
               FROM bss.products_sku
              WHERE sku_code = '{SKU_CODE}' AND lifecycle_state <> 'discarded'
              ORDER BY id"
        ),
    ))
    .await
    .expect("read the surviving holders")
    .iter()
    .map(|row| row.try_get::<String>("", "id").expect("read the id"))
    .collect()
}
