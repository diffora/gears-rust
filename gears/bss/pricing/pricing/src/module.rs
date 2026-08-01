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

use std::sync::Arc;

use anyhow::{Context, Result};
use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use axum::Router;
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
use crate::infra::storage::repo::PinFrontierRepo;

/// Per-process state built by [`Gear::init`] and read by
/// [`RestApiCapability::register_rest`] and [`BssPricingGear::serve`].
pub(crate) struct PricingRuntime {
    /// Database provider for the background work (the read-model warm re-drive
    /// runs under a system context, not a per-request `SecureORM` scope).
    ///
    /// Acquired here rather than later on purpose: `db_required()` is what
    /// proves the declared `db` capability actually resolved, and a boot that
    /// cannot reach the database must fail at init, not at the first publish.
    #[allow(
        dead_code,
        reason = "read by the read-model warm re-drive and the repositories, which land with the Foundation tables"
    )]
    pub db: DBProvider<DbError>,
    /// The validated configuration, carried so the lifecycle reads the same
    /// values `init()` validated rather than re-parsing.
    pub config: BssPricingConfig,
    /// Platform PEP, built in `init()` from the `authz-resolver` `ClientHub`
    /// client and cloned into every request as an `Extension` by
    /// `register_rest`. Authz is security-critical, so a missing client fails
    /// init — there is no no-op fallback.
    pub enforcer: Arc<authz_resolver_sdk::PolicyEnforcer>,
    /// The `CatalogVersion` registry, resolved from `ClientHub` with a
    /// fail-closed default. Unlike the PEP this does NOT hard-fail init: the
    /// registry gear has no code in this repository yet, and a catalog that
    /// cannot boot without it could not be developed at all. The cost is paid
    /// at the right moment instead — publish requests addressability and stops
    /// when the answer is "unconfigured", so nothing becomes consumer-visible
    /// without a real version.
    #[allow(
        dead_code,
        reason = "requested by the publish engine, which lands with the publish path"
    )]
    pub catalog_version_registry: Arc<dyn CatalogVersionRegistryV1>,
    /// The joint-conformance publish gate, loaded once at init from
    /// `config.fixtures.registry_path`.
    ///
    /// Loaded here rather than per publish because the registry is a static
    /// generated artifact and the gate runs inside a publish transaction, where
    /// reading a file is not an option. A registry that cannot be read leaves
    /// the gate CLOSED for every kind (see [`FixtureGate::load`]) and does NOT
    /// abort the boot: the read path this gear serves to Rating and Tariffs
    /// keeps working, and every publish then fails per kind with
    /// `FIXTURE_MISSING` rather than the whole gear disappearing.
    #[allow(
        dead_code,
        reason = "consulted by the publish engine, which lands with the publish path"
    )]
    pub fixture_gate: FixtureGate,
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
    /// The Foundation's background work is the read-model warm re-drive: a
    /// degraded publish's projection continues past the 5s SLO until it
    /// completes. Its ticker lands with the projector; until then `serve`
    /// parks on the cancellation token so the lifecycle contract is honest —
    /// the gear starts, stops on cancel, and claims no work it does not do.
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
        cancel.cancelled().await;
        info!("bss-pricing: lifecycle cancelled");
        Ok(())
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

        self.runtime.store(Some(Arc::new(PricingRuntime {
            db,
            config,
            enforcer,
            catalog_version_registry,
            fixture_gate,
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
