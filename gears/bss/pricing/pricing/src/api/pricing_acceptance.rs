//! Receipt capability, separate from catalog preview and money reads.
use crate::{authz::actions, infra::commercial_terms::CommercialTermsService};
use bss_pricing_sdk::acceptance::{
    AcceptanceQuery, AcceptanceReceipt, CommandMeta, FulfilmentQuery, HeldBindings,
    PricingAcceptanceV1,
};
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
/// Authorized receipt reads and the pending Task 6 hold boundary.
pub struct PricingAcceptanceProvider {
    service: Arc<CommercialTermsService>,
}
impl PricingAcceptanceProvider {
    /// Share the commercial service with sellability.
    #[must_use]
    pub fn new(service: Arc<CommercialTermsService>) -> Self {
        Self { service }
    }
}
#[async_trait::async_trait]
impl PricingAcceptanceV1 for PricingAcceptanceProvider {
    async fn acceptance(
        &self,
        ctx: &SecurityContext,
        query: AcceptanceQuery,
    ) -> Result<AcceptanceReceipt, CanonicalError> {
        self.service.acceptance(ctx, query).await
    }
    async fn hold(
        &self,
        ctx: &SecurityContext,
        query: FulfilmentQuery,
        _meta: CommandMeta,
    ) -> Result<HeldBindings, CanonicalError> {
        self.service
            .scope(
                ctx,
                query.tenant_axes.seller_tenant_id,
                actions::HOLD,
                Some(query.acceptance.acceptance_id),
            )
            .await?;
        Err(self.service.pending("PricingAcceptanceV1::hold"))
    }
}
