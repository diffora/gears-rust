//! `BssPricingGear` — toolkit gear declaration and lifecycle.
//!
//! The gear is one deployable modular monolith running in two roles: a
//! synchronous authoring / publish / preview API and a read-model service, over
//! one `toolkit-db` backend. `init()` is the composition root — one flat wiring
//! sequence — and stores a [`PricingRuntime`] the REST capability and the
//! background lifecycle both read.
//!
//! Capabilities: `db` (the Foundation tables and their append-only enforcement
//! are migrations), `rest` (the authoring + read-model surfaces), `stateful`
//! (the read-model warm re-drive, and later the window-activation job Slice 7
//! owns). Declared dependencies are the PEP and the type registry: every
//! ctx-bearing path gates through `access_scope` before touching a repository,
//! and the AuthZ label type-schemas are registered at init.
//!
//! The `stateful` entry now drives the read-model warm sweep, which is the only
//! thing in this gear that turns a publish's pending handle into a version a
//! consumer can pin: without the ticker `pricing_read_model` stays empty
//! whatever else is built.

use std::sync::Arc;

use anyhow::{Context, Result};
use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use axum::Router;
use chrono::Utc;
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use tokio_util::sync::CancellationToken;
use toolkit::api::OpenApiRegistry;
use toolkit::config::ConfigError;
use toolkit::contracts::{DatabaseCapability, RestApiCapability};
use toolkit::{Gear, GearCtx};
use toolkit_db::{DBProvider, DbError};
use tracing::info;

use crate::api::rest::frontier::ApiState as CatalogVersionApiState;
use crate::config::BssPricingConfig;
use crate::domain::ports::{CatalogVersionRegistryV1, UnconfiguredCatalogVersionRegistryV1};
use crate::infra::fixture_gate::FixtureGate;
use crate::infra::jobs::readmodel_warm::ReadModelWarmJob;
use crate::infra::publish::PublishService;
use crate::infra::storage::repo::PinFrontierRepo;

/// The coordination-lease key of the read-model warm sweep.
///
/// Per gear and per pass, never per tenant: one sweep is one pass over every
/// tenant, so there is nothing per-tenant to hold.
const WARM_LEASE_KEY: &str = "bss-pricing:readmodel-warm";

/// Per-process state built by [`Gear::init`] and read by
/// [`RestApiCapability::register_rest`] and [`BssPricingGear::serve`].
pub(crate) struct PricingRuntime {
    /// Database provider for the background work: the read-model warm sweep
    /// runs under a system context (`AccessScope::allow_all`, narrowed per
    /// tenant before every write), not a per-request `SecureORM` scope, and the
    /// coordination lease that makes it a singleton is built over the same
    /// handle.
    ///
    /// Acquired here rather than later on purpose: `db_required()` is what
    /// proves the declared `db` capability actually resolved, and a boot that
    /// cannot reach the database must fail at init, not at the first publish.
    /// Its `dead_code` allow is **discharged** — the reason it named has come
    /// true, and an allow whose reason has come true is an allow that hides the
    /// next thing.
    pub db: DBProvider<DbError>,
    /// The validated configuration, carried so the lifecycle reads the same
    /// values `init()` validated rather than re-parsing.
    pub config: BssPricingConfig,
    /// Platform PEP, built in `init()` from the `authz-resolver` `ClientHub`
    /// client and cloned into every request as an `Extension` by
    /// `register_rest`. Authz is security-critical, so a missing client fails
    /// init — there is no no-op fallback.
    pub enforcer: Arc<authz_resolver_sdk::PolicyEnforcer>,
    /// The publish engine: the subject assembler, the aggregate rule set and
    /// the §4.2 step-2 pre-check, over the repositories and the
    /// joint-conformance gate.
    ///
    /// **This is where the fixture gate and the `CatalogVersion` registry now
    /// live**, and both used to sit on this struct with a `dead_code` allow
    /// waiting for exactly this field. The registry is still resolved from
    /// `ClientHub` with a fail-closed default and still does NOT hard-fail init:
    /// the registry gear has no code in this repository, and a catalog that
    /// could not boot without it could not be developed at all. The cost is paid
    /// at the right moment instead — the commit requests addressability and
    /// stops when the answer is "unconfigured", so nothing becomes
    /// consumer-visible without a real version.
    ///
    /// The gate's own story is the same: it used to sit here with a
    /// `dead_code` allow reading "consulted by the publish engine, which lands
    /// with the publish path" — the publish path has landed, the
    /// gate is consulted by [`PublishService::precheck`], and the field it was
    /// waiting for is this one. The gate is still loaded once at init rather
    /// than per publish, because the registry is a static generated artifact and
    /// the gate runs inside a publish transaction, where reading a file is not
    /// an option; a registry that cannot be read leaves it CLOSED for every kind
    /// and does not abort the boot.
    ///
    /// The allow that remains is a narrower debt than the one it replaces: the
    /// engine has no **caller** because this gear has no authoring REST surface
    /// at all. `POST /bss-pricing/v1/plans/{planId}/publish` is G7's.
    #[allow(
        dead_code,
        reason = "called by the authoring REST surface (G7), which is the only thing that can reach a publish"
    )]
    pub publish: PublishService,
    /// The `CatalogVersion` registry, resolved at init with the fail-closed
    /// default.
    ///
    /// It has **two** holders now, deliberately, and
    /// [`PublishService`]'s own doc is narrowed rather than deleted to say why:
    /// the invariant it protects is that there is one place a version can be
    /// **requested** from, and the sweep never calls `request_version` — only
    /// `committed_version`, which asks what a handle the commit already
    /// obtained resolved to. One requester, two readers.
    pub catalog_version_registry: Arc<dyn CatalogVersionRegistryV1>,
    /// Per-request state for the catalog-version REST surface, built here so
    /// `register_rest` composes routers and does no wiring of its own.
    pub catalog_version_api: Arc<CatalogVersionApiState>,
}

#[toolkit::gear(name = "bss-pricing", capabilities = [db, rest, stateful], deps = [types_registry, authz_resolver], lifecycle(entry = "serve", stop_timeout = "30s"))]
pub struct BssPricingGear {
    /// `None` until `init()` completes, and on a boot where the gear is
    /// compiled in but not configured.
    runtime: ArcSwapOption<PricingRuntime>,
}

impl Default for BssPricingGear {
    fn default() -> Self {
        Self {
            runtime: ArcSwapOption::from(None),
        }
    }
}

impl BssPricingGear {
    /// Lifecycle entry (`stateful` capability).
    ///
    /// The Foundation's background work is the read-model warm re-drive, and it
    /// is not optional: the publish commit leaves a **pending**
    /// `CatalogVersion` handle and no version, so without this ticker nothing
    /// ever resolves it, `pricing_read_model` stays empty and no version ever
    /// becomes pin-eligible. §4.4's "the re-drive continues past the SLO with
    /// no bound" is this loop coming round again.
    ///
    /// An unconfigured gear still parks on the token: a gear compiled in but
    /// absent from `gears:` has no runtime and therefore no work.
    ///
    /// # Errors
    /// Never returns `Err` today; the signature is the lifecycle contract's,
    /// and a spawned ticker's join error will surface through it.
    pub(crate) async fn serve(self: Arc<Self>, cancel: CancellationToken) -> Result<()> {
        let Some(rt) = self.runtime.load_full() else {
            cancel.cancelled().await;
            return Ok(());
        };

        info!(
            warm_tick_secs = rt.config.jobs.readmodel_warm_tick_secs,
            "bss-pricing: lifecycle started"
        );
        let tasks = cancel.child_token();
        let warm = Self::spawn_warm_ticker(Arc::clone(&rt), tasks.clone());

        cancel.cancelled().await;
        tasks.cancel();
        Self::stop(warm).await;
        Ok(())
    }

    /// Wind the ticker down and say so.
    ///
    /// A join error is a **panic** in the sweep, and it is reported rather than
    /// swallowed: the loop catches every tick failure itself, so reaching this
    /// arm means something the job's own error handling did not model.
    async fn stop(warm: tokio::task::JoinHandle<()>) {
        if let Err(e) = warm.await {
            tracing::warn!(error = %e, "bss-pricing: warm ticker did not join cleanly");
        }
        info!("bss-pricing: lifecycle cancelled");
    }

    /// Spawn the read-model warm ticker: a cancellable loop driving one
    /// [`ReadModelWarmJob`] pass every `jobs.readmodel_warm_tick_secs`.
    ///
    /// The sibling ledger's `spawn_*_ticker` shape, and its three properties
    /// are all load-bearing here. `MissedTickBehavior::Delay` keeps a slow pass
    /// from queueing a burst of catch-up ticks behind it. The `biased` select
    /// takes cancellation first, so a shutdown does not wait out a tick. And a
    /// tick failure is **logged and the loop continues**: a transient storage
    /// or registry fault must not kill the gear, and the next pass re-drives
    /// exactly what this one did not finish.
    ///
    /// # The lease, and what it is actually for
    ///
    /// §3.8 makes this work "a singleton via the coordination lease library",
    /// and [`coord::LeaseManager`] is what enforces it —
    /// [`CoordError::LeaseHeld`] meaning a peer replica is already sweeping, at
    /// **debug**, because that is the normal state of a multi-replica
    /// deployment and not a fault.
    ///
    /// Worth being precise about what the lease buys, because it is less than
    /// it looks: two concurrent sweeps would not corrupt anything. Every write
    /// on the path is guarded by a key or a predicate — the delta INSERT by
    /// `pricing_read_model`'s primary key, the finalize by its
    /// `catalog_version IS NULL` compare-and-swap, the frontier by its
    /// forward-only predicate, the degraded event by
    /// `uq_pricing_outbox_dedup_key` — so a loser's transaction rolls back
    /// whole and its refs stay pending for the next tick. What the lease
    /// removes is **wasted work**, not a correctness hole.
    ///
    /// That is also why the TTL is the tick interval rather than some larger
    /// invented number: the TTL only bounds how long a **crashed** holder
    /// blocks its peers, and holding a slot for longer than the cadence would
    /// stall the sweep for a whole deployment to protect against a race that is
    /// already safe.
    ///
    /// The key is **per gear and per pass** (`bss-pricing:readmodel-warm`), not
    /// per tenant: one sweep is one pass over every tenant, so a per-tenant key
    /// would be a lock on a thing nobody takes.
    fn spawn_warm_ticker(
        rt: Arc<PricingRuntime>,
        token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(rt.config.jobs.readmodel_warm_interval());
            iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let lease = coord::LeaseManager::new(rt.db.db());
            let job = ReadModelWarmJob::new(
                rt.db.clone(),
                Arc::clone(&rt.catalog_version_registry),
                rt.config.jobs.clone(),
            );
            loop {
                tokio::select! {
                    biased;
                    () = token.cancelled() => break,
                    _ = iv.tick() => {
                        Self::warm_pass(&lease, &job, rt.config.jobs.readmodel_warm_interval())
                            .await;
                    }
                }
            }
        })
    }

    /// One leased pass. Extracted so the ticker stays a ticker.
    async fn warm_pass(
        lease: &coord::LeaseManager,
        job: &ReadModelWarmJob,
        ttl: std::time::Duration,
    ) {
        let Some(guard) = Self::take_warm_lease(lease, ttl).await else {
            return;
        };
        if let Err(e) = job.run(Utc::now()).await {
            tracing::error!(error = %e, "bss-pricing: read-model warm sweep tick failed");
        }
        if let Err(e) = guard.release().await {
            // The slot frees itself at the TTL, so a failed release costs one
            // skipped tick rather than a stuck sweep.
            tracing::warn!(error = %e, "bss-pricing: could not release the warm sweep lease");
        }
    }

    /// Take the singleton slot, or say why not.
    ///
    /// `LeaseHeld` is **debug**: in a multi-replica deployment every replica
    /// but one loses this every tick, which is the mechanism working rather
    /// than a fault.
    async fn take_warm_lease(
        lease: &coord::LeaseManager,
        ttl: std::time::Duration,
    ) -> Option<coord::LeaseGuard> {
        match lease.acquire(WARM_LEASE_KEY, ttl).await {
            Ok(guard) => Some(guard),
            Err(coord::CoordError::LeaseHeld) => {
                tracing::debug!("bss-pricing: read-model warm sweep skipped (a peer holds it)");
                None
            }
            Err(e) => {
                tracing::error!(error = %e, "bss-pricing: could not acquire the warm sweep lease");
                None
            }
        }
    }
}

#[async_trait]
impl Gear for BssPricingGear {
    /// Build the runtime when the gear is configured. Absent from `gears:` →
    /// no-op (compiled in but unconfigured); present-but-invalid config aborts
    /// init loudly rather than booting a catalog whose caps or cadences are
    /// nonsense.
    async fn init(&self, ctx: &GearCtx) -> Result<()> {
        match ctx.config::<BssPricingConfig>() {
            // Configured, or present with no `config:` section (defaults only).
            Ok(_) | Err(ConfigError::MissingConfigSection { .. }) => {}
            Err(ConfigError::GearNotFound { .. }) => {
                info!(
                    "bss-pricing: not present in the `gears:` config block, \
                     skipping init() (module compiled in but unconfigured)"
                );
                return Ok(());
            }
            Err(e) => return Err(e).context("bss-pricing: invalid `bss-pricing` config section"),
        }

        // Both fall-through arms above land here: `unwrap_or_default()` yields
        // the parsed config or the all-defaults config respectively.
        let config: BssPricingConfig = ctx.config().unwrap_or_default();
        config
            .validate()
            .map_err(|e| anyhow::anyhow!("bss-pricing: invalid config: {e}"))?;

        let db = ctx.db_required().context(
            "bss-pricing: ctx.db_required() failed; the `db` capability is declared \
             but no DbHandle is available",
        )?;

        // Platform PEP. Authz is security-critical — a catalog whose price book
        // is commercially sensitive must not run unauthorized — so a missing
        // `AuthZResolverClient` fails init loudly rather than degrading. No
        // `with_capabilities`: the PDP pre-expands the subtree to a flat `In`.
        let authz_client = ctx
            .client_hub()
            .get::<dyn authz_resolver_sdk::AuthZResolverClient>()
            .context(
                "bss-pricing: AuthZResolverClient absent from ClientHub; \
                 authz-resolver module must be registered",
            )?;
        let enforcer = Arc::new(authz_resolver_sdk::PolicyEnforcer::new(authz_client));

        // Register the authz-label stub schemas so RBAC role definitions
        // targeting the catalog labels pass target-type validation. Mandatory:
        // without them no custom catalog role can be defined, and the labels
        // deliberately sit outside `gts.cf.resources.*` where no built-in role
        // would cover them either — a silent skip would leave the whole
        // authoring surface ungrantable.
        let registry = ctx
            .client_hub()
            .get::<dyn types_registry_sdk::TypesRegistryClient>()
            .context(
                "bss-pricing: TypesRegistryClient absent from ClientHub; \
                 types-registry module must be registered",
            )?;
        let results = registry
            .register(crate::authz::authz_label_type_schemas())
            .await
            .context("bss-pricing: register authz label schemas")?;
        for result in results {
            if let types_registry_sdk::RegisterResult::Err { gts_id, error } = result {
                anyhow::bail!(
                    "bss-pricing: failed to register authz label {}: {error}",
                    gts_id.as_deref().unwrap_or("?")
                );
            }
        }

        // The `CatalogVersion` registry, with the fail-safe default. Absence is
        // survivable at boot and fatal at publish, which is the right split:
        // the registry gear is not in this repository yet, and a version this
        // gear invented locally would make it a second incrementer.
        let catalog_version_registry: Arc<dyn CatalogVersionRegistryV1> = ctx
            .client_hub()
            .get::<dyn CatalogVersionRegistryV1>()
            .unwrap_or_else(|_| {
                info!(
                    "bss-pricing: no CatalogVersionRegistryV1 registered; publish will fail \
                     closed until the registry gear is wired"
                );
                Arc::new(UnconfiguredCatalogVersionRegistryV1)
            });

        // The joint-conformance publish gate. Deliberately NOT fatal when the
        // registry cannot be read: `FixtureGate::load` returns a gate that is
        // closed for every kind and logs the cause, so the gear boots, the read
        // path that serves Rating and Tariffs keeps working, and every publish
        // fails per kind with `FIXTURE_MISSING`. There is no configuration value
        // that opens the gate — the corpus decides, not the deployment.
        let fixture_gate = FixtureGate::load(&config.fixtures.registry_path);
        let open_kinds = fixture_gate.open_kinds();
        if open_kinds.is_empty() {
            tracing::warn!(
                registry_path = %config.fixtures.registry_path.display(),
                "bss-pricing: the joint conformance fixture gate is CLOSED for EVERY model kind; \
                 reads are served normally and every publish will fail with FIXTURE_MISSING"
            );
        } else {
            info!(
                registry_path = %config.fixtures.registry_path.display(),
                open_kinds = ?open_kinds,
                // The kinds alone are a floor: a kind whose own fixture is green
                // still cannot publish a level or a tiered usage row unless the
                // matching cross-cutting variant is green too. Logging the pairs
                // is what keeps the first such refusal legible as the state of
                // the corpus rather than as a bug.
                open_variants = ?fixture_gate.open_variants(),
                "bss-pricing: joint conformance fixture gate loaded"
            );
        }

        // The catalog-version REST surface's state. The repository is cheap to
        // clone (it holds the provider), so the runtime keeps `db` for the
        // background work and the API layer gets its own handle.
        let catalog_version_api = Arc::new(CatalogVersionApiState {
            pin_frontier: PinFrontierRepo::new(db.clone()),
        });

        // The publish engine takes the gate by value - it is the only thing
        // that consults it, and a second holder would be a second answer to
        // "is this deployment's corpus green for that shape". The registry is
        // **cloned**, which narrows that sentence rather than deleting it: the
        // engine stays the only **requester** of a `CatalogVersion`, and the
        // warm sweep is a second **reader**, asking only what a handle the
        // commit already obtained resolved to.
        let publish = PublishService::new(
            db.clone(),
            &config.limits,
            fixture_gate,
            Arc::clone(&catalog_version_registry),
        );

        self.runtime.store(Some(Arc::new(PricingRuntime {
            db,
            config,
            enforcer,
            publish,
            catalog_version_registry,
            catalog_version_api,
        })));
        info!("bss-pricing: runtime published");
        Ok(())
    }
}

/// `DatabaseCapability` impl: `toolkit` runs these at platform startup, before
/// any catalog code reads the database.
///
/// The list is the gear's own Foundation chain plus the shared `coord` lease
/// migration — the read-model warm re-drive is a singleton job, so it needs the
/// lease table (see `infra::storage::migrations`). The platform runner applies
/// them in **name** order and rejects duplicate names outright, which is what
/// `tests/module_test.rs` pins.
impl DatabaseCapability for BssPricingGear {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        crate::infra::storage::migrations::Migrator::migrations()
    }
}

/// `RestApiCapability` impl. The authoring and read-model routers mount here as
/// their slices land; the gear reserves its prefix either way, so an
/// unconfigured boot answers 404 under `/bss-pricing/v1` rather than colliding
/// with another gear's namespace.
///
/// Two layers wrap the merged routers, exactly as the sibling ledger does: the
/// per-request PEP (the value, not the `Arc` — `PolicyEnforcer: Clone`), which
/// every gated handler extracts, and the canonical-error middleware that renders
/// a `CanonicalError` as its RFC 9457 problem document.
impl RestApiCapability for BssPricingGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: Router,
        openapi: &dyn OpenApiRegistry,
    ) -> Result<Router> {
        let Some(rt) = self.runtime.load_full() else {
            return Ok(router.nest("/bss-pricing/v1", Router::new()));
        };
        Ok(router
            .merge(crate::api::rest::frontier::router(
                Arc::clone(&rt.catalog_version_api),
                openapi,
            ))
            .layer(axum::Extension((*rt.enforcer).clone()))
            .layer(axum::middleware::from_fn(
                toolkit::api::canonical_error_middleware,
            )))
    }
}
