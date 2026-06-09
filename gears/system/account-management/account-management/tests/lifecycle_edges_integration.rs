//! Integration tests for the `repo_impl/lifecycle.rs` methods the
//! happy-path `lifecycle_integration.rs` suite does not touch:
//! `upsert_idp_metadata` / `find_idp_metadata`, the two
//! `mark_*_terminal_failure` parking writes, and the full
//! `check_hard_delete_eligibility` branch ladder.
//!
//! Single-row fixtures seed roots (depth 0, NULL parent); the
//! deferred-child case seeds one root + one child. `SQLite` has foreign
//! keys disabled so dangling `parent_id`s are accepted.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use account_management::domain::tenant::TenantRepo;
use account_management::domain::tenant::retention::HardDeleteEligibility;
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use common::*;

fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

// =====================================================================
// upsert_idp_metadata / find_idp_metadata
// =====================================================================

#[tokio::test]
async fn find_idp_metadata_absent_returns_none() {
    let h = setup_sqlite().await.expect("setup");
    let got = h
        .repo
        .find_idp_metadata(&allow_all(), Uuid::new_v4())
        .await
        .expect("find");
    assert!(got.is_none(), "no row -> None");
}

#[tokio::test]
async fn upsert_idp_metadata_inserts_then_updates_on_conflict() {
    let h = setup_sqlite().await.expect("setup");
    let id = Uuid::new_v4();
    insert_tenant(&h.provider, id, None, "t", ACTIVE, false, 0)
        .await
        .expect("seed");

    // Insert path.
    let v1 = json!({"realm": "r1"});
    h.repo
        .upsert_idp_metadata(&allow_all(), id, Some(&v1))
        .await
        .expect("insert metadata");
    assert_eq!(
        h.repo
            .find_idp_metadata(&allow_all(), id)
            .await
            .expect("find"),
        Some(v1),
    );

    // On-conflict update path (same tenant_id).
    let v2 = json!({"realm": "r2", "extra": 7});
    h.repo
        .upsert_idp_metadata(&allow_all(), id, Some(&v2))
        .await
        .expect("update metadata");
    assert_eq!(
        h.repo
            .find_idp_metadata(&allow_all(), id)
            .await
            .expect("find"),
        Some(v2),
    );
}

// =====================================================================
// mark_provisioning_terminal_failure
// =====================================================================

#[tokio::test]
async fn mark_provisioning_terminal_failure_stamps_then_is_idempotent() {
    let h = setup_sqlite().await.expect("setup");
    let id = Uuid::new_v4();
    let worker = Uuid::new_v4();
    insert_provisioning_tenant(
        &h.provider,
        id,
        None,
        "p",
        0,
        now(),
        Some(worker),
        Some(now()),
        None,
    )
    .await
    .expect("seed claimed provisioning");

    let first = h
        .repo
        .mark_provisioning_terminal_failure(&allow_all(), id, worker, now())
        .await
        .expect("mark");
    assert!(first, "first mark stamps terminal_failure_at");
    let row = fetch_tenant(&h.provider, id).await.unwrap().unwrap();
    assert!(row.terminal_failure_at.is_some(), "stamped");

    // Idempotent: already stamped -> no-op false.
    let second = h
        .repo
        .mark_provisioning_terminal_failure(&allow_all(), id, worker, now())
        .await
        .expect("re-mark");
    assert!(!second, "second mark is a no-op (already parked)");
}

#[tokio::test]
async fn mark_provisioning_terminal_failure_respects_claim_and_status_fences() {
    let h = setup_sqlite().await.expect("setup");
    let worker = Uuid::new_v4();

    // Claim fence: row claimed by `worker`, but a different worker marks.
    let a = Uuid::new_v4();
    insert_provisioning_tenant(
        &h.provider,
        a,
        None,
        "a",
        0,
        now(),
        Some(worker),
        Some(now()),
        None,
    )
    .await
    .expect("seed a");
    let claim_lost = h
        .repo
        .mark_provisioning_terminal_failure(&allow_all(), a, Uuid::new_v4(), now())
        .await
        .expect("mark wrong worker");
    assert!(!claim_lost, "wrong worker cannot stamp");

    // Status fence: a Deleted row (claimed by `worker`) is not a
    // provisioning row, so the provisioning mark must skip it.
    let d = Uuid::new_v4();
    insert_deleted_tenant(
        &h.provider,
        d,
        Some(a),
        "d",
        1,
        now(),
        None,
        Some(worker),
        Some(now()),
        None,
    )
    .await
    .expect("seed deleted");
    let wrong_status = h
        .repo
        .mark_provisioning_terminal_failure(&allow_all(), d, worker, now())
        .await
        .expect("mark non-provisioning");
    assert!(
        !wrong_status,
        "status fence blocks marking a non-provisioning row"
    );
}

// =====================================================================
// mark_retention_terminal_failure
// =====================================================================

#[tokio::test]
async fn mark_retention_terminal_failure_stamps_deleted_row_only() {
    let h = setup_sqlite().await.expect("setup");
    let worker = Uuid::new_v4();

    // Happy: Deleted row claimed by `worker`.
    let del = Uuid::new_v4();
    insert_deleted_tenant(
        &h.provider,
        del,
        None,
        "del",
        0,
        now(),
        None,
        Some(worker),
        Some(now()),
        None,
    )
    .await
    .expect("seed deleted");
    let ok = h
        .repo
        .mark_retention_terminal_failure(&allow_all(), del, worker, now())
        .await
        .expect("mark");
    assert!(ok, "retention mark stamps a claimed Deleted row");

    // Status fence: a Provisioning row is not retention's to park.
    let prov = Uuid::new_v4();
    insert_provisioning_tenant(
        &h.provider,
        prov,
        Some(del),
        "p",
        1,
        now(),
        Some(worker),
        Some(now()),
        None,
    )
    .await
    .expect("seed provisioning");
    let blocked = h
        .repo
        .mark_retention_terminal_failure(&allow_all(), prov, worker, now())
        .await
        .expect("mark provisioning via retention");
    assert!(
        !blocked,
        "status fence blocks retention-marking a provisioning row"
    );
}

// =====================================================================
// check_hard_delete_eligibility
// =====================================================================

#[tokio::test]
async fn eligibility_not_eligible_when_row_gone() {
    let h = setup_sqlite().await.expect("setup");
    let got = h
        .repo
        .check_hard_delete_eligibility(&allow_all(), Uuid::new_v4(), Uuid::new_v4())
        .await
        .expect("preflight");
    assert!(
        matches!(got, HardDeleteEligibility::NotEligible),
        "got {got:?}"
    );
}

#[tokio::test]
async fn eligibility_not_eligible_when_not_deleted() {
    let h = setup_sqlite().await.expect("setup");
    let id = Uuid::new_v4();
    insert_tenant(&h.provider, id, None, "active", ACTIVE, false, 0)
        .await
        .expect("seed");
    let got = h
        .repo
        .check_hard_delete_eligibility(&allow_all(), id, Uuid::new_v4())
        .await
        .expect("preflight");
    assert!(
        matches!(got, HardDeleteEligibility::NotEligible),
        "active row not eligible: {got:?}"
    );
}

#[tokio::test]
async fn eligibility_not_eligible_when_claim_lost() {
    let h = setup_sqlite().await.expect("setup");
    let id = Uuid::new_v4();
    let worker = Uuid::new_v4();
    insert_deleted_tenant(
        &h.provider,
        id,
        None,
        "del",
        0,
        now(),
        None,
        Some(worker),
        Some(now()),
        None,
    )
    .await
    .expect("seed");
    let got = h
        .repo
        .check_hard_delete_eligibility(&allow_all(), id, Uuid::new_v4()) // different worker
        .await
        .expect("preflight");
    assert!(
        matches!(got, HardDeleteEligibility::NotEligible),
        "claim mismatch: {got:?}"
    );
}

#[tokio::test]
async fn eligibility_deferred_when_child_present() {
    let h = setup_sqlite().await.expect("setup");
    let parent = Uuid::new_v4();
    let child = Uuid::new_v4();
    let worker = Uuid::new_v4();
    insert_deleted_tenant(
        &h.provider,
        parent,
        None,
        "parent",
        0,
        now(),
        None,
        Some(worker),
        Some(now()),
        None,
    )
    .await
    .expect("seed parent");
    insert_tenant(&h.provider, child, Some(parent), "child", ACTIVE, false, 1)
        .await
        .expect("seed child");
    let got = h
        .repo
        .check_hard_delete_eligibility(&allow_all(), parent, worker)
        .await
        .expect("preflight");
    assert!(
        matches!(got, HardDeleteEligibility::DeferredChildPresent),
        "got {got:?}"
    );
}

#[tokio::test]
async fn eligibility_eligible_when_claimed_deleted_leaf() {
    let h = setup_sqlite().await.expect("setup");
    let id = Uuid::new_v4();
    let worker = Uuid::new_v4();
    insert_deleted_tenant(
        &h.provider,
        id,
        None,
        "leaf",
        0,
        now(),
        None,
        Some(worker),
        Some(now()),
        None,
    )
    .await
    .expect("seed");
    let got = h
        .repo
        .check_hard_delete_eligibility(&allow_all(), id, worker)
        .await
        .expect("preflight");
    assert!(
        matches!(got, HardDeleteEligibility::Eligible),
        "got {got:?}"
    );
}
