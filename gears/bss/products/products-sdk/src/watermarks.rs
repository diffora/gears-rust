//! The reference-watermark contract (`design/07-reference-signal.md` §2,
//! P-D-15, P-D-71) — the **third** trait on this SDK and the second write
//! one, beside [`IncrementRequests`](crate::increments::IncrementRequests)
//! and the read-only
//! [`ProductsClient`](crate::api::ProductsClient). The shape question that
//! governed all three was settled once, by **P-D-81** arm 4, and
//! `features/reference-signal.md`'s own `DoD` cites that row rather than
//! re-asking it.
//!
//! # What a post carries, and why the set is complete
//!
//! `(producer, watermark_at, the complete SKU id set)`. **Complete, not a
//! delta**: the predicate's fresh-zero verdict means *"this producer,
//! fresh, does not hold this SKU"*, which a delta cannot answer. The
//! `watermark_at` is the instant the set is complete **as of**, not the
//! instant of the post; the door stamps the second itself.
//!
//! # The four refusals live at the door
//!
//! An unregistered poster, a regressing `watermark_at`, an equal
//! `watermark_at` carrying a different set, and one above the receiving
//! clock plus the configured skew. They are the door's, not this trait's,
//! and reach a consumer as the canonical errors this SDK's sibling
//! projections classify.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-watermark-port:p1

use async_trait::async_trait;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// One producer's complete reference set as of an instant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatermarkPost {
    /// The registered producer posting. An unregistered name is refused
    /// `PRODUCER_UNREGISTERED`.
    pub producer: String,
    /// The instant the set below is complete as of.
    pub watermark_at: chrono::DateTime<chrono::Utc>,
    /// **Every** SKU this producer references at `watermark_at` — the
    /// complete set, never a delta.
    pub sku_ids: Vec<Uuid>,
}

/// What a post answers: the stored watermark after the write, so a caller
/// can tell an accepted post from an admitted idempotent replay without a
/// second call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatermarkAck {
    /// The stored `watermark_at` after the post.
    pub watermark_at: chrono::DateTime<chrono::Utc>,
    /// How many SKUs the stored set holds.
    pub member_count: usize,
    /// `true` when the post matched the stored watermark exactly and
    /// nothing was written — the admitted replay.
    pub replayed: bool,
}

/// The watermark client — the typed contract a producer resolves from
/// `ClientHub`, with the in-process binding as the default deployment mode
/// and `POST /bss-products/v1/reference-watermarks` as the out-of-process
/// one. Both pass the same `reference_signal x post` gate.
#[async_trait]
pub trait WatermarkPosts: Send + Sync {
    /// Post one producer's complete set.
    ///
    /// # Errors
    /// The canonical projection of the authorization gate's refusal;
    /// `PRODUCER_UNREGISTERED` (403), `WATERMARK_REGRESSION` /
    /// `WATERMARK_CONFLICT` (409), or `WATERMARK_FUTURE` (400 on the wire).
    async fn post(
        &self,
        ctx: &SecurityContext,
        tenant_id: Uuid,
        post: WatermarkPost,
    ) -> Result<WatermarkAck, CanonicalError>;
}

/// Fail-closed default until a binding is wired, the sibling contracts'
/// posture: a producer that cannot post must not believe it did.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredWatermarkPosts;

#[async_trait]
impl WatermarkPosts for UnconfiguredWatermarkPosts {
    async fn post(
        &self,
        _ctx: &SecurityContext,
        _tenant_id: Uuid,
        _post: WatermarkPost,
    ) -> Result<WatermarkAck, CanonicalError> {
        Err(crate::increments::unconfigured_watermarks())
    }
}
