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
//! (**every ticker `serve` spawns** — `crate::infra::jobs` is the roster and this
//! clause deliberately does not repeat it). Declared dependencies are the PEP and
//! the type registry: every ctx-bearing path gates through `access_scope` before
//! touching a repository, and the AuthZ label type-schemas are registered at init.
//!
//! That clause has now been wrong twice by enumerating. It said "and later the
//! window-activation job Slice 7 owns" and later arrived; it was corrected to name
//! the warm re-drive **and** the activation sweep, and the gated-market refresher
//! arrived. A list beside a roster leaves only one of the two true, and it is never
//! the prose.
//!
//! The `stateful` entry drives independent leased tickers, each with its own key and
//! cadence (`crate::infra::jobs` states what each is for). **No ticker waits on
//! another**: the warm re-drive is what turns a publish's pending handle into a
//! version a consumer can pin — without it `pricing_read_model` stays empty whatever
//! else is built; the activation sweep is what makes a scheduled window take effect
//! at its own instant rather than at the next publish; and the gated-market
//! refresher is what keeps §7's backlog gauge from reading zero while markets are
//! gated.

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

use crate::api::rest::audit::ApiState as AuditApiState;
use crate::api::rest::frontier::ApiState as CatalogVersionApiState;
use crate::api::rest::history::ApiState as HistoryApiState;
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

/// The gated-market refresher's lease. Per gear and per pass, for
/// [`WARM_LEASE_KEY`]'s reason: the value is catalog-wide, so one replica reading it
/// is the whole answer and every other replica reading it concurrently is the same
/// scan run N times for one number.
const GATED_MARKETS_LEASE_KEY: &str = "bss-pricing:gated-markets";

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
    /// Slice 12's price-history read (D-270). Its own state rather than a field
    /// on the authoring one, for `frontier`'s reason: it is a read, it carries no
    /// correlation edge and it opens no transaction, so sharing a state with the
    /// mutating surfaces would give it reach it has no use for.
    pub history_api: Arc<HistoryApiState>,
    /// Slice 5's Auditor read over `pricing_audit_log` (`inst-au-read`, Z13-8).
    /// Its own state for [`Self::history_api`]'s reason, and separate from it
    /// because the two surfaces share a permission and not a store.
    pub audit_api: Arc<AuditApiState>,
    /// Per-request state for the plan and price authoring surfaces, built here
    /// for the same reason: one place wires, one place composes.
    pub authoring_api: Arc<AuthoringState>,
    /// Per-request state for the approval surface and the publish mount.
    pub governance_api: Arc<GovernanceState>,
    /// Per-request state for the three membership mutations
    /// (`api::rest::customer_groups::governance_router`).
    ///
    /// Its own state rather than a field on [`GovernanceState`] —
    /// `api::rest::customer_groups`'s section banner states why: the same
    /// `Arc<dyn CatalogVersionRegistryV1>` [`Self::governance_api`] holds, one
    /// more requester of one registry.
    pub membership_api: Arc<crate::api::rest::customer_groups::MembershipState>,
    /// The alarm and metric plane, carried so the **background** work reports on
    /// the same port the request paths do (D-238).
    ///
    /// Stored on the runtime rather than rebuilt in `serve`: the adapter caches
    /// its instruments and holds the observable gauge's registration, so a second
    /// build would be a second set of instruments and — for the gauge — a second
    /// callback observing a different cell.
    pub metrics: Arc<dyn crate::domain::ports::metrics::PricingMetricsPort>,
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
    /// Every ticker is spawned below and all of them are joined —
    /// `crate::infra::jobs` says what each is for and why none waits on another.
    ///
    /// **Two of them are load-bearing for correctness and one is not**, which is a
    /// distinction worth stating rather than a count: the publish commit leaves a
    /// **pending** `CatalogVersion` handle and no version, so without the warm
    /// re-drive nothing ever resolves it, `pricing_read_model` stays empty and no
    /// version becomes pin-eligible (§4.4's "the re-drive continues past the SLO with
    /// no bound" is that loop coming round again); and without the activation sweep a
    /// scheduled window takes effect at no instant at all. The gated-market refresher
    /// costs observability rather than correctness — [`Self::spawn_gated_markets_ticker`]
    /// says what exactly — and this paragraph said *"Neither is optional"* over three
    /// tickers while that function said the opposite about one of them.
    ///
    /// An unconfigured gear still parks on the token: a gear compiled in but
    /// absent from `gears:` has no runtime and therefore no work.
    ///
    /// # Errors
    /// When a ticker resolves before cancellation — which its loop shape does
    /// only by panicking, since every tick failure is caught and logged inside
    /// it.
    ///
    /// This said *"never returns `Err` today … a spawned ticker's join error
    /// will surface through it"* until 2026-08-11, and the second half was the
    /// claim that made the first read as a statement about the present. It was
    /// not: the handles were awaited only after shutdown, so a join error
    /// surfaced as a `warn` whenever the process finally stopped. A premise
    /// stated in prose outlives the code that made it true.
    pub(crate) async fn serve(self: Arc<Self>, cancel: CancellationToken) -> Result<()> {
        let Some(rt) = self.runtime.load_full() else {
            cancel.cancelled().await;
            return Ok(());
        };

        // **One field per ticker spawned below**, and the third was missing for two
        // days: a deployment could not see the gated-market cadence it was running,
        // which is the one instance of this drift that cannot be read as prose going
        // stale. The sibling ledger's `module.rs` logs all nine of its own.
        info!(
            warm_tick_secs = rt.config.jobs.readmodel_warm_tick_secs,
            window_activation_tick_secs = rt.config.jobs.window_activation_tick_secs,
            gated_markets_tick_secs = rt.config.jobs.gated_markets_tick_secs,
            "bss-pricing: lifecycle started"
        );
        let tasks = cancel.child_token();
        let mut warm = Self::spawn_warm_ticker(Arc::clone(&rt), tasks.clone());
        let mut activation = Self::spawn_activation_ticker(Arc::clone(&rt), tasks.clone());
        let mut gated = Self::spawn_gated_markets_ticker(Arc::clone(&rt), tasks.clone());

        // **`select!` on the handles, not just on the token.** A ticker that
        // panicked was previously awaited only at shutdown: `serve` sat on
        // `cancel.cancelled()`, the handle resolved to `Err` in the background,
        // and `join_ticker` demoted it to a `warn` emitted whenever the process
        // finally stopped — possibly days later. So a panic on tick 1 left the
        // gear serving traffic, answering 200s, with `serve` still `Ok(())`.
        //
        // That is not a cosmetic gap here: the warm re-drive is what resolves a
        // pending `CatalogVersion` handle, so a dead warm ticker means
        // `pricing_read_model` stays empty and no version ever becomes
        // pin-eligible — and the two Criticals `readmodel_warm` raises cannot
        // fire, because the task that raises them is the dead one.
        //
        // `bss-ledger`'s shape, which this module's own doc already claimed to
        // copy: each arm cancels the shared token, drains the survivors, and maps
        // a join error onto the return. The three properties of the *tickers*
        // were inherited when this was written; the supervision around them was
        // not.
        let outcome = tokio::select! {
            () = cancel.cancelled() => {
                tasks.cancel();
                Self::stop(warm, activation, gated).await;
                Ok(())
            }
            res = &mut warm => {
                tasks.cancel();
                Self::exited_first("readmodel-warm", res, activation, gated).await
            }
            res = &mut activation => {
                tasks.cancel();
                Self::exited_first("window-activation", res, warm, gated).await
            }
            res = &mut gated => {
                tasks.cancel();
                Self::exited_first("gated-markets", res, warm, activation).await
            }
        };
        outcome
    }

    /// One ticker resolved before cancellation: drain the other two and report.
    ///
    /// A ticker's loop runs until the token is cancelled and catches every tick
    /// failure itself, so resolving early means a **panic** — something the job's
    /// own error handling did not model. It is mapped onto `serve`'s return
    /// rather than logged, because the caller is the lifecycle and a gear whose
    /// background plane is dead should not keep reporting healthy.
    ///
    /// The survivors are still drained first, for [`Self::stop`]'s reason: each
    /// holds a coordination lease, and abandoning a handle would leave a task
    /// writing to a database the process is closing.
    async fn exited_first(
        ticker: &'static str,
        res: std::result::Result<(), tokio::task::JoinError>,
        first: tokio::task::JoinHandle<()>,
        second: tokio::task::JoinHandle<()>,
    ) -> Result<()> {
        let survivors = tokio::join!(first, second);
        let failure = res.err().map(|e| (ticker, e));
        let failure = failure
            .or_else(|| survivors.0.err().map(|e| ("a sibling ticker", e)))
            .or_else(|| survivors.1.err().map(|e| ("a sibling ticker", e)));
        if let Some((name, e)) = failure {
            tracing::error!(error = %e, ticker = name, "bss-pricing: a ticker died");
            Err(anyhow::anyhow!("bss-pricing: {name} ticker: {e}"))
        } else {
            // No panic: the ticker's loop returned on its own, which its shape
            // does not do while the token stands. Reported as an error rather
            // than treated as a clean stop, because a background plane that
            // quietly stopped is the state this whole arm exists to make visible.
            tracing::error!(ticker, "bss-pricing: a ticker stopped before cancellation");
            Err(anyhow::anyhow!(
                "bss-pricing: {ticker} ticker stopped before cancellation"
            ))
        }
    }

    /// Release one sweep's lease, warning rather than failing.
    ///
    /// One function for all three passes. `LeaseGuard` has **no `Drop` impl** —
    /// releasing is async DB I/O — so a guard that is merely dropped leaves the
    /// row standing until its TTL, and every TTL here *is* the tick. That is what
    /// halved the gated-market gauge's cadence: the next tick found the slot held
    /// by its own previous pass and logged "a peer holds its lease", naming a peer
    /// where the holder was this same task.
    ///
    /// A failed release costs one skipped tick rather than a stuck sweep, because
    /// the TTL frees the slot either way.
    async fn release_lease(guard: coord::LeaseGuard, sweep: &'static str) {
        if let Err(e) = guard.release().await {
            tracing::warn!(error = %e, sweep, "bss-pricing: could not release a sweep lease");
        }
    }

    /// Wind the three tickers down and say so — the **cancellation** path.
    ///
    /// A join error here is a panic that raced the shutdown, and it stays a
    /// `warn`: the process is stopping either way and there is no caller left to
    /// tell. A panic *before* cancellation is a different event and takes
    /// [`Self::exited_first`], which fails `serve`.
    ///
    /// **All of them are joined, and none is skipped when another faults.** They
    /// are cancelled by one token and each holds a coordination lease whose slot
    /// frees itself at the TTL, so a shutdown that abandoned a later handle
    /// would leave a task writing to a database the process is closing — the one
    /// state a `stop_timeout` cannot help with.
    async fn stop(
        warm: tokio::task::JoinHandle<()>,
        activation: tokio::task::JoinHandle<()>,
        gated: tokio::task::JoinHandle<()>,
    ) {
        Self::join_ticker(warm, "readmodel-warm").await;
        Self::join_ticker(activation, "window-activation").await;
        Self::join_ticker(gated, "gated-markets").await;
        info!("bss-pricing: lifecycle cancelled");
    }

    /// Await one ticker's handle, reporting a panic rather than swallowing it.
    ///
    /// One function for every ticker, and the `ticker` field is what tells them
    /// apart: a copy of this `if let` per handle is a place the warning could be
    /// dropped from, and the last handle is exactly the one a shutdown is tempted to
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
            let job = Self::warm_job(
                rt.db.clone(),
                Arc::clone(&rt.catalog_version_registry),
                rt.config.jobs.clone(),
                Arc::clone(&rt.metrics),
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

    /// The warm job as the lifecycle builds it — **the metrics attachment
    /// included**.
    ///
    /// A named function rather than four lines inside `tokio::spawn`, and it is the
    /// attachment that earns it. `ReadModelWarmJob::new` installs
    /// `NoopPricingMetrics` and `with_metrics` is a separate call, so a lifecycle
    /// that forgot it would build a job whose two **Critical** alarms — this gear's
    /// only two — report to nothing, while every job-level suite (which builds its
    /// own job) stayed green. Nothing inside `spawn` can be reached by a test: the
    /// closure needs a whole [`PricingRuntime`], and a runtime needs a PEP, a
    /// registry and eight API states.
    ///
    /// So the wiring decision moved to somewhere a case can call, and
    /// `module_tests::the_warm_job_the_lifecycle_builds_reports_on_the_metrics_port_it_is_handed`
    /// drives a pass through it against a real `SdkMeterProvider`. What is left
    /// unproved is that `spawn_warm_ticker` calls **this** function, which is one
    /// line holding no decision.
    ///
    /// [`Self::gated_markets_pass`]' job needs no equivalent: `GatedMarketsJob::new`
    /// takes the port as a parameter, so its attachment is a compile error to omit
    /// — the shape this one cannot have while `with_metrics` is the seam three other
    /// services share.
    fn warm_job(
        db: DBProvider<DbError>,
        registry: Arc<dyn CatalogVersionRegistryV1>,
        jobs: crate::config::JobsConfig,
        metrics: Arc<dyn crate::domain::ports::metrics::PricingMetricsPort>,
    ) -> ReadModelWarmJob {
        ReadModelWarmJob::new(db, registry, jobs).with_metrics(metrics)
    }

    /// The activation job as the lifecycle builds it, for [`Self::warm_job`]'s
    /// reason: `with_metrics` carries §7's only window-plane alarm and is a
    /// separate call that a rebuild of this ticker could drop.
    fn activation_job(
        db: DBProvider<DbError>,
        jobs: crate::config::JobsConfig,
        metrics: Arc<dyn crate::domain::ports::metrics::PricingMetricsPort>,
    ) -> WindowActivationJob {
        WindowActivationJob::new(db, jobs).with_metrics(metrics)
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
        match job.run(Utc::now()).await {
            Ok(report) => Self::log_sweep(&report),
            Err(e) => {
                tracing::error!(error = %e, "bss-pricing: read-model warm sweep tick failed");
            }
        }
        Self::release_lease(guard, "readmodel-warm").await;
    }

    /// Report a warm pass that did something, and stay silent about one that did
    /// not — [`Self::log_activation`]'s rule, on the sweep that had none (Z13-7).
    ///
    /// `SweepReport` carries eleven counters and this was the only production caller
    /// of `run`: it matched `Err` and dropped the `Ok` whole. So the pass `serve`'s
    /// own doc calls the one *"without which `pricing_read_model` stays empty"*
    /// produced no per-pass operational signal at all, while its less load-bearing
    /// sibling forty lines below emitted one. The asymmetry was the finding.
    ///
    /// What is *not* lost without this — and why it is `info!` rather than something
    /// louder — is the two Criticals and every failed subject: those reach the
    /// metrics port and `tracing::error!` from inside the job and the projector,
    /// independently of this. What was lost is the **aggregate**, including
    /// `degraded_emitted` and `versions_complete`, whose only other channel is a
    /// `debug!`.
    fn log_sweep(report: &crate::infra::jobs::readmodel_warm::SweepReport) {
        if !Self::sweep_is_noteworthy(report) {
            return;
        }
        info!(
            tenants_seen = report.tenants_seen,
            pending_seen = report.pending_seen,
            versions_projected = report.versions_projected,
            versions_complete = report.versions_complete,
            subjects_projected = report.subjects_projected,
            subjects_failed = report.subjects_failed,
            frontiers_advanced = report.frontiers_advanced,
            degraded_emitted = report.degraded_emitted,
            commit_overdue = report.commit_overdue,
            pin_eligibility_overdue = report.pin_eligibility_overdue,
            frontier_scan_failed = report.frontier_scan_failed,
            "bss-pricing: read-model warm sweep pass"
        );
    }

    /// Has this pass anything to tell an operator?
    ///
    /// The **decision**, split from the emission so it can be asserted: this crate
    /// has no tracing capture, so a `log_sweep` that made the choice inline would be
    /// a rule nothing could redden. [`Self::log_activation`] carries the same rule
    /// inline and has the same gap.
    ///
    /// Two states are deliberately *not* noteworthy, and both are steady states
    /// rather than events. A pass that swept tenants and moved nothing is what every
    /// tick inside D-47's batching budget looks like — at a five-second cadence,
    /// logging it would bury the passes that did something under roughly twelve
    /// passes a minute that did not. And `inert` is a **deployment state**: with no
    /// registry wired every pass is inert forever, and `readmodel_warm`'s module doc
    /// puts that at `debug` precisely because the e2e that boots this gear without a
    /// registry has to stay readable.
    ///
    /// **Destructured exhaustively on purpose.** A twelfth counter added to
    /// `SweepReport` is then a compile error here rather than a field this rule
    /// silently ignores — which is the one way a report grows a signal that never
    /// reaches an operator.
    fn sweep_is_noteworthy(report: &crate::infra::jobs::readmodel_warm::SweepReport) -> bool {
        let crate::infra::jobs::readmodel_warm::SweepReport {
            // A deployment state, not an event — see above.
            inert: _,
            // Seeing a tenant or a ref is not doing anything with it.
            tenants_seen: _,
            pending_seen: _,
            versions_projected,
            // Implied by `versions_projected`: nothing is judged complete or
            // projected without a version having been handed to the projector.
            versions_complete: _,
            subjects_projected: _,
            subjects_failed,
            frontiers_advanced,
            degraded_emitted,
            commit_overdue,
            pin_eligibility_overdue,
            frontier_scan_failed,
        } = report;
        *versions_projected > 0
            || *subjects_failed > 0
            || *frontiers_advanced > 0
            || *degraded_emitted > 0
            || *commit_overdue > 0
            || *pin_eligibility_overdue > 0
            || *frontier_scan_failed
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
            let job = Self::activation_job(
                rt.db.clone(),
                rt.config.jobs.clone(),
                Arc::clone(&rt.metrics),
            );
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

    /// The gated-market gauge's refresher (D-246's missing half, on D-250's tick).
    ///
    /// **The third ticker, and unlike the other two it is not load-bearing for
    /// correctness** — the gear serves every request without it. What it costs to
    /// omit is observability: `pricing_tax_not_sellable_ga` is an observable gauge
    /// over a cached value, and with nothing refreshing the cache the exporter
    /// reports `0` forever while markets are gated, which is §7's alarm silently
    /// never firing. That is why it is spawned here rather than left to a caller.
    fn spawn_gated_markets_ticker(
        rt: Arc<PricingRuntime>,
        token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(rt.config.jobs.gated_markets_interval());
            iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let lease = coord::LeaseManager::new(rt.db.db());
            let job = crate::infra::jobs::gated_markets::GatedMarketsJob::new(
                rt.db.clone(),
                Arc::clone(&rt.metrics),
                rt.config.jobs.clone(),
            );
            loop {
                tokio::select! {
                    biased;
                    () = token.cancelled() => break,
                    _ = iv.tick() => {
                        Self::gated_markets_pass(
                            &lease,
                            &job,
                            rt.config.jobs.gated_markets_interval(),
                        )
                        .await;
                    }
                }
            }
        })
    }

    /// One leased refresh. Extracted so the ticker stays a ticker.
    ///
    /// A failed pass is logged and nothing is published — `GatedMarketsJob::run_once`
    /// documents why that is the safe direction, and the next tick tries again.
    async fn gated_markets_pass(
        lease: &coord::LeaseManager,
        job: &crate::infra::jobs::gated_markets::GatedMarketsJob,
        ttl: std::time::Duration,
    ) {
        let Some(guard) = Self::take_lease(lease, GATED_MARKETS_LEASE_KEY, ttl).await else {
            return;
        };
        match job.run_once().await {
            Ok(report) => {
                tracing::debug!(
                    gated_markets = report.gated_markets,
                    "bss-pricing: gated-market gauge refreshed"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "bss-pricing: gated-market refresh failed; the gauge keeps its previous value"
                );
            }
        }
        // **Released, not dropped**, and this was the one outlier among three
        // look-alike passes: it called `drop(guard)`. With no `Drop` impl to run
        // the async release, the row stood until its TTL — and the TTL here *is*
        // the tick (60s). The slot was claimed at `T+δ`, so the next tick at
        // `T+60` found `locked_until` still ahead of it, took the `LeaseHeld` arm,
        // and logged "a peer holds its lease" at debug, naming a peer where the
        // holder was this same task's previous pass. The gauge refreshed every
        // **other** tick, at ~120s, against the 60s D-250 ratifies and this
        // module's own "the value is up to one tick old".
        Self::release_lease(guard, "gated-markets").await;
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
        Self::release_lease(guard, "window-activation").await;
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
    /// One function for every pass because the acquire is one rule — the key is
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
        let history_api = Arc::new(HistoryApiState {
            history: crate::infra::history::HistoryExporter::new(db.clone()),
        });

        // Slice 5's Auditor read. A reader with a provider of its own is safe in a
        // way the audit *writer* is not: `audit_repo`'s CONTRACT forbids the writer
        // a transaction of its own because the record must commit inside the
        // mutation's, and a read has no mutation to be inside of.
        let audit_api = Arc::new(AuditApiState {
            audit: crate::infra::audit_read::AuditReader::new(db.clone()),
        });

        let catalog_version_api = Arc::new(CatalogVersionApiState {
            pin_frontier: PinFrontierRepo::new(db.clone()),
        });

        // The authoring surface's state. The repositories are cheap clones over
        // the provider; `db` is carried because `infra::idempotent` opens the
        // one transaction the at-most-once contract requires, and the gate holds
        // the configured retention window because expiry is decided on the claim
        // path rather than by a reaper.
        // **One `ApprovalService`, two states.** The authoring surface's overlay
        // submit opens a unit (D-50) and the governance surface decides it; two
        // services over one provider would be two transaction owners for one table.
        let approvals = ApprovalService::new(db.clone());
        // The OTel-backed metrics port. Built once per process and shared: the
        // adapter caches its instruments, and a second build would look them up
        // again on a path that is only reporting.
        //
        // A **no-op until the host installs a meter provider**, so this is safe
        // to construct unconditionally — a missing exporter can never be the
        // reason a publish fails.
        //
        // **Built here rather than beside the publish engine**, which is where it
        // stood: the bundle service takes it too (`T-17` cases (ii)/(iii)) and is
        // constructed with the authoring state above that point. One binding, two
        // consumers, and the ordering is the only reason it moved.
        let metrics: Arc<dyn crate::domain::ports::metrics::PricingMetricsPort> =
            Arc::new(crate::infra::metrics::PricingMetricsMeter::new());
        let authoring_api = Arc::new(AuthoringState {
            db: db.clone(),
            plans: PlanRepo::new(db.clone()),
            shapes: PlanShapeRepo::new(db.clone()),
            prices: PriceRepo::new(db.clone()),
            // Slice 8's two: the composition store, and the seam that assembles
            // a composition for the pure rules to judge.
            bundles: BundleRepo::new(db.clone()),
            bundle_service: crate::infra::bundle::BundleService::new(db.clone())
                .with_metrics(Arc::clone(&metrics)),
            // Slice 9's overlay store. Here and not on `GovernanceState`
            // because it requests no `CatalogVersion`, which is the criterion
            // that split the two.
            overlays: crate::infra::storage::repo::OverlayRepo::new(db.clone()),
            approvals: approvals.clone(),
            // Slice 4's taxonomy store — the writer the four scope-value
            // universes had never had, and without which a brand-scoped overlay
            // could not be authored end to end.
            taxonomies: crate::infra::storage::repo::taxonomy_repo::TaxonomyRepo::new(db.clone()),
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
        )
        .with_metrics(Arc::clone(&metrics));

        // The governance surface's state, and the publish engine's **only**
        // holder. It sat on `PricingRuntime` behind a `dead_code` allow for two
        // phases because nothing could reach `commit`; the route that reaches it
        // is mounted here, so the engine moves to the state that serves it
        // rather than staying somewhere with an allow and a second reader.
        let governance_api = Arc::new(GovernanceState {
            db: db.clone(),
            plans: PlanRepo::new(db.clone()),
            prices: PriceRepo::new(db.clone()),
            approvals,
            publish,
            overlays: crate::infra::storage::repo::OverlayRepo::new(db.clone()),
            // The **fifth** requester of the one registry `Arc` (D-234). Five
            // requesters is still one incrementer, and the argument is
            // `api::rest::state`'s.
            overlay_publish: crate::infra::overlay_publish::OverlayPublishService::new(
                db.clone(),
                Arc::clone(&catalog_version_registry),
            ),
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
            // The **fourth**, on the same argument (D-100).
            cutovers: crate::infra::cutover::CutoverService::new(
                db.clone(),
                Arc::clone(&catalog_version_registry),
            ),
            // The **sixth** (D-128): retirement is a publish unit, so it requests
            // a version too. The argument has not had to change once.
            retirements: crate::infra::retirement::RetirementService::new(
                db.clone(),
                Arc::clone(&catalog_version_registry),
            ),
            // **Not** a seventh requester of the registry `Arc`, and the exception
            // is worth stating: a migration schedule changes no plan content, so
            // no version is requested and no subject is re-projected. What it
            // needs instead is the tenant policy reader, for D-49's notice period.
            migrations: crate::infra::migration::MigrationService::new(db.clone(), &config.limits),
            // Neither a registry requester nor a policy reader: synthesis freezes
            // a payload nothing can look up, which is the whole of D-87.
            synthesis: crate::infra::synthesis::SynthesisService::new(db.clone()),
            // The window `POST`'s at-most-once gate (D-191), under the **same** TTL the
            // authoring plane's claims expire on: the expiry is a deployment knob about
            // how long a client key is honoured, and two windows for it would mean one
            // caller's retry is protected on one surface and not on another.
            idempotency: IdempotencyGate::new(config.limits.idempotency_key_ttl()),
            thresholds: crate::infra::threshold::ThresholdService::new(db.clone()),
            metrics: Arc::clone(&metrics),
        });

        // Slice 9's membership mutations (`dod-customer-group`'s MUST): the
        // **seventh** requester of the one registry `Arc`. Built here rather
        // than folded into `governance_api` above — see
        // `api::rest::customer_groups`'s section banner for why a route that
        // requests a `CatalogVersion` does not have to widen the crate-wide
        // governance state to reach one.
        let membership_api = Arc::new(crate::api::rest::customer_groups::MembershipState {
            db: db.clone(),
            idempotency: IdempotencyGate::new(config.limits.idempotency_key_ttl()),
            registry: Arc::clone(&catalog_version_registry),
        });

        self.runtime.store(Some(Arc::new(PricingRuntime {
            db,
            config,
            enforcer,
            catalog_version_registry,
            catalog_version_api,
            history_api,
            audit_api,
            authoring_api,
            governance_api,
            membership_api,
            metrics,
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
/// **The design set declares roughly forty surfaces across Slices 2-12**, and which
/// of them are mounted is the question `declared_paths()` answers and this doc does
/// not. The criterion is what is worth keeping here: a route whose handler has
/// nothing to call is not a route, so a declared surface stays absent until its
/// engine exists.
///
/// **The enumeration that used to follow is withdrawn whole** (2026-08-13), and
/// withdrawn rather than corrected, because it is exactly the shape the paragraph
/// above forbids. It read: *"there is no overlay, bundle, customer-group, import,
/// migration, bulk or preview table, no audit or history read, and no read-model
/// resolution query."* **Seven of its clauses were false.** `pricing_price_overlay`,
/// `pricing_bundle`, `pricing_customer_group_taxonomy`, `pricing_migration` and
/// `pricing_bulk_operation` all exist under `infra::storage::entity`, and every one
/// of them — plus `history::router`, and the customer-group taxonomy and
/// membership pair — has its router merged **in this same function**; and
/// `read_model_repo::delta_at`, a read-model resolution query by its own error
/// string, is reached from the mounted preview and sellability reads.
///
/// A list beside a roster leaves only one of the two true, and it is never the
/// prose. The 2026-08-10 review counted five false clauses; mounting
/// `customer_groups::router` made six without anybody touching the words; and the
/// read-model clause is a seventh that review did not count.
///
/// **Two absences survive it**, and are named because each is adjacent to something
/// that *is* mounted — the case where a reader would otherwise reasonably conclude
/// the feature is wired. `POST /bss-pricing/v1/historical-imports` has no
/// reference-price store. And **`audit × export` is gated by no route**: S5 §5's
/// audit surface carries two permissions, `GET /bss-pricing/v1/audit` serves the
/// `read` half (mounted below, Z13-8), and the export is a chunked shape under its
/// own p95 that nothing here builds — a large `limit` on the page is not it.
///
/// **The audit *read* left this list on 2026-08-14** and the clause is withdrawn
/// rather than edited around, since half of it was a correct warning against the
/// other half's inference. It read that `pricing_audit_log` had "**no reader on any
/// mounted surface**: `GET /history` gates on `audit × read`, but it serves plan and
/// price data and takes its actor from `pricing_price.created_by` and never from the
/// audit log". The second half is still exactly true and is stated where it belongs
/// (see [`crate::api::rest::history`] and [`crate::api::rest::audit`]); the first is
/// what [`crate::api::rest::audit`] discharges, and it had a dependant —
/// `infra::error_mapping`'s three 403 arms drop their detail on the ground that the
/// attempt is recoverable from this trail.
///
/// **The three window surfaces had left that sentence earlier**, and the clause that
/// used to keep them on it is withdrawn rather than edited around. It read that
/// `POST …/prices/{priceId}/windows` and `PATCH`/`DELETE …/price-windows/{windowId}`
/// "still have nothing to call", namely the `WindowService` — which
/// [`crate::infra::window::WindowService`] now is, built a few lines above and held
/// on `GovernanceState`. D-99's requirement is what it implements rather than what
/// blocks it: each mutation requests a pending `CatalogVersion` ref, re-projects the
/// plan subject and answers **202**, so nothing advertises coverage a consumer's
/// pinned read model has not seen.
///
/// **`GET/PUT /bss-pricing/v1/config/approval-threshold-policy` had left it too**,
/// and the clause that kept it there is withdrawn rather than edited
/// around. It read that the surface "has no policy store, which is why every
/// publish is material (D-10's fail-safe)" — `pricing_approval_threshold` is that
/// store, and the two routes are mounted above. What the withdrawn sentence got
/// right is worth restating at its real strength: a publish is still material
/// wherever a tenant has no **approved** version, because the fail-safe is a rule
/// about the policy's absence and not about the store's, and configuring one is
/// itself an always-material act (D-10) that a second principal has to sign.
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
            // Slice 12's first reachable surface. A read, so it is merged like
            // the frontier rather than like the authoring routers: no
            // correlation edge, nothing behind it writing an audit record.
            .merge(crate::api::rest::history::router(
                Arc::clone(&rt.history_api),
                openapi,
            ))
            // Slice 5's Auditor read, merged like the two reads above it and for
            // the same reason: it writes no audit record of its own, so it needs no
            // correlation edge.
            .merge(crate::api::rest::audit::router(
                Arc::clone(&rt.audit_api),
                openapi,
            ))
            // Slice 12's bulk import. Mounted with the authoring routers because
            // two of its three surfaces write, and it takes the authoring state
            // whole rather than a state of its own: Phase 2's engine wants the
            // provider and the price repository, and the run's own statements are
            // free functions taking a runner.
            .merge(crate::api::rest::bulk_imports::router(
                Arc::new(crate::api::rest::bulk_imports::ApiState {
                    authoring: Arc::clone(&rt.authoring_api),
                }),
                openapi,
            ))
            // Slice 12's mass repricing, mounted beside its bulk sibling and on
            // the same state for the same reason: the selector's expansion is a
            // free function over a runner, and so are the run's own statements.
            // The `POST` writes, so this router carries D-178's correlation edge
            // of its own.
            .merge(crate::api::rest::repricing_runs::router(
                Arc::new(crate::api::rest::repricing_runs::ApiState {
                    authoring: Arc::clone(&rt.authoring_api),
                    // The same `Arc` `governance_api`'s own services request
                    // through — one requester, `api::rest::state`'s argument,
                    // not a second incrementer.
                    registry: Arc::clone(&rt.catalog_version_registry),
                    // A fresh reader over the same provider and the same
                    // ratified defaults `PublishService::new` builds its own
                    // from — this state carries no `PublishService` to borrow
                    // one off, and the reader itself is a cheap clone of a
                    // `DBProvider` plus a deployment constant, not a second
                    // source of truth for the tenant's policy row.
                    policies: crate::infra::storage::repo::PolicyObjectRepo::new(
                        rt.db.clone(),
                        &rt.config.limits,
                    ),
                }),
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
            // The submit route is mounted apart from its siblings and on the
            // **governance** state, because it publishes (D-234). See
            // `api::rest::state::GovernanceState::overlay_publish`.
            .merge(crate::api::rest::overlays::governance_router(
                Arc::clone(&rt.governance_api),
                openapi,
            ))
            .merge(crate::api::rest::taxonomies::router(
                Arc::clone(&rt.authoring_api),
                openapi,
            ))
            // Slice 9's own taxonomy (`inst-cg-taxonomy`), on its own route and
            // its own `customer_group` gate — not a fifth arm of the `taxonomies`
            // router above, and not filed under `config`. See
            // `api::rest::customer_groups`'s module doc.
            .merge(crate::api::rest::customer_groups::router(
                Arc::clone(&rt.authoring_api),
                openapi,
            ))
            // The three membership mutations (`dod-customer-group`'s MUST).
            // Mounted apart from the taxonomy pair above because every one of
            // them requests a `CatalogVersion` — `api::rest::customer_groups`'s
            // section banner is the criterion, `overlays::governance_router`'s
            // own split one plane over.
            .merge(crate::api::rest::customer_groups::governance_router(
                Arc::clone(&rt.membership_api),
                openapi,
            ))
            .merge(crate::api::rest::tax_display_policy::router(
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
            .merge(crate::api::rest::cutovers::router(
                Arc::clone(&rt.governance_api),
                openapi,
            ))
            .merge(crate::api::rest::retirement::router(
                Arc::clone(&rt.governance_api),
                openapi,
            ))
            .merge(crate::api::rest::migrations::router(
                Arc::clone(&rt.governance_api),
                openapi,
            ))
            .merge(crate::api::rest::migrated_origin_snapshots::router(
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
            .merge(crate::api::rest::preview::router(
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

#[cfg(all(test, feature = "test-support"))]
#[path = "module_tests.rs"]
mod module_tests;
