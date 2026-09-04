//! `BssProductsGear` — the toolkit gear declaration.
//!
//! One deployable gear over one `toolkit-db` backend, now declaring both the
//! `db` and the `rest` capabilities. The Foundation tables and their guards
//! are migrations, and they are what everything else in the gear is built on;
//! the `rest` capability claims the gear's service prefix so the runtime
//! knows, from boot, that `/bss-products/v1` belongs to this gear and to no
//! other.
//!
//! **This slice (Phase 4, Slice C) mounts the read door**:
//! `GET /bss-products/v1/products/{id}` and `GET /bss-products/v1/skus/{id}`
//! (`crate::api::rest::products`, `crate::api::rest::skus`). It is deliberately
//! the first door in the phase: an author who has not just written a row has
//! no `ETag` to send back as `If-Match`, so every mutating door this phase
//! still owes — the create doors first, then save/publish/discard — depends
//! on this one existing already. Before this slice the router this gear
//! mounted was empty; a gear that declared `rest` but mounted nothing at all
//! would leave its prefix unclaimed, so an unconfigured boot would either
//! fall through to another gear's router or answer whatever the merge order
//! happens to produce, which is why the runtime-absent branch below still
//! nests an empty router rather than erroring — an unconfigured boot still
//! answers `404` from this gear, not a fall-through.
//!
//! The `authz_resolver` dependency and the `PolicyEnforcer` it builds in
//! [`Gear::init`] were wired ahead of the routes that would gate through
//! them, for the same reason the sibling pricing gear wires its own PEP at
//! init: authorization is security-critical, so a missing
//! `AuthZResolverClient` must fail the boot rather than be discovered the
//! first time a handler reaches for an enforcer that was never built. This
//! slice is the first to read it, cloning it once per boot into its own
//! `Extension` layer in `RestApiCapability::register_rest`.
//!
//! **The transactional outbox is wired the same way, and deliberately not the
//! way the sibling gears wired theirs.** `DECISIONS.md` P-D-22 struck
//! `products_outbox` as a gear-authored table: pricing's own `pricing_outbox`
//! is a private re-invention of a platform facility, measured against
//! mini-chat, the one gear that imports `toolkit_db::outbox::Outbox` directly.
//! This gear follows mini-chat's donor shape, not pricing's. [`Gear::init`]
//! builds the pipeline with the outbox's own table prefix
//! ([`OUTBOX_TABLE_PREFIX`]) and stores the running handle in
//! [`ProductsRuntime`] beside the enforcer; `DatabaseCapability::migrations`
//! appends the facility's own migrations
//! (`toolkit_db::outbox::outbox_migrations_with_prefix`) rather than declaring
//! any outbox table in this gear's own migration chain. [`Gear::init`]
//! registers **one** queue — `infra::events::QUEUE_NAME`, over
//! `infra::events::PARTITIONS` — and it must, because `enqueue` refuses an
//! unregistered queue with `OutboxError::QueueNotRegistered` and every create
//! door enqueues inside its own transaction.
//!
//! **Its processor is decided by a fork.** With an `EventBrokerApi` in the
//! `ClientHub`, the queue is registered by the broker SDK's own
//! `ProducerOutboxQueue` and its processor publishes (**P-D-47**); without one,
//! `init` registers it in `leased` mode with `infra::events::PendingBrokerProducer`,
//! which holds every message so rows accumulate undelivered rather than being
//! discarded. The queue **name** is `QUEUE_NAME` on both arms deliberately —
//! an earlier revision gave the producer arm the table prefix instead, which
//! stranded every row an interim boot had accumulated. `ProductsConfig::require_broker`
//! turns the second arm into a boot failure for a deployment that must publish.
//!
//! An earlier revision of this paragraph said no queue was registered yet and
//! named `.transactional(handler)` as the call a later slice would add. Both
//! halves were false by the time the file below was written: the queue is
//! registered, and in `leased` mode.
//!
//! @cpt-cf-bss-products-component-registry-foundation

use std::sync::Arc;

use anyhow::Context;
use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use axum::Router;
use toolkit::api::OpenApiRegistry;
use toolkit::contracts::RestApiCapability;
use toolkit::{Gear, GearCtx};

use crate::config::ProductsConfig;

/// Table-family prefix for this gear's `toolkit_db::outbox` instance
/// (P-D-22: "its tables ... carry a configurable prefix"). Names the gear
/// rather than reusing the facility's own default (`toolkit_outbox`), so this
/// gear's `_body`/`_partitions`/`_incoming`/`_outgoing`/`_dead_letters` tables
/// are identifiable on sight in a database another gear's default-prefixed
/// outbox might also live in, and never collide with it.
///
/// MUST match between this constant's use in [`Gear::init`]
/// (`Outbox::builder(..).table_prefix(..)`) and
/// `DatabaseCapability::migrations`'s `outbox_migrations_with_prefix(..)`
/// call below — a mismatch would point the running pipeline at tables this
/// chain never created.
use crate::infra::events::OUTBOX_TABLE_PREFIX;

/// Per-process state built by [`Gear::init`] and read by
/// [`RestApiCapability::register_rest`].
///
/// Carries only the platform PEP for now. Phase 4 adds the per-request state
/// each authoring surface needs, following the sibling pricing and ledger
/// gears' shape: one field per surface, built once here rather than wired
/// inside `register_rest`.
pub(crate) struct ProductsRuntime {
    /// Platform PEP, built in `init()` from the `authz-resolver` `ClientHub`
    /// client. `Arc`-held so a future per-request `Extension` clones the
    /// value, not the enforcer, exactly as the sibling pricing gear does.
    ///
    /// Read from `register_rest` since this slice's read door: `(*rt.enforcer)
    /// .clone()` is layered onto the merged router as its own `Extension`, the
    /// same way the sibling ledger gear's per-request PEP is wired.
    pub enforcer: Arc<authz_resolver_sdk::PolicyEnforcer>,

    /// The transactional-outbox pipeline (P-D-22), built in `init()` from
    /// `toolkit_db::outbox::Outbox::builder`. Held as the full
    /// [`toolkit_db::outbox::OutboxHandle`], not just the inner `Arc<Outbox>`
    /// it wraps: dropping the handle drops its `TaskSet`, which cancels the
    /// pipeline's background tasks (sequencer, processors, vacuum) on drop —
    /// so the handle must outlive the process, not just the field access.
    ///
    /// Read by [`RestApiCapability::register_rest`], which clones it onto
    /// `api::rest::ApiState` so a door can enqueue inside its own transaction.
    pub sink: crate::infra::broker::EventSink,

    /// The configured freeze timeout (P-D-84), read by the coalescer's
    /// overdue scan.
    pub freeze_timeout_hours: u32,

    /// `inst-bm-limits`' two operands, carried for the import door and the
    /// batch worker's claim-time re-check.
    pub bulk_max_rows_per_batch: u32,
    /// The tenant's concurrent-batch ceiling.
    pub bulk_max_concurrent_batches_per_tenant: u32,

    /// The watermark door's skew bound and the predicate's freshness
    /// threshold, resolved once from `ProductsConfig` (P-D-87 arm 1).
    pub watermark_skew_tolerance: std::time::Duration,

    /// The `ApiState` the in-process SDK bindings and the batch worker
    /// share — built once at `init` so the lifecycle's own passes reach
    /// the same database, outbox and bounds a door does.
    pub sdk_state: Arc<crate::api::rest::ApiState>,

    /// The taxonomy and metadata ceilings the `02` doors enforce, resolved
    /// once at `init` from `ProductsConfig` (**P-D-107** arm 1), exactly as
    /// `watermark_skew_tolerance` is. `register_rest` has no `cfg` in scope,
    /// so the runtime is where the doors reach them.
    pub taxonomy_caps: crate::api::rest::TaxonomyCaps,

    /// The pseudonymous ref the gear's own background acts attribute to.
    /// Server-minted per boot: the batch worker is not a person, and an
    /// audit row that named one would be a lie.
    pub system_actor_ref: uuid::Uuid,

    /// Whichever handle keeps the running pipeline's background tasks alive.
    ///
    /// Held for its `Drop`, never read: dropping either handle drops its
    /// `TaskSet` and cancels the sequencer, processors and vacuum, so the
    /// handle must outlive the process rather than the field access. Two
    /// shapes because the two pipelines are started by different builders —
    /// the SDK owns its own handle type when it owns the processor.
    #[allow(dead_code, reason = "held for its Drop; see the type's own note")]
    pub pipeline: OutboxLifetime,

    /// The database provider `api::rest::ApiState` clones into the read
    /// door's per-request state. Kept on the runtime rather than built fresh
    /// in `register_rest` because `ctx.db_required()` is `init()`'s to call —
    /// the same acquisition point the outbox handle above is built from —
    /// and a repeated call from `register_rest` would be a second,
    /// unnecessary place this gear's boot could fail on a missing `db`
    /// capability.
    pub db: toolkit_db::DBProvider<toolkit_db::DbError>,

    /// The operator's `idempotency_retention_hours` **as
    /// `ProductsConfig::resolved_idempotency_retention_hours` resolved it**,
    /// carried here for the reason the three fields above are:
    /// `ctx.config_or_default()` is `init()`'s call, and `register_rest`
    /// copies the resolved value onto `api::rest::ApiState` so the create
    /// doors stamp a claim's `expires_at` from what the operator configured.
    /// `api::rest`'s `idempotency_expiry` previously read
    /// `ProductsConfig::default()` itself and silently gave every operator
    /// the design's 24-hour floor.
    ///
    /// **Resolved, never raw**: this is the boot-time enforcement point for
    /// the `max(24h, max_freeze_timeout)` floor (C6,
    /// `dod-idempotency-store`). A raw `0` stored here would reach
    /// `idempotency_expiry` and stamp `expires_at == now`, which switches
    /// at-most-once off with no failure anywhere to see it.
    pub idempotency_retention_hours: u32,
}

/// The products gear.
#[toolkit::gear(name = "bss-products", deps = [authz_resolver], capabilities = [db, rest, stateful], lifecycle(entry = "serve", stop_timeout = "30s"))]
pub struct BssProductsGear {
    /// `None` until `init()` completes, and on a boot where the gear is
    /// compiled in but not configured.
    runtime: ArcSwapOption<ProductsRuntime>,
}

impl Default for BssProductsGear {
    fn default() -> Self {
        Self {
            runtime: ArcSwapOption::from(None),
        }
    }
}

impl BssProductsGear {
    /// The lifecycle entry: one ticker, the increment coalescer's sweep
    /// (`dod-coalescer`). Each tick discovers tenants with pending demand
    /// and runs one [`crate::infra::increment::drain_tenant`] pass per
    /// tenant; the per-tenant lease inside the pass is what makes
    /// concurrent deployments safe, so the tick itself needs no lease. The
    /// sibling pricing gear's `serve` is the shape this follows.
    pub(crate) async fn serve(
        self: std::sync::Arc<Self>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<()> {
        let Some(rt) = self.runtime.load_full() else {
            cancel.cancelled().await;
            return Ok(());
        };
        tracing::info!(
            coalescer_tick_secs = COALESCER_TICK.as_secs(),
            "bss-products: lifecycle started"
        );
        let db = rt.db.clone();
        // The composition root's one job: the worker is infra and must not
        // read `api::rest::ApiState`, so its context is built here from the
        // same boot state the doors get theirs from.
        let worker_ctx = crate::infra::bulk_worker::BulkWorkerContext {
            db: rt.sdk_state.db.clone(),
            sink: rt.sdk_state.sink.clone(),
            bulk_max_concurrent_batches_per_tenant: rt
                .sdk_state
                .bulk_max_concurrent_batches_per_tenant,
        };
        let mut interval = tokio::time::interval(COALESCER_TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The overdue-freeze scan runs on its own coarser cadence: an
        // overdue version stays overdue for hours, so a per-second re-scan
        // and re-warn is ~3,600 identical lines per hour per version, and
        // the answer only changes on ledger writes.
        let mut tick_count: u64 = 0;
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                _ = interval.tick() => {
                    coalescer_tick(&db, &cancel).await;
                    batch_tick(&worker_ctx, rt.system_actor_ref, &cancel).await;
                    if tick_count.is_multiple_of(OVERDUE_SCAN_EVERY_TICKS) {
                        let now = crate::domain::canonical::write_instant(chrono::Utc::now());
                        report_overdue_freezes(&db, now, rt.freeze_timeout_hours).await;
                    }
                    tick_count += 1;
                }
            }
        }
        Ok(())
    }
}

/// One batch-worker tick: stage every tenant's oldest `staging` batch
/// (`dod-stage-phase`). A failed sweep is logged and retried next tick —
/// the ledger is the record, so nothing is lost.
/// The principal the gear acts under when nobody asked it to — the bulk
/// worker's sweeps, the activation runner's flips, and every audit row they
/// write (**P-D-113** arm 2).
///
/// A UUID **v5** from a fixed namespace and the name `bss-products:system`:
/// the same value in every process on every host, computed rather than
/// configured. Until 2026-09-03 this was `Uuid::new_v4()` at runtime
/// construction, so every restart gave the gear's own acts a fresh actor that
/// resolved to nothing and the audit trail could not say two sweeps were the
/// same principal. `seeded_by = 'registry'` is the precedent — a fixed system
/// name for acts the gear performs on its own behalf.
///
/// Not a config field, deliberately: one more value to misconfigure, for a
/// principal that has no reason to differ between deployments. Which *host*
/// ran a sweep is an operational fact for logs; which *principal* did is an
/// audit fact for records, and the v4 served neither.
#[must_use]
pub(crate) fn system_actor_ref() -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b"bss-products:system")
}

async fn batch_tick(
    ctx: &crate::infra::bulk_worker::BulkWorkerContext,
    actor_ref: uuid::Uuid,
    cancel: &tokio_util::sync::CancellationToken,
) {
    let now = crate::domain::canonical::write_instant(chrono::Utc::now());
    if let Err(error) = crate::infra::bulk_worker::sweep(ctx, actor_ref, now, cancel).await {
        tracing::warn!(%error, "bss-products: batch worker sweep failed");
    }
}

/// One coalescer tick: sweep every tenant with pending demand at a
/// truncated now (P-D-82). A failed sweep is logged and retried next tick —
/// demand rows are never lost (the queue is the ledger), and a persistent
/// failure repeats this line at tick cadence, which is the operator's
/// signal.
async fn coalescer_tick(
    db: &toolkit_db::DBProvider<toolkit_db::DbError>,
    cancel: &tokio_util::sync::CancellationToken,
) {
    let now = crate::domain::canonical::write_instant(chrono::Utc::now());
    if let Err(error) = crate::infra::increment::sweep(db, now, cancel).await {
        tracing::warn!(%error, "bss-products: coalescer sweep failed");
    }
}

/// The freeze-timeout telemetry (`dod-freeze-timeout`): fail-closed is the
/// resolver's own posture, so the scan only names the silence, one warning
/// per overdue version.
async fn report_overdue_freezes(
    db: &toolkit_db::DBProvider<toolkit_db::DbError>,
    now: chrono::DateTime<chrono::Utc>,
    freeze_timeout_hours: u32,
) {
    match crate::infra::increment::overdue_freezes(db, now, freeze_timeout_hours).await {
        Ok(overdue) => {
            for entry in overdue {
                tracing::warn!(
                    tenant_id = %entry.tenant_id,
                    catalog_version_id = entry.catalog_version_id,
                    silent_participants = entry.silent_participants.join(","),
                    "bss-products: freeze_overdue"
                );
            }
        }
        Err(error) => {
            tracing::warn!(%error, "bss-products: freeze-overdue scan failed");
        }
    }
}

/// The coalescer's tick — well inside the interactive window so a lone
/// request still lands within ≤ 5 s of itself.
const COALESCER_TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// How many coalescer ticks between overdue-freeze scans — roughly once a
/// minute at the one-second tick, which is telemetry cadence for a state
/// that changes on the scale of hours.
const OVERDUE_SCAN_EVERY_TICKS: u64 = 60;

/// Whichever running pipeline the gear started, held only so its background
/// tasks are not cancelled.
#[allow(
    dead_code,
    reason = "each variant is held only for its Drop: dropping the handle cancels the pipeline's \
              background tasks, so the value must outlive the process rather than be read"
)]
pub(crate) enum OutboxLifetime {
    /// The SDK producer's own handle.
    Broker(Box<event_broker_sdk::ProducerOutboxHandle>),
    /// The plain toolkit handle, for the no-broker fallback.
    Interim(toolkit_db::outbox::OutboxHandle),
}

impl toolkit::contracts::DatabaseCapability for BssProductsGear {
    fn migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        use sea_orm_migration::MigratorTrait;
        let mut migrations = crate::infra::storage::migrations::Migrator::migrations();

        // The outbox's own tables are migrated by the facility, not by this
        // chain (P-D-22's consequences: "C1's 'one migration per table,
        // guards defined once' does not reach these tables — they are
        // migrated by `outbox_migrations()`, and the schema oracle must
        // therefore golden them as imported rather than as gear-authored").
        // Appended, never declared: no `CreateProductsOutbox`-shaped
        // migration exists anywhere in `crate::infra::storage::migrations`.
        #[allow(clippy::expect_used)]
        let outbox_migrations =
            toolkit_db::outbox::outbox_migrations_with_prefix(OUTBOX_TABLE_PREFIX).expect(
                "OUTBOX_TABLE_PREFIX is a fixed compile-time identifier, validated once here \
                 rather than at every call site: alphabetic-first, alnum/underscore only, and \
                 well under the facility's length limit",
            );
        migrations.extend(outbox_migrations);

        // The producer's own registration table, appended for the same reason
        // and on the same terms: the SDK owns it, this chain does not declare
        // it, and the README requires it run *before* a `DbProducer` is
        // constructed. Appended unconditionally rather than only where a broker
        // is configured — a migration set that varies with runtime wiring gives
        // two deployments two different schemas, and the cost here is **one**
        // table a no-broker deployment never writes
        // (`producer_registration_migrations` returns a single migration whose
        // `up` creates `event_broker_producer_registrations`).
        migrations.extend(event_broker_sdk::producer_registration_migrations());
        migrations
    }
}

#[async_trait]
impl Gear for BssProductsGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        // The configuration is read at init so a malformed operator file fails
        // the boot here rather than at the first request that happens to need
        // a field from it.
        let cfg: ProductsConfig = ctx.config_or_default()?;
        // P-D-84 arm 6: an inverted retention clamp is refused at boot, not
        // discovered as a panic on the first keyed request.
        cfg.validate()
            .map_err(|reason| anyhow::anyhow!("bss-products: invalid config: {reason}"))?;
        // The retention window is resolved once, here, and only the resolved
        // value ever leaves this function. `ProductsConfig::
        // resolved_idempotency_retention_hours` states why a bad value is
        // clamped rather than refused; what it cannot do is make the raise
        // visible, so this is where the operator hears about it. A `0` that
        // reached `api::rest::idempotency_expiry` would stamp
        // `expires_at == now`, and the next request on that key would read it
        // as expired, take it over and run the guarded mutation again.
        let idempotency_retention_hours = cfg.resolved_idempotency_retention_hours();
        if idempotency_retention_hours != cfg.idempotency_retention_hours {
            tracing::warn!(
                configured_retention_hours = cfg.idempotency_retention_hours,
                resolved_retention_hours = idempotency_retention_hours,
                floor_hours = crate::config::IDEMPOTENCY_RETENTION_FLOOR_HOURS,
                ceiling_hours = crate::config::IDEMPOTENCY_RETENTION_CEILING_HOURS,
                "bss-products: configured idempotency_retention_hours is outside the \
                 design's retention bounds and was clamped"
            );
        }
        tracing::info!(idempotency_retention_hours, "bss-products initialised");

        // Platform PEP. Authz is security-critical — the catalog this gear
        // authors is what pricing and every downstream reader depend on — so a
        // missing `AuthZResolverClient` fails init loudly rather than
        // degrading to an unguarded router.
        let authz_client = ctx
            .client_hub()
            .get::<dyn authz_resolver_sdk::AuthZResolverClient>()
            .context(
                "bss-products: AuthZResolverClient absent from ClientHub; \
                 authz-resolver module must be registered",
            )?;
        let enforcer = Arc::new(authz_resolver_sdk::PolicyEnforcer::new(authz_client));

        // Register the authz-label stub schemas so RBAC role definitions
        // targeting this gear's labels pass the platform's target-type
        // validation. Mandatory, as it is in the sibling pricing gear: without
        // them no custom catalog role can be defined, and the labels sit
        // outside `gts.cf.resources.*` where no built-in role covers them — a
        // silent skip would leave the authoring surface ungrantable.
        // `authz_label_type_schemas()` had no production caller until
        // **P-D-134** (2026-09-04) named that a defect of this slice.
        let types_registry = ctx
            .client_hub()
            .get::<dyn types_registry_sdk::TypesRegistryClient>()
            .context(
                "bss-products: TypesRegistryClient absent from ClientHub; \
                 types-registry module must be registered",
            )?;
        let results = types_registry
            .register(crate::authz::authz_label_type_schemas())
            .await
            .context("bss-products: register authz label schemas")?;
        for result in results {
            if let types_registry_sdk::RegisterResult::Err { gts_id, error } = result {
                anyhow::bail!(
                    "bss-products: failed to register authz label {}: {error}",
                    gts_id.as_deref().unwrap_or("?")
                );
            }
        }

        // Transactional outbox (P-D-22). The registry enqueues through the
        // platform's own `toolkit_db::outbox` pipeline rather than a
        // gear-authored `products_outbox` table — see this module's doc for
        // why. The gear's own database is required for the outbox exactly as
        // it is for the Foundation tables, so a missing configuration fails
        // the boot the same way the missing `AuthZResolverClient` above does.
        let db_provider = ctx
            .db_required()
            .context("bss-products: database not configured for the outbox pipeline")?;
        let outbox_db = db_provider.db();
        // The queue is declared here because `enqueue` refuses an
        // unregistered one (`OutboxError::QueueNotRegistered`), and the
        // create door enqueues inside its own transaction. Its processor is
        // a holding one: P-D-47 puts the real processor — the broker SDK's
        // `DbProducer` — in Phase 8's `dod-outbox-eventing`, so until then
        // rows accumulate undelivered rather than being discarded. See
        // `crate::infra::events::PendingBrokerProducer` for why it must not
        // answer `Ok`.
        // **P-D-47, with the owner's fallback.** The processor is the broker
        // SDK's own producer where a broker is reachable, and the holding
        // processor where none is. Absence of an `EventBrokerApi` in the
        // `ClientHub` is the whole condition — no config key of this gear's —
        // so a deployment that never registered the event-broker module boots
        // exactly as it did before, and one that did gets the producer without
        // being asked anything.
        //
        // A broker that is *present but refuses* is not this fallback's case:
        // `bind_producer` answers `Err` there, and this `?` fails the boot,
        // because a half-configured broker is an operator's mistake and must
        // not degrade quietly into an envelope no consumer reads.
        let partitions = toolkit_db::outbox::Partitions::of(crate::infra::events::PARTITIONS);
        let bound = crate::infra::broker::bind_producer(
            &ctx.client_hub(),
            outbox_db.clone(),
            OUTBOX_TABLE_PREFIX,
            partitions,
        )
        .await
        .context("bss-products: the event-broker producer could not be bound")?;

        let (sink, pipeline) = if let Some((sink, handle)) = bound {
            tracing::info!(
                queue = OUTBOX_TABLE_PREFIX,
                topic = crate::infra::broker::TOPIC,
                "bss-products: publishing through the event-broker SDK producer"
            );
            (sink, OutboxLifetime::Broker(Box::new(handle)))
        } else {
            anyhow::ensure!(
                !cfg.require_broker,
                "bss-products: require_broker is set and no EventBrokerApi is registered in the \
                 ClientHub; refusing to boot into the holding processor, which would accumulate \
                 every catalog event undelivered"
            );
            tracing::warn!(
                "bss-products: no EventBrokerApi in the ClientHub; events \
                     accumulate undelivered on the interim queue and no delivery \
                     is ever reported"
            );
            let handle = toolkit_db::outbox::Outbox::builder(outbox_db)
                .table_prefix(OUTBOX_TABLE_PREFIX)
                .context("bss-products: invalid outbox table prefix")?
                .queue(crate::infra::events::QUEUE_NAME, partitions)
                .leased(crate::infra::events::PendingBrokerProducer)
                .start()
                .await
                .context("bss-products: outbox pipeline failed to start")?;
            let sink = crate::infra::broker::EventSink::Interim(Arc::clone(handle.outbox()));
            (sink, OutboxLifetime::Interim(handle))
        };

        // The increment-request contract's default in-process binding
        // (P-D-15, `design/06` §2 rule 1): registered here so a sibling gear
        // resolves `dyn IncrementRequests` from the hub without knowing the
        // implementation package. The out-of-process binding is the REST
        // door `register_rest` merges; both run the identical
        // `catalog_version x request` gate.
        let sdk_state = Arc::new(crate::api::rest::ApiState {
            db: db_provider.clone(),
            sink: sink.clone(),
            taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&cfg),
            idempotency_retention_hours,
            bulk_max_rows_per_batch: cfg.bulk_max_rows_per_batch,
            bulk_max_concurrent_batches_per_tenant: cfg.bulk_max_concurrent_batches_per_tenant,
            watermark_skew_tolerance: cfg.watermark_skew_tolerance(),
        });
        ctx.client_hub()
            .register::<dyn bss_products_sdk::watermarks::WatermarkPosts>(Arc::new(
                crate::api::rest::reference::InProcessWatermarkPosts {
                    state: Arc::clone(&sdk_state),
                    enforcer: (*enforcer).clone(),
                },
            ));
        ctx.client_hub()
            .register::<dyn bss_products_sdk::increments::IncrementRequests>(Arc::new(
                crate::api::rest::catalog_version::InProcessIncrementRequests {
                    state: Arc::new(crate::api::rest::ApiState {
                        db: db_provider.clone(),
                        sink: sink.clone(),
                        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&cfg),
                        idempotency_retention_hours,
                        bulk_max_rows_per_batch: cfg.bulk_max_rows_per_batch,
                        bulk_max_concurrent_batches_per_tenant: cfg
                            .bulk_max_concurrent_batches_per_tenant,
                        watermark_skew_tolerance: cfg.watermark_skew_tolerance(),
                    }),
                    enforcer: (*enforcer).clone(),
                },
            ));

        self.runtime.store(Some(Arc::new(ProductsRuntime {
            enforcer,
            sink,
            freeze_timeout_hours: cfg.freeze_timeout_hours,
            bulk_max_rows_per_batch: cfg.bulk_max_rows_per_batch,
            bulk_max_concurrent_batches_per_tenant: cfg.bulk_max_concurrent_batches_per_tenant,
            watermark_skew_tolerance: cfg.watermark_skew_tolerance(),
            sdk_state: Arc::clone(&sdk_state),
            taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&cfg),
            system_actor_ref: system_actor_ref(),
            pipeline,
            db: db_provider,
            idempotency_retention_hours,
        })));

        Ok(())
    }
}

impl RestApiCapability for BssProductsGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<Router> {
        let Some(rt) = self.runtime.load_full() else {
            // Unconfigured boot: claim the prefix anyway, so a probe under
            // `/bss-products/v1` gets a `404` from this gear rather than
            // falling through to whatever else the host router matches.
            //
            // Logged rather than silent: reaching here means `init` never ran
            // or never populated the slot, and from the outside that is
            // indistinguishable from the ordinary configured-but-routeless
            // state this phase also produces. An operator debugging a 404
            // under this prefix needs the two told apart.
            tracing::warn!(
                "bss-products: register_rest reached with no runtime; \
                 mounting an empty router under the reserved prefix"
            );
            return Ok(crate::api::rest::router(router));
        };
        // Phase 4 Slice C: the read door. `products::router`/`skus::router`
        // each register their own absolute path
        // (`/bss-products/v1/{products|skus}/{id}`), so they are `.merge()`d
        // onto the host router directly rather than nested under the
        // reserved prefix by `api::rest::router` — the same shape the
        // sibling ledger gear's own door modules use. The `PolicyEnforcer` is
        // layered once here as its own `Extension`, cloned from the `Arc` the
        // runtime holds (RMS layers the value, not the `Arc`;
        // `PolicyEnforcer: Clone`), rather than carried on `ApiState`, so
        // every door added in this and later slices reaches it the same way.
        let api_state = Arc::new(crate::api::rest::ApiState {
            db: rt.db.clone(),
            sink: rt.sink.clone(),
            taxonomy_caps: rt.taxonomy_caps,
            idempotency_retention_hours: rt.idempotency_retention_hours,
            bulk_max_rows_per_batch: rt.bulk_max_rows_per_batch,
            bulk_max_concurrent_batches_per_tenant: rt.bulk_max_concurrent_batches_per_tenant,
            watermark_skew_tolerance: rt.watermark_skew_tolerance,
        });
        Ok(router
            .merge(crate::api::rest::products::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .merge(crate::api::rest::skus::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .merge(crate::api::rest::catalog_version::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .merge(crate::api::rest::bulk::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .merge(crate::api::rest::reference::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .merge(crate::api::rest::recognized_sets::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .merge(crate::api::rest::taxonomy::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .merge(crate::api::rest::retention::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .merge(crate::api::rest::materiality_policy::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .layer(axum::Extension((*rt.enforcer).clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_leaves_the_runtime_slot_empty() {
        let gear = BssProductsGear::default();
        assert!(gear.runtime.load_full().is_none());
    }

    /// `register_rest`'s empty-runtime branch returns a router and does not
    /// error — the behaviour that distinguishes this gear from
    /// `simple-user-settings`, whose `register_rest` errors out of an
    /// uninitialised `service` slot.
    ///
    /// Calling `register_rest` itself needs a `GearCtx` and a
    /// `dyn OpenApiRegistry`; the former needs a
    /// `tokio_util::sync::CancellationToken`, which this slice's dependency
    /// delta does not carry. What is exercised directly, without either, is
    /// [`crate::api::rest::router`] — the helper both of `register_rest`'s
    /// branches call, and the only place the nesting happens. It is
    /// infallible (`Router -> Router`, no `Result`), which is what makes
    /// `register_rest`'s `Ok(...)` around it unconditional in both branches.
    /// A request under the reserved prefix is answered by **this** gear with
    /// a `404`, and a path outside it is untouched by the nest.
    ///
    /// The earlier version of this test built the router and dropped it,
    /// which asserted nothing: it passed just as well if `router` returned
    /// `host_router` unnested, or nested under the wrong prefix, or swapped
    /// its arguments. The prefix reservation is the one behaviour this
    /// module exists to deliver, so it is asserted where it is observable —
    /// through a request — rather than by trusting the type.
    #[tokio::test]
    async fn a_request_under_the_reserved_prefix_is_answered_by_this_gear() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use axum::routing::get;
        use tower::ServiceExt as _;

        let host = Router::new().route("/elsewhere", get(|| async { "host" }));
        let mounted = crate::api::rest::router(host);

        let under_prefix = mounted
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/bss-products/v1/anything")
                    .body(Body::empty())
                    .expect("build the probe request"),
            )
            .await
            .expect("the router answers");
        assert_eq!(
            under_prefix.status(),
            StatusCode::NOT_FOUND,
            "the prefix is claimed, so an unmounted path under it is this gear's 404"
        );

        let outside = mounted
            .oneshot(
                Request::builder()
                    .uri("/elsewhere")
                    .body(Body::empty())
                    .expect("build the control request"),
            )
            .await
            .expect("the router answers");
        assert_eq!(
            outside.status(),
            StatusCode::OK,
            "nesting under the prefix must not shadow the host router's own paths"
        );
    }
}
