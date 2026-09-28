//! Authoritative mapping from domain rejections to canonical errors.
use crate::domain::error::DomainError;
use crate::domain::validation::ValidationReport;
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

/// A dependency that did not answer: logged for the operator, a 503 for the caller, carrying the
/// code in its detail where the design names one.
fn unavailable(dependency: &str, detail: &str, code: Option<&str>) -> CanonicalError {
    tracing::error!(dependency, detail, "bss-products: dependency unavailable");
    let builder = CanonicalError::service_unavailable();
    match code {
        Some(code) => builder.with_detail(format!("{code}: {detail}")).create(),
        None => builder.create(),
    }
}

/// P-D-207: the usage-type catalog refused the caller. The detail is the gear's own sentence; the
/// collector's PDP reason stays in the operator log (`infra::usage_types`).
fn catalog_denied(detail: &str) -> CanonicalError {
    tracing::warn!(
        detail,
        "bss-products: the usage-type catalog refused the caller"
    );
    denied("USAGE_TYPE_FORBIDDEN")
}

/// A validation report: a catalog's refusal of the caller is a 403 and a catalog outage a 503,
/// whichever stage reported them; otherwise every violation, in the order collected (P-D-202).
fn validation(report: &ValidationReport) -> CanonicalError {
    // A catalog that refused the caller is a permission, never a field fix (P-D-207).
    if report
        .violations()
        .iter()
        .any(|v| v.code == "USAGE_TYPE_FORBIDDEN")
    {
        return CanonicalError::from(DomainError::UsageTypeForbidden(
            "the usage-type catalog refused this caller".into(),
        ));
    }
    // Outages are retryable even when reported by the pure publish validator.
    if report
        .violations()
        .iter()
        .any(|v| v.code == "USAGE_TYPE_UNAVAILABLE")
    {
        return CanonicalError::from(DomainError::UsageTypeUnavailable(
            "the usage-type catalog did not answer".into(),
        ));
    }
    let mut violations = report.violations().iter();
    let Some(first) = violations.next() else {
        return CanonicalError::internal("products: validation failed with an empty report")
            .create();
    };
    let mut builder = ProductResource::failed_precondition().with_precondition_violation(
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

impl From<DomainError> for CanonicalError {
    fn from(err: DomainError) -> Self {
        use DomainError as D;
        match err {
            D::Conflict { code, detail } => aborted(detail, code),
            D::Forbidden { code, .. } => denied(code),
            D::NotFound { what, id } => ProductResource::not_found(format!("{what} {id}"))
                .with_resource(id.to_string())
                .create(),
            D::StaleUnit { generation } => precondition(
                "unit",
                &format!("the unit was refreshed; review generation {generation}"),
                "UNIT_STALE",
            ),
            // InvalidSubmit and ApplyRefused enter through Validation/Conflict
            // so their static subject-specific codes and fields survive intact.
            // Database errors are mapped only after the transaction retry loop.
            D::Approval(r) => match r.code {
                "SOD_VIOLATION" | "NOT_SUBMITTER" => denied(r.code),
                "NOTE_REQUIRED" => precondition("note", "a reject needs a note", "NOTE_REQUIRED"),
                "VALIDATION" => precondition("items", &r.detail, "VALIDATION"),
                "GENERATION_MISMATCH" => {
                    precondition("generation", &r.detail, "GENERATION_MISMATCH")
                }
                "DB" | "STORE" => CanonicalError::internal("products: approval store").create(),
                other => aborted(r.detail, other),
            },
            D::Validation(report) => validation(&report),
            D::StaleRevision { expected, found } => aborted(
                format!("expected {expected}, found {found}"),
                "STALE_REVISION",
            ),
            D::IdempotencyConflict(detail) => aborted(detail, "IDEMPOTENCY_CONFLICT"),
            D::IdempotencyKeyInFlight(detail) => aborted(detail, "IDEMPOTENCY_KEY_IN_FLIGHT"),
            D::AuditUnavailable(detail) => unavailable("audit_log", &detail, None),
            D::UsageTypeUnresolved(detail) => {
                precondition("usage_type_ref", &detail, "USAGE_TYPE_UNRESOLVED")
            }
            D::UsageTypeUnavailable(detail) => unavailable(
                "usage_type_catalog",
                &detail,
                Some("USAGE_TYPE_UNAVAILABLE"),
            ),
            D::UsageTypeForbidden(detail) => catalog_denied(&detail),
            D::UsageUnavailable(detail) => {
                unavailable("sku_usage_port", &detail, Some("USAGE_UNAVAILABLE"))
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
