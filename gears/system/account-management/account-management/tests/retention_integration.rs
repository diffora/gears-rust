//! Integration tests for the retention scanner repo methods
//! (`scan_retention_due`, `clear_retention_claim`) against a real
//! in-memory `SQLite` database.
//!
//! These drive the production `TenantRepoImpl` execution paths that the
//! inline unit tests in `repo_impl/retention_tests.rs` cannot reach —
//! those pin the SQL *string* (filter-bind order + leaf-first ORDER BY
//! snapshot), while these exercise the actual claim-and-go transaction
//! against rows on disk.
//!
//! Coverage map (vs the historical scaffold spec):
//!
//! 1. **`scan_retention_due` ordering** — rows at multiple depths with
//!    `deleted_at` straddling the window are returned in the canonical
//!    `(depth DESC, deleted_at ASC, id ASC)` order, and not-yet-due rows
//!    are excluded.
//! 2. **`is_due` SQL vs Rust parity** — a row whose `deleted_at` is
//!    exactly `now - retention_window` IS due (inclusive boundary).
//! 3. **Default vs per-row retention window** — `retention_window_secs
//!    = NULL` falls back to the scan's gear default; an explicit
//!    override makes a row due (or not) on its own clock.
//! 4. **Claim-and-go** — a scanned row is stamped with the worker
//!    claim; an immediate re-scan returns nothing (claim is fresh).
//! 5. **Stale-claim takeover** — a claim older than `RETENTION_CLAIM_TTL`
//!    is re-claimable; a fresh peer claim is skipped.
//! 6. **Park filters** — `terminal_failure_at`-stamped and non-`Deleted`
//!    rows never enter the scan.
//! 7. **`clear_retention_claim`** — releases only when the worker token
//!    matches; a wrong token is a no-op.
//!
//! All scanned rows are seeded as **non-root** (depth > 0, non-NULL
//! `parent_id`): `ck_tenants_root_depth` ties depth 0 to a NULL parent
//! and `ux_tenants_single_root` allows only one such row, while the
//! retention scan never walks `parent_id`. `SQLite` has foreign keys
//! disabled (`toolkit-db` does not set `PRAGMA foreign_keys`), so the
//! dangling `parent_id` is accepted. The `Postgres`-only leaf-first
//! FK-guard scenario stays in the `*_pg.rs` companion suite.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::time::Duration;

use account_management::domain::tenant::TenantRepo;
use account_management::domain::tenant::retention::is_due;
use time::OffsetDateTime;
use uuid::Uuid;

use common::*;

/// Gear-default retention window the scan applies to rows whose
/// `retention_window_secs` column is NULL. One hour keeps the
/// arithmetic well clear of sub-second clock skew.
const DEFAULT_WINDOW: Duration = Duration::from_hours(1);

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// Dangling, non-NULL parent so seeded rows are non-root (depth > 0).
/// Returns `Option` to drop straight into the `parent_id` seed args.
#[allow(clippy::unnecessary_wraps)]
fn parent() -> Option<Uuid> {
    Some(id(0xF000))
}

fn hours(n: i64) -> time::Duration {
    time::Duration::hours(n)
}

/// Seed a due/undue `Deleted` row with a NULL window at `depth`.
async fn seed(h: &Harness, n: u128, depth: i32, deleted_ago: time::Duration) {
    insert_deleted_tenant(
        &h.provider,
        id(n),
        parent(),
        "t",
        depth,
        OffsetDateTime::now_utc() - deleted_ago,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap_or_else(|e| panic!("seed {n:#x}: {e:?}"));
}

/// `scan_retention_due` returns every due row exactly once, ordered
/// leaf-first by `(depth DESC, deleted_at ASC, id ASC)`, and omits a
/// row still inside its retention window.
#[tokio::test]
async fn scan_returns_due_rows_leaf_first_and_excludes_not_yet_due() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();

    // All due (deleted well past the 1h default window).
    seed(&h, 0x01, 3, hours(2)).await; // deepest
    seed(&h, 0x02, 2, hours(3)).await; // mid, deleted earliest
    seed(&h, 0x03, 2, hours(2)).await; // mid, deleted later
    seed(&h, 0x04, 1, hours(5)).await; // shallow
    // Not yet due — deleted only 30 min ago, default window is 1h.
    seed(&h, 0x05, 2, time::Duration::minutes(30)).await;
    // A non-Deleted row must never enter the scan.
    insert_tenant(&h.provider, id(0x06), parent(), "active", ACTIVE, false, 1)
        .await
        .expect("seed active");

    let rows = h
        .repo
        .scan_retention_due(&allow_all(), now, DEFAULT_WINDOW, 100)
        .await
        .expect("scan");

    let ordered: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    assert_eq!(
        ordered,
        vec![id(0x01), id(0x02), id(0x03), id(0x04)],
        "expected leaf-first (depth DESC, deleted_at ASC, id ASC) order, got {ordered:?}"
    );
    // Every returned row carries the same freshly-stamped worker claim.
    let worker = rows[0].claimed_by;
    assert_ne!(worker, Uuid::nil(), "claim worker must be stamped");
    assert!(
        rows.iter().all(|r| r.claimed_by == worker),
        "all rows in one scan share the scan's worker token"
    );
}

/// The inclusive boundary: `deleted_at == now - window` is due. Asserts
/// the SQL predicate and the Rust `is_due` defense-in-depth agree.
#[tokio::test]
async fn scan_due_at_exact_window_boundary() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();
    let deleted_at = now - hours(1); // exactly one default window ago

    insert_deleted_tenant(
        &h.provider,
        id(0x11),
        parent(),
        "boundary",
        1,
        deleted_at,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("seed");

    assert!(
        is_due(now, deleted_at, DEFAULT_WINDOW),
        "Rust is_due must treat the exact boundary as due"
    );
    let rows = h
        .repo
        .scan_retention_due(&allow_all(), now, DEFAULT_WINDOW, 100)
        .await
        .expect("scan");
    assert_eq!(
        rows.len(),
        1,
        "boundary row must be selected by the SQL scan"
    );
    assert_eq!(rows[0].id, id(0x11));
}

/// `retention_window_secs = NULL` uses the scan's gear default; an
/// explicit per-row override is honoured independently.
#[tokio::test]
async fn scan_applies_default_and_per_row_window() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();

    let mk = |n: u128, deleted_ago: time::Duration, window: Option<i64>| {
        insert_deleted_tenant(
            &h.provider,
            id(n),
            parent(),
            "t",
            1,
            now - deleted_ago,
            window,
            None,
            None,
            None,
        )
    };

    // NULL window → default 1h applies. Deleted 2h ago ⇒ due.
    mk(0x21, hours(2), None).await.expect("seed null-due");
    // NULL window, deleted 30 min ago ⇒ NOT due under the 1h default.
    mk(0x22, time::Duration::minutes(30), None)
        .await
        .expect("seed null-fresh");
    // Short 60s override, deleted 2 min ago ⇒ due despite the 1h default.
    mk(0x23, time::Duration::minutes(30), Some(600))
        .await
        .expect("seed override-due");
    // Long 1-week override, deleted 2h ago ⇒ NOT due despite outliving
    // the default window.
    mk(0x24, hours(2), Some(7 * 24 * 3600))
        .await
        .expect("seed override-fresh");

    let due: std::collections::BTreeSet<Uuid> = h
        .repo
        .scan_retention_due(&allow_all(), now, DEFAULT_WINDOW, 100)
        .await
        .expect("scan")
        .into_iter()
        .map(|r| r.id)
        .collect();

    assert!(
        due.contains(&id(0x21)),
        "NULL-window row past default must be due"
    );
    assert!(due.contains(&id(0x23)), "short-override row must be due");
    assert!(
        !due.contains(&id(0x22)),
        "NULL-window fresh row must not be due"
    );
    assert!(
        !due.contains(&id(0x24)),
        "long-override row must not be due"
    );
}

/// `limit` selects leaf-first under the starvation contract: a deeper
/// (leaf) row deleted *latest* still beats shallow parents deleted
/// earliest when `limit` is smaller than the candidate set.
#[tokio::test]
async fn scan_limit_picks_leaf_first_over_older_parents() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();

    // Two shallow rows deleted long ago.
    seed(&h, 0x31, 1, hours(10)).await;
    seed(&h, 0x32, 1, hours(9)).await;
    // One deep leaf deleted most recently (but still due).
    seed(&h, 0x33, 3, hours(2)).await;

    let rows = h
        .repo
        .scan_retention_due(&allow_all(), now, DEFAULT_WINDOW, 1)
        .await
        .expect("scan");

    assert_eq!(rows.len(), 1, "limit=1 must return exactly one row");
    assert_eq!(
        rows[0].id,
        id(0x33),
        "leaf (depth DESC) must win the LIMIT window over older shallow rows"
    );
}

/// Claim-and-go: a scanned row is stamped with the worker claim, so an
/// immediate re-scan (claim still fresh) returns nothing.
#[tokio::test]
async fn scan_claims_row_so_immediate_rescan_is_empty() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();
    seed(&h, 0x41, 1, hours(2)).await;

    let first = h
        .repo
        .scan_retention_due(&allow_all(), now, DEFAULT_WINDOW, 100)
        .await
        .expect("first scan");
    assert_eq!(first.len(), 1, "first scan claims the row");

    let second = h
        .repo
        .scan_retention_due(&allow_all(), now, DEFAULT_WINDOW, 100)
        .await
        .expect("second scan");
    assert!(
        second.is_empty(),
        "row claimed by the first scan must not be re-claimed while the claim is fresh"
    );

    // The claim is persisted on the row.
    let row = fetch_tenant(&h.provider, id(0x41))
        .await
        .expect("fetch")
        .expect("row exists");
    assert_eq!(row.claimed_by, Some(first[0].claimed_by));
    assert!(row.claimed_at.is_some(), "claimed_at stamped");
}

/// A claim older than `RETENTION_CLAIM_TTL` (10 min) is re-claimable;
/// a fresh peer claim is skipped.
#[tokio::test]
async fn scan_reclaims_stale_claim_but_skips_fresh_peer_claim() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();
    let peer = id(0xDEAD);

    // Stale claim — 11 min old, past the 10-min TTL ⇒ reclaimable.
    insert_deleted_tenant(
        &h.provider,
        id(0x51),
        parent(),
        "stale",
        1,
        now - hours(2),
        None,
        Some(peer),
        Some(now - time::Duration::minutes(11)),
        None,
    )
    .await
    .expect("seed stale");
    // Fresh claim — 1 min old ⇒ owned by the peer, skipped.
    insert_deleted_tenant(
        &h.provider,
        id(0x52),
        parent(),
        "fresh-claim",
        1,
        now - hours(2),
        None,
        Some(peer),
        Some(now - time::Duration::minutes(1)),
        None,
    )
    .await
    .expect("seed fresh-claim");

    let rows = h
        .repo
        .scan_retention_due(&allow_all(), now, DEFAULT_WINDOW, 100)
        .await
        .expect("scan");

    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    assert_eq!(
        ids,
        vec![id(0x51)],
        "only the stale-claim row is reclaimable"
    );
    assert_ne!(
        rows[0].claimed_by, peer,
        "stale claim taken over by a new worker"
    );
}

/// Rows parked with `terminal_failure_at` never enter the scan.
#[tokio::test]
async fn scan_excludes_terminal_failure_rows() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();

    insert_deleted_tenant(
        &h.provider,
        id(0x61),
        parent(),
        "parked",
        1,
        now - hours(2),
        None,
        None,
        None,
        Some(now - hours(1)),
    )
    .await
    .expect("seed parked");
    seed(&h, 0x62, 1, hours(2)).await; // live

    let ids: Vec<Uuid> = h
        .repo
        .scan_retention_due(&allow_all(), now, DEFAULT_WINDOW, 100)
        .await
        .expect("scan")
        .into_iter()
        .map(|r| r.id)
        .collect();

    assert_eq!(
        ids,
        vec![id(0x62)],
        "terminal-failure row must be parked out of the scan"
    );
}

/// `clear_retention_claim` releases the row only for the worker that
/// holds it; a wrong token is a no-op (stale-takeover protection).
#[tokio::test]
async fn clear_retention_claim_matches_worker_token() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();
    seed(&h, 0x71, 1, hours(2)).await;

    let claimed = h
        .repo
        .scan_retention_due(&allow_all(), now, DEFAULT_WINDOW, 100)
        .await
        .expect("scan");
    let worker = claimed[0].claimed_by;

    // Wrong token: no-op, claim survives.
    h.repo
        .clear_retention_claim(&allow_all(), id(0x71), Uuid::new_v4())
        .await
        .expect("clear wrong token");
    let still = fetch_tenant(&h.provider, id(0x71))
        .await
        .expect("fetch")
        .expect("row");
    assert_eq!(
        still.claimed_by,
        Some(worker),
        "wrong token must not release the claim"
    );

    // Correct token: claim released, row becomes scannable again.
    h.repo
        .clear_retention_claim(&allow_all(), id(0x71), worker)
        .await
        .expect("clear right token");
    let released = fetch_tenant(&h.provider, id(0x71))
        .await
        .expect("fetch")
        .expect("row");
    assert_eq!(
        released.claimed_by, None,
        "matching token must release the claim"
    );

    let rescan = h
        .repo
        .scan_retention_due(&allow_all(), now, DEFAULT_WINDOW, 100)
        .await
        .expect("rescan");
    assert_eq!(rescan.len(), 1, "released row is re-claimable");
    assert_eq!(rescan[0].id, id(0x71));
}

// ---------------------------------------------------------------------
// scan_stuck_provisioning — the provisioning-reaper scan
// ---------------------------------------------------------------------

fn mins(n: i64) -> time::Duration {
    time::Duration::minutes(n)
}

/// Seed an unclaimed `Provisioning` row created `created_ago` before now.
async fn seed_prov(h: &Harness, n: u128, created_ago: time::Duration) {
    insert_provisioning_tenant(
        &h.provider,
        id(n),
        parent(),
        "p",
        1,
        OffsetDateTime::now_utc() - created_ago,
        None,
        None,
        None,
    )
    .await
    .unwrap_or_else(|e| panic!("seed prov {n:#x}: {e:?}"));
}

/// Only `Provisioning` rows created at/before `older_than` are claimed,
/// oldest-first; recent and non-`Provisioning` rows are excluded.
#[tokio::test]
async fn stuck_provisioning_claims_old_rows_oldest_first() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();
    let older_than = now - hours(1);

    seed_prov(&h, 0x81, hours(3)).await; // oldest stuck
    seed_prov(&h, 0x82, hours(2)).await; // stuck
    seed_prov(&h, 0x83, mins(30)).await; // too recent ⇒ not stuck
    // Active row is never reaper-eligible regardless of age.
    insert_tenant(&h.provider, id(0x84), parent(), "active", ACTIVE, false, 1)
        .await
        .expect("seed active");

    let rows = h
        .repo
        .scan_stuck_provisioning(&allow_all(), now, older_than, 100)
        .await
        .expect("scan");

    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    assert_eq!(
        ids,
        vec![id(0x81), id(0x82)],
        "stuck rows returned oldest-first (created_at ASC), recent + active excluded"
    );
    let worker = rows[0].claimed_by;
    assert_ne!(worker, Uuid::nil());
    assert!(rows.iter().all(|r| r.claimed_by == worker));
}

/// `terminal_failure_at`-stamped provisioning rows are parked out of
/// the reaper scan.
#[tokio::test]
async fn stuck_provisioning_excludes_terminal_failure() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();
    let older_than = now - hours(1);

    insert_provisioning_tenant(
        &h.provider,
        id(0x91),
        parent(),
        "parked",
        1,
        now - hours(3),
        None,
        None,
        Some(now - hours(1)),
    )
    .await
    .expect("seed parked");
    seed_prov(&h, 0x92, hours(3)).await; // live

    let ids: Vec<Uuid> = h
        .repo
        .scan_stuck_provisioning(&allow_all(), now, older_than, 100)
        .await
        .expect("scan")
        .into_iter()
        .map(|r| r.id)
        .collect();

    assert_eq!(
        ids,
        vec![id(0x92)],
        "terminal-failure row parked out of reaper scan"
    );
}

/// Claim-and-go + claim TTL: a scanned row is claimed (second scan
/// empty); a stale claim is reclaimable while a fresh peer claim is
/// skipped.
#[tokio::test]
async fn stuck_provisioning_claim_and_go_and_ttl_takeover() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();
    let older_than = now - hours(1);
    let peer = id(0xBEEF);

    seed_prov(&h, 0xA1, hours(3)).await; // unclaimed
    // Stale claim (11 min old, past the 10-min TTL) ⇒ reclaimable.
    insert_provisioning_tenant(
        &h.provider,
        id(0xA2),
        parent(),
        "stale",
        1,
        now - hours(3),
        Some(peer),
        Some(now - mins(11)),
        None,
    )
    .await
    .expect("seed stale");
    // Fresh peer claim (1 min old) ⇒ skipped.
    insert_provisioning_tenant(
        &h.provider,
        id(0xA3),
        parent(),
        "fresh",
        1,
        now - hours(3),
        Some(peer),
        Some(now - mins(1)),
        None,
    )
    .await
    .expect("seed fresh");

    let first: std::collections::BTreeSet<Uuid> = h
        .repo
        .scan_stuck_provisioning(&allow_all(), now, older_than, 100)
        .await
        .expect("first scan")
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert!(first.contains(&id(0xA1)), "unclaimed row claimed");
    assert!(first.contains(&id(0xA2)), "stale claim taken over");
    assert!(!first.contains(&id(0xA3)), "fresh peer claim skipped");

    // Re-scan: A1/A2 now freshly claimed by this worker ⇒ empty.
    let second = h
        .repo
        .scan_stuck_provisioning(&allow_all(), now, older_than, 100)
        .await
        .expect("second scan");
    assert!(second.is_empty(), "freshly-claimed rows are not re-claimed");
}

/// `limit` caps the claimed batch, oldest-first.
#[tokio::test]
async fn stuck_provisioning_respects_limit() {
    let h = setup_sqlite().await.expect("setup");
    let now = OffsetDateTime::now_utc();
    let older_than = now - hours(1);

    seed_prov(&h, 0xB1, hours(5)).await; // oldest
    seed_prov(&h, 0xB2, hours(4)).await;
    seed_prov(&h, 0xB3, hours(3)).await;

    let rows = h
        .repo
        .scan_stuck_provisioning(&allow_all(), now, older_than, 2)
        .await
        .expect("scan");

    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    assert_eq!(
        ids,
        vec![id(0xB1), id(0xB2)],
        "limit=2 returns the two oldest"
    );
}
