//! `ToolKit` runtime runner.
//!
//! Supported DB modes:
//!   - `DbOptions::None` — gears get no DB in their contexts.
//!   - `DbOptions::Manager` — gears use `GearContextBuilder` to resolve per-gear `DbHandles`.
//!
//! Design notes:
//! - We use **`GearContextBuilder`** to resolve per-gear `DbHandles` at runtime.
//! - Phase order is orchestrated by `HostRuntime` (see `runtime/host_runtime.rs` docs).
//! - Gears receive a fully-scoped `GearCtx` with a resolved Option<DbHandle>.
//! - Shutdown can be driven by OS signals, an external `CancellationToken`,
//!   or an arbitrary future.
//! - Pre-registered clients can be injected into the `ClientHub` via `RunOptions::clients`.
//! - `OoP` gears are spawned after the start phase so that `grpc-hub` is already running
//!   and the real directory endpoint is known.

use crate::backends::OopBackend;
use crate::client_hub::ClientHub;
use crate::config::ConfigProvider;
use crate::registry::GearRegistry;
use crate::runtime::shutdown;
use crate::runtime::{DbOptions, HostRuntime};
use std::collections::HashMap;
use std::path::PathBuf;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// A type-erased client registration for injecting clients into the `ClientHub`.
///
/// This is used to pass pre-created clients (like gRPC clients) from bootstrap code
/// into the runtime's `ClientHub` before gears are initialized.
pub struct ClientRegistration {
    /// Callback that registers the client into the hub.
    register_fn: Box<dyn FnOnce(&ClientHub) + Send>,
}

impl ClientRegistration {
    /// Create a new client registration for a trait object type.
    ///
    /// # Example
    /// ```ignore
    /// let api: Arc<dyn DirectoryClient> = Arc::new(client);
    /// ClientRegistration::new::<dyn DirectoryClient>(api)
    /// ```
    pub fn new<T>(client: Arc<T>) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
    {
        Self {
            register_fn: Box::new(move |hub| {
                hub.register::<T>(client);
            }),
        }
    }

    /// Execute the registration against the given hub.
    pub(crate) fn apply(self, hub: &ClientHub) {
        (self.register_fn)(hub);
    }
}

/// How the runtime should decide when to stop.
pub enum ShutdownOptions {
    /// Listen for OS signals (Ctrl+C / SIGTERM).
    Signals,
    /// An external `CancellationToken` controls the lifecycle.
    Token(CancellationToken),
    /// An arbitrary future; when it completes, we initiate shutdown.
    Future(Pin<Box<dyn Future<Output = ()> + Send>>),
}

/// Configuration for a single `OoP` gear to be spawned.
#[derive(Clone)]
pub struct OopGearSpawnConfig {
    /// Name of the gear (e.g., "calculator").
    pub gear_name: String,
    /// Path to the gear executable.
    pub binary: PathBuf,
    /// Command-line arguments passed to the gear binary.
    ///
    /// Note: the user controls `--config` via `execution.args` in the master config.
    pub args: Vec<String>,
    /// Environment variables to set for the spawned process.
    pub env: HashMap<String, String>,
    /// Working directory for the spawned process.
    pub working_directory: Option<String>,
    /// Rendered gear configuration JSON, injected as the `TOOLKIT_MODULE_CONFIG` environment variable.
    pub rendered_config_json: String,
}

/// Options for spawning `OoP` gears.
pub struct OopSpawnOptions {
    /// Gears to spawn after the start phase, once shared infrastructure is ready.
    pub gears: Vec<OopGearSpawnConfig>,
    /// Backend used to spawn gear processes (e.g. `LocalProcessBackend`).
    pub backend: Box<dyn OopBackend>,
}

/// Options for running the `ToolKit` runner.
pub struct RunOptions {
    /// Provider of gear config sections (raw JSON by gear name).
    pub gears_cfg: Arc<dyn ConfigProvider>,
    /// DB strategy: none, or `DbManager`.
    pub db: DbOptions,
    /// Shutdown strategy.
    pub shutdown: ShutdownOptions,
    /// Pre-registered clients to inject into the `ClientHub` before gear initialization.
    ///
    /// This is useful for `OoP` bootstrap where clients (like `DirectoryGrpcClient`)
    /// are created before calling `run()` and need to be available in the `ClientHub`.
    pub clients: Vec<ClientRegistration>,
    /// Process-level instance ID.
    ///
    /// This is a unique identifier for this process instance, generated once at bootstrap
    /// (either in `run_oop_with_options` for `OoP` gears or in the main host).
    /// It is propagated to all gears via `GearCtx::instance_id()` and `SystemContext::instance_id()`.
    pub instance_id: Uuid,
    /// `OoP` gear spawn configuration.
    ///
    /// These gears are spawned after the start phase, once `grpc-hub` is running
    /// and the real directory endpoint is known.
    pub oop: Option<OopSpawnOptions>,
    /// Maximum time allowed for each gear's graceful shutdown before hard-stop.
    ///
    /// If `None`, uses `DEFAULT_SHUTDOWN_DEADLINE` (30 seconds).
    ///
    /// See `HostRuntime::with_shutdown_deadline` for details on the relationship
    /// with `WithLifecycle::stop_timeout`.
    pub shutdown_deadline: Option<std::time::Duration>,
}

/// Construct a `HostRuntime` by wiring shutdown, discovering gears, building
/// the `ClientHub`, and injecting pre-registered clients.
///
/// Shared by [`run`] and [`run_oop_serving`]. `opts` is consumed; the returned
/// `HostRuntime` owns the root lifecycle `CancellationToken`.
fn build_host_runtime(opts: RunOptions) -> anyhow::Result<HostRuntime> {
    // 1. Prepare cancellation token based on shutdown options.
    let cancel = match &opts.shutdown {
        ShutdownOptions::Token(t) => t.clone(),
        _ => CancellationToken::new(),
    };

    // 2. Spawn shutdown waiter (Signals / Future).
    match opts.shutdown {
        ShutdownOptions::Signals => {
            let c = cancel.clone();
            tokio::spawn(async move {
                match shutdown::wait_for_shutdown().await {
                    Ok(()) => {
                        tracing::info!(target: "", "------------------");
                        tracing::info!("shutdown: signal received");
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "shutdown: primary waiter failed; falling back to ctrl_c()"
                        );
                        _ = tokio::signal::ctrl_c().await;
                    }
                }
                c.cancel();
            });
        }
        ShutdownOptions::Future(waiter) => {
            let c = cancel.clone();
            tokio::spawn(async move {
                waiter.await;
                tracing::info!("shutdown: external future completed");
                c.cancel();
            });
        }
        ShutdownOptions::Token(_) => {
            tracing::info!("shutdown: external token will control lifecycle");
        }
    }

    // 3. Discover gears.
    let registry = GearRegistry::discover_and_build()?;

    // 4. Build shared `ClientHub` and inject pre-registered clients.
    let hub = Arc::new(ClientHub::default());
    for registration in opts.clients {
        registration.apply(&hub);
    }

    // 5. Instantiate `HostRuntime`.
    let mut host = HostRuntime::new(
        registry,
        opts.gears_cfg.clone(),
        opts.db,
        hub,
        cancel,
        opts.instance_id,
        opts.oop,
    );
    if let Some(deadline) = opts.shutdown_deadline {
        host = host.with_shutdown_deadline(deadline);
    }

    Ok(host)
}

/// Run the full gear lifecycle.
///
/// Discovers gears, wires shutdown, and delegates phase execution to [`HostRuntime`].
///
/// # Errors
/// Returns an error if any lifecycle phase fails.
pub async fn run(opts: RunOptions) -> anyhow::Result<()> {
    let host = build_host_runtime(opts)?;
    host.run_gear_phases().await
}

/// Run an out-of-process gear and serve its HTTP routes.
///
/// Uses the same setup as [`run`], then drives the `OoP` serving lifecycle:
/// lifecycle phases, an Axum HTTP server with framework probes, background
/// self-registration, dependency resolution, and graceful drain.
///
/// # Errors
/// Returns an error if discovery, any lifecycle phase, or the HTTP server fails.
#[cfg(feature = "bootstrap")]
pub async fn run_oop_serving(
    opts: RunOptions,
    serve: crate::runtime::OopServeOptions,
) -> anyhow::Result<()> {
    let host = build_host_runtime(opts)?;
    host.run_oop_serving(serve).await
}
