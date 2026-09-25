//! Pricing gear lifecycle, database capability and reserved REST prefix.

use crate::config::BssPricingConfig;
use anyhow::{Context, Result};
use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use axum::Router;
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use toolkit::api::OpenApiRegistry;
use toolkit::config::ConfigError;
use toolkit::contracts::{DatabaseCapability, RestApiCapability};
use toolkit::{Gear, GearCtx};

struct PricingRuntime {
    enforcer: Arc<authz_resolver_sdk::PolicyEnforcer>,
    state: Arc<crate::api::rest::authoring::AuthoringState>,
}

#[toolkit::gear(name = "bss-pricing", capabilities = [db, rest, stateful], deps = [types_registry, authz_resolver, account_management], lifecycle(entry = "serve", stop_timeout = "30s"))]
pub struct BssPricingGear {
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
    /// Spawn the reference recovery task and cancel in-flight work on shutdown.
    pub(crate) async fn serve(self: Arc<Self>, cancel: CancellationToken) -> Result<()> {
        let Some(runtime) = self.runtime.load_full() else {
            cancel.cancelled().await;
            return Ok(());
        };
        let child = cancel.child_token();
        let state = runtime.state.clone();
        let task = tokio::spawn(async move {
            let mut ticker = crate::infra::reference_ticker::Ticker::new(
                state,
                Arc::new(crate::infra::reference_work::WallClock),
                100,
                10,
            );
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                tokio::select! { biased;
                    () = child.cancelled() => break,
                    _ = interval.tick() => {
                        tokio::select! { biased;
                            () = child.cancelled() => break,
                            result = ticker.tick() => if let Err(error) = result { tracing::warn!(error=%error, "pricing reference ticker failed"); }
                        }
                    }
                }
            }
        });
        task.await
            .context("pricing reference ticker stopped unexpectedly")?;
        runtime.state.stop().await;
        Ok(())
    }
}

#[async_trait]
impl Gear for BssPricingGear {
    async fn init(&self, ctx: &GearCtx) -> Result<()> {
        match ctx.config::<BssPricingConfig>() {
            Ok(_) | Err(ConfigError::MissingConfigSection { .. }) => {}
            Err(ConfigError::GearNotFound { .. }) => return Ok(()),
            Err(error) => return Err(error).context("bss-pricing: invalid config"),
        }
        let db = ctx
            .db_required()
            .context("bss-pricing: database is required")?;
        let authz_client = ctx
            .client_hub()
            .get::<dyn authz_resolver_sdk::AuthZResolverApi>()
            .context(
                "bss-pricing: AuthZResolverApi absent from ClientHub; \
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

        self.runtime.store(Some(Arc::new(PricingRuntime {
            enforcer,
            state: Arc::new(
                crate::api::rest::authoring::AuthoringState::new(db, ctx.client_hub()).await?,
            ),
        })));
        Ok(())
    }
}

impl DatabaseCapability for BssPricingGear {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations = crate::infra::storage::migrations::Migrator::migrations();
        match toolkit_db::outbox::outbox_migrations_with_prefix("bss_pricing_outbox") {
            Ok(outbox) => migrations.extend(outbox),
            Err(error) => migrations.push(Box::new(InvalidOutboxMigration(error.to_string()))),
        }
        migrations.extend(event_broker_sdk::producer_registration_migrations());
        migrations
    }
}

impl RestApiCapability for BssPricingGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: Router,
        openapi: &dyn OpenApiRegistry,
    ) -> Result<Router> {
        let inner = Router::new();
        let inner = if let Some(runtime) = self.runtime.load_full() {
            inner
                .merge(crate::api::rest::authoring::router(
                    runtime.state.clone(),
                    openapi,
                ))
                .layer(axum::Extension((*runtime.enforcer).clone()))
                .layer(axum::middleware::from_fn(
                    toolkit::api::canonical_error_middleware,
                ))
        } else {
            inner
        };
        Ok(router.merge(inner))
    }
}

// The capability cannot return Result. Preserve a prefix error as a failing migration
// rather than panicking or silently omitting delivery tables.
struct InvalidOutboxMigration(String);
impl sea_orm_migration::MigrationName for InvalidOutboxMigration {
    fn name(&self) -> &'static str {
        "invalid_pricing_outbox_prefix"
    }
}
#[async_trait]
impl MigrationTrait for InvalidOutboxMigration {
    async fn up(&self, _: &sea_orm_migration::SchemaManager) -> Result<(), sea_orm::DbErr> {
        Err(sea_orm::DbErr::Migration(self.0.clone()))
    }
    async fn down(&self, _: &sea_orm_migration::SchemaManager) -> Result<(), sea_orm::DbErr> {
        Err(sea_orm::DbErr::Migration(self.0.clone()))
    }
}

// Run-3 route contract: method | path | resource:action | If-Match | Idempotency-Key
// POST /price-books price_book:author false true
// GET /price-books price_book:read false false
// GET /price-books/{id} price_book:read false false
// PATCH /price-books/{id} price_book:author true false
// GET /price-books/{id}/prices price:read false false
// GET /price-books/{id}/export price_book:read false false
// GET /settings config:read false false
// PUT /settings config:settings true false
// GET /dimension-keys config:read false false
// PUT /dimension-keys config:settings true false

// POST /price-books/{id}/prices price:author false true
// GET /prices/{id} price:read false false
// PATCH /prices/{id} price:author true false
// DELETE /prices/{id} price:author false false

// GET /reference-ops config:settings false false

// Run-4 rows: method | path | resource:action | If-Match | Idempotency-Key
// POST /prices/{id}/rows price:author false true
// PATCH /rows/{id} price:author true false
// DELETE /rows/{id} price:author true false

// Run-4 approvals: method | path | resource:action | If-Match | Idempotency-Key
// POST /rows/{id}/submit price:submit false true
// GET /price-books/{id}/publish-changes price_book:read false false
// POST /price-books/{id}/publish-changes price_book:submit false true
// GET /approval-units approval_unit:read false false
// GET /approval-units/{id} approval_unit:read false false
// POST /approval-units/{id}/approve approval_unit:approve false true
// POST /approval-units/{id}/reject approval_unit:approve false true
// POST /approval-units/{id}/withdraw approval_unit:submit false true
// GET /approval-policy config:read false false
// PUT /approval-policy config:settings true false
