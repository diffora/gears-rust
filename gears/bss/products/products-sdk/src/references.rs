//! Owner-bound reference protocol for trusted consumers in the same binary/deployment.
//! The provider binds ownership; callers never supply an owner on an operation.
use crate::models::{ReferenceKind, ReferenceState, ReservationReceipt, Sku, SkuVersion};
use async_trait::async_trait;
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;
/// Stable identity for pricing's tenant-scoped recovery ticker.
pub const PRICING_SYSTEM_ACTOR: Uuid = Uuid::from_u128(0x00000000_0000_0f01_0000_627373722d70);
/// Pricing-specific `ClientHub` key. Products constructs the bound implementation.
pub struct PricingReferenceRegistry(pub Arc<dyn ReferenceRegistryV1>);
/// Same reservation rules and error codes as the Products REST reference door.
/// Missing or foreign-owner batch entries fail the batch; order follows the input.
#[async_trait]
pub trait ReferenceRegistryV1: Send + Sync {
    async fn reserve(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        sku_id: Uuid,
        kind: ReferenceKind,
        ref_id: Uuid,
    ) -> Result<ReservationReceipt, CanonicalError>;
    async fn confirm(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        reservation_id: Uuid,
    ) -> Result<(), CanonicalError>;
    async fn release(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        reservation_id: Uuid,
    ) -> Result<(), CanonicalError>;
    async fn states(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        reservation_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, ReferenceState)>, CanonicalError>;
    async fn sku_for_write(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        sku_id: Uuid,
    ) -> Result<Sku, CanonicalError>;
    async fn sku_version_as_of(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        sku_id: Uuid,
        date: time::Date,
    ) -> Result<Option<SkuVersion>, CanonicalError>;
}
