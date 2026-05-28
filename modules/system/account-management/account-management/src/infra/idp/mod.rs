//! Infrastructure-layer glue for the optional
//! [`account_management_sdk::IdpPluginClient`] plugin.
//!
//! AM can boot without an `IdP` adapter present — dev deployments and
//! tests do not need one. The services store the provisioner as
//! `Arc<dyn IdpPluginClient>` directly; this module contributes
//! the [`NoopIdpProvider`] fallback wired in when no plugin resolves
//! from `ClientHub`.
//!
//! The fallback inherits every trait default, so every mutating call
//! surfaces as the category-appropriate `UnsupportedOperation` —
//! deployments without an `IdP` plugin keep booting and the call sites
//! see a consistent error envelope for both tenant and user
//! operations.

use account_management_sdk::IdpPluginClient;
use async_trait::async_trait;

mod lazy;
pub use lazy::LazyIdpProvider;

/// No-op `IdP` provider plugin: inherits the trait's
/// `UnsupportedOperation` defaults for every tenant / user operation.
/// Used when AM boots without an `IdP` plugin AND when
/// [`LazyIdpProvider`] cannot resolve a plugin under `required = false`.
#[derive(Debug, Default, Clone)]
pub struct NoopIdpProvider;

#[async_trait]
impl IdpPluginClient for NoopIdpProvider {}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "noop_tests.rs"]
mod noop_tests;
