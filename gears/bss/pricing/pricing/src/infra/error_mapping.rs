//! Shared transport errors and their canonical wire mapping.

use toolkit::api::canonical_prelude::{CanonicalError, resource_error};

#[resource_error(gts_id!("cf.bss.pricing.plan.v1~"))]
struct PricingResource;

/// Errors still constructed by the retained request plumbing.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    /// A request body or header cannot be interpreted.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// Internal serialization or middleware wiring failure.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<DomainError> for CanonicalError {
    fn from(error: DomainError) -> Self {
        match error {
            DomainError::InvalidRequest(detail) => PricingResource::invalid_argument()
                .with_constraint(detail)
                .create(),
            DomainError::Internal(detail) => {
                CanonicalError::internal(format!("pricing: {detail}")).create()
            }
        }
    }
}
