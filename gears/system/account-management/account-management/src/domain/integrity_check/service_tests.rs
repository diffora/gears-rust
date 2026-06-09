//! Unit tests for the periodic hierarchy-integrity loop.
//!
//! All tests use `tokio::test(start_paused = true)` so the loop's
//! `initial_delay` + per-tick `interval` sleeps advance virtual time
//! deterministically, and a counting [`FakeChecker`] captures every
//! invocation without standing up the production
//! [`crate::domain::tenant::service::TenantService`]. Per-tick
//! responses are scripted via a `VecDeque<TickResp>` queue (with `Ok`
//! as the implicit fallback), and an `mpsc::unbounded_channel` lets
//! the driver wait on individual ticks without spinning on the call
//! counter.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::missing_panics_doc,
    reason = "test helpers"
)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::domain::error::DomainError;
use crate::domain::integrity_check::IntegrityRepairConfig;
use crate::domain::tenant::integrity::{
    IntegrityCategory, IntegrityReport, RepairReport, Violation,
};

#[derive(Clone)]
enum TickResp {
    Ok,
    /// Check tick returns success but with one derivable violation
    /// (`MissingClosureSelfRow`) seeded — used to drive the
    /// auto-repair branch of the periodic loop.
    OkWithDerivable,
    InProgress,
    Other,
}

struct FakeChecker {
    check_calls: AtomicU64,
    repair_calls: AtomicU64,
    queue: Mutex<VecDeque<TickResp>>,
    notify: UnboundedSender<()>,
}

impl FakeChecker {
    fn new() -> (Arc<Self>, UnboundedReceiver<()>) {
        let (tx, rx) = unbounded_channel();
        let checker = Arc::new(Self {
            check_calls: AtomicU64::new(0),
            repair_calls: AtomicU64::new(0),
            queue: Mutex::new(VecDeque::new()),
            notify: tx,
        });
        (checker, rx)
    }

    fn calls(&self) -> u64 {
        self.check_calls.load(Ordering::SeqCst)
    }

    fn repair_calls(&self) -> u64 {
        self.repair_calls.load(Ordering::SeqCst)
    }

    fn queue(&self, resp: TickResp) {
        self.queue.lock().push_back(resp);
    }
}

#[async_trait]
impl IntegrityChecker for FakeChecker {
    async fn run_whole_integrity_check(&self) -> Result<IntegrityReport, DomainError> {
        self.check_calls.fetch_add(1, Ordering::SeqCst);
        let resp = self.queue.lock().pop_front().unwrap_or(TickResp::Ok);
        // `try_send` on UnboundedSender — the receiver is always
        // present in practice (the test driver holds it), but we
        // ignore send failure to keep the trait method
        // infallibility-respecting in the rare race where the test
        // ends before notify is observed.
        if self.notify.send(()).is_err() {
            // Test driver dropped the receiver: nothing to do.
        }
        match resp {
            TickResp::Ok => Ok(IntegrityReport {
                violations_by_category: IntegrityCategory::all()
                    .into_iter()
                    .map(|c| (c, Vec::new()))
                    .collect(),
            }),
            TickResp::OkWithDerivable => Ok(IntegrityReport {
                violations_by_category: IntegrityCategory::all()
                    .into_iter()
                    .map(|c| {
                        if c == IntegrityCategory::MissingClosureSelfRow {
                            (
                                c,
                                vec![Violation {
                                    category: c,
                                    tenant_id: None,
                                    details: String::new(),
                                }],
                            )
                        } else {
                            (c, Vec::new())
                        }
                    })
                    .collect(),
            }),
            TickResp::InProgress => Err(DomainError::IntegrityCheckInProgress),
            TickResp::Other => Err(DomainError::internal("test-induced failure")),
        }
    }

    async fn run_whole_integrity_repair(&self) -> Result<RepairReport, DomainError> {
        self.repair_calls.fetch_add(1, Ordering::SeqCst);
        if self.notify.send(()).is_err() {
            // Test driver dropped the receiver: nothing to do.
        }
        Ok(RepairReport::default())
    }
}

fn cfg(interval_secs: u64, initial_delay_secs: u64, jitter: f64) -> IntegrityCheckConfig {
    IntegrityCheckConfig {
        enabled: true,
        interval_secs,
        initial_delay_secs,
        jitter,
        repair: IntegrityRepairConfig::default(),
        ..IntegrityCheckConfig::default()
    }
}

/// Like [`cfg`] but with `repair.enabled = true` AND
/// `repair.auto_after_check = true` so the periodic loop chains a
/// repair tick after every check that observed derivable
/// violations.
fn cfg_auto_repair(interval_secs: u64, initial_delay_secs: u64) -> IntegrityCheckConfig {
    IntegrityCheckConfig {
        enabled: true,
        interval_secs,
        initial_delay_secs,
        jitter: 0.0,
        repair: IntegrityRepairConfig {
            enabled: true,
            auto_after_check: true,
        },
        ..IntegrityCheckConfig::default()
    }
}

/// Spawn the loop and return its handle plus the cancel token.
fn spawn_loop(
    checker: Arc<dyn IntegrityChecker>,
    cfg: IntegrityCheckConfig,
) -> (tokio::task::JoinHandle<()>, CancellationToken) {
    let cancel = CancellationToken::new();
    let cancel_for_task = cancel.clone();
    let handle = tokio::spawn(async move {
        run_integrity_check_loop(checker, cfg, cancel_for_task).await;
    });
    (handle, cancel)
}

#[tokio::test(start_paused = true)]
async fn disabled_config_does_not_run_check() {
    let (checker, mut rx) = FakeChecker::new();
    let cfg = IntegrityCheckConfig {
        enabled: false,
        interval_secs: 60,
        initial_delay_secs: 0,
        jitter: 0.0,
        repair: IntegrityRepairConfig::default(),
        ..IntegrityCheckConfig::default()
    };
    let (handle, cancel) = spawn_loop(Arc::clone(&checker) as Arc<dyn IntegrityChecker>, cfg);

    // Advance well past any reasonable interval — the loop must not
    // call the checker even once.
    tokio::time::sleep(Duration::from_hours(3)).await;

    assert_eq!(checker.calls(), 0, "disabled job must not call checker");
    assert!(
        rx.try_recv().is_err(),
        "no tick notifications must arrive while disabled"
    );

    cancel.cancel();
    handle.await.expect("disabled task joins cleanly on cancel");
}

#[tokio::test(start_paused = true)]
async fn runs_first_check_after_initial_delay() {
    let (checker, mut rx) = FakeChecker::new();
    let start = Instant::now();
    let (handle, cancel) = spawn_loop(
        Arc::clone(&checker) as Arc<dyn IntegrityChecker>,
        cfg(3600, 60, 0.0),
    );

    rx.recv().await.expect("first tick fires");

    let elapsed = Instant::now() - start;
    assert!(
        elapsed >= Duration::from_mins(1),
        "first tick must wait initial_delay; got {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(61),
        "first tick must fire promptly after initial_delay; got {elapsed:?}"
    );
    assert_eq!(checker.calls(), 1);

    cancel.cancel();
    handle.await.expect("task joins cleanly");
}

#[tokio::test(start_paused = true)]
async fn subsequent_runs_respect_interval() {
    let (checker, mut rx) = FakeChecker::new();
    let (handle, cancel) = spawn_loop(
        Arc::clone(&checker) as Arc<dyn IntegrityChecker>,
        // initial_delay = 0 so the first tick is immediate; jitter = 0
        // so the second tick lands at exactly `interval`.
        cfg(120, 0, 0.0),
    );

    rx.recv().await.expect("first tick");
    let after_first = Instant::now();

    rx.recv().await.expect("second tick");
    let gap = Instant::now() - after_first;
    assert!(
        gap >= Duration::from_mins(2),
        "second tick must follow at >= interval; got {gap:?}"
    );
    assert!(
        gap < Duration::from_secs(121),
        "second tick must follow at exactly interval (jitter=0); got {gap:?}"
    );
    assert_eq!(checker.calls(), 2);

    cancel.cancel();
    handle.await.expect("task joins cleanly");
}

#[tokio::test(start_paused = true)]
async fn applies_jitter_within_bounds() {
    let (checker, mut rx) = FakeChecker::new();
    let interval_secs = 600u64;
    let jitter = 0.1f64;
    let (handle, cancel) = spawn_loop(
        Arc::clone(&checker) as Arc<dyn IntegrityChecker>,
        cfg(interval_secs, 0, jitter),
    );

    rx.recv().await.expect("first tick");
    let mut last = Instant::now();
    let mut diffs: Vec<Duration> = Vec::with_capacity(30);
    for _ in 0..30 {
        rx.recv().await.expect("subsequent tick");
        let now = Instant::now();
        diffs.push(now - last);
        last = now;
    }

    let interval = Duration::from_secs(interval_secs);
    let lo = interval.mul_f64(1.0 - jitter);
    let hi = interval.mul_f64(1.0 + jitter);
    let mut distinct: std::collections::HashSet<u128> = std::collections::HashSet::new();
    for d in &diffs {
        assert!(*d >= lo, "jitter must not undershoot bound: {d:?} < {lo:?}");
        assert!(*d <= hi, "jitter must not overshoot bound: {d:?} > {hi:?}");
        distinct.insert(d.as_nanos());
    }
    assert!(
        distinct.len() > 1,
        "jitter must produce variation across ticks; saw {distinct:?}"
    );

    cancel.cancel();
    handle.await.expect("task joins cleanly");
}

#[tokio::test(start_paused = true)]
async fn treats_429_as_skip_and_continues_loop() {
    let (checker, mut rx) = FakeChecker::new();
    checker.queue(TickResp::InProgress);
    let (handle, cancel) = spawn_loop(
        Arc::clone(&checker) as Arc<dyn IntegrityChecker>,
        cfg(60, 0, 0.0),
    );

    rx.recv().await.expect("first tick fires (and gets 429)");
    assert_eq!(checker.calls(), 1);
    rx.recv().await.expect("loop continues after 429");
    assert_eq!(checker.calls(), 2);

    cancel.cancel();
    handle.await.expect("task joins cleanly");
}

#[tokio::test(start_paused = true)]
async fn treats_other_errors_as_skip_and_continues_loop() {
    let (checker, mut rx) = FakeChecker::new();
    checker.queue(TickResp::Other);
    let (handle, cancel) = spawn_loop(
        Arc::clone(&checker) as Arc<dyn IntegrityChecker>,
        cfg(60, 0, 0.0),
    );

    rx.recv()
        .await
        .expect("first tick fires (and gets Internal)");
    assert_eq!(checker.calls(), 1);
    rx.recv().await.expect("loop continues after non-429 error");
    assert_eq!(checker.calls(), 2);

    cancel.cancel();
    handle.await.expect("task joins cleanly");
}

#[tokio::test(start_paused = true)]
async fn shutdown_during_initial_delay_breaks_loop_promptly() {
    let (checker, _rx) = FakeChecker::new();
    let (handle, cancel) = spawn_loop(
        Arc::clone(&checker) as Arc<dyn IntegrityChecker>,
        // 1h initial_delay — way longer than any test would wait —
        // verifies that shutdown does not block on the warmup sleep.
        cfg(3600, 3600, 0.0),
    );

    // Yield once so the spawned task definitely reaches its first
    // `select!` (entering the initial-delay sleep), then cancel.
    tokio::task::yield_now().await;
    cancel.cancel();

    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("shutdown unblocks the warmup sleep")
        .expect("task joins cleanly");

    assert_eq!(
        checker.calls(),
        0,
        "no tick must fire when shutdown precedes the first interval"
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_during_post_tick_sleep_breaks_loop_promptly() {
    let (checker, mut rx) = FakeChecker::new();
    let (handle, cancel) = spawn_loop(
        Arc::clone(&checker) as Arc<dyn IntegrityChecker>,
        // initial_delay = 0 so the first tick fires immediately,
        // then the loop sleeps `interval` waiting for the next tick.
        cfg(3600, 0, 0.0),
    );

    rx.recv().await.expect("first tick fires");
    let calls_before_cancel = checker.calls();

    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("shutdown unblocks the post-tick sleep")
        .expect("task joins cleanly");

    assert_eq!(
        checker.calls(),
        calls_before_cancel,
        "no further tick must fire after cancel"
    );
}

// ---------------------------------------------------------------------
// Auto-repair (`repair.enabled = true` + `repair.auto_after_check = true`)
// ---------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn auto_after_check_triggers_repair_on_derivable_violations() {
    let (checker, mut rx) = FakeChecker::new();
    // First check tick returns a report carrying a
    // MissingClosureSelfRow violation — the auto-repair branch must
    // chain a repair tick before the next check.
    checker.queue(TickResp::OkWithDerivable);
    let (handle, cancel) = spawn_loop(
        Arc::clone(&checker) as Arc<dyn IntegrityChecker>,
        cfg_auto_repair(60, 0),
    );

    rx.recv().await.expect("check tick fires");
    rx.recv()
        .await
        .expect("repair tick chains after derivable check");

    assert_eq!(checker.calls(), 1, "exactly one check before repair");
    assert_eq!(checker.repair_calls(), 1, "exactly one repair triggered");

    cancel.cancel();
    handle.await.expect("task joins cleanly");
}

#[tokio::test(start_paused = true)]
async fn auto_after_check_skipped_on_clean_check() {
    let (checker, mut rx) = FakeChecker::new();
    // Default TickResp::Ok = clean snapshot; auto-repair must NOT
    // fire because there's nothing to repair.
    let (handle, cancel) = spawn_loop(
        Arc::clone(&checker) as Arc<dyn IntegrityChecker>,
        cfg_auto_repair(60, 0),
    );

    rx.recv().await.expect("first check tick fires");
    // Wait for the next check tick to fire so we know the loop
    // didn't pause to call repair between.
    rx.recv().await.expect("second check tick fires");

    assert_eq!(
        checker.repair_calls(),
        0,
        "clean check must not trigger repair"
    );
    assert!(checker.calls() >= 2);

    cancel.cancel();
    handle.await.expect("task joins cleanly");
}

#[tokio::test(start_paused = true)]
async fn auto_after_check_disabled_skips_repair_even_on_derivable() {
    let (checker, mut rx) = FakeChecker::new();
    // Repair master switch off → auto_after_check is inert.
    checker.queue(TickResp::OkWithDerivable);
    let cfg = IntegrityCheckConfig {
        enabled: true,
        interval_secs: 60,
        initial_delay_secs: 0,
        jitter: 0.0,
        repair: IntegrityRepairConfig {
            enabled: false,
            auto_after_check: false,
        },
        ..IntegrityCheckConfig::default()
    };
    let (handle, cancel) = spawn_loop(Arc::clone(&checker) as Arc<dyn IntegrityChecker>, cfg);

    rx.recv().await.expect("first check tick fires");
    rx.recv().await.expect("second check tick fires");

    assert_eq!(
        checker.repair_calls(),
        0,
        "repair must not fire when master switch is off"
    );

    cancel.cancel();
    handle.await.expect("task joins cleanly");
}
