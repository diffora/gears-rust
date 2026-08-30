//! The name index's serialization point, on real Postgres.
//!
//! # Why this is a suite of its own and not a case beside the code races
//!
//! `uq_products_product_name` is the *other* arm of
//! `api::rest::products::classify_insert_conflict`, and on Postgres it is the
//! arm that survives only because of its **second disjunct**. The classifier
//! reads:
//!
//! ```ignore
//! } else if lower.contains("name_normalized") || lower.contains("uq_products_product_name") {
//! ```
//!
//! On `SQLite` the driver lists the covered columns, so `name_normalized`
//! matches and the first disjunct carries the arm. On Postgres the driver names
//! the **constraint**, and `uq_products_product_name` does not contain
//! `name_normalized` anywhere — it contains `product_name`. So on this engine
//! the first disjunct never fires and the whole arm rests on the second, which
//! no test has ever executed. Delete that second disjunct and every `SQLite`
//! case stays green while Postgres starts answering `None` — an unrelated
//! storage failure, a `500`, for an ordinary duplicate name.
//!
//! That is the same class of defect as the `json`-column publish failure of
//! Phase 6: correct on the mirror, inoperative on the engine production runs.
//!
//! # Why the two rows differ in their codes
//!
//! `products_product` carries two partial unique indexes. The pair here shares
//! `tenant_id`, `brand_id` and `name_normalized` and differs in `product_id`
//! and `product_code`, so the **name** index is the only one either row can
//! collide on. A pair sharing both operands would let Postgres choose which
//! constraint to name, and this suite would be measuring that choice.
//!
//! # The choreography
//!
//! As `postgres_code_reservation`: the winner inserts and parks holding the
//! pending index entry, the loser's insert blocks on it, a third connection
//! observes the block in `pg_locks` — which is what proves the loser reached
//! the index rather than arriving after the winner had committed — and only
//! then is the winner released.
//!
//! Ignored by default; it needs Docker. Run with
//! `cargo test -p cf-gears-bss-products --test postgres_name_reservation -- --ignored`.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-concurrency:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use std::sync::Arc;
use std::time::Duration;

use bss_products::infra::storage::RepoError;
use bss_products::infra::storage::repo::{self, NewProduct};
use chrono::{DateTime, TimeZone, Utc};
use pg_support::Pg;
use tokio::sync::Notify;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_42);
const BRAND: Uuid = Uuid::from_u128(0xb2_a0);
const WINNER: Uuid = Uuid::from_u128(0x_1111);
const LOSER: Uuid = Uuid::from_u128(0x_2222);

/// The contested name, already normalized as the door would hand it over.
const NAME: &str = "Fibre 500";
const NAME_NORMALIZED: &str = "fibre 500";

const RACE_TIMEOUT: Duration = Duration::from_secs(30);

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 30, hour, 0, 0).unwrap()
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

/// A Product carrying the contested name under the same brand, and its **own**
/// code so the code index cannot be what refuses it.
fn contender(product_id: Uuid, code: &str) -> NewProduct {
    NewProduct {
        product_id,
        tenant_id: TENANT,
        brand_id: BRAND,
        name: NAME.to_owned(),
        name_normalized: NAME_NORMALIZED.to_owned(),
        product_code: Some(code.to_owned()),
        region_scope: "eu,apac".to_owned(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: at(9),
    }
}

/// Two creates of one name, in flight at once: exactly one row survives, and
/// the loser's refusal is the **name** index's, in a form the classifier's
/// second disjunct can still read.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn two_creates_of_one_name_contend_on_the_name_index_and_one_is_refused() {
    let pg = Pg::applied().await;

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
                        repo::insert_product(txn, &scope(), contender(WINNER, "FIBRE-500-A"))
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

    let loser = {
        let db = pg.db().await;
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        repo::insert_product(txn, &scope(), contender(LOSER, "FIBRE-500-B"))
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
        .expect_err("the name was reserved under it");

    let refusal = refusal.into_domain(|infra| RepoError::Db(format!("race transaction: {infra}")));
    let rendered = refusal.to_string().to_ascii_lowercase();

    assert!(
        rendered.contains("unique constraint") || rendered.contains("duplicate key"),
        "the classifier's unique-violation gate would not open on this text: {rendered}"
    );

    // **The whole point of this suite.** The classifier's name arm is
    // `contains("name_normalized") || contains("uq_products_product_name")`,
    // and on this engine only the second can match. Both halves are asserted
    // separately so a future driver that started listing columns would show up
    // as a changed measurement rather than as a silent pass.
    assert!(
        rendered.contains("uq_products_product_name"),
        "Postgres must name the constraint, which is the only thing the name arm can match \
         on this engine: {rendered}"
    );
    assert!(
        !rendered.contains("name_normalized"),
        "if this driver has started listing covered columns, the arm's first disjunct now \
         carries it too and this suite's premise needs re-reading: {rendered}"
    );
    // And it must not be mistakable for the code index's failure: the
    // classifier tests `product_code` **first**, so a name collision whose text
    // contained that substring would be answered DUPLICATE_CODE.
    assert!(
        !rendered.contains("product_code"),
        "a name collision whose text names product_code would be classified as the wrong \
         refusal: {rendered}"
    );

    let conn = pg.raw().await;
    let holders = surviving_holders(&conn).await;
    assert_eq!(
        holders,
        vec![WINNER.to_string()],
        "the name reservation admitted other than exactly one winner"
    );
}

/// The `product_id`s holding the contested name under the index's own
/// predicate.
async fn surviving_holders(conn: &sea_orm::DatabaseConnection) -> Vec<String> {
    use sea_orm::{ConnectionTrait, Statement};

    conn.query_all_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT product_id::text AS id
               FROM bss.products_product
              WHERE name_normalized = '{NAME_NORMALIZED}' AND lifecycle_state <> 'discarded'
              ORDER BY id"
        ),
    ))
    .await
    .expect("read the surviving holders")
    .iter()
    .map(|row| row.try_get::<String>("", "id").expect("read the id"))
    .collect()
}
