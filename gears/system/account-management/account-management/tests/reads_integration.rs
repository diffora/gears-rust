//! Integration tests for the read-side `repo_impl/reads.rs` methods
//! that the listing/HTTP suites don't exercise directly: `find_many`,
//! `count_children` (both filter modes), `count_tenants_by_status`,
//! `count_closure_rows`, and `is_descendant`.
//!
//! `SQLite` has foreign keys disabled, so closure-only probes
//! (`is_descendant`, `count_closure_rows`) can seed edges without the
//! referenced tenant rows.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use account_management::domain::tenant::TenantRepo;
use account_management::domain::tenant::model::{ChildCountFilter, TenantStatus};
use time::OffsetDateTime;
use uuid::Uuid;

use common::*;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[tokio::test]
async fn find_many_returns_existing_subset() {
    let h = setup_sqlite().await.expect("setup");
    insert_tenant(&h.provider, id(1), None, "root", ACTIVE, false, 0)
        .await
        .unwrap();
    insert_tenant(&h.provider, id(2), Some(id(1)), "a", ACTIVE, false, 1)
        .await
        .unwrap();
    insert_tenant(&h.provider, id(3), Some(id(1)), "b", ACTIVE, false, 1)
        .await
        .unwrap();

    let found = h
        .repo
        .find_many(&allow_all(), &[id(2), id(3), id(999)])
        .await
        .expect("find_many");
    let mut ids: Vec<Uuid> = found.iter().map(|t| t.id).collect();
    ids.sort();
    assert_eq!(ids, vec![id(2), id(3)], "absent id silently dropped");
}

#[tokio::test]
async fn find_many_empty_ids_returns_empty() {
    let h = setup_sqlite().await.expect("setup");
    let found = h
        .repo
        .find_many(&allow_all(), &[])
        .await
        .expect("find_many");
    assert!(found.is_empty());
}

#[tokio::test]
async fn count_children_non_deleted_vs_all() {
    let h = setup_sqlite().await.expect("setup");
    let p = id(0x10);
    insert_tenant(&h.provider, p, None, "parent", ACTIVE, false, 0)
        .await
        .unwrap();
    insert_tenant(&h.provider, id(0x11), Some(p), "active", ACTIVE, false, 1)
        .await
        .unwrap();
    insert_tenant(
        &h.provider,
        id(0x12),
        Some(p),
        "suspended",
        SUSPENDED,
        false,
        1,
    )
    .await
    .unwrap();
    insert_provisioning_tenant(
        &h.provider,
        id(0x13),
        Some(p),
        "prov",
        1,
        OffsetDateTime::now_utc(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    insert_deleted_tenant(
        &h.provider,
        id(0x14),
        Some(p),
        "del",
        1,
        OffsetDateTime::now_utc(),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let non_deleted = h
        .repo
        .count_children(&allow_all(), p, ChildCountFilter::NonDeleted)
        .await
        .expect("count non-deleted");
    assert_eq!(
        non_deleted, 3,
        "Active + Suspended + Provisioning (Deleted excluded)"
    );

    let all = h
        .repo
        .count_children(&allow_all(), p, ChildCountFilter::All)
        .await
        .expect("count all");
    assert_eq!(all, 4, "every status including Deleted");
}

#[tokio::test]
async fn count_tenants_by_status_emits_all_combos_with_counts() {
    let h = setup_sqlite().await.expect("setup");
    // 2 Active/not-self-managed, 1 Active/self-managed, 1 Suspended/not.
    insert_tenant(&h.provider, id(0x21), None, "root", ACTIVE, false, 0)
        .await
        .unwrap();
    insert_tenant(
        &h.provider,
        id(0x22),
        Some(id(0x21)),
        "a2",
        ACTIVE,
        false,
        1,
    )
    .await
    .unwrap();
    insert_tenant(&h.provider, id(0x23), Some(id(0x21)), "sm", ACTIVE, true, 1)
        .await
        .unwrap();
    insert_tenant(
        &h.provider,
        id(0x24),
        Some(id(0x21)),
        "susp",
        SUSPENDED,
        false,
        1,
    )
    .await
    .unwrap();

    let rows = h
        .repo
        .count_tenants_by_status(&allow_all())
        .await
        .expect("count by status");

    // All 8 (status, self_managed) combos emitted, even zero ones.
    assert_eq!(rows.len(), 8, "stable 8-combo series");
    let map: BTreeMap<(i16, bool), u64> = rows
        .iter()
        .map(|(s, sm, c)| ((s.as_smallint(), *sm), *c))
        .collect();
    assert_eq!(map[&(TenantStatus::Active.as_smallint(), false)], 2);
    assert_eq!(map[&(TenantStatus::Active.as_smallint(), true)], 1);
    assert_eq!(map[&(TenantStatus::Suspended.as_smallint(), false)], 1);
    assert_eq!(
        map[&(TenantStatus::Deleted.as_smallint(), true)],
        0,
        "zero combo still present"
    );
}

#[tokio::test]
async fn count_closure_rows_counts_whole_table() {
    let h = setup_sqlite().await.expect("setup");
    assert_eq!(h.repo.count_closure_rows(&allow_all()).await.unwrap(), 0);
    insert_closure(&h.provider, id(1), id(1), 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, id(1), id(2), 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, id(2), id(2), 0, ACTIVE)
        .await
        .unwrap();
    assert_eq!(h.repo.count_closure_rows(&allow_all()).await.unwrap(), 3);
}

#[tokio::test]
async fn is_descendant_probes_closure_edge() {
    let h = setup_sqlite().await.expect("setup");
    let anc = id(0x31);
    let desc = id(0x32);
    insert_closure(&h.provider, anc, desc, 0, ACTIVE)
        .await
        .unwrap();

    assert!(
        h.repo.is_descendant(&allow_all(), anc, desc).await.unwrap(),
        "edge (anc, desc) present"
    );
    assert!(
        !h.repo.is_descendant(&allow_all(), desc, anc).await.unwrap(),
        "reverse edge absent"
    );
    assert!(
        !h.repo
            .is_descendant(&allow_all(), anc, id(0x99))
            .await
            .unwrap(),
        "unrelated pair absent"
    );
}
