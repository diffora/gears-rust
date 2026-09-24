//! Authoritative mapping from domain rejections to canonical errors.
use crate::domain::error::DomainError;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};

#[resource_error(gts_id!("cf.bss.products.product.v1~"))]
struct ProductResource;

/// Shared canonical error constructor.
#[must_use]
pub fn precondition(field: &'static str, detail: &str, code: &'static str) -> CanonicalError {
    ProductResource::failed_precondition()
        .with_precondition_violation(field, detail, code)
        .create()
}

/// Shared canonical error constructor.
#[must_use]
pub fn aborted(detail: String, code: &'static str) -> CanonicalError {
    ProductResource::aborted(detail).with_reason(code).create()
}

/// Shared canonical error constructor.
#[must_use]
pub fn denied(code: &'static str) -> CanonicalError {
    ProductResource::permission_denied()
        .with_reason(code)
        .create()
}

impl From<DomainError> for CanonicalError {
    fn from(err: DomainError) -> Self {
        use DomainError as D;
        match err {
            D::Validation(report) => {
                let mut violations = report.violations().iter();
                let Some(first) = violations.next() else {
                    return CanonicalError::internal(
                        "products: validation failed with an empty report",
                    )
                    .create();
                };
                let mut builder = ProductResource::failed_precondition()
                    .with_precondition_violation(
                        first.subject.clone(),
                        first.detail.clone(),
                        first.code,
                    );
                for violation in violations {
                    builder = builder.with_precondition_violation(
                        violation.subject.clone(),
                        violation.detail.clone(),
                        violation.code,
                    );
                }
                builder.create()
            }
            D::StaleRevision { expected, found } => aborted(
                format!("expected {expected}, found {found}"),
                "STALE_REVISION",
            ),
            D::IdempotencyConflict(detail) => aborted(detail, "IDEMPOTENCY_CONFLICT"),
            D::IdempotencyKeyInFlight(detail) => aborted(detail, "IDEMPOTENCY_KEY_IN_FLIGHT"),
            D::AuditUnavailable(detail) => {
                tracing::error!(
                    dependency = "audit_log",
                    detail,
                    "bss-products: dependency unavailable"
                );
                CanonicalError::service_unavailable().create()
            }
            D::UsageTypeUnresolved(detail) => {
                precondition("usage_type_ref", &detail, "USAGE_TYPE_UNRESOLVED")
            }
            D::UsageTypeUnavailable(detail) => {
                tracing::error!(
                    dependency = "usage_type_catalog",
                    detail,
                    "bss-products: dependency unavailable"
                );
                CanonicalError::service_unavailable().create()
            }
            D::UnrecognizedUnit(detail) => precondition("meter", &detail, "UNRECOGNIZED_UNIT"),
            D::MeterDeclarationIncomplete(detail) => {
                precondition("meter", &detail, "METER_DECLARATION_INCOMPLETE")
            }
        }
    }
}

#[cfg(test)]
#[path = "error_mapping_tests.rs"]
mod error_mapping_tests;
