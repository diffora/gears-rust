//! Pricing's usage of SKUs, as the SKU reads show it — one port that pricing
//! fills (**P-D-197**; pricing **D-428**).
//!
//! # Why the port is here and pricing fills it
//!
//! The SKUs screen shows, per SKU, how many prices and plans use it ("5 prices"
//! with currency chips, "unpriced", "3 plans"). Those facts are pricing's, and
//! products must not depend on pricing. So the contract sits in this crate,
//! pricing implements it and registers it in `ClientHub` at its init as
//! `dyn SkuUsageV1`, and the gear resolves it at each read — not at its own
//! init, because the two gears boot in either order.
//!
//! # Information, never a fence input
//!
//! A SKU read shows the answer, or `null` when no port is registered, when it
//! refuses the caller, or when it cannot answer; the read never fails for it.
//! Fences, retirement and type changes read the local reference registry only
//! (P-D-188, P-D-194): no remote count sits on a fence.
//!
//! # No serde here
//!
//! As in [`crate::usage_types`]: the gear's REST DTOs own serde and map onto
//! these types.

use async_trait::async_trait;
use toolkit_canonical_errors::{CanonicalError, resource_error};
use toolkit_security::SecurityContext;
use uuid::Uuid;

#[resource_error(gts_id!("cf.bss.products.sku.v1~"))]
struct SkuUsageResource;

/// The canonical error when the port refuses the caller — a **403**: the
/// caller holds no pricing `price_book_entry:read`. A SKU read shows it as
/// `usage: null`. Carries no PDP detail, which stays in the implementation's
/// logs.
#[must_use]
pub fn sku_usage_denied() -> CanonicalError {
    SkuUsageResource::permission_denied()
        .with_reason("the SKU usage port refused this caller")
        .create()
}

/// The canonical error when the port cannot answer — a **503**: its storage or
/// its authorization is unavailable. A SKU read shows it as `usage: null`.
#[must_use]
pub fn sku_usage_unavailable(detail: impl Into<String>) -> CanonicalError {
    CanonicalError::service_unavailable()
        .with_detail(detail)
        .create()
}

/// A SKU's prices by state; a rejected price is not counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PriceCounts {
    pub approved: u64,
    pub pending: u64,
    pub draft: u64,
}

/// What pricing reports about one SKU (pricing D-428).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkuUsage {
    /// The SKU asked about.
    pub sku_id: Uuid,
    /// The SKU's price-book entries in every book of the tenant, in every
    /// reference state.
    pub entries: u64,
    /// The distinct currencies of those entries' books, sorted.
    pub currencies: Vec<String>,
    /// The prices of those entries, added up by state.
    pub prices: PriceCounts,
    /// The distinct plans with a draft, pending or published revision whose
    /// items name one of those entries — distinct across the SKU's entries,
    /// never a sum of the entries' counts.
    pub plans: u64,
}

/// Pricing's usage of SKUs, which pricing registers on `ClientHub` as
/// `dyn SkuUsageV1`.
#[async_trait]
pub trait SkuUsageV1: Send + Sync + 'static {
    /// The usage of each distinct id of `sku_ids` in `tenant`, once, in the
    /// order first asked. An unknown id, a SKU of another tenant and a bundle
    /// SKU (it has no entry) answer zeros. The tenant argument narrows the
    /// read; it never grants access.
    ///
    /// # Errors
    ///
    /// [`sku_usage_denied`] (403) for a caller without pricing
    /// `price_book_entry:read`; [`sku_usage_unavailable`] (503) when the
    /// answer cannot be read. **Neither is a page of zeros.**
    async fn usage(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        sku_ids: &[Uuid],
    ) -> Result<Vec<SkuUsage>, CanonicalError>;
}
