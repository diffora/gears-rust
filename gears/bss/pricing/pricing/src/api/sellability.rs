//! Authorized sellability boundary; execution arrives in 5c and Task 6.
use crate::{authz::actions, infra::commercial_terms::CommercialTermsService};
use bss_pricing_sdk::acceptance::{
    AcceptanceReceipt, CommandMeta, FulfilmentEligibility, FulfilmentQuery, NewSaleQuery,
    SellabilityV1,
};
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
/// Acceptance command and live fulfilment capability.
pub struct SellabilityProvider {
    service: Arc<CommercialTermsService>,
}
impl SellabilityProvider {
    /// Share the commercial service with receipt reads/holds.
    #[must_use]
    pub fn new(service: Arc<CommercialTermsService>) -> Self {
        Self { service }
    }
}
#[async_trait::async_trait]
impl SellabilityV1 for SellabilityProvider {
    async fn check(
        &self,
        ctx: &SecurityContext,
        query: NewSaleQuery,
        _meta: CommandMeta,
    ) -> Result<AcceptanceReceipt, CanonicalError> {
        self.service
            .scope(
                ctx,
                query.tenant_axes.seller_tenant_id,
                actions::CREATE,
                None,
            )
            .await?;
        Err(self.service.pending("SellabilityV1::check"))
    }
    async fn check_fulfilment(
        &self,
        ctx: &SecurityContext,
        query: FulfilmentQuery,
    ) -> Result<FulfilmentEligibility, CanonicalError> {
        self.service
            .scope(
                ctx,
                query.tenant_axes.seller_tenant_id,
                actions::READ,
                Some(query.acceptance.acceptance_id),
            )
            .await?;
        Err(self.service.pending("SellabilityV1::check_fulfilment"))
    }
}
