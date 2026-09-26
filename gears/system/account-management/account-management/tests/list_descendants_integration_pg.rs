//! Real-Postgres tests for the recursive children listing. Only the
//! cases whose behaviour differs from `SQLite`:
//!
//! * `LIKE` is case-sensitive on Postgres (ASCII-case-insensitive on
//!   `SQLite`), so `contains(name,'X')` must NOT match `x…` here —
//!   the contract documents `contains` as case-sensitive.
//! * The closure `IN (subquery)` pin and the keyset cursor run against
//!   the real planner and real `SERIALIZABLE` snapshot rules.
//!
//! Gated behind `#[cfg(feature = "integration")]` like every other
//! `*_integration_pg.rs` file: it needs a running Docker daemon
//! (testcontainers) and is not part of the default `cargo test`. Run
//! with `--features integration`; a missing daemon fails loudly, it
//! never skips.
#![cfg(feature = "integration")]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use account_management::domain::tenant::TenantRepo;
use toolkit_odata::{CursorV1, ODataQuery};
use uuid::Uuid;

use common::pg::bring_up_postgres;
use common::{
    BarrierTopology, contains_name, relaxed_scope, respect_scope, seed_barrier_topology, sorted,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_list_descendants_visible_set_matches_the_topology_table() {
    let h = bring_up_postgres().await.expect("postgres");
    let t = BarrierTopology::new();
    seed_barrier_topology(&h.provider, &t).await.expect("seed");

    let page = h
        .repo
        .list_descendants(
            &respect_scope(t.root),
            &relaxed_scope(t.root),
            t.root,
            &ODataQuery::default(),
        )
        .await
        .expect("list");
    let ids: Vec<Uuid> = page.items.iter().map(|m| m.id).collect();
    assert_eq!(sorted(ids), sorted(vec![t.x, t.xc, t.s, t.y]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_list_descendants_contains_is_case_sensitive() {
    let h = bring_up_postgres().await.expect("postgres");
    let t = BarrierTopology::new();
    seed_barrier_topology(&h.provider, &t).await.expect("seed");

    let lower = h
        .repo
        .list_descendants(
            &respect_scope(t.root),
            &relaxed_scope(t.root),
            t.root,
            &contains_name("x"),
        )
        .await
        .expect("lower");
    let lower_ids: Vec<Uuid> = lower.items.iter().map(|m| m.id).collect();
    assert_eq!(sorted(lower_ids), sorted(vec![t.x, t.xc]));

    let upper = h
        .repo
        .list_descendants(
            &respect_scope(t.root),
            &relaxed_scope(t.root),
            t.root,
            &contains_name("X"),
        )
        .await
        .expect("upper");
    assert!(
        upper.items.is_empty(),
        "Postgres LIKE is case-sensitive; the contract documents contains() as such"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_list_descendants_cursor_walk_covers_visible_set_once() {
    let h = bring_up_postgres().await.expect("postgres");
    let t = BarrierTopology::new();
    seed_barrier_topology(&h.provider, &t).await.expect("seed");

    let mut seen: Vec<Uuid> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let mut q = ODataQuery::default().with_limit(1);
        if let Some(c) = cursor.as_deref() {
            q = q.with_cursor(CursorV1::decode(c).expect("decode"));
        }
        let page = h
            .repo
            .list_descendants(&respect_scope(t.root), &relaxed_scope(t.root), t.root, &q)
            .await
            .expect("page");
        seen.extend(page.items.iter().map(|m| m.id));
        pages += 1;
        assert!(pages <= 5, "walk must terminate");
        match page.page_info.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(pages, 4);
    assert_eq!(sorted(seen), sorted(vec![t.x, t.xc, t.s, t.y]));
}
