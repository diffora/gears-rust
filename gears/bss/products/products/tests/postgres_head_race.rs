//! The head row's serialization point, on real Postgres: two writers meeting
//! on one `products_product` — or one `products_sku` — row.
//!
//! # The two contested resources this suite covers, on both twins
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
//! # Why each case is run on both head tables
//!
//! `dod-concurrency` names six **resources**, not twelve, so a narrow reading
//! under which the Product half discharges each was available. It is declined
//! here, and the ground it is declined on is **not** the one first recorded.
//!
//! **The retracted argument, kept because it was load-bearing and wrong.** The
//! first version of this paragraph said `products_sku`'s tenth trigger clause —
//! `composition_pending`, admitted only alongside a `published_version` bump,
//! for which `products_product` has no twin — is contended by the publish case
//! below, and that no probe would reach it under the narrow reading. The second
//! half is true. The first is not, and could not be: the trigger is
//! `BEFORE UPDATE … FOR EACH ROW`, every head write filters on
//! `internal_revision`, and the *loser* of a contended pair therefore matches
//! zero rows and fires no row-level trigger at all. The publisher below is the
//! **winner** — it writes uncontended, before any second backend exists — so
//! its trigger evaluation is byte-for-byte an uncontended publish's. **The
//! tenth clause cannot be contended by two writers on this schema**, by either
//! reading, and `postgres_head_guards` is where it is judged.
//!
//! **The argument that survives** is plainer and is enough: `save_sku_head` and
//! `publish_sku_head` are separate functions over a separate table with their
//! own filters, and this gear's Product and SKU doors have already drifted
//! apart six times. A probe that runs on one twin measures one twin. That is a
//! twin-drift argument, not a coverage-of-a-unique-clause argument, and it is
//! what these two cases actually deliver.
//!
//! The strongest argument the other way, recorded rather than argued down:
//! `dod-concurrency` spells out *"the reservation index **on both code
//! columns**"* and spells out no such thing for either head race, so its
//! silence here may well be deliberate. It is answered on the evidence above
//! and on nothing else — had the two triggers been actual twins, the narrow
//! reading would have carried.
//!
//! So each case below appears twice, and the pair is kept in one file for the
//! reason stated above: a repair aimed at one half must be visible from the
//! other.
//!
//! # The duplication here is measured and accepted, not overlooked
//!
//! Four cases, 98 to 110 lines each, and the save pair is **0.88** similar line
//! for line. Adding the `SKU` half doubled a choreography this file already
//! carried twice, which is the very shape review wave D went and removed from
//! the door suites — so the asymmetry is deliberate and the reason is recorded
//! here rather than left for the next reader to re-derive.
//!
//! Two things resist extraction. The choreography lives **inside**
//! `in_transaction`'s closure, whose `for<'a> FnMut(&'a DbTx<'a>)` shape cannot
//! be bounded by any lifetime a helper holds — the obstacle
//! `api::rest::products::HeadActInputs` documents at length — and the winner's
//! notify-and-park must run *within* that closure, so a helper would have to
//! take the racer's body and its two `Notify` handles and would save nothing.
//! The eight `timeout(..).expect().expect().expect()` triples look extractable
//! until their messages are read: each names **which** racer and **why** its
//! outcome is the expected one, and a shared helper would either take those
//! strings as parameters, saving nothing, or drop them and delete the file's
//! reasoning.
//!
//! What is shared instead is everything that can be: one `head_column` reader
//! behind all seven column accessors, one `frozen`/`frozen_sku` pair, one
//! seed per entity. The residue is the race sequence itself.
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
    self, HeadWrite, NewEntityVersion, NewProduct, NewSku, ProductHeadSave, SavedName, SkuHeadSave,
    VersionedEntityKind,
};
use chrono::{DateTime, TimeZone, Utc};
use pg_support::Pg;
use tokio::sync::Notify;
use toolkit_db::secure::AccessScope;
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_42);
const BRAND: Uuid = Uuid::from_u128(0xb2_a0);
const SUBJECT: Uuid = Uuid::from_u128(0x_5b_1e);
/// The SKU the two SKU cases race for, a child of [`SUBJECT`].
const SKU: Uuid = Uuid::from_u128(0x_5b_2e);
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
        // No `02` content in this probe: `internal_revision` is the single
        // operand under test and a content write would give the loser's
        // filter a second reason to miss.
        content_moved: false,
    }
}

/// A save of the region scope alone: the SKU twin of [`rename`].
///
/// **Bucket iii for [`rename`]'s reason, and one more.** `sku_code` and
/// `product_id` — this table's bucket i — are admitted only while
/// `published_version = 0`, which the publish race below moves; a bucket-i
/// save would therefore give the loser's filter a second reason to miss and
/// the assertion could not say which one fired.
///
/// **Every value passed here stays inside the parent Product's
/// `region_scope`** (`"eu,apac"`), the loser's included. Containment is a
/// phase of the save *door*, above the repository these probes call, so an
/// uncontained value is admitted at this layer today and the probe would still
/// be sound — but only by accident. Were containment ever pushed down to the
/// repository or to a `CHECK`, an uncontained loser would be refused for the
/// wrong reason and this file would stay green while measuring nothing.
fn rescope(to: &str) -> SkuHeadSave {
    SkuHeadSave {
        sku_type: None,
        sellable: None,
        plan_tier: None,
        tax_category_ref: None,
        gl_code_ref: None,
        sku_code: None,
        product_id: None,
        region_scope: Some(to.to_owned()),
        brand_scope: None,
        metering_unit: None,
        usage_type_ref: None,
        // See `rename`: one operand under test.
        content_moved: false,
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
                        cloned_from: None,
                        cloned_from_version: None,
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
        binding_snapshot: None,
    }
}

/// Commit one `draft` SKU at [`CONTESTED_REVISION`], under the parent Product
/// its foreign key requires.
///
/// [`seed_draft`] first, in its own transaction: `fk_products_sku_product`
/// refuses a child whose parent is not committed, and seeding both on one
/// transaction would make a failure of either read as a failure of the other.
async fn seed_sku_draft(pg: &Pg) {
    seed_draft(pg).await;
    let db = pg.db().await;
    let (_db, out) = db
        .in_transaction::<(), RepoError, _>(move |txn| {
            Box::pin(async move {
                repo::insert_sku(
                    txn,
                    &scope(),
                    NewSku {
                        sku_type: "bundle".to_owned(),
                        sellable: true,
                        plan_tier: "standard".to_owned(),
                        tax_category_ref: None,
                        gl_code_ref: None,
                        sku_id: SKU,
                        tenant_id: TENANT,
                        product_id: SUBJECT,
                        sku_code: "FIBRE-500-STD".to_owned(),
                        region_scope: "eu".to_owned(),
                        brand_scope: String::new(),
                        created_by: "principal:author-1".to_owned(),
                        created_at: at(9),
                        cloned_from: None,
                        cloned_from_version: None,
                    },
                )
                .await
                .map(|_| ())
            })
        })
        .await;
    out.expect("the draft SKU must commit before anything races for it");
}

/// [`frozen`] for the SKU half: the same row under this table's own
/// `entity_kind`, which is the discriminator the head-row guard's
/// `EXISTS` sub-select filters on.
fn frozen_sku(version: i64) -> NewEntityVersion {
    NewEntityVersion {
        tenant_id: TENANT,
        entity_kind: VersionedEntityKind::Sku,
        entity_id: SKU,
        published_version: version,
        content: r#"{"skuCode":"FIBRE-500-STD","regionScope":"eu"}"#.to_owned(),
        content_digest: (1..=32_u8).collect(),
        digest_version: 1,
        approval_ref: None,
        actor_ref: ACTOR,
        published_at: at(10),
        binding_snapshot: None,
    }
}

/// One scalar column of one head row, on whichever of the two tables owns it.
///
/// A reader per column per table is precisely how a twin pair drifts: each
/// half would carry its own copy of this SQL and a schema move would correct
/// the copy whose test happened to fail. Both halves read through this one.
async fn head_column<T: sea_orm::TryGetable>(
    conn: &sea_orm::DatabaseConnection,
    table: &str,
    key: &str,
    id: Uuid,
    column: &str,
) -> T {
    use sea_orm::{ConnectionTrait, Statement};

    conn.query_one_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!("SELECT {column} AS v FROM bss.{table} WHERE {key} = '{id}'"),
    ))
    .await
    .expect("read the head")
    .expect("the head is there")
    .try_get::<T>("", "v")
    .expect("read the column")
}

/// The Product head's `internal_revision` as the database now holds it.
async fn head_revision(conn: &sea_orm::DatabaseConnection) -> i64 {
    head_column(
        conn,
        "products_product",
        "product_id",
        SUBJECT,
        "internal_revision",
    )
    .await
}

/// The SKU head's `internal_revision` as the database now holds it.
async fn sku_head_revision(conn: &sea_orm::DatabaseConnection) -> i64 {
    head_column(conn, "products_sku", "sku_id", SKU, "internal_revision").await
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

    // Budgeted like every other wait in this file, and for a reason the others
    // do not have: `notify_one` fires *inside* the winner's closure, after its
    // repository call succeeds. A winner whose write is refused — a tightened
    // trigger, a seeding regression, a foreign key — short-circuits before the
    // notify, and an unbudgeted wait here hangs the run with no failing test
    // name instead of reporting the refusal.
    tokio::time::timeout(RACE_TIMEOUT, written.notified())
        .await
        .expect(
            "the winner must reach its parking point; not reaching it means its own write \
                 was refused, which is a defect and not a slow machine",
        );

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

    // Budgeted like every other wait in this file, and for a reason the others
    // do not have: `notify_one` fires *inside* the winner's closure, after its
    // repository call succeeds. A winner whose write is refused — a tightened
    // trigger, a seeding regression, a foreign key — short-circuits before the
    // notify, and an unbudgeted wait here hangs the run with no failing test
    // name instead of reporting the refusal.
    tokio::time::timeout(RACE_TIMEOUT, written.notified())
        .await
        .expect(
            "the winner must reach its parking point; not reaching it means its own write \
                 was refused, which is a defect and not a slow machine",
        );

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

/// **Two SKU saves presenting one `If-Match`: exactly one lands.**
///
/// [`two_saves_presenting_one_if_match_serialize_and_the_loser_is_refused`]'s
/// twin. It is not a paste: `save_sku_head` is a separate function over a
/// separate table with its own trigger, so the shared invariant is shared only
/// as long as a probe holds both halves to it.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn two_sku_saves_presenting_one_if_match_serialize_and_the_loser_is_refused() {
    let pg = Pg::applied().await;
    seed_sku_draft(&pg).await;

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
                        let outcome = repo::save_sku_head(
                            txn,
                            &scope(),
                            TENANT,
                            SKU,
                            CONTESTED_REVISION,
                            &rescope("eu,apac"),
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

    // Budgeted like every other wait in this file, and for a reason the others
    // do not have: `notify_one` fires *inside* the winner's closure, after its
    // repository call succeeds. A winner whose write is refused — a tightened
    // trigger, a seeding regression, a foreign key — short-circuits before the
    // notify, and an unbudgeted wait here hangs the run with no failing test
    // name instead of reporting the refusal.
    tokio::time::timeout(RACE_TIMEOUT, written.notified())
        .await
        .expect(
            "the winner must reach its parking point; not reaching it means its own write \
                 was refused, which is a defect and not a slow machine",
        );

    let loser = {
        let db = pg.db().await;
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<HeadWrite, RepoError, _>(move |txn| {
                    Box::pin(async move {
                        repo::save_sku_head(
                            txn,
                            &scope(),
                            TENANT,
                            SKU,
                            CONTESTED_REVISION,
                            &rescope("apac"),
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
        sku_head_revision(&conn).await,
        CONTESTED_REVISION + 1,
        "two attempts under one If-Match must move the head by exactly one revision"
    );
    assert_eq!(
        surviving_region_scope(&conn).await,
        "eu,apac",
        "the loser must not have written its own scope under an unmatched filter"
    );
}

/// **A SKU publish and an edit presenting one `If-Match`: the publish lands,
/// the edit is refused, and `composition_pending` moves with it.**
///
/// The twin of
/// [`a_publish_and_an_edit_presenting_one_if_match_serialize_and_the_edit_is_refused`],
/// on the table whose door is a different function.
///
/// **What this measures**: an edit cannot land against a revision a *`SKU`*
/// publish already consumed. The publisher additionally passes
/// `composition_pending = true`, so the flag's final value is asserted — but
/// that write is the **winner's**, and a winner is uncontended by construction.
/// An earlier revision of this doc claimed the contended re-evaluation carried
/// the tenth trigger clause's operand; it cannot. Postgres re-evaluates the
/// *blocked* statement's filter, which is the editor's `save_sku_head`, and a
/// loser matches zero rows so no row-level trigger fires on it at all. See this
/// module's doc for the retraction and for the argument that does hold.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_sku_publish_and_an_edit_presenting_one_if_match_serialize_and_the_edit_is_refused() {
    let pg = Pg::applied().await;
    seed_sku_draft(&pg).await;

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
                        // Freeze first, bump second, exactly as the Product
                        // half does: the guard admits the bump only where the
                        // matching frozen row is already visible to it.
                        repo::insert_entity_version(txn, &scope(), frozen_sku(1)).await?;
                        let outcome = repo::publish_sku_head(
                            txn,
                            &scope(),
                            TENANT,
                            SKU,
                            CONTESTED_REVISION,
                            true,
                            at(11),
                            None,
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

    // Budgeted like every other wait in this file, and for a reason the others
    // do not have: `notify_one` fires *inside* the winner's closure, after its
    // repository call succeeds. A winner whose write is refused — a tightened
    // trigger, a seeding regression, a foreign key — short-circuits before the
    // notify, and an unbudgeted wait here hangs the run with no failing test
    // name instead of reporting the refusal.
    tokio::time::timeout(RACE_TIMEOUT, written.notified())
        .await
        .expect(
            "the winner must reach its parking point; not reaching it means its own write \
                 was refused, which is a defect and not a slow machine",
        );

    let editor = {
        let db = pg.db().await;
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<HeadWrite, RepoError, _>(move |txn| {
                    Box::pin(async move {
                        repo::save_sku_head(
                            txn,
                            &scope(),
                            TENANT,
                            SKU,
                            CONTESTED_REVISION,
                            &rescope("apac"),
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
    assert_eq!(
        publish_outcome,
        HeadWrite::Applied,
        "the publish must land, flag and all"
    );

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
        sku_head_revision(&conn).await,
        CONTESTED_REVISION + 1,
        "the publish moved the head once and the edit moved it not at all"
    );
    assert_eq!(
        surviving_region_scope(&conn).await,
        "eu",
        "the edit's scope must not be on a row it never matched"
    );
    assert_eq!(
        sku_published_version(&conn).await,
        1,
        "the publish's own column must carry the version it froze"
    );
    assert!(
        sku_composition_pending(&conn).await,
        "the tenth clause's column must carry what the contended statement wrote"
    );
}

/// The Product name that survived the race.
async fn surviving_name(conn: &sea_orm::DatabaseConnection) -> String {
    head_column(conn, "products_product", "product_id", SUBJECT, "name").await
}

/// The SKU `region_scope` that survived the race: [`surviving_name`]'s twin,
/// reading the bucket-iii column [`rescope`] moves.
async fn surviving_region_scope(conn: &sea_orm::DatabaseConnection) -> String {
    head_column(conn, "products_sku", "sku_id", SKU, "region_scope").await
}

/// The Product head's `published_version`.
async fn published_version(conn: &sea_orm::DatabaseConnection) -> i64 {
    head_column(
        conn,
        "products_product",
        "product_id",
        SUBJECT,
        "published_version",
    )
    .await
}

/// The SKU head's `published_version`.
async fn sku_published_version(conn: &sea_orm::DatabaseConnection) -> i64 {
    head_column(conn, "products_sku", "sku_id", SKU, "published_version").await
}

/// The SKU head's `composition_pending` — the column the tenth trigger clause
/// guards, and the one this table has that its twin does not.
async fn sku_composition_pending(conn: &sea_orm::DatabaseConnection) -> bool {
    head_column(conn, "products_sku", "sku_id", SKU, "composition_pending").await
}
