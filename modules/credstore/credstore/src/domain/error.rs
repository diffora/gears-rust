use std::time::Duration;

use modkit_macros::domain_model;
use thiserror::Error;

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[domain_model]
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DomainError {
    #[error("invalid secret reference: {detail}")]
    InvalidSecretRef { detail: String },
    #[error("secret not found")]
    NotFound,
    #[error("secret already exists")]
    Conflict,
    #[error("version precondition failed")]
    VersionConflict,
    #[error("invalid precondition: {detail}")]
    InvalidPrecondition { detail: String },
    #[error("unsupported sharing transition: {detail}")]
    UnsupportedTransition { detail: String },
    #[error("access denied")]
    AccessDenied {
        #[source]
        cause: Option<BoxError>,
    },
    #[error("service unavailable: {detail}")]
    ServiceUnavailable {
        detail: String,
        retry_after: Option<Duration>,
        #[source]
        cause: Option<BoxError>,
    },
    #[error("internal error")]
    Internal {
        diagnostic: String,
        #[source]
        cause: Option<BoxError>,
    },
}

impl DomainError {
    #[must_use]
    pub fn internal(diagnostic: impl Into<String>) -> Self {
        Self::Internal {
            diagnostic: diagnostic.into(),
            cause: None,
        }
    }
}
