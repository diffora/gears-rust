//! Canonical failures at the commercial provider boundary.
use crate::{authz::AuthzError, infra::storage::RepoError};
use toolkit_canonical_errors::CanonicalError;

#[toolkit_canonical_errors::resource_error("gts.cf.bss.pricing.acceptance.v1~")]
struct AcceptanceResource;

/// Named missing dependencies are configuration errors, never commercial refusals.
#[derive(Debug, thiserror::Error)]
#[error("unconfigured dependency: {dependency}")]
pub struct UnconfiguredDependency {
    /// Required provider contract.
    pub dependency: &'static str,
}
impl From<UnconfiguredDependency> for CanonicalError {
    fn from(error: UnconfiguredDependency) -> Self {
        AcceptanceResource::failed_precondition()
            .with_precondition_violation(
                "UNCONFIGURED_DEPENDENCY",
                error.dependency,
                error.to_string(),
            )
            .create()
    }
}

/// Pending fulfilment/hold boundary, replaced by Task 6 before public release.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("NotYetAvailable: {operation}")]
pub struct NotYetAvailable {
    /// Method whose implementation is pending.
    pub operation: &'static str,
}
impl From<NotYetAvailable> for CanonicalError {
    fn from(error: NotYetAvailable) -> Self {
        AcceptanceResource::unimplemented(error.to_string())
            .with_resource(error.operation)
            .create()
    }
}

pub(super) fn authorization(error: AuthzError) -> CanonicalError {
    match error {
        AuthzError::Denied(attempt) => AcceptanceResource::permission_denied()
            .with_reason(attempt.reason)
            .create(),
        AuthzError::Unavailable(reason) => {
            tracing::warn!(%reason, "commercial authorization unavailable");
            CanonicalError::service_unavailable()
                .with_detail(format!("AuthZResolverApi: {reason}"))
                .create()
        }
    }
}

pub(super) fn storage(error: RepoError) -> CanonicalError {
    match error {
        RepoError::Conflict { code } => {
            AcceptanceResource::aborted(code).with_reason(code).create()
        }
        RepoError::CorruptRow(reason) => {
            tracing::error!(%reason, "invalid stored commercial receipt");
            CanonicalError::internal("invalid stored commercial receipt").create()
        }
        error => {
            tracing::warn!(%error, "commercial storage unavailable");
            CanonicalError::service_unavailable()
                .with_detail("PricingStorage: receipt storage unavailable")
                .create()
        }
    }
}
