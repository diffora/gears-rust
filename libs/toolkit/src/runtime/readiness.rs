//! Framework-managed readiness state for the `OoP` bootstrap (`cpt-cf-fr-eventual-readiness`).
//!
//! An `OoP` gear becomes *live* the moment its HTTP server binds (`/healthz`),
//! but only becomes *ready* (`/readyz`) once startup is complete, every critical
//! dependency has been resolved, **and** the gear's registered healthchecks
//! report it can serve traffic (Spring Boot-style health groups per
//! `cpt-cf-fr-eventual-readiness`).
//!
//! Readiness reuses the framework's standard healthcheck mechanism
//! ([`crate::healthcheck`]): a gear expresses readiness once, via
//! [`RestApiCapability::healthcheck`](crate::contracts::RestApiCapability::healthcheck),
//! and it is honored identically whether the gear is hosted in-process by the
//! `api-gateway` or run `OoP`. This module layers the three `OoP`-only concerns
//! the gateway path lacks — startup completion, critical-dependency resolution
//! gating, and the graceful-drain readiness flip — on top of that shared
//! healthcheck report.
//!
//! The healthcheck report itself is fanned out concurrently, timeout-bounded,
//! panic-isolated, and cached inside the [`RestHealthcheckRegistry`], so a burst
//! of probe traffic cannot storm the registered checks. The dependency and
//! draining state layered on top are cheap live reads.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use crate::healthcheck::{HealthcheckReport, HealthcheckStatus, RestHealthcheckRegistry};

/// Default per-check timeout for readiness healthchecks.
///
/// Matches the `api-gateway` `healthcheck_timeout_ms` default so a gear's
/// healthcheck behaves identically in-process and `OoP`.
pub const DEFAULT_HEALTHCHECK_TIMEOUT: Duration = Duration::from_millis(500);

/// Lifecycle state reported on `/readyz` (`cpt-cf-adr-eventual-readiness`).
///
/// Serialized lowercase; the four variants are a stable wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadinessLifecycle {
    /// Not yet able to serve — startup is not complete, critical deps are
    /// unresolved, or a healthcheck is `Unhealthy`. Maps to `503`.
    Starting,
    /// Fully serving traffic. Maps to `200`.
    Ready,
    /// Serving with reduced functionality — a healthcheck reported `Degraded`
    /// (e.g. an optional backend is down but a fallback is acceptable). Kept in
    /// rotation: maps to `200`.
    Degraded,
    /// Graceful shutdown in progress; upstreams should stop routing. Maps to
    /// `503`.
    Draining,
}

/// The aggregate readiness report rendered as the `/readyz` response body.
///
/// Readiness still depends on the aggregated [`HealthcheckReport`] internally,
/// but the detailed per-component report is intentionally not echoed here; it
/// belongs on the separate `/health` endpoint (see `oop_serve.rs`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReadinessReport {
    /// Lifecycle state — the primary readiness signal (`cpt-cf-adr-eventual-readiness`).
    pub state: ReadinessLifecycle,
    /// Whether the gear is ready to receive traffic (`true` → `200`, else `503`).
    /// Convenience mirror of `state ∈ {ready, degraded}` for probes/clients that
    /// do not want to know the `state → status` mapping.
    pub ready: bool,
    /// Critical dependencies not yet resolved via `DirectoryService` / DNS.
    /// Non-empty only while `starting`. Omitted from the body when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unresolved_deps: Vec<String>,
}

/// Shared, framework-owned readiness state for an `OoP` gear instance.
///
/// Created by the `OoP` bootstrap with the gear's critical dependency names and
/// a shared [`RestHealthcheckRegistry`] (populated from each gear's
/// [`RestApiCapability::healthcheck`](crate::contracts::RestApiCapability::healthcheck)).
/// Cloned as an `Arc` into the probe router and the dependency-resolution task.
pub struct ReadinessState {
    /// Critical deps still awaiting resolution. Empty ⇒ deps satisfied.
    unresolved_deps: Mutex<BTreeSet<String>>,
    /// Graceful-shutdown flag; when set, `/readyz` reports `503` (readiness flip).
    draining: AtomicBool,
    /// Whether the gear has finished startup and is actually serving traffic.
    /// Remains `false` until the bootstrap publishes the composed routes.
    startup_complete: AtomicBool,
    /// Shared gear healthcheck registry; supplies the "custom checks" dimension
    /// of readiness (fan-out, timeout, panic isolation, and caching live here).
    healthchecks: Arc<RestHealthcheckRegistry>,
    /// Per-check timeout passed to [`RestHealthcheckRegistry::report`].
    check_timeout: Duration,
}

impl std::fmt::Debug for ReadinessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadinessState")
            .field("unresolved_deps", &self.unresolved_deps.lock())
            .field("draining", &self.draining.load(Ordering::Relaxed))
            .field(
                "startup_complete",
                &self.startup_complete.load(Ordering::Relaxed),
            )
            .field("check_timeout", &self.check_timeout)
            .finish_non_exhaustive()
    }
}

impl ReadinessState {
    /// Create a new readiness state seeded with the gear's critical dependency
    /// names and the shared healthcheck registry. All listed deps start
    /// unresolved; the gear is not ready until startup is complete, each dep is
    /// marked resolved via [`mark_dep_resolved`](Self::mark_dep_resolved), and
    /// the healthchecks pass. Uses [`DEFAULT_HEALTHCHECK_TIMEOUT`] as the
    /// per-check timeout.
    #[must_use]
    pub fn new<I, S>(critical_deps: I, healthchecks: Arc<RestHealthcheckRegistry>) -> Arc<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::with_check_timeout(critical_deps, healthchecks, DEFAULT_HEALTHCHECK_TIMEOUT)
    }

    /// Like [`new`](Self::new) but with an explicit per-check timeout.
    #[must_use]
    pub fn with_check_timeout<I, S>(
        critical_deps: I,
        healthchecks: Arc<RestHealthcheckRegistry>,
        check_timeout: Duration,
    ) -> Arc<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Arc::new(Self {
            unresolved_deps: Mutex::new(critical_deps.into_iter().map(Into::into).collect()),
            draining: AtomicBool::new(false),
            startup_complete: AtomicBool::new(false),
            healthchecks,
            check_timeout,
        })
    }

    /// Mark startup as complete. Idempotent; subsequent calls are ignored.
    /// `/readyz` will not report `Ready` or `Degraded` until this is called.
    pub fn mark_startup_complete(&self) {
        self.startup_complete.store(true, Ordering::SeqCst);
    }

    /// Whether startup is complete.
    #[must_use]
    pub fn is_startup_complete(&self) -> bool {
        self.startup_complete.load(Ordering::SeqCst)
    }

    /// Mark a critical dependency as resolved. Idempotent; unknown names are
    /// ignored.
    pub fn mark_dep_resolved(&self, name: &str) {
        let removed = self.unresolved_deps.lock().remove(name);
        if removed {
            tracing::info!(dep = %name, "critical dependency resolved");
        }
    }

    /// Whether all critical dependencies have been resolved.
    #[must_use]
    pub fn all_deps_resolved(&self) -> bool {
        self.unresolved_deps.lock().is_empty()
    }

    /// Set (or clear) the draining flag. Setting it flips `/readyz` to `503`
    /// immediately so upstreams pull the instance out of rotation while
    /// in-flight requests drain.
    pub fn set_draining(&self, draining: bool) {
        self.draining.store(draining, Ordering::SeqCst);
    }

    /// Whether the gear is currently draining.
    #[must_use]
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }

    /// Run the registered healthchecks and return the aggregated report.
    ///
    /// Used by `/health` to expose full per-component detail and by
    /// [`Self::evaluate`] to decide readiness state. The registry caches the
    /// report, so repeated calls within the cache window do not re-run checks.
    pub async fn health_report(&self) -> HealthcheckReport {
        self.healthchecks.report(self.check_timeout).await
    }

    /// Evaluate the aggregate readiness.
    ///
    /// The gear is ready when it is not draining, startup is complete, all
    /// critical deps are resolved, and the aggregated healthcheck report is not
    /// `Unhealthy`. `Degraded` healthchecks keep the gear ready (`state =
    /// degraded`, `ready = true`) but the detailed per-component messages belong
    /// on `/health`, not `/readyz`. The healthcheck fan-out is cached inside the
    /// registry, so repeated probe traffic does not re-run checks; dependency and
    /// draining state are read live so transitions take effect immediately.
    pub async fn evaluate(&self) -> ReadinessReport {
        let health = self.health_report().await;
        let draining = self.is_draining();
        let startup_complete = self.is_startup_complete();
        let unresolved_deps: Vec<String> = self.unresolved_deps.lock().iter().cloned().collect();

        // Draining wins; then any not-ready condition (startup not complete,
        // unresolved deps, or an Unhealthy check) is `Starting`; then `Degraded`;
        // else `Ready`.
        let state = if draining {
            ReadinessLifecycle::Draining
        } else if !startup_complete
            || !unresolved_deps.is_empty()
            || health.status == HealthcheckStatus::Unhealthy
        {
            ReadinessLifecycle::Starting
        } else if health.status == HealthcheckStatus::Degraded {
            ReadinessLifecycle::Degraded
        } else {
            ReadinessLifecycle::Ready
        };

        let ready = matches!(
            state,
            ReadinessLifecycle::Ready | ReadinessLifecycle::Degraded
        );

        ReadinessReport {
            state,
            ready,
            unresolved_deps,
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "readiness_tests.rs"]
mod tests;
