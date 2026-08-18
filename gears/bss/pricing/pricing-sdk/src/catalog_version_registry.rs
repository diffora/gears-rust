//! The `CatalogVersion` registry contract (`CatalogVersionRegistryV1`).
//!
//! The Product & SKU registry is the **sole** incrementer of `CatalogVersion`:
//! this gear requests addressability at publish and is told, later, which
//! committed version its publish landed in. The registry MAY batch approved
//! publishes, so the request returns a **pending** ref and the commit arrives
//! asynchronously — the shape the design set fixes (`design/01-foundation.md`
//! §3.6, D-47).
//!
//! The contract lives here, in the catalog's own SDK, because the registry gear
//! has no code in this repository yet. That is a temporary asymmetry, not a
//! claim of ownership: when the registry publishes its own SDK this trait
//! becomes an adapter over it. What must not happen in the meantime is the
//! catalog inventing versions locally — it would become a second incrementer,
//! and two incrementers make `CatalogVersion` unordered.
//!
//! The default [`UnconfiguredCatalogVersionRegistryV1`] is fail-closed: with no
//! registry wired, publish stops rather than producing rows no consumer can
//! address.

use async_trait::async_trait;
use toolkit_security::SecurityContext;

use crate::catalog_version::CatalogVersion;

/// A registry-side acknowledgement that a publish will be assigned a version.
///
/// The `request_id` is the catalog's own idempotency handle: re-requesting with
/// the same id after a crash must not enqueue a second publish for versioning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingVersionRef {
    /// The catalog-supplied request id this pending ref answers.
    pub request_id: String,
    /// The registry's handle for the pending assignment, carried on
    /// `PlanPublished` and replaced by the committed version when
    /// `CatalogVersionPublished` fires.
    pub pending_ref: String,
}

/// A registry failure. Semantic — never an HTTP status here; the gear maps it
/// to its fail-closed publish rejection.
#[derive(Debug, thiserror::Error)]
pub enum CatalogVersionRegistryError {
    /// No registry client is wired.
    #[error("no catalog-version registry configured")]
    Unconfigured,
    /// The registry could not be reached.
    #[error("registry unreachable: {0}")]
    Unreachable(String),
    /// The registry refused the request (an unknown SKU, a closed version).
    #[error("registry rejected the request: {0}")]
    Rejected(String),
    /// Anything else, preserved verbatim for the operator.
    #[error("internal: {0}")]
    Internal(String),
}

/// The `CatalogVersion` increment contract
/// (`cpt-cf-bss-pricing-contract-registry-catalogversion`).
#[async_trait]
pub trait CatalogVersionRegistryV1: Send + Sync {
    /// Request addressability for a committed publish.
    ///
    /// Idempotent on `request_id`: a retry after a crash returns the same
    /// pending ref rather than queueing a second assignment.
    ///
    /// # Errors
    /// [`CatalogVersionRegistryError`] when no registry is configured, it is
    /// unreachable, or it refuses the request. Every case blocks the publish
    /// from becoming consumer-visible; none of them is retried into a locally
    /// invented version.
    async fn request_version(
        &self,
        ctx: &SecurityContext,
        request_id: &str,
    ) -> Result<PendingVersionRef, CatalogVersionRegistryError>;

    /// Resolve a pending ref to its committed version, or `None` while the
    /// registry has not yet committed the batch it belongs to.
    ///
    /// # Errors
    /// [`CatalogVersionRegistryError`] when the registry is unavailable. A
    /// pending ref that stays unresolved past the batching SLO is an alarm, not
    /// an error here — the caller decides that, since only it knows how long
    /// the ref has been outstanding.
    async fn committed_version(
        &self,
        ctx: &SecurityContext,
        pending_ref: &str,
    ) -> Result<Option<CatalogVersion>, CatalogVersionRegistryError>;
}

/// Fail-safe default until the registry gear is wired: every request fails, so
/// no publish becomes addressable and none is silently assigned a version this
/// gear made up. Mirrors the sibling ledger's `UnconfiguredRateProviderV1`.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredCatalogVersionRegistryV1;

#[async_trait]
impl CatalogVersionRegistryV1 for UnconfiguredCatalogVersionRegistryV1 {
    async fn request_version(
        &self,
        _ctx: &SecurityContext,
        _request_id: &str,
    ) -> Result<PendingVersionRef, CatalogVersionRegistryError> {
        Err(CatalogVersionRegistryError::Unconfigured)
    }

    async fn committed_version(
        &self,
        _ctx: &SecurityContext,
        _pending_ref: &str,
    ) -> Result<Option<CatalogVersion>, CatalogVersionRegistryError> {
        Err(CatalogVersionRegistryError::Unconfigured)
    }
}
