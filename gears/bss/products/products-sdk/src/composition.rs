//! The bundle composition-completed signal (`PRD` §9.2
//! `contract-bundle-composition-signal`, P-D-14) — the third of §9.2's four
//! inbound machine contracts and `inst-sdk-surface`'s sixth row. Plan-price
//! tells the registry that a `bundle` SKU has been composed; the registry
//! clears `compositionPending` through a **`system_signal` approval subject**
//! and a re-publish (`design/06` `inst-cc-clear`), never through an exemption
//! from the gate, and emits `SkuCompositionCleared` as its own outbound event.
//! The inbound signal keeps the PRD's name, `BundleCompositionCompleted`.
//!
//! The door — `POST /bss-products/v1/skus/{id}/composition-clears`, spending
//! `sku × publish` — is this contract's out-of-process binding; the default
//! deployment resolves this trait from `ClientHub` in-process (P-D-15) and
//! runs the identical gate and core.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-sdk-surface:p1

use async_trait::async_trait;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// What one composition signal did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionOutcome {
    /// The head was clean: the flag is cleared and the SKU re-published at
    /// `published_version`.
    Cleared {
        /// The version the clearing publish minted.
        published_version: i64,
    },
    /// The head was dirty, or an approval was open on it: the signal is
    /// kept and re-evaluated by the activation runner, the flag stays raised,
    /// and `on` names the blocker (`dirty_head` / `open_approval`).
    Held {
        /// The blocker's name.
        on: String,
    },
    /// Nothing was written because this `signal_ref` already ran: the
    /// stored answer, replayed.
    Replayed,
    /// Nothing was written because there was nothing to clear — not a
    /// bundle, or the flag already down (P-D-159; the door said `replayed`
    /// for this until then, and a consumer looking for the run it named
    /// found none).
    Nothing,
}

/// The composition-completed signal, resolved from `ClientHub`.
///
/// Every method returns [`CanonicalError`]; the codes a caller may see are the
/// [`crate::errors::ErrorCode`] vocabulary's.
#[async_trait]
pub trait CompositionSignals: Send + Sync {
    /// Announce that `sku_id` has been composed. `signal_ref` is the
    /// signaller's own idempotency handle: the same ref twice is one clear.
    ///
    /// # Errors
    /// The canonical projection of the door's refusal — a SKU outside the
    /// caller's scope is the canonical `NotFound`, an authorization refusal
    /// its canonical projection.
    async fn composed(
        &self,
        ctx: &SecurityContext,
        sku_id: Uuid,
        signal_ref: Uuid,
    ) -> Result<CompositionOutcome, CanonicalError>;
}
