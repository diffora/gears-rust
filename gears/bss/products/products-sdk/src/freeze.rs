//! The freeze-acknowledgment contract **with its release half**
//! (`PRD` §9.2 `contract-freeze-ack`, P-D-18, P-D-48) — the fourth inbound
//! machine contract of §9.2 and `inst-sdk-surface`'s fifth row: the doors
//! (`POST /bss-products/v1/catalog-versions/{id}/acks` and `…/releases`,
//! `design/06` `inst-fz-ack` / `inst-fz-liveness`) shipped first, and this is
//! the typed client a participant resolves from `ClientHub` in the default
//! in-process deployment (P-D-15), the REST doors being its out-of-process
//! binding. Both bindings spend the same `catalog_version × ack` / `× release`
//! grants under the participant's own identity (P-D-67), which is why every
//! method takes the caller's [`SecurityContext`].
//!
//! # The release is a duty, not a courtesy
//!
//! A participant that holds no more live references to a `CatalogVersion`
//! records that through `release`; snapshot GC is gated on every registered
//! participant having released (`design/10` `inst-rt-gc`). A participant that
//! acks and never releases pins the version's snapshot for the tenant's
//! lifetime, which is the loud state the protocol intends rather than a leak
//! it hides.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-sdk-surface:p1

use async_trait::async_trait;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

/// What a freeze edge answered: the participant's row after the act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreezeEdgeReceipt {
    /// The participant the edge was recorded for — the caller's registered
    /// name, echoed.
    pub participant: String,
    /// The participant's ledger state after the edge: `acked`, `released`,
    /// or the state it already held when the edge was a no-op.
    pub state: String,
    /// Whether this call moved the ledger. `false` is the idempotent
    /// re-delivery of an edge already recorded — not a refusal.
    pub changed: bool,
}

/// The participant-side freeze protocol, resolved from `ClientHub`.
///
/// Every method returns [`CanonicalError`]: the gear's single
/// `From<DomainError>` ladder classifies a refusal once, and this port adds
/// no second classification. The codes a caller may see are the
/// [`crate::errors::ErrorCode`] vocabulary's — `PARTICIPANT_UNKNOWN` for a
/// name outside the registered set, `CATALOG_VERSION_UNKNOWN` for a version
/// the tenant never published, `ILLEGAL_TRANSITION` for an edge the ledger's
/// state machine does not admit.
#[async_trait]
pub trait FreezeAcks: Send + Sync {
    /// Acknowledge that the participant has frozen its content against
    /// `catalog_version_id` — the edge `freezeComplete` counts.
    ///
    /// # Errors
    /// The canonical projection of the door's refusal; see the trait doc.
    async fn ack(
        &self,
        ctx: &SecurityContext,
        catalog_version_id: i64,
        participant: &str,
    ) -> Result<FreezeEdgeReceipt, CanonicalError>;

    /// Record that the participant holds no more live references to
    /// `catalog_version_id` — the release half (P-D-18).
    ///
    /// # Errors
    /// The canonical projection of the door's refusal; see the trait doc.
    async fn release(
        &self,
        ctx: &SecurityContext,
        catalog_version_id: i64,
        participant: &str,
    ) -> Result<FreezeEdgeReceipt, CanonicalError>;
}
