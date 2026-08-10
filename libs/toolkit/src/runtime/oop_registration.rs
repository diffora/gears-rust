//! Background self-registration and dependency resolution for `OoP` gears
//! (`cpt-cf-component-oop-bootstrap`).
//!
//! Both run as non-blocking background tasks so the HTTP server and its probes
//! are up immediately:
//!
//! - **Presence** ([`presence_loop`]) is the single owner of this instance's
//!   `DirectoryService` presence. It registers the instance's gRPC/REST
//!   endpoints and `OpenAPI` spec (retrying with exponential backoff,
//!   100ms → 30s cap), then runs one loop that both sends periodic **heartbeats**
//!   (the steady-state liveness signal the directory uses to keep the instance
//!   routable) and periodically **re-registers** to self-heal after a
//!   `DirectoryService` restart / connection loss. Consolidating both into one task (owned
//!   by the `OoP` serve lifecycle) avoids the split-brain where an independent
//!   heartbeat task and a re-registration task raced to write conflicting
//!   liveness state onto the same directory record.
//! - **Dependency resolution** ([`resolve_deps`]) polls
//!   `DirectoryService.resolve_rest_service` for each declared dependency, wires
//!   the resolved base URL into the [`ClientHub`] via [`ResolvedRestEndpoints`],
//!   and marks the dep resolved so `/readyz` can flip to `200`.
//!
//! In Profile 1 (in-process) dependency resolution is a no-op — the topo-sorted
//! `HostRuntime` already guarantees deps are initialized.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio_util::sync::CancellationToken;

use cf_system_sdks::directory::{DirectoryClient, RegisterInstanceInfo};

use super::readiness::ReadinessState;

/// Initial retry backoff for registration and dependency polling.
const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
/// Maximum retry backoff (cap).
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Interval at which a successfully-registered instance re-registers to
/// self-heal after a directory restart / connection loss.
const RE_REGISTER_INTERVAL: Duration = Duration::from_secs(30);

/// Next backoff in the exponential schedule (doubles, capped at [`MAX_BACKOFF`]).
fn next_backoff(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_BACKOFF)
}

/// Sleep for `dur`, returning early (`false`) if `cancel` fires first.
async fn sleep_or_cancel(dur: Duration, cancel: &CancellationToken) -> bool {
    tokio::select! {
        () = cancel.cancelled() => false,
        () = tokio::time::sleep(dur) => true,
    }
}

/// Resolved REST base URLs for the gear's dependencies, keyed by gear name.
///
/// Registered into the [`ClientHub`] by the `OoP` bootstrap and populated as
/// dependencies resolve. Until typed REST client codegen lands, gears look up a
/// dependency's base URL here (`hub.get::<ResolvedRestEndpoints>()`).
#[derive(Debug, Default)]
pub struct ResolvedRestEndpoints {
    inner: DashMap<String, String>,
}

impl ResolvedRestEndpoints {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a resolved base URL for `gear`.
    pub fn set(&self, gear: impl Into<String>, uri: impl Into<String>) {
        self.inner.insert(gear.into(), uri.into());
    }

    /// Look up the resolved base URL for `gear`, if known.
    #[must_use]
    pub fn get(&self, gear: &str) -> Option<String> {
        self.inner.get(gear).map(|v| v.value().clone())
    }

    /// Number of resolved dependencies.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if no dependencies have been resolved yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Register `info` with the directory, retrying with exponential backoff until
/// success or cancellation. Returns `true` on success, `false` if cancelled.
async fn register_once_with_backoff(
    directory: &Arc<dyn DirectoryClient>,
    info: &RegisterInstanceInfo,
    cancel: &CancellationToken,
) -> bool {
    let mut backoff = INITIAL_BACKOFF;
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        match directory.register_instance(info.clone()).await {
            Ok(()) => {
                tracing::info!(gear = %info.gear, instance = %info.instance_id, "registered with DirectoryService");
                return true;
            }
            Err(e) => {
                tracing::warn!(
                    gear = %info.gear,
                    error = %e,
                    backoff_ms = backoff.as_millis(),
                    "registration attempt failed; retrying"
                );
                if !sleep_or_cancel(backoff, cancel).await {
                    return false;
                }
                backoff = next_backoff(backoff);
            }
        }
    }
}

/// Single directory-presence loop for an `OoP` instance.
///
/// Registers `info` (with backoff), then owns **both** liveness signals in one
/// task until `cancel` fires:
///
/// - a **heartbeat** every `heartbeat_interval` — the steady-state signal the
///   directory uses to keep the instance `Healthy`/routable (and to avoid
///   heartbeat-timeout eviction);
/// - an idempotent **re-registration** every [`RE_REGISTER_INTERVAL`] — self-heals
///   after a `DirectoryService` restart / connection loss (a heartbeat alone cannot,
///   since the directory silently ignores heartbeats for an instance it has
///   forgotten). Re-registration preserves the directory-side liveness state
///   (see `GearManager::register_instance`), so it does not disturb the
///   heartbeat-maintained `Healthy` state.
///
/// Registering *before* the first heartbeat is deliberate: a heartbeat for an
/// unregistered instance is a no-op on the directory.
pub(super) async fn presence_loop(
    directory: Arc<dyn DirectoryClient>,
    info: RegisterInstanceInfo,
    heartbeat_interval: Duration,
    cancel: CancellationToken,
) {
    if !register_once_with_backoff(&directory, &info, &cancel).await {
        return;
    }

    // `tokio::time::interval` panics on a zero period; clamp defensively.
    let heartbeat_interval = heartbeat_interval.max(Duration::from_secs(1));
    let mut heartbeat = tokio::time::interval(heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await; // consume the immediate first tick

    let mut reregister = tokio::time::interval(RE_REGISTER_INTERVAL);
    reregister.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    reregister.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            _ = heartbeat.tick() => {
                if let Err(e) = directory.send_heartbeat(&info.gear, &info.instance_id).await {
                    tracing::warn!(
                        gear = %info.gear,
                        instance = %info.instance_id,
                        error = %e,
                        "heartbeat failed; re-registering to self-heal"
                    );
                    if !register_once_with_backoff(&directory, &info, &cancel).await {
                        return;
                    }
                } else {
                    tracing::trace!(gear = %info.gear, "heartbeat sent");
                }
            }
            _ = reregister.tick() => {
                if !register_once_with_backoff(&directory, &info, &cancel).await {
                    return;
                }
            }
        }
    }
}

/// Poll for a single dependency until resolved (or cancelled), then wire it into
/// the resolved-endpoints registry and mark it resolved in readiness.
async fn resolve_one_dep(
    directory: Arc<dyn DirectoryClient>,
    dep: String,
    readiness: Arc<ReadinessState>,
    resolved: Arc<ResolvedRestEndpoints>,
    cancel: CancellationToken,
) {
    let mut backoff = INITIAL_BACKOFF;
    loop {
        if cancel.is_cancelled() {
            return;
        }
        match directory.resolve_rest_service(&dep).await {
            Ok(endpoint) => {
                tracing::info!(dep = %dep, endpoint = %endpoint.uri, "resolved REST dependency");
                resolved.set(dep.clone(), endpoint.uri);
                readiness.mark_dep_resolved(&dep);
                return;
            }
            Err(e) => {
                tracing::debug!(dep = %dep, error = %e, "dependency not yet resolvable; retrying");
                if !sleep_or_cancel(backoff, &cancel).await {
                    return;
                }
                backoff = next_backoff(backoff);
            }
        }
    }
}

/// Spawn one resolution task per dependency. Each resolves independently and
/// gates `/readyz` via `readiness`. A no-op when `deps` is empty (Profile 1).
pub(super) fn resolve_deps(
    directory: &Arc<dyn DirectoryClient>,
    deps: Vec<String>,
    readiness: &Arc<ReadinessState>,
    resolved: &Arc<ResolvedRestEndpoints>,
    cancel: &CancellationToken,
) {
    for dep in deps {
        tokio::spawn(resolve_one_dep(
            Arc::clone(directory),
            dep,
            Arc::clone(readiness),
            Arc::clone(resolved),
            cancel.clone(),
        ));
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "oop_registration_tests.rs"]
mod tests;
