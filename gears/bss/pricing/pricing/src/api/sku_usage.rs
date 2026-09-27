//! Products' SKU usage port, as pricing fills it (D-428, P-D-197).
//!
//! Pricing registers [`PricingSkuUsage`] in the `ClientHub` at init as `dyn SkuUsageV1`, and
//! Products resolves it at each SKU read. The port is a read door into pricing like the REST
//! ones: the caller must hold `price_book_entry:read` (else 403), the SKUs' entries are read under
//! the scope that grant gives, in the tenant asked for, and the counts are read in one
//! transaction with a fixed number of statements. Pricing reads only the SKU ids it is given;
//! nothing Products holds flows back into pricing.
use crate::api::rest::authoring::{AuthoringState, support};
use crate::authz::{self, actions, resource_types};
use async_trait::async_trait;
use authz_resolver_sdk::PolicyEnforcer;
use bss_products_sdk::sku_usage::{SkuUsage, SkuUsageV1, sku_usage_denied, sku_usage_unavailable};
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Pricing's implementation of Products' `SkuUsageV1`.
pub struct PricingSkuUsage {
    state: Arc<AuthoringState>,
    enforcer: PolicyEnforcer,
}
impl PricingSkuUsage {
    /// The port over the gear's database and policy.
    #[must_use]
    pub fn new(state: Arc<AuthoringState>, enforcer: PolicyEnforcer) -> Self {
        Self { state, enforcer }
    }
}
#[async_trait]
impl SkuUsageV1 for PricingSkuUsage {
    async fn usage(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        sku_ids: &[Uuid],
    ) -> Result<Vec<SkuUsage>, CanonicalError> {
        let scope = authz::access_scope(
            &self.enforcer,
            ctx,
            &resource_types::PRICE_BOOK_ENTRY,
            actions::READ,
            None,
            None,
        )
        .await
        .map_err(|error| match error {
            authz::AuthzError::Denied(denial) => {
                tracing::debug!(reason = %denial.reason, "bss-pricing: SKU usage refused");
                sku_usage_denied()
            }
            authz::AuthzError::Unavailable(detail) => {
                tracing::warn!(detail, "bss-pricing: SKU usage authorization unavailable");
                sku_usage_unavailable("pricing authorization is unavailable")
            }
        })?;
        let skus = sku_ids.to_vec();
        support::transaction(&self.state.db.db(), move |tx| {
            let (scope, skus) = (scope.clone(), skus.clone());
            Box::pin(
                async move { Ok(crate::infra::usage::sku_usage(tx, &scope, tenant, &skus).await?) },
            )
        })
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "bss-pricing: SKU usage could not be read");
            sku_usage_unavailable("pricing could not read the SKU usage")
        })
    }
}
