//! Outbound ports the publish engine depends on.
//!
//! The `CatalogVersion` registry is the one external actor the publish path
//! cannot proceed without: the Product & SKU registry is the **sole**
//! incrementer, and a publish that cannot request addressability has produced
//! nothing a consumer can pin. The contract itself lives in the SDK
//! (`bss_pricing_sdk::catalog_version_registry`) so the registry gear can
//! implement it without depending on this crate; this module is where the
//! engine resolves it.

pub use bss_pricing_sdk::catalog_version_registry::{
    CatalogVersionRegistryError, CatalogVersionRegistryV1, PendingVersionRef,
    UnconfiguredCatalogVersionRegistryV1,
};

use crate::domain::error::DomainError;

/// Map a registry failure onto the gear's rejection vocabulary.
///
/// Every registry failure lands on the same fail-closed answer: the publish does
/// not become addressable. The distinction between "unreachable" and "rejected"
/// is diagnostic, not behavioural — neither produces a version, and inventing a
/// local one would make this gear a second incrementer.
///
/// Deliberately a named function rather than a `From` impl. `DomainError` is a
/// `Clone + Eq` value type carried into responses and compared in tests, so it
/// cannot hold a boxed source and the chain flattens here whatever the shape;
/// making the conversion explicit at least keeps it visible at the call site
/// instead of happening silently through a `?`.
#[must_use]
pub fn registry_failure(err: &CatalogVersionRegistryError) -> DomainError {
    DomainError::CatalogVersionUnavailable(err.to_string())
}
