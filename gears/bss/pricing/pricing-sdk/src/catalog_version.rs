//! `CatalogVersion` and the pin-eligibility frontier.
//!
//! A consumer resolves the published read model at exactly one **pin-eligible**
//! `CatalogVersion` for the duration of a resolution or rating run. Whether a
//! version is pin-eligible is a version-level, prefix-closed predicate
//! (`design/01-foundation.md` §4.4, D-101 + D-114): the version is committed,
//! every subject row it projects is warm-complete, and every earlier version is
//! itself pin-eligible. No consumer can evaluate that for itself, so the gear
//! materializes the frontier and publishes it (D-136) — this type is what it
//! serves.
//!
//! **What a pin buys today, stated because the predicate above does not say
//! it.** The frontier is real: a version reaches it only once every subject the
//! gear knows it published is warm. What a consumer cannot yet do is *resolve*
//! anything at that pin — there is no read-model resolution surface in this
//! gear at all, and the delta a version freezes deliberately omits the facts
//! whose slices are unbuilt (the `PriceWindow` intervals and the derived
//! coverage end, the GA-gate flags, the registry `sellable` flag, the grant set
//! and the Slice-6 cross-boundary pair). So a version is pinnable and its
//! payload is incomplete, and a reader is entitled to know which of the six
//! sellability predicates it can evaluate from one: not (1), not (5), not (6).

use chrono::{DateTime, Utc};

/// A committed catalog version. Monotonic per tenant; the registry (Product &
/// SKU) is the sole incrementer, so this crate only ever carries a value the
/// catalog received, never one it derived.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CatalogVersion(u64);

impl CatalogVersion {
    /// Wrap a committed version number.
    #[must_use]
    pub const fn new(version: u64) -> Self {
        Self(version)
    }

    /// The underlying version number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The tenant's current pin-eligibility frontier: the newest version a consumer
/// may pin, and when it last advanced.
///
/// `advanced_at` is the referent of the ≤ 5s pin-lag rule and of the
/// `pricing.readmodel.pin_eligibility_overdue` alarm — a stuck version holds the
/// frontier, which is exactly what the alarm signals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinFrontier {
    /// The newest pin-eligible version. Advanced only forward, and only by the
    /// projector inside the transaction completing the frontier's next version
    /// in order — a later version's completion never advances it past a gap.
    pub catalog_version: CatalogVersion,
    /// UTC instant the frontier last advanced.
    pub advanced_at: DateTime<Utc>,
}
