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
//! (the read-model warm re-drive **and** the Slice 7 window-activation sweep —
//! that clause used to say "and later the window-activation job Slice 7 owns",
//! and later arrived). Declared dependencies are the PEP and the type registry:
//! every ctx-bearing path gates through `access_scope` before touching a
//! repository, and the AuthZ label type-schemas are registered at init.
//!
//! The `stateful` entry drives **two** independent leased tickers, each with its
//! own key and cadence (`crate::infra::jobs` states what each is for). Neither
//! waits on the other: the warm re-drive is what turns a publish's pending handle
//! into a version a consumer can pin — without it `pricing_read_model` stays
//! empty whatever else is built — and the activation sweep is what makes a
//! scheduled window take effect at its own instant rather than at the next
//! publish.

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
use crate::api::rest::state::{AuthoringState, GovernanceState};
use crate::config::BssPricingConfig;
use crate::domain::ports::{CatalogVersionRegistryV1, UnconfiguredCatalogVersionRegistryV1};
use crate::infra::approval::ApprovalService;
use crate::infra::fixture_gate::FixtureGate;
use crate::infra::jobs::readmodel_warm::ReadModelWarmJob;
use crate::infra::jobs::window_activation::WindowActivationJob;
use crate::infra::publish::PublishService;
use crate::infra::storage::repo::{
    BundleRepo, IdempotencyGate, PinFrontierRepo, PlanRepo, PlanShapeRepo, PriceRepo,
};
use crate::infra::window::WindowService;

/// The coordination-lease key of the read-model warm sweep.
///
/// Per gear and per pass, never per tenant: one sweep is one pass over every
/// tenant, so there is nothing per-tenant to hold.
const WARM_LEASE_KEY: &str = "bss-pricing:readmodel-warm";

/// The coordination-lease key of the window activation/expiry sweep.
///
/// Per gear and per pass, never per tenant, for [`WARM_LEASE_KEY`]'s reason
/// stated above — one sweep is one pass over every tenant, so a per-tenant key
/// would be a lock on a thing nobody takes.
///
/// A **second** key rather than a share of the first, because the two passes are
/// independent (`crate::infra::jobs` says so): one key would make them a queue,
/// and a window boundary would then wait on a registry that is not answering.
const WINDOW_ACTIVATION_LEASE_KEY: &str = "bss-pricing:window-activation";

/// Per-process state built by [`Gear::init`] and read by
/// [`RestApiCapability::register_rest`] and [`BssPricingGear::serve`].
pub(crate) struct PricingRuntime {
    /// Database provider for the background work: the sweeps run under a system
    /// context (`AccessScope::allow_all`, narrowed per tenant before every write),
    /// not a per-request `SecureORM` scope, and the coordination leases that make
    /// each of them a singleton are built over this same handle — one
    /// `LeaseManager` per ticker, each on its own key.
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
    /// Per-request state for the plan and price authoring surfaces, built here
    /// for the same reason: one place wires, one place composes.
    pub authoring_api: Arc<AuthoringState>,
    /// Per-request state for the approval surface and the publish mount.
    pub governance_api: Arc<GovernanceState>,
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
    /// Two tickers, spawned below and both joined — `crate::infra::jobs` says what
    /// each is for and why neither waits on the other. Neither is optional, for two
    /// different reasons: the publish commit leaves a **pending** `CatalogVersion`
    /// handle and no version, so without the warm re-drive nothing ever resolves it,
    /// `pricing_read_model` stays empty and no version becomes pin-eligible (§4.4's
    /// "the re-drive continues past the SLO with no bound" is that loop coming round
    /// again); and without the activation sweep a scheduled window takes effect at
    /// no instant at all.
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
            window_activation_tick_secs = rt.config.jobs.window_activation_tick_secs,
            "bss-pricing: lifecycle started"
        );
        let tasks = cancel.child_token();
        let warm = Self::spawn_warm_ticker(Arc::clone(&rt), tasks.clone());
        let activation = Self::spawn_activation_ticker(Arc::clone(&rt), tasks.clone());

        cancel.cancelled().await;
        tasks.cancel();
        Self::stop(warm, activation).await;
        Ok(())
    }

    /// Wind both tickers down and say so.
    ///
    /// A join error is a **panic** in a sweep, and it is reported rather than
    /// swallowed: each loop catches every tick failure itself, so reaching one of
    /// these arms means something the job's own error handling did not model.
    ///
    /// **Both are joined, and neither is skipped when the other faults.** They
    /// are cancelled by one token and each holds a coordination lease whose slot
    /// frees itself at the TTL, so a shutdown that abandoned the second handle
    /// would leave a task writing to a database the process is closing — the one
    /// state a `stop_timeout` cannot help with.
    async fn stop(warm: tokio::task::JoinHandle<()>, activation: tokio::task::JoinHandle<()>) {
        Self::join_ticker(warm, "readmodel-warm").await;
        Self::join_ticker(activation, "window-activation").await;
        info!("bss-pricing: lifecycle cancelled");
    }

    /// Await one ticker's handle, reporting a panic rather than swallowing it.
    ///
    /// One function for both, and the `ticker` field is what tells them apart:
    /// two copies of this `if let` is two places the warning could be dropped
    /// from, and the second handle is exactly the one a shutdown is tempted to
    /// stop caring about.
    async fn join_ticker(handle: tokio::task::JoinHandle<()>, ticker: &'static str) {
        if let Err(e) = handle.await {
            tracing::warn!(error = %e, ticker, "bss-pricing: a ticker did not join cleanly");
        }
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
        let Some(guard) = Self::take_lease(lease, WARM_LEASE_KEY, ttl).await else {
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

    /// Spawn the window activation/expiry ticker: a cancellable loop driving one
    /// [`WindowActivationJob`] pass every `jobs.window_activation_tick_secs`.
    ///
    /// [`Self::spawn_warm_ticker`]'s shape, and its three properties hold here
    /// for the same reasons — `MissedTickBehavior::Delay`, the `biased` select,
    /// and a tick failure logged rather than fatal. What is **not** inherited is
    /// the argument about what the lease is for.
    ///
    /// # What THIS lease buys, which is not what the warm sweep's buys
    ///
    /// [`Self::spawn_warm_ticker`]'s doc is precise that its lease removes
    /// **wasted work, not a correctness hole**, because every write on that path
    /// is guarded by a key or a predicate. That is a claim measured on that path,
    /// and it is not inherited: it was checked here, and the check had two halves
    /// with two different answers.
    ///
    /// * **The flip was already guarded.** `window_repo::transition` carries
    ///   `state = <expected>` **and** §4's boundary condition into its `UPDATE`'s
    ///   `WHERE`, so a second sweep's flip matches zero rows and `activated_at` —
    ///   the instant a price took effect — is written exactly once. So far the
    ///   warm sweep's sentence holds.
    /// * **The event was not.** Two concurrent sweeps flipping one window would
    ///   have enqueued **two** `PriceWindowActivated` rows for one transition.
    ///   At-least-once delivery lets a consumer see one event twice; it does not
    ///   let one transition *be* two events, at two `seq` positions of the plan's
    ///   stream, dedupable by nothing a consumer holds. A lease cannot close that,
    ///   because a lease can be **lost** — losing it is what a takeover is.
    ///
    /// So it was closed in the store instead:
    /// `outbox_repo::price_window_transition_dedup_key` covers
    /// `(window_id, transition)`, and the row is refused by the **pair** of
    /// constraints that key reaches — `uq_pricing_outbox_dedup_key` and the
    /// `outbox_id` primary key, which `outbox_repo::outbox_id` derives from the same
    /// `(tenant_id, dedup_key)` precisely so a repeat collides on both. Which of the
    /// two answers first is not knowable from here and does not matter:
    /// `contention_or_db`'s own comment records that the driver's error class cannot
    /// tell a table's unique constraints apart, so naming one member would be
    /// picking a winner nothing observes.
    ///
    /// The flip and the enqueue share **one transaction**, so a committed flip
    /// cannot stand without its event nor an event without its flip. That is a
    /// property of the code's shape — one `in_transaction` closure containing both
    /// writes — and it is stated as one rather than as suite-proved, because the
    /// suite does not prove it. `tests/postgres_window_activation.rs` races two
    /// sweeps and proves **one flip, one event**; in that race the loser's `UPDATE`
    /// matches zero rows and `transition` answers `Ok` on the self-edge once the
    /// winner commits, so the loser has nothing on the window side to roll back. The
    /// property that needs the transaction is a *committed* flip whose enqueue then
    /// fails, and no test exercises that — it would take an enqueue made to fail
    /// after a successful flip, which is a fault this suite has no way to inject.
    ///
    /// **With that key in place the sentence is true of this path too**, and the
    /// difference from the warm sweep is where the guard lives rather than
    /// whether there is one: this lease removes wasted work, and the dedup key is
    /// what makes losing the lease safe.
    ///
    /// The TTL is the tick interval, for [`Self::spawn_warm_ticker`]'s reason: it
    /// bounds only how long a **crashed** holder blocks its peers, and a longer
    /// slot would stall the sweep for a whole deployment — here at the cost of
    /// window boundaries going uncrossed, which is what the Warn alarm the pass
    /// raises would then report.
    fn spawn_activation_ticker(
        rt: Arc<PricingRuntime>,
        token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(rt.config.jobs.window_activation_interval());
            iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let lease = coord::LeaseManager::new(rt.db.db());
            let job = WindowActivationJob::new(rt.db.clone(), rt.config.jobs.clone());
            loop {
                tokio::select! {
                    biased;
                    () = token.cancelled() => break,
                    _ = iv.tick() => {
                        Self::activation_pass(
                            &lease,
                            &job,
                            rt.config.jobs.window_activation_interval(),
                        )
                        .await;
                    }
                }
            }
        })
    }

    /// One leased activation pass. Extracted so the ticker stays a ticker.
    async fn activation_pass(
        lease: &coord::LeaseManager,
        job: &WindowActivationJob,
        ttl: std::time::Duration,
    ) {
        let Some(guard) = Self::take_lease(lease, WINDOW_ACTIVATION_LEASE_KEY, ttl).await else {
            return;
        };
        match job.run(Utc::now()).await {
            Ok(report) => Self::log_activation(&report),
            Err(e) => {
                tracing::error!(error = %e, "bss-pricing: window activation sweep tick failed");
            }
        }
        if let Err(e) = guard.release().await {
            // The slot frees itself at the TTL, so a failed release costs one
            // skipped tick rather than a stuck sweep.
            tracing::warn!(
                error = %e,
                "bss-pricing: could not release the window activation sweep lease"
            );
        }
    }

    /// Report a pass that moved something, and stay silent about one that did not.
    ///
    /// A pass that found nothing due is the steady state at every tick for as long
    /// as no boundary falls inside one, so logging it at `info` would bury the
    /// passes that did something under the passes that did not. The alarm the pass
    /// raises for a late boundary is `tracing::error!` inside the job and is not
    /// gated by this.
    fn log_activation(report: &crate::infra::jobs::window_activation::ActivationReport) {
        if report.activated == 0 && report.expired == 0 && report.failed == 0 {
            return;
        }
        info!(
            activated = report.activated,
            expired = report.expired,
            failed = report.failed,
            overdue = report.overdue,
            "bss-pricing: window activation sweep moved windows"
        );
    }

    /// Take a singleton slot, or say why not.
    ///
    /// `LeaseHeld` is **debug**: in a multi-replica deployment every replica
    /// but one loses this every tick, which is the mechanism working rather
    /// than a fault.
    ///
    /// One function for both passes because the acquire is one rule — the key is
    /// what differs, and a second copy of this match is a second place the
    /// `LeaseHeld` arm could be promoted to a warning by somebody who had not
    /// read the sentence above.
    async fn take_lease(
        lease: &coord::LeaseManager,
        key: &'static str,
        ttl: std::time::Duration,
    ) -> Option<coord::LeaseGuard> {
        match lease.acquire(key, ttl).await {
            Ok(guard) => Some(guard),
            Err(coord::CoordError::LeaseHeld) => {
                tracing::debug!(
                    lease = key,
                    "bss-pricing: sweep skipped (a peer holds its lease)"
                );
                None
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    lease = key,
                    "bss-pricing: could not acquire a sweep lease"
                );
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

        // The authoring surface's state. The repositories are cheap clones over
        // the provider; `db` is carried because `infra::idempotent` opens the
        // one transaction the at-most-once contract requires, and the gate holds
        // the configured retention window because expiry is decided on the claim
        // path rather than by a reaper.
        let authoring_api = Arc::new(AuthoringState {
            db: db.clone(),
            plans: PlanRepo::new(db.clone()),
            shapes: PlanShapeRepo::new(db.clone()),
            prices: PriceRepo::new(db.clone()),
            // Slice 8's two: the composition store, and the seam that assembles
            // a composition for the pure rules to judge.
            bundles: BundleRepo::new(db.clone()),
            bundle_service: crate::infra::bundle::BundleService::new(db.clone()),
            // Slice 9's overlay store. Here and not on `GovernanceState`
            // because it requests no `CatalogVersion`, which is the criterion
            // that split the two.
            overlays: crate::infra::storage::repo::OverlayRepo::new(db.clone()),
            idempotency: IdempotencyGate::new(config.limits.idempotency_key_ttl()),
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

        // The governance surface's state, and the publish engine's **only**
        // holder. It sat on `PricingRuntime` behind a `dead_code` allow for two
        // phases because nothing could reach `commit`; the route that reaches it
        // is mounted here, so the engine moves to the state that serves it
        // rather than staying somewhere with an allow and a second reader.
        let governance_api = Arc::new(GovernanceState {
            db: db.clone(),
            plans: PlanRepo::new(db.clone()),
            prices: PriceRepo::new(db.clone()),
            approvals: ApprovalService::new(db.clone()),
            publish,
            // The same registry `Arc` the engine holds. Two requesters of one
            // registry, never two incrementers — `api::rest::state`'s module doc
            // carries the argument and the correction it replaces.
            windows: WindowService::new(db.clone(), Arc::clone(&catalog_version_registry)),
            // The **third** requester of that same `Arc` (D-88). Three requesters is
            // still one incrementer, and `api::rest::state` carries what keeps their
            // handles apart.
            supersessions: crate::infra::supersession::SupersessionService::new(
                db.clone(),
                Arc::clone(&catalog_version_registry),
            ),
            // The window `POST`'s at-most-once gate (D-191), under the **same** TTL the
            // authoring plane's claims expire on: the expiry is a deployment knob about
            // how long a client key is honoured, and two windows for it would mean one
            // caller's retry is protected on one surface and not on another.
            idempotency: IdempotencyGate::new(config.limits.idempotency_key_ttl()),
            thresholds: crate::infra::threshold::ThresholdService::new(db.clone()),
        });

        self.runtime.store(Some(Arc::new(PricingRuntime {
            db,
            config,
            enforcer,
            catalog_version_registry,
            catalog_version_api,
            authoring_api,
            governance_api,
        })));
        info!("bss-pricing: runtime published");
        Ok(())
    }
}

/// `DatabaseCapability` impl: `toolkit` runs these at platform startup, before
/// any catalog code reads the database.
///
/// The list is the gear's own Foundation chain plus the shared `coord` lease
/// migration — this gear's background work is coordinated as a singleton, so it
/// needs the lease table (see `infra::storage::migrations`). The platform runner
/// applies them in **name** order and rejects duplicate names outright, which is
/// what `tests/module_test.rs` pins.
impl DatabaseCapability for BssPricingGear {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        crate::infra::storage::migrations::Migrator::migrations()
    }
}

/// `RestApiCapability` impl.
///
/// # What is mounted, and what is not
///
/// **`tests/module_test.rs`'s `declared_paths()` is the roster of what is mounted,
/// and this doc deliberately neither counts it nor repeats it.** It used to open
/// "Fifteen routes" and enumerate them; the number was wrong by four the moment G3
/// and G4 mounted the window plane, and the enumeration was wrong in the same edit
/// that fixed a paragraph ten lines below it. A count beside a roster leaves only one
/// of the two true, and it is never the prose. What is worth saying here is the
/// property rather than the list: every mounted route gates on its catalogued
/// `(resource_type, action)` pair before touching a repository, and
/// `tests/rest_authz.rs` drives the whole set to prove it.
///
/// **The design set declares roughly forty surfaces across Slices 2-12** and most are
/// not mounted, because a route whose handler has nothing to call is not a route:
/// there is no overlay, bundle, customer-group, import, migration, bulk or preview
/// table, no audit or history read, and no read-model resolution query.
///
/// **The three window surfaces have left that list**, and the sentence that used
/// to keep them on it is withdrawn rather than edited around. It read that
/// `POST …/prices/{priceId}/windows` and `PATCH`/`DELETE …/price-windows/{windowId}`
/// "still have nothing to call", namely the `WindowService` — which
/// [`crate::infra::window::WindowService`] now is, built a few lines above and held
/// on `GovernanceState`. D-99's requirement is what it implements rather than what
/// blocks it: each mutation requests a pending `CatalogVersion` ref, re-projects the
/// plan subject and answers **202**, so nothing advertises coverage a consumer's
/// pinned read model has not seen.
///
/// **`GET/PUT /bss-pricing/v1/config/approval-threshold-policy` has left that list
/// too**, and the sentence that kept it there is withdrawn rather than edited
/// around. It read that the surface "has no policy store, which is why every
/// publish is material (D-10's fail-safe)" — `pricing_approval_threshold` is that
/// store, and the two routes are mounted above. What the withdrawn sentence got
/// right is worth restating at its real strength: a publish is still material
/// wherever a tenant has no **approved** version, because the fail-safe is a rule
/// about the policy's absence and not about the store's, and configuring one is
/// itself an always-material act (D-10) that a second principal has to sign.
///
/// One absence on the approval plane is still worth naming because it is adjacent
/// to what *is* mounted: `POST /bss-pricing/v1/historical-imports` has no
/// reference-price store.
///
/// The gear reserves its prefix either way, so an unconfigured boot answers 404
/// under `/bss-pricing/v1` rather than colliding with another gear's namespace.
///
/// Two layers wrap the merged routers, exactly as the sibling ledger does: the
/// per-request PEP (the value, not the `Arc` — `PolicyEnforcer: Clone`), which
/// every gated handler extracts, and the canonical-error middleware that renders
/// a `CanonicalError` as its RFC 9457 problem document.
///
/// **D-178's correlation edge is deliberately not a third one here.** It is
/// applied inside each mutating router's own `router()`
/// ([`correlation::establish`](crate::api::rest::correlation::establish)), so it
/// travels with the routes rather than with whoever composes them: the crate's
/// route suites build those routers directly and would otherwise drive a gear
/// whose edge is missing, which is the one configuration that must never be
/// tested as though it were production. The read-only `frontier` router does not
/// carry it, because nothing behind it writes an audit record or an outbox row.
/// A surface added without it cannot build an `AuditStamp` and answers 500 —
/// **to a caller who should have got 403**, because `require_correlation` runs
/// above the authz gate in every handler. That is the cost of the placement, and
/// `tests/rest_authz.rs::every_mutating_router_applies_the_correlation_edge`
/// is what stops a router being written without the edge: it scans the source
/// rather than a maintained list, so a fifth router none of the route suites
/// drive is caught before it is mounted.
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
            .merge(crate::api::rest::plans::router(
                Arc::clone(&rt.authoring_api),
                openapi,
            ))
            .merge(crate::api::rest::prices::router(
                Arc::clone(&rt.authoring_api),
                openapi,
            ))
            .merge(crate::api::rest::bundles::router(
                Arc::clone(&rt.authoring_api),
                openapi,
            ))
            .merge(crate::api::rest::overlays::router(
                Arc::clone(&rt.authoring_api),
                openapi,
            ))
            .merge(crate::api::rest::windows::router(
                Arc::clone(&rt.governance_api),
                openapi,
            ))
            .merge(crate::api::rest::supersessions::router(
                Arc::clone(&rt.governance_api),
                openapi,
            ))
            .merge(crate::api::rest::approvals::router(
                Arc::clone(&rt.governance_api),
                openapi,
            ))
            .merge(crate::api::rest::publish::router(
                Arc::clone(&rt.governance_api),
                openapi,
            ))
            .merge(crate::api::rest::threshold_policy::router(
                Arc::clone(&rt.governance_api),
                openapi,
            ))
            .layer(axum::Extension((*rt.enforcer).clone()))
            .layer(axum::middleware::from_fn(
                toolkit::api::canonical_error_middleware,
            )))
    }
}
