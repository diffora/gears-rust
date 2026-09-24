//! Rejections of the retained registry foundation.
use crate::domain::validation::ValidationReport;
use toolkit_macros::domain_model;

/// A registry operation rejection.
#[domain_model]
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("validation failed: {0}")]
    Validation(ValidationReport),
    #[error("stale revision: expected {expected}, found {found}")]
    StaleRevision {
        /// What the caller pinned.
        expected: i64,
        /// What the head actually carries.
        found: i64,
    },
    #[error("idempotency conflict on key {0}")]
    IdempotencyConflict(String),
    #[error("idempotency key in flight: {0}")]
    IdempotencyKeyInFlight(String),
    #[error("audit unavailable: {0}")]
    AuditUnavailable(String),
    #[error("usage type unresolved: {0}")]
    UsageTypeUnresolved(String),
    #[error("usage type unavailable: {0}")]
    UsageTypeUnavailable(String),
    #[error("unrecognized metering unit: {0}")]
    UnrecognizedUnit(String),
    #[error("incomplete meter declaration: {0}")]
    MeterDeclarationIncomplete(String),
}

impl DomainError {
    /// Stable machine-readable code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "VALIDATION",
            Self::StaleRevision { .. } => "STALE_REVISION",
            Self::IdempotencyConflict(_) => "IDEMPOTENCY_CONFLICT",
            Self::IdempotencyKeyInFlight(_) => "IDEMPOTENCY_KEY_IN_FLIGHT",
            Self::AuditUnavailable(_) => "AUDIT_UNAVAILABLE",
            Self::UsageTypeUnresolved(_) => "USAGE_TYPE_UNRESOLVED",
            Self::UsageTypeUnavailable(_) => "USAGE_TYPE_UNAVAILABLE",
            Self::UnrecognizedUnit(_) => "UNRECOGNIZED_UNIT",
            Self::MeterDeclarationIncomplete(_) => "METER_DECLARATION_INCOMPLETE",
        }
    }
}
