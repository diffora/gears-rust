//! Plugin-facing backend-provider traits (DESIGN §3.4 / §3.7).
//!
//! A plugin implements one or more of these traits to turn operator-supplied
//! config options into a running backend plus a shutdown hook. The wiring crate
//! (`cf-gears-cluster`) collects providers into a [`ProviderRegistry`] and
//! dispatches each profile's per-primitive binding against it.
//!
//! All four provider traits live in the SDK so plugins implement them while
//! depending only on `cluster-sdk`, never on the wiring crate. Options arrive as
//! a raw `serde_json::Map` (the wiring strips the framing `provider` and
//! `secret_ref` keys before the call), keeping the SDK free of any config schema.
//!
//! **Omit-default shorthand**: a profile that omits a primitive gets the SDK
//! default backend over the cache (`CasBasedLeaderElectionBackend`,
//! `CasBasedDistributedLockBackend`, `CacheBasedServiceDiscoveryBackend`). An
//! explicit binding always wins.
//!
//! **Non-cache providers do not receive the cache backend.** If a plugin natively
//! implements leader election (e.g. K8s Lease) it builds that backend from its own
//! options; it does not need the cache. The wiring layer provides the cache only to
//! the SDK-default auto-wrap path, not to native providers.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;

use crate::cache::ClusterCacheBackend;
use crate::discovery::ServiceDiscoveryBackend;
use crate::error::ClusterError;
use crate::leader::LeaderElectionBackend;
use crate::lock::DistributedLockBackend;

/// A boxed, owned shutdown action for a started backend. The cluster handle owns
/// it and awaits it once during shutdown — typically a plugin handle's `stop()`.
pub type StopHook = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// The cache provider-options key the omit-default service-discovery auto-wrap
/// reads for a non-default poll cadence (PGR-D3): a cache provider whose
/// options carry this key (e.g. the Postgres plugin's `sd_poll_interval_ms`)
/// gets that cadence threaded into `with_prefix_watch_polling` instead of the
/// SDK-default backend's own default. A documented convention over the raw
/// options map, not a schema field the SDK enforces — a provider opts in
/// simply by naming its own config field to match this constant, single-
/// sourcing the key so a rename is a compile error here rather than a
/// silent fallback to the default cadence at the wiring read site.
pub const SD_POLL_INTERVAL_MS_OPTION: &str = "sd_poll_interval_ms";

/// Builds the cache backend for one provider.
///
/// The cache backend is the foundational primitive; the wiring auto-wraps it with
/// the SDK-default leader-election, lock, and service-discovery backends for any
/// primitive the operator leaves unbound.
///
/// # Options contract
/// `options` is the flattened, provider-specific subset of one operator backend
/// binding. Keys evolve additively — new keys are optional with backward-compatible
/// defaults. An empty map means "all defaults".
///
/// # Errors
/// Returns [`ClusterError::InvalidConfig`] if `options` are invalid for this
/// provider, or propagates any startup error.
///
/// # Async
/// Building a backend is inherently async for most providers (opening a
/// connection pool, running migrations, establishing a subscribe/watch
/// connection) — Postgres, Redis, NATS, and etcd all need it. The trait is
/// `#[async_trait]` so plugins can `.await` that setup directly instead of
/// deferring it behind a readiness gate or a background task. The wiring
/// crate (`cf-gears-cluster`) calls providers from an already-`async fn`
/// context (`RunnableCapability::start`), so this never needs `block_on`.
#[async_trait]
pub trait ClusterCacheProvider: Send + Sync {
    /// The stable provider name matched against the operator config's `provider`
    /// field. Must be unique within a registry.
    fn provider(&self) -> &'static str;

    /// Builds and starts the cache backend. Returns the backend and a hook that
    /// stops any background work (sweeper, renewal loop, connection pool).
    ///
    /// # Errors
    /// Returns [`ClusterError::InvalidConfig`] if `options` are invalid.
    async fn build_cache(
        &self,
        options: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(Arc<dyn ClusterCacheBackend>, StopHook), ClusterError>;
}

/// Builds a native leader-election backend for one provider.
///
/// Implement this for plugins whose backend has a purpose-built leader-election
/// primitive (e.g. K8s Lease API, etcd election API). Plugins that only implement
/// cache leave leader election to the SDK default (`CasBasedLeaderElectionBackend`
/// over their cache).
///
/// # Errors
/// Returns [`ClusterError::InvalidConfig`] if `options` are invalid.
#[async_trait]
pub trait ClusterLeaderElectionProvider: Send + Sync {
    /// The stable provider name matched against the operator config's `provider`
    /// field for the `leader_election` primitive binding.
    fn provider(&self) -> &'static str;

    /// Builds and starts the leader-election backend.
    ///
    /// # Errors
    /// Returns [`ClusterError::InvalidConfig`] if `options` are invalid.
    async fn build_leader_election(
        &self,
        options: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(Arc<dyn LeaderElectionBackend>, StopHook), ClusterError>;
}

/// Builds a native distributed-lock backend for one provider.
///
/// Implement this for plugins whose backend has a purpose-built locking primitive
/// (e.g. `pg_advisory_lock`, etcd lock API, Redis `SET NX EX`). Plugins that only
/// implement cache leave locking to the SDK default (`CasBasedDistributedLockBackend`
/// over their cache).
///
/// # Errors
/// Returns [`ClusterError::InvalidConfig`] if `options` are invalid.
#[async_trait]
pub trait ClusterLockProvider: Send + Sync {
    /// The stable provider name matched against the operator config's `provider`
    /// field for the `lock` primitive binding.
    fn provider(&self) -> &'static str;

    /// Builds and starts the distributed-lock backend.
    ///
    /// # Errors
    /// Returns [`ClusterError::InvalidConfig`] if `options` are invalid.
    async fn build_lock(
        &self,
        options: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(Arc<dyn DistributedLockBackend>, StopHook), ClusterError>;
}

/// Builds a native service-discovery backend for one provider.
///
/// Implement this for plugins whose backend has a purpose-built service-discovery
/// primitive (e.g. K8s Lease-per-instance). Plugins that only implement cache
/// leave service discovery to the SDK default (`CacheBasedServiceDiscoveryBackend`
/// over their cache).
///
/// # Errors
/// Returns [`ClusterError::InvalidConfig`] if `options` are invalid.
#[async_trait]
pub trait ClusterServiceDiscoveryProvider: Send + Sync {
    /// The stable provider name matched against the operator config's `provider`
    /// field for the `service_discovery` primitive binding.
    fn provider(&self) -> &'static str;

    /// Builds and starts the service-discovery backend.
    ///
    /// # Errors
    /// Returns [`ClusterError::InvalidConfig`] if `options` are invalid.
    async fn build_service_discovery(
        &self,
        options: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(Arc<dyn ServiceDiscoveryBackend>, StopHook), ClusterError>;
}
