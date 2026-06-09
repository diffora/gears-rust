//! Periodic hierarchy-integrity check loop.
//!
//! The loop is driven by [`run_integrity_check_loop`], invoked from
//! [`crate::gear::AccountManagementGear::serve`] alongside the
//! retention + reaper interval loops. The dispatched work is hidden
//! behind the [`IntegrityChecker`] trait — production wires
//! [`crate::domain::integrity_check::coordinator::IntegrityCoordinator`],
//! tests inject a counting fake without standing up the DB layer.
//!
//! Per-tick outcome policy (driven by
//! [`crate::domain::integrity_check::coordinator::IntegrityCheckOutcome`]):
//!
//! * `CompletedClean` / `CompletedRepaired` → emit
//!   `RUNS{outcome=completed}` + `DURATION{phase=check}` +
//!   `LAST_SUCCESS`; the repaired path adds `REPAIR_RUNS{outcome=completed}`
//!   + `DURATION{phase=repair}`.
//! * `SkippedInProgress` → emit `RUNS{outcome=skipped_in_progress}`
//!   and a warn log. The loop intentionally does NOT retry inside the
//!   tick because a peer is producing fresh telemetry.
//! * `AbortedLeaseLost` → emit `RUNS{outcome=completed}` (check
//!   did run) and `REPAIR_RUNS{outcome=aborted_lease_lost}` so
//!   dashboards split mid-flight contention from "peer already
//!   running".
//! * Any error → emit `RUNS{outcome=failed}` + warn log; the loop
//!   continues so a transient DB blip does not silently disable
//!   the periodic audit.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use time::OffsetDateTime;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use toolkit_macros::domain_model;
use tracing::warn;

use crate::domain::error::DomainError;
use crate::domain::integrity_check::config::IntegrityCheckConfig;
use crate::domain::metrics::{
    AM_HIERARCHY_INTEGRITY_DURATION, AM_HIERARCHY_INTEGRITY_LAST_FAILURE,
    AM_HIERARCHY_INTEGRITY_LAST_SUCCESS, AM_HIERARCHY_INTEGRITY_REPAIR_RUNS,
    AM_HIERARCHY_INTEGRITY_RUNS, MetricKind, emit_gauge_value, emit_histogram_value, emit_metric,
};
use crate::domain::tenant::integrity::{IntegrityReport, RepairReport};

/// Outcome of one tick driven by the [`IntegrityChecker::run_tick`]
/// port. Each variant maps cleanly to a `RUNS{outcome=…}` /
/// `REPAIR_RUNS{outcome=…}` metric label emitted by [`run_one_tick`].
/// Per-phase [`Duration`]s are measured inside the production
/// coordinator so the loop driver does not re-time the phases.
#[domain_model]
#[derive(Debug, Clone)]
pub enum IntegrityCheckOutcome {
    /// Check completed; no derivable violations — no repair phase ran.
    CompletedClean {
        check_report: IntegrityReport,
        check_duration: Duration,
    },
    /// Check + repair both completed under one lease.
    CompletedRepaired {
        check_report: IntegrityReport,
        check_duration: Duration,
        repair_report: RepairReport,
        repair_duration: Duration,
    },
    /// Lease was held by a peer — this tick was a no-op.
    SkippedInProgress,
    /// Lease was acquired but lost mid-repair (peer took over). The
    /// fenced commit was rolled back; TTL handles release on the
    /// original holder's side; the new holder will produce fresh
    /// telemetry on its own tick. The check report **is** retained
    /// because the read phase still completed under the lease.
    AbortedLeaseLost {
        check_report: IntegrityReport,
        check_duration: Duration,
    },
}

/// Loop-driver abstraction over one hierarchy-integrity tick.
///
/// Production wires
/// [`crate::domain::integrity_check::coordinator::IntegrityCoordinator`];
/// tests inject a counting fake (`FakeChecker` in `service_tests.rs`).
///
/// Three methods live on the trait:
///
/// * [`Self::run_tick`] is the loop driver's entry point — it
///   returns an [`IntegrityCheckOutcome`] that the driver fans out
///   into metrics.
/// * [`Self::run_whole_integrity_check`] and
///   [`Self::run_whole_integrity_repair`] are the per-phase
///   accessors test fakes use to count check / repair calls
///   separately. The default [`Self::run_tick`] composes them, so
///   a fake that implements only these two automatically gets a
///   working `run_tick`.
#[async_trait]
pub trait IntegrityChecker: Send + Sync {
    /// Execute one whole-tree integrity check tick and return the
    /// report. Errors propagate verbatim so the loop can classify
    /// gate-conflict vs. transient failure.
    async fn run_whole_integrity_check(&self) -> Result<IntegrityReport, DomainError>;

    /// Execute one whole-tree repair tick and return the per-category
    /// repair report. Invoked by the loop driver only when
    /// [`IntegrityRepairConfig::auto_after_check`] is `true` AND the
    /// preceding check tick observed at least one derivable
    /// violation.
    async fn run_whole_integrity_repair(&self) -> Result<RepairReport, DomainError>;

    /// Composite tick — runs the check phase, and conditionally
    /// chains the repair phase when `auto_repair` is `true` and the
    /// check observed at least one derivable violation. Returns an
    /// [`IntegrityCheckOutcome`] for the loop driver to fan out into
    /// per-outcome metric labels.
    ///
    /// The default implementation calls
    /// [`Self::run_whole_integrity_check`] and (when warranted)
    /// [`Self::run_whole_integrity_repair`] sequentially. The
    /// production [`IntegrityCoordinator`] overrides this with a
    /// single-lease implementation so both phases share one lease
    /// instead of acquiring independently.
    async fn run_tick(&self, auto_repair: bool) -> Result<IntegrityCheckOutcome, DomainError> {
        let check_started = Instant::now();
        let check_report = match self.run_whole_integrity_check().await {
            Ok(r) => r,
            Err(DomainError::IntegrityCheckInProgress) => {
                return Ok(IntegrityCheckOutcome::SkippedInProgress);
            }
            Err(e) => return Err(e),
        };
        let check_duration = check_started.elapsed();
        if !(auto_repair && check_report.has_derivable_violations()) {
            return Ok(IntegrityCheckOutcome::CompletedClean {
                check_report,
                check_duration,
            });
        }
        let repair_started = Instant::now();
        match self.run_whole_integrity_repair().await {
            Ok(repair_report) => {
                let repair_duration = repair_started.elapsed();
                Ok(IntegrityCheckOutcome::CompletedRepaired {
                    check_report,
                    check_duration,
                    repair_report,
                    repair_duration,
                })
            }
            Err(DomainError::IntegrityCheckInProgress) => {
                // A peer took the lease between the two phases; the
                // check report is still valid, the repair was
                // preempted. `AbortedLeaseLost` reflects the partial
                // completion so the driver emits the right metric
                // label.
                Ok(IntegrityCheckOutcome::AbortedLeaseLost {
                    check_report,
                    check_duration,
                })
            }
            Err(e) => Err(e),
        }
    }
}

/// Lifecycle entry point invoked from [`crate::gear::AccountManagementGear::serve`].
///
/// Returns when `cancel` fires. When `cfg.enabled == false`, the loop
/// is not entered at all — the function still awaits cancellation so
/// the calling `select!` arm in `serve` keeps a uniform shape across
/// enabled/disabled configurations and never observes a prematurely
/// completed `JoinHandle`.
pub async fn run_integrity_check_loop(
    checker: Arc<dyn IntegrityChecker>,
    cfg: IntegrityCheckConfig,
    cancel: CancellationToken,
) {
    if !cfg.enabled {
        cancel.cancelled().await;
        return;
    }

    // Initial delay — cancellable so a fast shutdown after start does
    // not block on the configured warmup sleep.
    tokio::select! {
        biased;
        () = cancel.cancelled() => return,
        () = tokio::time::sleep(cfg.initial_delay()) => {}
    }

    let mut jitter_rng = JitterRng::seeded_from_clock();
    let auto_repair = cfg.repair.enabled && cfg.repair.auto_after_check;

    loop {
        run_one_tick(checker.as_ref(), auto_repair).await;

        let next_sleep = jittered_interval(cfg.interval(), cfg.jitter, &mut jitter_rng);
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            () = tokio::time::sleep(next_sleep) => {}
        }
    }
}

#[allow(
    clippy::cognitive_complexity,
    reason = "flat match over the IntegrityCheckOutcome variants — each arm is a single \
              metric-emit; splitting would scatter the outcome→telemetry mapping"
)]
async fn run_one_tick(checker: &dyn IntegrityChecker, auto_repair: bool) {
    match checker.run_tick(auto_repair).await {
        Ok(IntegrityCheckOutcome::CompletedClean { check_duration, .. }) => {
            emit_check_completed(check_duration);
        }
        Ok(IntegrityCheckOutcome::CompletedRepaired {
            check_duration,
            repair_report,
            repair_duration,
            ..
        }) => {
            emit_check_completed(check_duration);
            emit_repair_completed(repair_duration, &repair_report);
        }
        Ok(IntegrityCheckOutcome::SkippedInProgress) => {
            warn!(
                target: "am.integrity",
                "integrity check tick skipped: another worker holds the lease"
            );
            emit_metric(
                AM_HIERARCHY_INTEGRITY_RUNS,
                MetricKind::Counter,
                &[("outcome", "skipped_in_progress")],
            );
            emit_last_failure_gauge("skipped_in_progress");
        }
        Ok(IntegrityCheckOutcome::AbortedLeaseLost { check_duration, .. }) => {
            // The check phase did complete; per-category bucketing
            // happens inside the coordinator, so the driver only
            // emits the tick-level outcome. Surfacing lease-loss as
            // a distinct repair outcome lets dashboards split
            // mid-flight contention from the steady-state "peer
            // held the gate" case.
            emit_check_completed(check_duration);
            warn!(
                target: "am.integrity",
                "integrity repair aborted: lease lost to a peer mid-flight"
            );
            emit_metric(
                AM_HIERARCHY_INTEGRITY_REPAIR_RUNS,
                MetricKind::Counter,
                &[("outcome", "aborted_lease_lost")],
            );
            emit_last_failure_gauge("aborted_lease_lost");
        }
        Err(err) => {
            warn!(
                target: "am.integrity",
                error = %err,
                "integrity check tick failed"
            );
            emit_metric(
                AM_HIERARCHY_INTEGRITY_RUNS,
                MetricKind::Counter,
                &[("outcome", "failed")],
            );
            emit_last_failure_gauge("failed");
        }
    }
}

/// Emit the per-tick check-phase metrics: `RUNS{outcome=completed}`,
/// `DURATION{phase=check}`, and the `LAST_SUCCESS` gauge. The
/// `check_duration` comes from the coordinator, which measures
/// just the check phase (excluding any chained repair).
fn emit_check_completed(check_duration: Duration) {
    emit_metric(
        AM_HIERARCHY_INTEGRITY_RUNS,
        MetricKind::Counter,
        &[("outcome", "completed")],
    );
    #[allow(
        clippy::cast_precision_loss,
        reason = "millisecond duration <= a few minutes fits f64 mantissa exactly"
    )]
    let elapsed_ms = check_duration.as_millis() as f64;
    emit_histogram_value(
        AM_HIERARCHY_INTEGRITY_DURATION,
        elapsed_ms,
        &[("phase", "check")],
    );
    emit_gauge_value(
        AM_HIERARCHY_INTEGRITY_LAST_SUCCESS,
        OffsetDateTime::now_utc().unix_timestamp(),
        &[],
    );
}

/// Emit the repair-phase metrics on the chained auto-repair path:
/// `REPAIR_RUNS{outcome=completed}` + `DURATION{phase=repair}` +
/// an info log carrying per-category aggregates the unit dashboards
/// snapshot. The `repair_duration` from the coordinator measures
/// just the fenced repair-tx work.
fn emit_repair_completed(
    repair_duration: Duration,
    report: &crate::domain::tenant::integrity::RepairReport,
) {
    emit_metric(
        AM_HIERARCHY_INTEGRITY_REPAIR_RUNS,
        MetricKind::Counter,
        &[("outcome", "completed")],
    );
    #[allow(
        clippy::cast_precision_loss,
        reason = "millisecond duration <= a few minutes fits f64 mantissa exactly"
    )]
    let elapsed_ms = repair_duration.as_millis() as f64;
    emit_histogram_value(
        AM_HIERARCHY_INTEGRITY_DURATION,
        elapsed_ms,
        &[("phase", "repair")],
    );
    tracing::info!(
        target: "am.integrity",
        repaired_total = report.total_repaired(),
        deferred_total = report.total_deferred(),
        "integrity repair tick completed (auto_after_check)"
    );
}

/// Emit `AM_HIERARCHY_INTEGRITY_LAST_FAILURE` with the wall-clock
/// timestamp of this failed (or skipped) tick. The `outcome` label
/// matches the outcome label used on `AM_HIERARCHY_INTEGRITY_RUNS`
/// so dashboards can correlate the gauge sample with the run-counter
/// increment that produced it. Companion to the success-side gauge
/// emitted on `Ok(_)` ticks.
fn emit_last_failure_gauge(outcome: &'static str) {
    emit_gauge_value(
        AM_HIERARCHY_INTEGRITY_LAST_FAILURE,
        OffsetDateTime::now_utc().unix_timestamp(),
        &[("outcome", outcome)],
    );
}

fn jittered_interval(interval: Duration, jitter: f64, rng: &mut JitterRng) -> Duration {
    if jitter <= 0.0 {
        return interval;
    }
    // `next_neg_one_to_one` returns a value in [-1.0, 1.0); multiplying
    // by `jitter` (already validated to ∈ [0.0, 0.5]) keeps the offset
    // in [-jitter, +jitter), and `(1.0 + offset).max(0.0)` defends
    // against a future jitter bound > 1.0 producing a negative factor.
    let offset = rng.next_neg_one_to_one() * jitter;
    let factor = (1.0 + offset).max(0.0);
    Duration::from_secs_f64(interval.as_secs_f64() * factor)
}

/// Tiny self-contained PRNG for jitter. Uses splitmix64 stepping
/// (single multiply + xorshift mix per call) which is more than
/// sufficient for spread-the-load purposes; pulling in `rand` for one
/// use site would add a transitive dependency to the AM crate without
/// any cryptographic requirement to justify it.
#[domain_model]
struct JitterRng {
    state: u64,
}

impl JitterRng {
    fn seeded_from_clock() -> Self {
        // System time, not `tokio::time::Instant`, so each replica
        // seeds independently regardless of paused-test virtual
        // clocks. Wall-clock nanos are unique enough to spread two
        // replicas starting in the same second; the state OR'd with 1
        // forbids the all-zero degenerate case the splitmix step
        // tolerates.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0xDEAD_BEEF_CAFE_F00D, |d| {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "wrapping nanos into u64 is the seeding intent"
                )]
                let lo = d.as_nanos() as u64;
                lo
            });
        Self { state: seed | 1 }
    }

    fn next_u64(&mut self) -> u64 {
        // splitmix64 — stateless mixer, advances by a known constant.
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Returns a value uniformly distributed in `[-1.0, 1.0)`.
    fn next_neg_one_to_one(&mut self) -> f64 {
        // Take the top 53 bits → exact f64 mantissa precision.
        #[allow(
            clippy::cast_precision_loss,
            reason = "53-bit value casts exactly to f64"
        )]
        let unit = (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64);
        unit.mul_add(2.0, -1.0)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "service_tests.rs"]
mod service_tests;
