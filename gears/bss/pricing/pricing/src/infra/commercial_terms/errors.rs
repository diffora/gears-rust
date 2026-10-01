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
                error.dependency,
                error.to_string(),
                "UNCONFIGURED_DEPENDENCY",
            )
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

/// Keep definite Products refusals; transient contention and outages are unavailable.
pub(super) fn products(error: CanonicalError) -> CanonicalError {
    if crate::infra::reference_work::definite_refusal(&error) {
        error
    } else {
        crate::api::rest::authoring::support::registry_unavailable(&error)
    }
}
