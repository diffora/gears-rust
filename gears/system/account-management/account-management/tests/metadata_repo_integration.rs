//! Repo-level integration tests for `MetadataRepoImpl`
//! (`repo_impl/metadata.rs`): upsert insert/update paths, the
//! optimistic-lock `expected_version` preconditions, get hit/miss, and
//! idempotent delete. `metadata_integration.rs` drives the service
//! layer; this hits the repo trait directly.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::sync::Arc;

use account_management::domain::error::DomainError;
use account_management::domain::metadata::UpsertOutcome;
use account_management::domain::metadata::repo::MetadataRepo;
use account_management::infra::storage::repo_impl::MetadataRepoImpl;
use serde_json::json;
use time::OffsetDateTime;
use toolkit_odata::ODataQuery;
use uuid::Uuid;

use common::*;

fn repo(h: &Harness) -> MetadataRepoImpl {
    MetadataRepoImpl::new(Arc::clone(&h.provider))
}

fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

/// Seed a root tenant to own the metadata rows, returning its id.
async fn seed_owner(h: &Harness) -> Uuid {
    let id = Uuid::new_v4();
    insert_tenant(&h.provider, id, None, "owner", ACTIVE, false, 0)
        .await
        .expect("seed owner tenant");
    id
}

#[tokio::test]
async fn upsert_inserts_then_get_and_list_return_it() {
    let h = setup_sqlite().await.expect("setup");
    let repo = repo(&h);
    let tenant = seed_owner(&h).await;
    let schema = Uuid::new_v4();

    let out = repo
        .upsert_for_tenant(&allow_all(), tenant, schema, json!({"k": 1}), now(), None)
        .await
        .expect("insert");
    match out {
        UpsertOutcome::Inserted(row) => {
            assert_eq!(row.version, 1, "insert seeds version 1");
            assert_eq!(row.value, json!({"k": 1}));
        }
        UpsertOutcome::Updated(_) => panic!("first write must be an insert"),
    }

    let got = repo
        .get_for_tenant(&allow_all(), tenant, schema)
        .await
        .expect("get")
        .expect("row present");
    assert_eq!(got.value, json!({"k": 1}));

    let page = repo
        .list_for_tenant(&allow_all(), tenant, &ODataQuery::default())
        .await
        .expect("list");
    assert_eq!(page.items.len(), 1, "tenant has one direct entry");
}

#[tokio::test]
async fn upsert_update_increments_version_and_preserves_created_at() {
    let h = setup_sqlite().await.expect("setup");
    let repo = repo(&h);
    let tenant = seed_owner(&h).await;
    let schema = Uuid::new_v4();

    let inserted = match repo
        .upsert_for_tenant(&allow_all(), tenant, schema, json!({"v": 1}), now(), None)
        .await
        .expect("insert")
    {
        UpsertOutcome::Inserted(r) => r,
        UpsertOutcome::Updated(_) => panic!("expected insert"),
    };

    let out = repo
        .upsert_for_tenant(
            &allow_all(),
            tenant,
            schema,
            json!({"v": 2}),
            now(),
            Some(1),
        )
        .await
        .expect("update");
    match out {
        UpsertOutcome::Updated(row) => {
            assert_eq!(row.version, 2, "update bumps version");
            assert_eq!(row.value, json!({"v": 2}));
            assert_eq!(
                row.created_at.unix_timestamp(),
                inserted.created_at.unix_timestamp(),
                "update preserves created_at"
            );
        }
        UpsertOutcome::Inserted(_) => panic!("expected update"),
    }
}

#[tokio::test]
async fn upsert_insert_with_nonzero_expected_version_is_mismatch() {
    let h = setup_sqlite().await.expect("setup");
    let repo = repo(&h);
    // No row exists yet, but caller asserts version 5 → mismatch (current 0).
    let err = repo
        .upsert_for_tenant(
            &allow_all(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            json!({}),
            now(),
            Some(5),
        )
        .await
        .expect_err("must mismatch");
    assert!(
        matches!(
            err,
            DomainError::MetadataVersionMismatch {
                current: 0,
                expected: 5,
                ..
            }
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn upsert_update_with_wrong_expected_version_is_mismatch() {
    let h = setup_sqlite().await.expect("setup");
    let repo = repo(&h);
    let tenant = seed_owner(&h).await;
    let schema = Uuid::new_v4();
    repo.upsert_for_tenant(&allow_all(), tenant, schema, json!({"v": 1}), now(), None)
        .await
        .expect("insert"); // version now 1

    let err = repo
        .upsert_for_tenant(
            &allow_all(),
            tenant,
            schema,
            json!({"v": 2}),
            now(),
            Some(99),
        )
        .await
        .expect_err("stale expected_version");
    assert!(
        matches!(
            err,
            DomainError::MetadataVersionMismatch {
                current: 1,
                expected: 99,
                ..
            }
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn get_absent_returns_none() {
    let h = setup_sqlite().await.expect("setup");
    let got = repo(&h)
        .get_for_tenant(&allow_all(), Uuid::new_v4(), Uuid::new_v4())
        .await
        .expect("get");
    assert!(got.is_none());
}

#[tokio::test]
async fn delete_removes_row_and_is_idempotent_on_missing() {
    let h = setup_sqlite().await.expect("setup");
    let repo = repo(&h);
    let tenant = seed_owner(&h).await;
    let schema = Uuid::new_v4();
    repo.upsert_for_tenant(
        &allow_all(),
        tenant,
        schema,
        json!({"x": true}),
        now(),
        None,
    )
    .await
    .expect("insert");

    repo.delete_for_tenant(&allow_all(), tenant, schema)
        .await
        .expect("delete present");
    assert!(
        repo.get_for_tenant(&allow_all(), tenant, schema)
            .await
            .expect("get")
            .is_none(),
        "row deleted"
    );

    // Idempotent: deleting again is a no-op Ok.
    repo.delete_for_tenant(&allow_all(), tenant, schema)
        .await
        .expect("delete missing is Ok");
}
