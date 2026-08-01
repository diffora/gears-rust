//! The pin-eligibility frontier is **forward only** — against a real database,
//! not a mock.
//!
//! This is the property the whole materialization exists for (D-136). If the
//! watermark could recede, a consumer pinning it would resolve two different
//! contents at the same version over time, which is precisely the divergence
//! D-101/D-114 close and which recomputing the predicate at read time could not
//! afford. A mock would only assert that the repository's own `if` fires; the
//! guard that has to hold under concurrency is the `catalog_version < :to`
//! predicate on the UPDATE, and only a database evaluates that.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::error::DomainError;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::PinFrontierRepo;
use bss_pricing::infra::storage::{RepoError, repo_failure};
use bss_pricing_sdk::CatalogVersion;
use chrono::{TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

async fn repo() -> PinFrontierRepo {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    PinFrontierRepo::new(DBProvider::<DbError>::new(db))
}

#[tokio::test]
async fn the_frontier_only_ever_moves_forward() {
    let repo = repo().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let t1 = Utc.with_ymd_and_hms(2026, 8, 2, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 8, 2, 10, 0, 5).unwrap();
    let t3 = Utc.with_ymd_and_hms(2026, 8, 2, 10, 0, 9).unwrap();

    // No frontier yet: a consumer has nothing it may pin, and fails closed.
    assert_eq!(
        repo.read(&scope, tenant).await.expect("read"),
        None,
        "a tenant with no completed version has no frontier"
    );

    // First advance inserts the row.
    repo.advance(&scope, tenant, CatalogVersion::new(1), t1)
        .await
        .expect("advance to 1");
    let frontier = repo
        .read(&scope, tenant)
        .await
        .expect("read")
        .expect("frontier present after the first advance");
    assert_eq!(frontier.catalog_version, CatalogVersion::new(1));
    assert_eq!(frontier.advanced_at, t1);

    // 1 -> 5 moves it.
    repo.advance(&scope, tenant, CatalogVersion::new(5), t2)
        .await
        .expect("advance to 5");
    let frontier = repo
        .read(&scope, tenant)
        .await
        .expect("read")
        .expect("frontier present");
    assert_eq!(frontier.catalog_version, CatalogVersion::new(5));
    assert_eq!(frontier.advanced_at, t2);

    // 5 -> 3 does NOT move it, and says so.
    let err = repo
        .advance(&scope, tenant, CatalogVersion::new(3), t3)
        .await
        .expect_err("a backwards advance must be refused");
    assert_eq!(
        err,
        RepoError::FrontierRegression {
            tenant: tenant.to_string(),
            current: 5,
            requested: 3,
        }
    );

    // 5 -> 5 is refused too: re-advancing to the standing version means the
    // projector completed the same version twice, which is an ordering bug, not
    // a harmless retry.
    let err = repo
        .advance(&scope, tenant, CatalogVersion::new(5), t3)
        .await
        .expect_err("an equal advance must be refused");
    assert!(matches!(err, RepoError::FrontierRegression { .. }));

    // Neither refusal moved anything, including the timestamp.
    let frontier = repo
        .read(&scope, tenant)
        .await
        .expect("read")
        .expect("frontier present");
    assert_eq!(frontier.catalog_version, CatalogVersion::new(5));
    assert_eq!(
        frontier.advanced_at, t2,
        "a refused advance must not restamp advanced_at"
    );
}

#[tokio::test]
async fn a_foreign_tenants_frontier_is_invisible() {
    let repo = repo().await;
    let mine = Uuid::from_u128(0x7e_11);
    let theirs = Uuid::from_u128(0x7e_22);
    let at = Utc.with_ymd_and_hms(2026, 8, 2, 10, 0, 0).unwrap();

    repo.advance(
        &AccessScope::for_tenant(theirs),
        theirs,
        CatalogVersion::new(9),
        at,
    )
    .await
    .expect("advance the other tenant");

    // SQL-level BOLA: my scope sees no row of theirs, whichever tenant id I ask
    // for. The read model is commercially sensitive, so this fails to `None`
    // rather than to someone else's frontier.
    assert_eq!(
        repo.read(&AccessScope::for_tenant(mine), theirs)
            .await
            .expect("read"),
        None
    );
}

#[test]
fn a_refused_advance_maps_to_a_precondition_failure() {
    // The regression is not an internal fault: the store is healthy and the row
    // is intact, the transition is simply one monotonicity forbids — so it
    // lands on the same variant as any other refused lifecycle edge.
    let err = RepoError::FrontierRegression {
        tenant: Uuid::from_u128(1).to_string(),
        current: 5,
        requested: 3,
    };

    assert!(matches!(
        repo_failure(&err),
        DomainError::LifecycleForbidden(_)
    ));
}
