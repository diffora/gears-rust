//! Gear wiring retained for the phase 1c SKU registry.

use crate::config::ProductsConfig;
use crate::infra::events::OUTBOX_TABLE_PREFIX;
use anyhow::Context;
use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use axum::Router;
use std::sync::Arc;
use toolkit::api::OpenApiRegistry;
use toolkit::contracts::RestApiCapability;
use toolkit::{Gear, GearCtx};

/// `source` on the pick-list: a supplier another module registered.
pub const USAGE_TYPE_SOURCE_REGISTRY: &str = "registry";
/// `source`: this crate's adapter over the usage collector's own client.
pub const USAGE_TYPE_SOURCE_COLLECTOR: &str = "usage_collector";
/// `source`: the fabricated set a stand opted into.
pub const USAGE_TYPE_SOURCE_LOCAL_DEV: &str = "local_dev_static";
/// `source`: nothing answers, so the pick-list is a 501 and not an empty page.
pub const USAGE_TYPE_SOURCE_UNCONFIGURED: &str = "unconfigured";

/// Per-process dependencies built once at init.
pub(crate) struct ProductsRuntime {
    pub enforcer: Arc<authz_resolver_sdk::PolicyEnforcer>,
    pub api_state: Arc<crate::api::rest::ApiState>,
    // Held until the runtime drops, keeping the outbox workers alive.
    pub _pipeline: OutboxLifetime,
}

/// Register the one products SDK client against the same runtime dependencies.
fn register_products_client(
    hub: &toolkit::ClientHub,
    db: toolkit_db::Db,
    enforcer: Arc<authz_resolver_sdk::PolicyEnforcer>,
) {
    hub.register::<dyn bss_products_sdk::ProductsClient>(Arc::new(
        crate::infra::sdk_client::LocalProductsClient::new(db, enforcer),
    ));
}

/// The products gear.
#[toolkit::gear(name = "bss-products", deps = [authz_resolver, types_registry, usage_collector], capabilities = [db, rest, stateful], lifecycle(entry = "serve", stop_timeout = "30s"))]
#[toolkit::provides(
    contract = bss_pricing_sdk::product_catalog::ProductCatalogClientV1,
    local = Self::build_catalog_provider,
    rest_client = crate::infra::catalog_rest_client::ProductCatalogRestClient,
    transports = [local, rest],
)]

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
    /// Build the authorized local catalog transport over this gear's database.
    fn build_catalog_provider(
        ctx: &GearCtx,
        _policies: Arc<toolkit::contract_support::policy::PolicyStack>,
    ) -> anyhow::Result<Arc<dyn bss_pricing_sdk::product_catalog::ProductCatalogClientV1>> {
        let enforcer = authz_resolver_sdk::PolicyEnforcer::new(
            ctx.client_hub()
                .get::<dyn authz_resolver_sdk::AuthZResolverApi>()?,
        );
        Ok(Arc::new(
            crate::infra::catalog_provider::BrowseCatalogProvider::new(
                ctx.db_required()?.db(),
                Arc::new(enforcer),
            ),
        ))
    }

    /// Keep runtime resources alive until cooperative shutdown.
    pub(crate) async fn serve(
        self: Arc<Self>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<()> {
        let _runtime = self.runtime.load_full();
        cancel.cancelled().await;
        Ok(())
    }
}

/// Owns the running outbox pipeline until shutdown.
pub enum OutboxLifetime {
    /// Broker SDK pipeline.
    Broker(Box<event_broker_sdk::ProducerOutboxHandle>),
    /// Holding pipeline when no broker is registered.
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

/// `03`'s usage-type catalog and where it came from (**P-D-141**, and the
/// plugin seam of 2026-09-22).
///
/// Extracted from `init` rather than inlined, for the reason pricing's
/// `resolve_product_catalog` was: the two are the same shape and together they
/// pushed `init` past its line budget.
fn resolve_usage_type_catalog(
    ctx: &GearCtx,
    cfg: &crate::config::ProductsConfig,
) -> (
    Arc<dyn bss_products_sdk::usage_types::UsageTypeCatalog>,
    &'static str,
) {
    // `03`'s usage-type catalog (P-D-141): one narrow port, four steps, and
    // the provenance decided here rather than at a call site.
    //
    // **A registered catalog always wins over a config mode**, and the
    // argument is pricing's `module::resolve_product_catalog`: a deployment
    // that later gains a real supplier must take it even if a dev mode was
    // left in its file, since the failure to avoid is a stand quietly
    // serving types no supplier issued.
    //
    // **The `source` travels with the `Arc`.** Re-deriving it from the
    // config mode at the read would report `unconfigured` — "nobody was
    // asked" — for an answer a registered supplier gave, and would leave
    // the `registry` value unreachable.
    if let Ok(registered) = ctx
        .client_hub()
        .get::<dyn bss_products_sdk::usage_types::UsageTypeCatalog>()
    {
        (registered, USAGE_TYPE_SOURCE_REGISTRY)
    } else if let Ok(client) = ctx
        .client_hub()
        .get::<dyn usage_collector_sdk::UsageCollectorClientV1>()
    {
        (
            Arc::new(crate::infra::usage_types::CollectorUsageTypes::new(
                client,
                cfg.usage_type_resolver_timeout(),
            )),
            USAGE_TYPE_SOURCE_COLLECTOR,
        )
    } else {
        match cfg.usage_type_catalog_mode {
            crate::config::UsageTypeCatalogSource::LocalDevStaticUsageTypes => {
                tracing::warn!(
                    mode = "local_dev_static_usage_types",
                    id_prefix = crate::infra::usage_types::DEV_LOCAL_USAGE_TYPE_PREFIX,
                    "bss-products: serving a FABRICATED usage-type catalog. Operators are \
                     being shown types no collector issued, and a meter declared against \
                     one names a stream nothing will ever report. Every id is in the \
                     reserved namespace above so the rows can be found later."
                );
                (
                    Arc::new(crate::infra::usage_types::LocalDevStaticUsageTypes),
                    USAGE_TYPE_SOURCE_LOCAL_DEV,
                )
            }
            crate::config::UsageTypeCatalogSource::Unconfigured => {
                tracing::warn!(
                    "bss-products: no usage-type catalog registered and no mode configured; \
                     the pick-list answers 501 and every usage-SKU publish fails closed \
                     (P-D-131)"
                );
                (
                    Arc::new(crate::infra::usage_types::UnconfiguredUsageTypes),
                    USAGE_TYPE_SOURCE_UNCONFIGURED,
                )
            }
        }
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

        // `#[toolkit::provides]`-generated wiring: validates the contract IR
        // (and the browse HTTP binding), reads
        // `client_wiring.product_catalog_client_v1` (absent → Local), and
        // registers `Arc<dyn ProductCatalogClientV1>` in the ClientHub.
        self.wire_product_catalog_client_v1(ctx).await?;

        // Platform PEP. Authz is security-critical — the catalog this gear
        // authors is what pricing and every downstream reader depend on — so a
        // missing `AuthZResolverApi` fails init loudly rather than
        // degrading to an unguarded router.
        let authz_client = ctx
            .client_hub()
            .get::<dyn authz_resolver_sdk::AuthZResolverApi>()
            .context(
                "bss-products: AuthZResolverApi absent from ClientHub; \
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
        // the boot the same way the missing `AuthZResolverApi` above does.
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

        let (usage_type_catalog, usage_type_catalog_source) = resolve_usage_type_catalog(ctx, &cfg);
        let api_state = Arc::new(crate::api::rest::ApiState {
            db: db_provider,
            sink,
            usage_type_catalog,
            usage_type_catalog_source,
            idempotency_retention_hours,
            fence_ttl_minutes: cfg.fence_ttl_minutes,
            reference_principals: cfg.reference_principals.clone(),
        });
        register_products_client(&ctx.client_hub(), api_state.db.db(), Arc::clone(&enforcer));
        self.runtime.store(Some(Arc::new(ProductsRuntime {
            enforcer,
            api_state,
            _pipeline: pipeline,
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
        if let Some(rt) = self.runtime.load_full() {
            return Ok(router
                .merge(crate::api::rest::categories::router(
                    Arc::clone(&rt.api_state),
                    openapi,
                ))
                .merge(crate::api::rest::skus::router(
                    Arc::clone(&rt.api_state),
                    openapi,
                ))
                .merge(crate::api::rest::sku_governance::router(
                    Arc::clone(&rt.api_state),
                    openapi,
                ))
                .merge(crate::api::rest::approval_units::router(
                    Arc::clone(&rt.api_state),
                    openapi,
                ))
                .merge(crate::api::rest::approval_policy::router(
                    Arc::clone(&rt.api_state),
                    openapi,
                ))
                .merge(crate::api::rest::references::router(
                    Arc::clone(&rt.api_state),
                    openapi,
                ))
                .merge(crate::api::rest::browse::router(
                    Arc::clone(&rt.api_state),
                    openapi,
                ))
                .layer(axum::Extension((*rt.enforcer).clone())));
        }
        Ok(router)
    }
}

#[cfg(test)]
#[path = "gear_tests.rs"]
mod tests;
