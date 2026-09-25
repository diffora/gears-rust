//! Lazy resolution permits pricing to boot before Products registers its owner-bound client.
use bss_products_sdk::{PricingReferenceRegistry, ReferenceRegistryV1};
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
/// Resolve on each use; absence never gets cached as a permanent startup failure.
/// # Errors
/// Returns 503 `REGISTRY_UNAVAILABLE` when Products has not registered pricing's key.
pub fn resolve(hub: &toolkit::ClientHub) -> Result<Arc<dyn ReferenceRegistryV1>, CanonicalError> {
    hub.get::<PricingReferenceRegistry>()
        .map(|key| key.0.clone())
        .map_err(|_| {
            CanonicalError::service_unavailable()
                .with_detail("REGISTRY_UNAVAILABLE: Products reference registry is unavailable")
                .create()
        })
}
