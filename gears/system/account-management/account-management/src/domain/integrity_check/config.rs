//! Configuration for the periodic hierarchy-integrity check job.
//!
//! Operator-facing knobs consumed by
//! [`crate::domain::integrity_check::service::run_integrity_check_loop`].
//! Defaults match the FEATURE doc handoff: hourly cadence, 5-minute
//! initial delay (so the bootstrap saga in
//! [`crate::gear::AccountManagementGear::init`] can finish before
//! the first tick fires), 10% multiplicative jitter to spread load
//! across replicas without leader-election.
//!
//! `enabled = false` is a clean opt-out — the job is not spawned at all
//! and the on-demand SDK method
//! ([`crate::domain::tenant::service::TenantService::check_hierarchy_integrity`])
//! continues to work for admin-tool driven runs.

use std::time::Duration;

use serde::Deserialize;
use toolkit_macros::domain_model;

/// Periodic-job configuration.
///
/// The single-flight gate
/// ([`crate::domain::error::DomainError::IntegrityCheckInProgress`])
/// already coordinates concurrent runs at the DB level, so the job
/// does not implement leader-election; jitter is the only
/// multi-replica spreading mechanism.
#[domain_model]
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IntegrityCheckConfig {
    /// When `true` (default), [`crate::gear::AccountManagementGear::serve`]
    /// spawns the periodic job. When `false`, the loop is not entered
    /// and only the on-demand SDK method runs the check.
    pub enabled: bool,

    /// Tick interval in seconds. Bounded by
    /// [`Self::MIN_INTERVAL_SECS`] / [`Self::MAX_INTERVAL_SECS`] in
    /// [`Self::validate`] — anything tighter than 60s is pointless
    /// (the per-scope gate would serialize ticks anyway), anything
    /// looser than 24h belongs in an external cron rather than the
    /// in-process loop.
    pub interval_secs: u64,

    /// Delay between [`crate::gear::AccountManagementGear::serve`]
    /// reaching this loop and the first tick firing. Designed so the
    /// retention + reaper loops have a chance to settle and
    /// [`crate::gear::AccountManagementGear::init`]'s bootstrap
    /// saga has long finished before the first whole-tree snapshot is
    /// taken. Must be `<= interval_secs` so the first tick never lags
    /// more than one interval.
    pub initial_delay_secs: u64,

    /// Multiplicative jitter applied to every sleep. Each sleep is
    /// drawn from `interval * (1 + r)` where `r ∈ [-jitter, +jitter]`.
    /// Bounded by [`Self::MAX_JITTER`] in [`Self::validate`] —
    /// `jitter > 0.5` is indistinguishable from reducing the interval
    /// in the lower half of the range. `jitter = 0.0` produces a
    /// strictly periodic loop (used by deterministic tests).
    pub jitter: f64,

    /// Periodic repair sub-section. Default-off; see
    /// [`IntegrityRepairConfig`].
    pub repair: IntegrityRepairConfig,

    /// AM-local lease TTL (seconds) used by the
    /// [`crate::domain::integrity_check::coordinator::IntegrityCoordinator`].
    /// Sized as the worst-case single-tick check+repair budget;
    /// when a holder crashes mid-flight, this is the upper bound on
    /// how long a peer waits before the slot can be stolen.
    ///
    /// Bounded by [`Self::MIN_LEASE_TTL_SECS`] /
    /// [`Self::MAX_LEASE_TTL_SECS`] in [`Self::validate`]. Anything
    /// below 60s is risky (heartbeat misses become structurally
    /// likely); anything above the loop's `interval_secs` is
    /// pointless (a holder that survives one tick will always
    /// renew before the next acquire).
    pub lease_ttl_secs: u64,

    /// Renewal heartbeat period (seconds) for
    /// [`crate::infra::lease::LeaseGuard::spawn_renewal`].
    ///
    /// Convention is `lease_ttl_secs / 3` so the lease survives one
    /// missed tick (transient DB blip). [`Self::validate`] enforces
    /// `renew_period_secs * 2 <= lease_ttl_secs` — bare minimum so
    /// a single missed heartbeat does not lose the lease.
    pub lease_renew_period_secs: u64,
}

/// Periodic hierarchy-integrity repair configuration.
///
/// Repair is gated by a master switch (`enabled`) plus an
/// `auto_after_check` flag so operators can stage rollout in two
/// steps:
///
/// 1. Set `enabled = true` (without `auto_after_check`) — the repair
///    SDK method is wired and reachable via admin tools, but the
///    periodic loop never invokes it; operators run repair manually
///    against staging clusters and watch
///    [`crate::domain::metrics::AM_HIERARCHY_INTEGRITY_REPAIRED`].
/// 2. Flip `auto_after_check = true` once manual runs look clean —
///    the periodic loop now chains a repair tick after each check
///    that observed derivable violations.
///
/// Default is `enabled = false` so unmodified deployments retain
/// today's read-only diagnostic surface.
#[domain_model]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IntegrityRepairConfig {
    /// Master switch for the repair API surface. Default `false` so
    /// unmodified deployments retain today's read-only diagnostic
    /// surface. When `false`, both the on-demand SDK method
    /// (`TenantService::repair_hierarchy_integrity`) and the
    /// auto-trigger from the periodic loop are inert — on-demand
    /// calls are rejected with
    /// [`crate::domain::error::DomainError::UnsupportedOperation`]
    /// and the periodic auto-trigger is skipped entirely.
    pub enabled: bool,

    /// When `true` AND [`Self::enabled`] is also `true`, the periodic
    /// integrity-check loop chains a repair tick after each check
    /// that observed at least one derivable violation. Skipped when
    /// the check tick failed or returned only deferred
    /// (operator-triage) violations. Default `false`.
    pub auto_after_check: bool,
}

impl IntegrityRepairConfig {
    /// Validate the repair sub-section. Currently rejects only one
    /// invariant: `auto_after_check = true` without `enabled = true`
    /// is a misconfiguration that would silently pass — the
    /// auto-trigger is no-op when the master switch is off, but the
    /// operator likely meant both flags to flip together. Surfaces
    /// the slip as an `init` failure so deployment configs cannot
    /// drift into a half-on shape.
    ///
    /// # Errors
    ///
    /// Returns a human-readable string when validation fails.
    pub fn validate(&self) -> Result<(), String> {
        if self.auto_after_check && !self.enabled {
            return Err(
                "integrity_check.repair.auto_after_check (must not be true unless enabled = true)"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

impl Default for IntegrityCheckConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            // 1h — sweet spot between FEATURE-§5 NFR ("operator-visible
            // telemetry within 15 minutes of detection" — that is
            // delivery latency, not run cadence) and DB load on large
            // hierarchies (a whole-tree run loads the full closure
            // table; doing that every 15 min on a 100k-tenant tree is
            // wasteful, doing it every 24h leaves day-long MTTD).
            interval_secs: 3600,
            // 5m — long enough that the bootstrap saga's IdP-wait
            // backoff envelope (default 5m timeout) cannot collide
            // with the first tick under any realistic deployment.
            initial_delay_secs: 300,
            // 10% — empirically sufficient to spread load across a
            // small replica set; large enough to be observable, small
            // enough that the user-visible cadence is still "about
            // every hour".
            jitter: 0.1,
            repair: IntegrityRepairConfig::default(),
            // 15 min — sized well above any realistic single-tick
            // check + repair runtime on a multi-million-row tree
            // (typical repair is seconds-to-minutes; 15 min is a
            // ~30× headroom). Operators can shorten this if they
            // have tighter SLOs on multi-replica crash recovery.
            lease_ttl_secs: 15 * 60,
            // 5 min — `lease_ttl_secs / 3` per the convention.
            lease_renew_period_secs: 5 * 60,
        }
    }
}

/// Default lease TTL used by the trait-level
/// [`crate::domain::tenant::TenantRepo::run_integrity_check`] /
/// [`crate::domain::tenant::TenantRepo::repair_derivable_closure_violations`].
/// These run outside the coordinator (mostly from integration
/// tests) and have no [`IntegrityCheckConfig`] in scope. Production
/// callers go through the coordinator, which reads
/// [`IntegrityCheckConfig::lease_ttl`] instead.
pub const LEASE_TTL: std::time::Duration = std::time::Duration::from_mins(15);

impl IntegrityCheckConfig {
    /// Lower bound on `interval_secs`. Anything tighter is pointless
    /// because the AM-local lease serialises concurrent ticks across
    /// replicas and inside one replica the snapshot load alone takes
    /// seconds on non-trivial trees.
    pub const MIN_INTERVAL_SECS: u64 = 60;

    /// Upper bound on `interval_secs`. Beyond a day the cadence is
    /// indistinguishable from an external cron and shouldn't pay the
    /// cost of an in-process loop holding state.
    pub const MAX_INTERVAL_SECS: u64 = 86_400;

    /// Upper bound on `jitter`. A value of `0.5` already maps each
    /// nominal interval to `[0.5×, 1.5×]`; allowing `> 0.5` lets the
    /// real cadence drift below the `MIN_INTERVAL_SECS` floor and is
    /// indistinguishable from misconfiguring the interval itself.
    pub const MAX_JITTER: f64 = 0.5;

    /// Lower bound on `lease_ttl_secs`. Below 60s the renewal
    /// heartbeat (which must be `<= ttl/2`) collapses into the
    /// sub-30s range where transient DB blips structurally exceed
    /// the renew budget; operators wanting tighter coordination
    /// should use leader election, not a shorter lease.
    pub const MIN_LEASE_TTL_SECS: u64 = 60;

    /// Upper bound on `lease_ttl_secs`. Beyond 24h the lease
    /// degenerates into a persistent gate that a crash leaves
    /// holding for a day; the periodic loop's `MAX_INTERVAL_SECS`
    /// is also 24h, so anything longer would have the next
    /// scheduled tick contend with the previous one's stale row.
    pub const MAX_LEASE_TTL_SECS: u64 = 86_400;

    /// Lower bound on `lease_renew_period_secs`. Sub-second
    /// heartbeats produce sustained DB load that dwarfs the cost
    /// of the protected work; 5s is a sensible floor.
    pub const MIN_LEASE_RENEW_PERIOD_SECS: u64 = 5;

    /// `interval_secs` lifted to a [`Duration`].
    #[must_use]
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }

    /// `initial_delay_secs` lifted to a [`Duration`].
    #[must_use]
    pub fn initial_delay(&self) -> Duration {
        Duration::from_secs(self.initial_delay_secs)
    }

    /// `lease_ttl_secs` lifted to a [`Duration`].
    #[must_use]
    pub fn lease_ttl(&self) -> Duration {
        Duration::from_secs(self.lease_ttl_secs)
    }

    /// `lease_renew_period_secs` lifted to a [`Duration`].
    #[must_use]
    pub fn lease_renew_period(&self) -> Duration {
        Duration::from_secs(self.lease_renew_period_secs)
    }

    /// Reject configurations whose values would either make the loop
    /// pointless (interval below the gate's serialisation floor),
    /// silently re-route work to a different mechanism (interval
    /// beyond a day → use cron), or cause the first tick to lag more
    /// than one interval (`initial_delay > interval`).
    ///
    /// # Errors
    ///
    /// Returns a human-readable string naming each invalid field.
    /// Callers map this into
    /// [`crate::domain::error::DomainError::Internal`] (a fatal `init`
    /// failure).
    pub fn validate(&self) -> Result<(), String> {
        // `enabled = false` is documented as a clean opt-out — the
        // periodic loop is never spawned, so the scheduler-side
        // fields (`interval_secs`, `initial_delay_secs`, `jitter`)
        // are unused. Skipping their bounds when disabled lets a
        // deployment turn the loop off without re-tuning fields it
        // does not exercise. The repair sub-section still validates
        // because the on-demand SDK method continues to honour it.
        if !self.enabled {
            return self
                .repair
                .validate()
                .map_err(|err| format!("integrity-check configuration is invalid: {err}"));
        }

        let mut bad: Vec<&'static str> = Vec::new();
        if self.interval_secs < Self::MIN_INTERVAL_SECS {
            bad.push(
                "integrity_check.interval_secs (must be >= 60; the single-flight gate serialises ticks at coarser granularity)",
            );
        }
        if self.interval_secs > Self::MAX_INTERVAL_SECS {
            bad.push(
                "integrity_check.interval_secs (must be <= 86400; longer cadence belongs in an external cron, not the in-process loop)",
            );
        }
        // NaN / infinity bypass `< 0.0` and `> MAX_JITTER` because
        // float comparisons against NaN return false. Without an
        // explicit `is_finite()` guard, a NaN jitter passes
        // validation and lands in `jittered_interval`, where it
        // multiplies through to a zero-duration sleep and turns the
        // loop into a tight spin.
        if !self.jitter.is_finite() {
            bad.push("integrity_check.jitter (must be finite)");
        } else if self.jitter < 0.0 {
            bad.push("integrity_check.jitter (must be >= 0.0)");
        } else if self.jitter > Self::MAX_JITTER {
            bad.push(
                "integrity_check.jitter (must be <= 0.5; larger spread overlaps with reducing interval_secs)",
            );
        }
        if self.initial_delay_secs > self.interval_secs {
            bad.push(
                "integrity_check.initial_delay_secs (must be <= interval_secs; otherwise the first tick lags more than one interval)",
            );
        }
        if self.lease_ttl_secs < Self::MIN_LEASE_TTL_SECS {
            bad.push(
                "integrity_check.lease_ttl_secs (must be >= 60; below that the heartbeat budget collapses)",
            );
        }
        if self.lease_ttl_secs > Self::MAX_LEASE_TTL_SECS {
            bad.push(
                "integrity_check.lease_ttl_secs (must be <= 86400; longer leases overlap with the next scheduled tick)",
            );
        }
        if self.lease_renew_period_secs < Self::MIN_LEASE_RENEW_PERIOD_SECS {
            bad.push(
                "integrity_check.lease_renew_period_secs (must be >= 5; sub-second heartbeats produce sustained DB load)",
            );
        }
        // Single missed heartbeat must not lose the lease: require
        // `renew * 2 <= ttl`. Two missed ticks then *can* lose it,
        // which is the desired TTL fallback behaviour — operators
        // who want a tighter survival window set `renew = ttl / N`
        // for larger N themselves. `checked_mul` guards against a
        // user-supplied `renew_period_secs >= 2^63` that would
        // panic in debug builds and wrap in release builds; an
        // overflow simply means the value is absurdly out of
        // range, which is the same conclusion as the comparison.
        let renew_doubled = self.lease_renew_period_secs.checked_mul(2);
        if renew_doubled.is_none_or(|v| v > self.lease_ttl_secs) {
            bad.push(
                "integrity_check.lease_renew_period_secs (must be <= lease_ttl_secs / 2; otherwise a single missed heartbeat loses the lease)",
            );
        }
        let repair_err = self.repair.validate().err();
        if bad.is_empty() && repair_err.is_none() {
            Ok(())
        } else {
            let mut parts: Vec<String> = bad.into_iter().map(str::to_owned).collect();
            if let Some(err) = repair_err {
                parts.push(err);
            }
            Err(format!(
                "integrity-check configuration is invalid: {}",
                parts.join(", ")
            ))
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "config_tests.rs"]
mod tests;
