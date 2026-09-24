//! Retained domain error mapping census.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use crate::domain::{error::DomainError, validation::ValidationReport};
use std::collections::HashSet;
use toolkit::api::canonical_prelude::CanonicalError;

fn code_of(err: &CanonicalError) -> Option<&str> {
    match err {
        CanonicalError::Aborted { ctx, .. } => Some(ctx.reason.as_str()),
        CanonicalError::PermissionDenied { ctx, .. } => Some(ctx.reason.as_str()),
        CanonicalError::FailedPrecondition { ctx, .. } => {
            ctx.violations.first().map(|v| v.type_.as_str())
        }
        _ => None,
    }
}

fn declared_status_and_code(err: &DomainError) -> (u16, Option<&'static str>) {
    match err {
        DomainError::Validation(_)
        | DomainError::UsageTypeUnresolved(_)
        | DomainError::UnrecognizedUnit(_)
        | DomainError::MeterDeclarationIncomplete(_) => (400, Some(err.code())),
        DomainError::StaleRevision { .. }
        | DomainError::IdempotencyConflict(_)
        | DomainError::IdempotencyKeyInFlight(_) => (409, Some(err.code())),
        DomainError::AuditUnavailable(_) | DomainError::UsageTypeUnavailable(_) => (503, None),
    }
}

fn one_of_every_variant() -> Vec<DomainError> {
    let mut report = ValidationReport::new();
    report.violate("VALIDATION", "field", "invalid");
    vec![
        DomainError::Validation(report),
        DomainError::StaleRevision {
            expected: 1,
            found: 2,
        },
        DomainError::IdempotencyConflict("detail".to_owned()),
        DomainError::IdempotencyKeyInFlight("detail".to_owned()),
        DomainError::AuditUnavailable("detail".to_owned()),
        DomainError::UsageTypeUnresolved("detail".to_owned()),
        DomainError::UsageTypeUnavailable("detail".to_owned()),
        DomainError::UnrecognizedUnit("detail".to_owned()),
        DomainError::MeterDeclarationIncomplete("detail".to_owned()),
    ]
}
const DOMAIN_ERROR_VARIANTS: usize = 9;

#[test]
fn every_domain_error_variant_lands_in_its_declared_category() {
    let roster = one_of_every_variant();

    assert_eq!(
        roster.len(),
        DOMAIN_ERROR_VARIANTS,
        "the roster must carry one value of every variant; a variant added to `DomainError` and \
         to `declared_status_and_code` but not to the roster is a variant the ladder is not \
         checked on"
    );
    let distinct: HashSet<_> = roster.iter().map(std::mem::discriminant).collect();
    assert_eq!(
        distinct.len(),
        DOMAIN_ERROR_VARIANTS,
        "and one value **each**: a duplicate would satisfy the count while leaving a variant out"
    );

    for err in roster {
        let (expected_status, expected_code) = declared_status_and_code(&err);
        let wire_code = err.code();
        let name = format!("{err:?}");
        let canonical = CanonicalError::from(err);

        assert_eq!(
            canonical.status_code(),
            expected_status,
            "the ladder must answer {expected_status} for {name}"
        );
        assert_eq!(
            code_of(&canonical),
            expected_code,
            "the ladder must carry {expected_code:?} for {name}"
        );
        if let Some(code) = expected_code {
            // One deliberate exception: the request door's refusal carries
            // the CONSUMER'S discriminator on the wire (P-D-52 — pricing's
            // `Rejected` arm matches the violation type
            // `CATALOG_VERSION_REJECTED`), while `DomainError::code()` stays
            // the audit channel's `REQUEST_SOURCE_UNKNOWN`. For every other
            // variant the two are one string, and the assertion holds the
            // pair together so a second literal cannot drift in unnoticed.
            if wire_code == "REQUEST_SOURCE_UNKNOWN" {
                assert_eq!(
                    code, "CATALOG_VERSION_REJECTED",
                    "the request-source refusal must carry the consumer's discriminator"
                );
            } else {
                assert_eq!(
                    code, wire_code,
                    "the ladder's own code for {name} must be `DomainError::code()`'s, not a \
                     second literal"
                );
            }
        }
    }
}
