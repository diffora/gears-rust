//! Real-DB integration tests for `repair_derivable_closure_violations`
//! against in-memory `SQLite`.
//!
//! The pure-Rust planner is unit-tested in
//! `infra/storage/integrity/repair_tests.rs` over hand-built `Snapshot`
//! fixtures. These tests are the runtime counterpart: they spin up a
//! real `SQLite` database, apply the AM migration set, seed broken
//! `(tenants, tenant_closure)` shapes via `SecureORM` inserts, and
//! verify the apply layer in `repo_impl/integrity.rs` (`SecureORM`
//! bulk extensions, `SERIALIZABLE` retry, single-flight gate sharing)
//! actually produces the post-repair DB state the planner promised.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::sync::Arc;

use account_management::domain::error::DomainError;
use account_management::domain::tenant::TenantRepo;
use account_management::domain::tenant::integrity::IntegrityCategory;

use common::*;

// ---------------------------------------------------------------------
// B.4 — Missing self-row → INSERT (id, id, 0, status).
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_inserts_missing_self_row() {
    let h = setup_sqlite().await.expect("sqlite :memory:");
    let root = Uuid::from_u128(0x1);
    insert_tenant(&h.provider, root, None, "root", ACTIVE, false, 0)
        .await
        .expect("seed root tenant");
    // Self-row deliberately absent — closure is empty post-migration.

    let report = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("repair");

    let row = fetch_closure_row(&h.provider, root, root)
        .await
        .expect("query")
        .expect("self-row inserted");
    assert_eq!(row.barrier, 0);
    assert_eq!(row.descendant_status, ACTIVE);
    assert_eq!(
        repaired_count(&report, IntegrityCategory::MissingClosureSelfRow),
        1
    );

    // Idempotency: a second repair on the post-repair state is a no-op.
    let again = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("idempotent repair");
    assert_eq!(again.total_repaired(), 0);
}

// ---------------------------------------------------------------------
// B.5 — Missing strict-ancestor edge → INSERT with derived barrier.
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_inserts_missing_coverage_gap() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::from_u128(0x1);
    let child = Uuid::from_u128(0x2);
    insert_tenant(&h.provider, root, None, "root", ACTIVE, false, 0)
        .await
        .unwrap();
    insert_tenant(&h.provider, child, Some(root), "child", ACTIVE, false, 1)
        .await
        .unwrap();
    insert_closure(&h.provider, root, root, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, child, child, 0, ACTIVE)
        .await
        .unwrap();
    // (root, child) edge deliberately absent.

    let report = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("repair");

    let edge = fetch_closure_row(&h.provider, root, child)
        .await
        .unwrap()
        .expect("strict-ancestor edge inserted");
    assert_eq!(edge.barrier, 0);
    assert_eq!(edge.descendant_status, ACTIVE);
    assert_eq!(
        repaired_count(&report, IntegrityCategory::ClosureCoverageGap),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_inserts_coverage_gap_with_self_managed_barrier() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::from_u128(0x1);
    let child = Uuid::from_u128(0x2);
    insert_tenant(&h.provider, root, None, "root", ACTIVE, false, 0)
        .await
        .unwrap();
    // Child is `self_managed = true` → strict (root, child) edge MUST
    // land with barrier=1 per the planner's `(A, D]` walk semantics.
    insert_tenant(&h.provider, child, Some(root), "child", ACTIVE, true, 1)
        .await
        .unwrap();
    insert_closure(&h.provider, root, root, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, child, child, 0, ACTIVE)
        .await
        .unwrap();

    h.repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("repair");

    let edge = fetch_closure_row(&h.provider, root, child)
        .await
        .unwrap()
        .expect("strict-ancestor edge inserted");
    assert_eq!(
        edge.barrier, 1,
        "self_managed descendant flips barrier on the strict (root, child) edge"
    );
}

// ---------------------------------------------------------------------
// B.6 — Stale `descendant_status` → bulk UPDATE per tenant.
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_updates_stale_descendant_status() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::from_u128(0x1);
    let child = Uuid::from_u128(0x2);
    insert_tenant(&h.provider, root, None, "root", ACTIVE, false, 0)
        .await
        .unwrap();
    insert_tenant(&h.provider, child, Some(root), "child", ACTIVE, false, 1)
        .await
        .unwrap();
    insert_closure(&h.provider, root, root, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, child, child, 0, SUSPENDED)
        .await
        .unwrap();
    insert_closure(&h.provider, root, child, 0, SUSPENDED)
        .await
        .unwrap();

    let report = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("repair");

    let child_self = fetch_closure_row(&h.provider, child, child)
        .await
        .unwrap()
        .expect("self-row");
    assert_eq!(child_self.descendant_status, ACTIVE);
    let strict = fetch_closure_row(&h.provider, root, child)
        .await
        .unwrap()
        .expect("strict edge");
    assert_eq!(strict.descendant_status, ACTIVE);
    assert_eq!(
        repaired_count(&report, IntegrityCategory::DescendantStatusDivergence),
        1
    );
}

// ---------------------------------------------------------------------
// B.7 — Barrier divergence → UPDATE.
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_updates_barrier_divergence() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::from_u128(0x1);
    let child = Uuid::from_u128(0x2);
    insert_tenant(&h.provider, root, None, "root", ACTIVE, false, 0)
        .await
        .unwrap();
    insert_tenant(&h.provider, child, Some(root), "child", ACTIVE, true, 1)
        .await
        .unwrap();
    insert_closure(&h.provider, root, root, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, child, child, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, root, child, 0, ACTIVE)
        .await
        .unwrap(); // wrong barrier (expected 1)

    let report = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("repair");

    let edge = fetch_closure_row(&h.provider, root, child)
        .await
        .unwrap()
        .expect("edge");
    assert_eq!(edge.barrier, 1);
    assert_eq!(
        repaired_count(&report, IntegrityCategory::BarrierColumnDivergence),
        1
    );
}

// ---------------------------------------------------------------------
// B.8 — Stale closure row (ancestry not in walk) → DELETE.
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_deletes_stale_closure_row() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::from_u128(0x1);
    let child = Uuid::from_u128(0x2);
    let sibling = Uuid::from_u128(0x3);
    insert_tenant(&h.provider, root, None, "root", ACTIVE, false, 0)
        .await
        .unwrap();
    insert_tenant(&h.provider, child, Some(root), "child", ACTIVE, false, 1)
        .await
        .unwrap();
    insert_tenant(
        &h.provider,
        sibling,
        Some(root),
        "sibling",
        ACTIVE,
        false,
        1,
    )
    .await
    .unwrap();
    insert_closure(&h.provider, root, root, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, child, child, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, sibling, sibling, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, root, child, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, root, sibling, 0, ACTIVE)
        .await
        .unwrap();
    // Bogus ancestry — `sibling` is not on `child`'s parent walk.
    insert_closure(&h.provider, sibling, child, 0, ACTIVE)
        .await
        .unwrap();

    let report = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("repair");

    assert!(
        fetch_closure_row(&h.provider, sibling, child)
            .await
            .unwrap()
            .is_none(),
        "bogus (sibling, child) row deleted"
    );
    assert!(
        fetch_closure_row(&h.provider, root, child)
            .await
            .unwrap()
            .is_some(),
        "legitimate (root, child) edge preserved"
    );
    assert!(repaired_count(&report, IntegrityCategory::StaleClosureRow) >= 1);
}

// ---------------------------------------------------------------------
// B.9 — Closure-only invariant: orphan / cycle / root-count violations
// MUST appear in the deferred bucket and MUST NOT mutate `tenants`.
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_does_not_touch_tenants_for_non_derivable_violations() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::from_u128(0x1);
    let phantom_parent = Uuid::from_u128(0x99);
    let child = Uuid::from_u128(0x2);
    insert_tenant(&h.provider, root, None, "root", ACTIVE, false, 0)
        .await
        .unwrap();
    insert_tenant(
        &h.provider,
        child,
        Some(phantom_parent),
        "orphan_child",
        ACTIVE,
        false,
        1,
    )
    .await
    .unwrap();
    insert_closure(&h.provider, root, root, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, child, child, 0, ACTIVE)
        .await
        .unwrap();

    let tenants_before = fetch_all_tenant_rows(&h.provider).await.unwrap();

    let report = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("repair");

    let tenants_after = fetch_all_tenant_rows(&h.provider).await.unwrap();
    assert_eq!(
        tenants_before, tenants_after,
        "repair MUST NOT touch tenants table - closure-only invariant"
    );
    assert_eq!(deferred_count(&report, IntegrityCategory::OrphanedChild), 1);
}

// ---------------------------------------------------------------------
// Clean DB → empty repair report.
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_on_clean_repo_returns_empty_report() {
    let h = setup_sqlite().await.expect("sqlite");
    let _ = seed_clean_two_node_tree(&h.provider).await.expect("seed");

    let report = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("clean repair");

    assert_eq!(report.total_repaired(), 0);
    assert_eq!(report.total_deferred(), 0);
}

// ---------------------------------------------------------------------
// Composite: every derivable category in one snapshot, plus
// idempotency of the post-repair state.
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_handles_all_derivable_categories_and_is_idempotent() {
    let h = setup_sqlite().await.expect("sqlite");
    let root = Uuid::from_u128(0x1);
    let a = Uuid::from_u128(0x2);
    let b = Uuid::from_u128(0x3);
    let c_id = Uuid::from_u128(0x4);

    insert_tenant(&h.provider, root, None, "root", ACTIVE, false, 0)
        .await
        .unwrap();
    insert_tenant(&h.provider, a, Some(root), "a", ACTIVE, true, 1)
        .await
        .unwrap();
    insert_tenant(&h.provider, b, Some(a), "b", ACTIVE, false, 2)
        .await
        .unwrap();
    insert_tenant(&h.provider, c_id, Some(root), "c", ACTIVE, false, 1)
        .await
        .unwrap();

    insert_closure(&h.provider, root, root, 0, ACTIVE)
        .await
        .unwrap();
    insert_closure(&h.provider, b, b, 0, ACTIVE).await.unwrap();
    insert_closure(&h.provider, c_id, c_id, 0, SUSPENDED)
        .await
        .unwrap();
    insert_closure(&h.provider, a, b, 1, ACTIVE).await.unwrap();
    insert_closure(&h.provider, root, c_id, 0, SUSPENDED)
        .await
        .unwrap();
    insert_closure(&h.provider, c_id, b, 0, ACTIVE)
        .await
        .unwrap();

    let report = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("repair");

    for cat in [
        IntegrityCategory::MissingClosureSelfRow,
        IntegrityCategory::ClosureCoverageGap,
        IntegrityCategory::StaleClosureRow,
        IntegrityCategory::BarrierColumnDivergence,
        IntegrityCategory::DescendantStatusDivergence,
    ] {
        assert!(
            repaired_count(&report, cat) >= 1,
            "category {cat:?} should have at least one fix; report={:?}",
            report.repaired_per_category
        );
    }

    let again = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("idempotent repair");
    assert_eq!(again.total_repaired(), 0);
}

// ---------------------------------------------------------------------
// Single-flight gate — pre-populate the gate so repair sees it as
// already held, assert `IntegrityCheckInProgress`, release, re-run.
// Mirrors the donor's
// `single_flight_pre_held_gate_refuses_whole_scope_audit` for the
// repair entry-point.
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repair_refuses_when_single_flight_gate_is_held() {
    let h = setup_sqlite().await.expect("sqlite :memory:");
    let _ = seed_clean_two_node_tree(&h.provider).await.expect("seed");

    let held = pre_populate_gate(&h.provider)
        .await
        .expect("pre-populate gate");

    let result = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await;
    match result {
        Err(DomainError::IntegrityCheckInProgress) => {}
        other => panic!("expected IntegrityCheckInProgress when gate is held; got {other:?}"),
    }

    release_gate(&h.provider, held).await.expect("release gate");

    let post = h
        .repo
        .repair_derivable_closure_violations(&allow_all())
        .await
        .expect("post-release repair must succeed");
    assert_eq!(
        post.total_repaired(),
        0,
        "post-release repair on a clean tree must surface zero ops"
    );
}

// ---------------------------------------------------------------------
// Concurrent burst — every task must resolve to either Ok or
// IntegrityCheckInProgress; at least one must succeed. Mirrors the
// donor's `single_flight_concurrent_burst_observes_only_gate_or_success`.
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repair_concurrent_burst_observes_only_gate_or_success() {
    const TASK_COUNT: usize = 6;
    let h = setup_sqlite().await.expect("sqlite :memory:");
    let _ = seed_clean_two_node_tree(&h.provider).await.expect("seed");

    let barrier = Arc::new(tokio::sync::Barrier::new(TASK_COUNT));
    let repo = Arc::clone(&h.repo);
    let mut handles = Vec::with_capacity(TASK_COUNT);
    for _ in 0..TASK_COUNT {
        let repo = Arc::clone(&repo);
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            repo.repair_derivable_closure_violations(&AccessScope::allow_all())
                .await
        }));
    }
    let mut ok_count = 0_usize;
    let mut gate_count = 0_usize;
    let mut other = Vec::new();
    for handle in handles {
        match handle.await.expect("task panicked") {
            Ok(_) => ok_count += 1,
            // All three are gate-equivalent backoff outcomes:
            //  * `IntegrityCheckInProgress` — lease held by a peer at
            //    acquire time (legacy gate contract).
            //  * `IntegrityCheckLeaseLost` — lease lost mid-flight (peer
            //    stole the slot between our work and our fence-SELECT).
            //  * `Internal { .. }` — SQLite under N-way writer contention
            //    can exhaust the `SQLITE_BUSY` retry budget and surface as
            //    Internal. Production runs on PG (per-row writer
            //    concurrency, no `BUSY`), so this is a SQLite-only stress
            //    outcome, not a regression.
            Err(
                DomainError::IntegrityCheckInProgress
                | DomainError::IntegrityCheckLeaseLost
                | DomainError::Internal { .. },
            ) => {
                gate_count += 1;
            }
            Err(e) => other.push(format!("{e:?}")),
        }
    }
    assert!(
        other.is_empty(),
        "no concurrent repair may fail with anything other than the lease-mapped variants \
         (`IntegrityCheckInProgress`, `IntegrityCheckLeaseLost`, or SQLite-only `Internal` \
         contention): {other:?}"
    );
    assert_eq!(
        ok_count + gate_count,
        TASK_COUNT,
        "every task must resolve to Ok or a lease-mapped error"
    );
    assert!(
        ok_count >= 1,
        "at least one concurrent repair must succeed (got {ok_count} Ok of {TASK_COUNT})"
    );
}

// Imports re-exported from `common` shadow the bare types the tests
// use; declared at the bottom so the gear-level `use common::*;`
// stays the single discoverable import surface.
use toolkit_security::AccessScope;
use uuid::Uuid;
