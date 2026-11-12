//! `AuthN` resolver gear.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use authn_resolver_sdk::{AuthNResolverBearerAuthenticator, AuthNResolverClient};
use toolkit::Gear;
use toolkit::context::GearCtx;
use toolkit_security::DynBearerAuthenticator;
use tracing::info;

use crate::config::AuthNResolverConfig;
use crate::domain::{AuthNResolverLocalClient, Service};

/// `AuthN` Resolver gear.
///
/// This gear:
/// 1. Discovers plugin instances via types-registry
/// 2. Routes requests to the selected plugin based on vendor configuration
///
/// The registry is consumed, not linked: `PlatformTypesRegistryApi` is the registry's own
/// local client where the registry runs in this process, and its resolving REST client
/// otherwise. This gear publishes the SDK's `AuthNResolverPluginSpecV1` schema
/// (`gts_publication(...)`), admitted before any application `init`. Plugin discovery is lazy:
/// it happens on the first API call, after every plugin bound its client in `init`.
#[toolkit::consumes(contract = types_registry_sdk::PlatformTypesRegistryApi, from = "types-registry")]
#[toolkit::gear(
    name = "authn-resolver",
    gts_publication(crates = [authn_resolver_sdk]),
)]
pub(crate) struct AuthNResolver {
    service: OnceLock<Arc<Service>>,
}

impl Default for AuthNResolver {
    fn default() -> Self {
        Self {
            service: OnceLock::new(),
        }
    }
}

#[async_trait]
impl Gear for AuthNResolver {
    #[tracing::instrument(skip_all, fields(vendor))]
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let cfg: AuthNResolverConfig = ctx.config_or_default()?;
        tracing::Span::current().record("vendor", cfg.vendor.as_str());
        info!(vendor = %cfg.vendor);

        // Create service
        let hub = ctx.client_hub();
        let svc = Arc::new(Service::new(Arc::clone(&hub), cfg.vendor));
        self.service
            .set(svc.clone())
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;

        // Register client in ClientHub. Its plugin instances are admitted before
        // this `init` and bound before anything is served, so no gate is needed.
        let api: Arc<dyn AuthNResolverClient> = Arc::new(AuthNResolverLocalClient::new(svc));
        hub.register::<dyn AuthNResolverClient>(Arc::clone(&api));

        // Register the tenant-plane bearer bridge for the OoP runtime to pick up
        // (if-absent, so an explicitly-supplied authenticator wins).
        if hub.get::<DynBearerAuthenticator>().is_err() {
            let bridge = DynBearerAuthenticator::new(AuthNResolverBearerAuthenticator::new(api));
            hub.register::<DynBearerAuthenticator>(Arc::new(bridge));
        }

        Ok(())
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "gear_tests.rs"]
mod gear_tests;
