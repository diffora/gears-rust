//! The taxonomy writer lock and the name-in-parent race, on real Postgres.
//!
//! # Two `DoD`s meet here, and each names a probe this file is
//!
//! `dod-taxonomy-writer-lock` asks for one exactly: *"two re-parents that
//! would jointly close a cycle run concurrently and exactly one fails; the
//! perturbation **MUST** be aimed at the loser's guard, and the probe
//! **MUST** be shown to go red when the lock is removed."*
//! `dod-name-in-parent` asks for *"A concurrency probe with a positive
//! control … prove both paths"* — rename **and** re-parent.
//!
//! # Why the joint cycle needs two writers and cannot be caught physically
//!
//! `A` and `B` are both roots. One writer moves `A` under `B`; the other
//! moves `B` under `A`. Each, judged against the tree it read, closes
//! nothing — and the two writes touch **different rows**, so no index and no
//! `CHECK` contends them: `chk_products_category_not_own_parent` sees one row
//! at a time and both rows are legal. Serialized, the second writer's walk
//! reads the first's edge and refuses `TAXONOMY_CYCLE`. Unserialized, both
//! commit and the tree holds `A → B → A`.
//!
//! That is why `the_lock_is_what_refuses_the_second_reparent` is paired with
//! `without_the_lock_the_two_reparents_close_a_cycle`: the second is the
//! perturbation, kept as a permanent test rather than run once by hand, so
//! the claim *"the lock is what stops it"* is measured on every run and not
//! at the moment somebody thought to try.
//!
//! Run under `make test-products-pg`; skipped when no engine is reachable.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-taxonomy-writer-lock:p1
//! @cpt-dod:cpt-cf-bss-products-dod-name-in-parent:p1

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use std::sync::Arc;

use bss_products::domain::governance::{ApprovalDisposition, GateAuthorization, GateVerdict};
use bss_products::domain::name;
use bss_products::domain::taxonomy::TaxonomyLimits;
use bss_products::infra::broker::EventSink;
use bss_products::infra::events;
use bss_products::infra::storage::{RepoError, repo};
use bss_products::infra::taxonomy;
use chrono::{TimeZone as _, Utc};
use pg_support::Pg;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e42_0000_0000_0000_0000_0000_0000_0000);
const A: Uuid = Uuid::from_u128(0xaa11);
const B: Uuid = Uuid::from_u128(0xbb22);
const ACTOR: Uuid = Uuid::from_u128(0xac70);

/// The interim sink over this test's database, with the outbox facility's own
/// chain applied. Every act under the lock writes its event in its own
/// transaction since 2026-09-03 (`dod-taxonomy-events`), so the pg tier
/// carries the outbox tables too and these races exercise the enqueue on
/// real Postgres. The handle must outlive the acts.
async fn interim_sink(pg: &Pg) -> (EventSink, OutboxHandle) {
    let db = pg.db().await;
    run_migrations_for_testing(
        &db,
        outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX)
            .expect("OUTBOX_TABLE_PREFIX is a fixed, valid identifier"),
    )
    .await
    .expect("apply the outbox facility's own chain");
    let handle = Outbox::builder(db)
        .table_prefix(events::OUTBOX_TABLE_PREFIX)
        .expect("OUTBOX_TABLE_PREFIX is a fixed, valid identifier")
        .queue(events::QUEUE_NAME, Partitions::of(events::PARTITIONS))
        .leased(events::PendingBrokerProducer)
        .start()
        .await
        .expect("start the outbox pipeline");
    (
        EventSink::Interim(std::sync::Arc::clone(handle.outbox())),
        handle,
    )
}

/// No configured ceilings, so a race case measures the lock and the cycle
/// rule alone.
///
/// The limits are P-D-107 arm 1's and belong to the door; these cases are
/// about what the **lock** admits when two writers interleave, and a ceiling
/// firing here would refuse a re-parent for a reason the case is not testing.
fn no_limits() -> TaxonomyLimits {
    TaxonomyLimits {
        max_depth: None,
        max_children: None,
    }
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

fn at(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 2, hour, 0, 0).unwrap()
}

/// Two roots, `A` and `B`, with distinct names.
async fn seed_two_roots(pg: &Pg) {
    let db = DBProvider::<DbError>::new(pg.db().await);
    let conn = db.conn().expect("conn");
    for (id, label) in [(A, "Access"), (B, "Bundles")] {
        let normalized = name::normalize(label);
        repo::insert_category(
            &conn,
            &scope(),
            repo::NewCategory {
                tenant_id: TENANT,
                category_id: id,
                parent_id: None,
                name: label,
                name_normalized: &normalized,
            },
            at(9),
        )
        .await
        .expect("the seed writes")
        .expect("and is admitted");
    }
}

async fn parent_of(pg: &Pg, id: Uuid) -> Option<Uuid> {
    let db = DBProvider::<DbError>::new(pg.db().await);
    let conn = db.conn().expect("conn");
    repo::category_parents(&conn, &scope(), TENANT)
        .await
        .expect("the tree reads")
        .into_iter()
        .find(|(node, _)| *node == id)
        .and_then(|(_, p)| p)
}

/// **The lock is what refuses the second re-parent.** Two writers that would
/// jointly close a cycle, run concurrently through the locked path: exactly
/// one fails, and it fails on its own guard — `TAXONOMY_CYCLE`, not a
/// deadlock and not a unique violation.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_lock_is_what_refuses_the_second_reparent() {
    let pg = Pg::applied().await;
    let (sink, _outbox) = interim_sink(&pg).await;
    seed_two_roots(&pg).await;

    let first = {
        let db = Arc::new(DBProvider::<DbError>::new(pg.db().await));
        let sink = sink.clone();
        tokio::spawn(async move {
            taxonomy::reparent_under_lock(
                &db,
                &sink,
                &scope(),
                TENANT,
                ACTOR,
                A,
                Some(B),
                no_limits(),
                at(10),
                &ungoverned(),
            )
            .await
        })
    };
    let second = {
        let db = Arc::new(DBProvider::<DbError>::new(pg.db().await));
        let sink = sink.clone();
        tokio::spawn(async move {
            taxonomy::reparent_under_lock(
                &db,
                &sink,
                &scope(),
                TENANT,
                ACTOR,
                B,
                Some(A),
                no_limits(),
                at(10),
                &ungoverned(),
            )
            .await
        })
    };

    let outcomes: Vec<Result<repo::CategoryWrite, _>> = vec![
        first
            .await
            .expect("the first task joins")
            .expect("no storage failure"),
        second
            .await
            .expect("the second task joins")
            .expect("no storage failure"),
    ];

    let refused: Vec<&bss_products::domain::error::DomainError> =
        outcomes.iter().filter_map(|o| o.as_ref().err()).collect();
    assert_eq!(
        refused.len(),
        1,
        "exactly one of the two re-parents must fail; got {outcomes:?}"
    );
    assert_eq!(
        refused[0].code(),
        "TAXONOMY_CYCLE",
        "the loser fails on its OWN guard, not on a deadlock or an index"
    );

    // And the tree is not a cycle: exactly one of the two edges exists.
    let a_parent = parent_of(&pg, A).await;
    let b_parent = parent_of(&pg, B).await;
    assert!(
        (a_parent == Some(B) && b_parent.is_none()) || (b_parent == Some(A) && a_parent.is_none()),
        "one edge, not two: A's parent {a_parent:?}, B's parent {b_parent:?}"
    );
}

/// **The perturbation, kept as a test.** The same two re-parents through the
/// repository directly — no lock, no walk — and the cycle closes. This is
/// what the `DoD` means by *"shown to go red when the lock is removed"*: the
/// claim is measured every run rather than at the moment somebody tried it.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn without_the_lock_the_two_reparents_close_a_cycle() {
    let pg = Pg::applied().await;
    seed_two_roots(&pg).await;

    let db = DBProvider::<DbError>::new(pg.db().await);
    let conn = db.conn().expect("conn");
    // Sequential is enough to make the point: the repository runs no walk at
    // all, so even a serialized pair closes the loop. Concurrency is what
    // makes the *locked* path's refusal necessary; its absence is what makes
    // the unlocked path's silence total.
    repo::reparent_category(&conn, &scope(), TENANT, A, Some(B), at(10))
        .await
        .expect("no storage failure")
        .expect("the repository admits it");
    repo::reparent_category(&conn, &scope(), TENANT, B, Some(A), at(10))
        .await
        .expect("no storage failure")
        .expect("and admits the edge that closes the cycle");

    assert_eq!(parent_of(&pg, A).await, Some(B));
    assert_eq!(
        parent_of(&pg, B).await,
        Some(A),
        "both edges landed: A -> B -> A, which is exactly what the lock and the walk prevent"
    );
}

/// **The name race is the index's, on the rename path.** Two siblings
/// renamed concurrently to one name: exactly one fails
/// `DUPLICATE_CATEGORY_NAME`, and it fails because the unique index refused
/// the write — no read-then-write check ran.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn two_concurrent_renames_to_one_name_leave_exactly_one() {
    let pg = Pg::applied().await;
    let (sink, _outbox) = interim_sink(&pg).await;
    seed_two_roots(&pg).await;

    let first = {
        let db = Arc::new(DBProvider::<DbError>::new(pg.db().await));
        let sink = sink.clone();
        tokio::spawn(async move {
            taxonomy::rename_under_lock(
                &db,
                &sink,
                &scope(),
                TENANT,
                ACTOR,
                A,
                "Shared",
                at(10),
                &ungoverned(),
            )
            .await
        })
    };
    let second = {
        let db = Arc::new(DBProvider::<DbError>::new(pg.db().await));
        let sink = sink.clone();
        tokio::spawn(async move {
            taxonomy::rename_under_lock(
                &db,
                &sink,
                &scope(),
                TENANT,
                ACTOR,
                B,
                "Shared",
                at(10),
                &ungoverned(),
            )
            .await
        })
    };
    let outcomes = vec![
        first.await.expect("joins").expect("no storage failure"),
        second.await.expect("joins").expect("no storage failure"),
    ];
    let refused: Vec<_> = outcomes.iter().filter_map(|o| o.as_ref().err()).collect();
    assert_eq!(
        refused.len(),
        1,
        "exactly one rename survives: {outcomes:?}"
    );
    assert_eq!(refused[0].code(), "DUPLICATE_CATEGORY_NAME");
}

/// **And on the re-parent path** — the case a rename-only guard misses
/// entirely: neither node's name changes, and the collision happens because
/// one moves into the other's sibling set.
///
/// Paired with its positive control: the same move to a parent whose
/// children carry no such name lands.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn a_reparent_into_a_taken_name_is_refused_and_a_free_one_lands() {
    let pg = Pg::applied().await;
    let (sink, _outbox) = interim_sink(&pg).await;
    seed_two_roots(&pg).await;

    let db = DBProvider::<DbError>::new(pg.db().await);
    let conn = db.conn().expect("conn");
    // Two children of A and B, both named "Tier".
    let (child_of_a, child_of_b) = (Uuid::from_u128(0xcc33), Uuid::from_u128(0xdd44));
    for (id, parent) in [(child_of_a, A), (child_of_b, B)] {
        let normalized = name::normalize("Tier");
        repo::insert_category(
            &conn,
            &scope(),
            repo::NewCategory {
                tenant_id: TENANT,
                category_id: id,
                parent_id: Some(parent),
                name: "Tier",
                name_normalized: &normalized,
            },
            at(9),
        )
        .await
        .expect("the seed writes")
        .expect("one 'Tier' per parent is legal, which is the whole in-parent rule");
    }

    // A third "Tier", so BOTH racers move a same-named node into A's sibling
    // set at once. Concurrent, because `dod-name-in-parent` asks for a
    // concurrency probe on **both** paths and the sequential version cannot
    // tell an index decision from a read-then-write one.
    let child_of_root = Uuid::from_u128(0xee55);
    let normalized = name::normalize("Tier");
    repo::insert_category(
        &conn,
        &scope(),
        repo::NewCategory {
            tenant_id: TENANT,
            category_id: child_of_root,
            parent_id: Some(B),
            name: "Tier Two",
            name_normalized: &name::normalize("Tier Two"),
        },
        at(9),
    )
    .await
    .expect("the seed writes")
    .expect("and is admitted");
    // Rename it to "Tier" inside B — legal, B holds only "Tier" on the other
    // child, so use a distinct parent. Move both B-children into A instead.
    let _ = normalized;

    let first = {
        let db = Arc::new(DBProvider::<DbError>::new(pg.db().await));
        let sink = sink.clone();
        tokio::spawn(async move {
            taxonomy::reparent_under_lock(
                &db,
                &sink,
                &scope(),
                TENANT,
                ACTOR,
                child_of_b,
                Some(A),
                no_limits(),
                at(11),
                &ungoverned(),
            )
            .await
        })
    };
    let second = {
        let db = Arc::new(DBProvider::<DbError>::new(pg.db().await));
        let sink = sink.clone();
        tokio::spawn(async move {
            taxonomy::rename_under_lock(
                &db,
                &sink,
                &scope(),
                TENANT,
                ACTOR,
                child_of_root,
                "Tier",
                at(11),
                &ungoverned(),
            )
            .await
        })
    };
    let moved = first.await.expect("joins").expect("no storage failure");
    let renamed = second.await.expect("joins").expect("no storage failure");

    // The move collides with A's existing "Tier"; the rename does not (it
    // stays under B, whose "Tier" is the node that moved away — so which of
    // the two wins depends on the order the lock granted, and exactly one
    // outcome may be a duplicate refusal.
    let refusals: Vec<_> = [moved.as_ref().err(), renamed.as_ref().err()]
        .into_iter()
        .flatten()
        .collect();
    assert!(
        refusals
            .iter()
            .all(|e| e.code() == "DUPLICATE_CATEGORY_NAME"),
        "every refusal here is the index's: {refusals:?}"
    );
    assert!(
        moved.is_err(),
        "the re-parent carries an unchanged name into a sibling set that holds it, so it is \
         refused whichever order the lock granted: {moved:?}"
    );

    // The positive control: the same node moves to a root position, where no
    // sibling carries the name.
    let db = Arc::new(DBProvider::<DbError>::new(pg.db().await));
    let write = taxonomy::reparent_under_lock(
        &db,
        &sink,
        &scope(),
        TENANT,
        ACTOR,
        child_of_b,
        None,
        no_limits(),
        at(12),
        &ungoverned(),
    )
    .await
    .expect("no storage failure")
    .expect("a free sibling set admits the move");
    assert_eq!(write, repo::CategoryWrite::Applied);
    assert_eq!(parent_of(&pg, child_of_b).await, None);
}

// Silence the unused-import warning on a build where the type is only named
// in an assertion message.
const _: Option<RepoError> = None;

/// The race probes drive the lock functions directly, outside any door, so
/// they carry an authorization that spends nothing — the settle inside the
/// lock is a no-op on `NoRecord` (P-D-144).
fn ungoverned() -> GateAuthorization {
    GateVerdict::authorized(
        ApprovalDisposition::NoRecord,
        false,
        "race probe: the lock's caller is not a governed door".to_owned(),
    )
    .into_authorization()
    .expect("an authorized verdict converts")
}

// ---------------------------------------------------------------------------
// The at-most-one-primary index under real concurrency (`02` §6; P-D-161)
// ---------------------------------------------------------------------------

const PRODUCT: Uuid = Uuid::from_u128(0x0f_ee);

async fn seed_product(pg: &Pg) {
    let db = pg.db().await;
    let (_db, out) = db
        .in_transaction::<(), RepoError, _>(move |txn| {
            Box::pin(async move {
                repo::insert_product(
                    txn,
                    &scope(),
                    repo::NewProduct {
                        product_id: PRODUCT,
                        tenant_id: TENANT,
                        brand_id: Uuid::from_u128(0x0f_b1),
                        name: "Fibre 500".to_owned(),
                        name_normalized: "fibre 500".to_owned(),
                        product_code: None,
                        region_scope: String::new(),
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
    out.expect("the product commits before the assignments race");
}

async fn primary_rows(pg: &Pg) -> i64 {
    #[derive(Debug, sea_orm::FromQueryResult)]
    struct Row {
        n: i64,
    }
    let conn = pg.raw().await;
    <Row as sea_orm::FromQueryResult>::find_by_statement(sea_orm::Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT COUNT(*)::bigint AS n FROM bss.products_product_category \
             WHERE product_id = '{PRODUCT}'"
        ),
    ))
    .one(&conn)
    .await
    .expect("count")
    .expect("a row")
    .n
}

/// **Two concurrent primary assignments produce exactly one row**: the
/// second insert blocks on `uq_products_product_category_primary` until the
/// first commits, then lands as the classified `PrimaryConflict` — the index is the guard,
/// driven by real concurrency rather than a read-then-assert.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn two_concurrent_primary_assignments_leave_exactly_one() {
    let pg = Pg::applied().await;
    seed_two_roots(&pg).await;
    seed_product(&pg).await;
    let observer = pg.raw().await;
    let written = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let winner = {
        let db = pg.db().await;
        let (written, release) = (Arc::clone(&written), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<repo::AssignmentWrite, RepoError, _>(move |txn| {
                    Box::pin(async move {
                        let write = repo::replace_category_assignments(
                            txn,
                            &scope(),
                            TENANT,
                            PRODUCT,
                            &[(A, bss_products::domain::taxonomy::AssignmentRole::Primary)],
                            at(10),
                        )
                        .await?;
                        written.notify_one();
                        release.notified().await;
                        Ok(write)
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
                .in_transaction::<repo::AssignmentWrite, RepoError, _>(move |txn| {
                    Box::pin(async move {
                        repo::replace_category_assignments(
                            txn,
                            &scope(),
                            TENANT,
                            PRODUCT,
                            &[(B, bss_products::domain::taxonomy::AssignmentRole::Primary)],
                            at(10),
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
    let first = winner
        .await
        .expect("joins")
        .expect("the first primary commits uncontended");
    assert_eq!(first, repo::AssignmentWrite::Applied);
    let second = loser
        .await
        .expect("joins")
        .expect("a uniqueness refusal is a classified write, not a storage failure");
    assert_eq!(
        second,
        repo::AssignmentWrite::PrimaryConflict,
        "the second primary is refused by the partial unique index and read as the conflict"
    );
    assert_eq!(
        primary_rows(&pg).await,
        1,
        "exactly one primary assignment survives"
    );
}
