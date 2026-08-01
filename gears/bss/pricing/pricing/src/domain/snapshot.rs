//! `pricingSnapshotRef` — the composite reference consumers pin.
//!
//! A snapshot ref is the catalog-side identifiers sufficient for the manifest's
//! `pricingSnapshotRef`: the version ref, the resolved price ids, and the
//! evaluation-policy version. It is stamped at publish with a **pending**
//! version ref and finalized to the committed `CatalogVersion` when
//! `CatalogVersionPublished` fires — **immutable thereafter**.
//!
//! Two properties are normative rather than convenient
//! (`design/01-foundation.md` §4.4):
//!
//! - **The normative composition system of record is Tariffs.** What this gear stamps is the
//!   *aligned entry*, and it MUST NOT diverge from the Tariffs composition. A
//!   second, subtly different composition here would not be a duplicate — it
//!   would be a second answer to "what was this subscriber priced from".
//! - **Posted invoice periods never re-query mutable catalog rows.** They
//!   resolve through the pin, which is why the finalize step is one-way: a ref
//!   that could be re-pointed would let a later publish change what an already
//!   posted period was priced from.

use bss_pricing_sdk::CatalogVersion;
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::error::DomainError;

/// The version half of a snapshot ref: pending at publish, committed once the
/// registry has assigned a version.
///
/// Two states rather than an `Option<CatalogVersion>` plus a handle string: the
/// registry batches approved publishes (D-47), so between the publish commit
/// and `CatalogVersionPublished` the ref genuinely *has* an identity — the
/// pending handle — and callers must be able to carry and match it. An
/// `Option` would model that interval as "no version yet" and lose the handle
/// that resolves it.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionRef {
    /// The registry's pending handle, stamped at publish.
    Pending(String),
    /// The committed version. Terminal.
    Committed(CatalogVersion),
}

impl VersionRef {
    /// Has this ref been finalized?
    #[must_use]
    pub const fn is_committed(&self) -> bool {
        matches!(self, Self::Committed(_))
    }

    /// The committed version, when there is one.
    #[must_use]
    pub const fn committed(&self) -> Option<CatalogVersion> {
        match self {
            Self::Pending(_) => None,
            Self::Committed(version) => Some(*version),
        }
    }

    /// The pending handle, while the ref is still pending.
    #[must_use]
    pub fn pending_ref(&self) -> Option<&str> {
        match self {
            Self::Pending(handle) => Some(handle),
            Self::Committed(_) => None,
        }
    }

    /// Move `Pending -> Committed`, exactly once.
    ///
    /// Consuming rather than `&mut self` on purpose: the caller cannot keep the
    /// pending ref it just finalized, so there is no path where an old handle
    /// and its committed version are both live.
    ///
    /// # Errors
    ///
    /// [`DomainError::LifecycleForbidden`] when the ref is already committed.
    /// Re-finalizing is not idempotent-and-harmless: a duplicate
    /// `CatalogVersionPublished` carrying a different version would silently
    /// re-point a pin that posted periods already resolved through.
    pub fn finalize(self, version: CatalogVersion) -> Result<Self, DomainError> {
        match self {
            Self::Pending(_) => Ok(Self::Committed(version)),
            Self::Committed(existing) => Err(DomainError::LifecycleForbidden(format!(
                "version ref already committed at {}; refused re-finalize to {}",
                existing.get(),
                version.get()
            ))),
        }
    }
}

/// The composite reference pinned on charges and `BillableItem`s.
///
/// Fields are private and [`PricingSnapshotRef::finalize`] is the only way to
/// move the version ref, so the immutability the design set states is a
/// property of the type rather than a convention every call site has to keep.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PricingSnapshotRef {
    version_ref: VersionRef,
    price_ids: Vec<Uuid>,
    evaluation_policy_version: String,
}

impl PricingSnapshotRef {
    /// Stamp a snapshot ref at publish.
    #[must_use]
    pub fn new(
        version_ref: VersionRef,
        price_ids: Vec<Uuid>,
        evaluation_policy_version: String,
    ) -> Self {
        Self {
            version_ref,
            price_ids,
            evaluation_policy_version,
        }
    }

    /// The version ref.
    #[must_use]
    pub const fn version_ref(&self) -> &VersionRef {
        &self.version_ref
    }

    /// The resolved price ids this snapshot froze.
    #[must_use]
    pub fn price_ids(&self) -> &[Uuid] {
        &self.price_ids
    }

    /// The evaluation-policy version the resolved rows are evaluated under.
    #[must_use]
    pub fn evaluation_policy_version(&self) -> &str {
        &self.evaluation_policy_version
    }

    /// Finalize the version ref on `CatalogVersionPublished`.
    ///
    /// # Errors
    ///
    /// [`DomainError::LifecycleForbidden`] when the ref is already committed;
    /// see [`VersionRef::finalize`].
    pub fn finalize(self, version: CatalogVersion) -> Result<Self, DomainError> {
        Ok(Self {
            version_ref: self.version_ref.finalize(version)?,
            price_ids: self.price_ids,
            evaluation_policy_version: self.evaluation_policy_version,
        })
    }
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod snapshot_tests;
