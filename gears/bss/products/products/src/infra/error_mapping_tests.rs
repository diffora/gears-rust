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
        DomainError::StaleUnit { .. }
        | DomainError::Validation(_)
        | DomainError::UsageTypeUnresolved(_)
        | DomainError::UnrecognizedUnit(_)
        | DomainError::MeterDeclarationIncomplete(_) => (400, Some(err.code())),
        DomainError::Conflict { .. }
        | DomainError::StaleRevision { .. }
        | DomainError::IdempotencyConflict(_)
        | DomainError::IdempotencyKeyInFlight(_) => (409, Some(err.code())),
        DomainError::Forbidden { .. } => (403, Some(err.code())),
        DomainError::NotFound { .. } => (404, None),
        DomainError::Approval(r) => match r.code {
            "SOD_VIOLATION" | "NOT_SUBMITTER" => (403, Some(r.code)),
            "NOTE_REQUIRED" | "VALIDATION" | "GENERATION_MISMATCH" => (400, Some(r.code)),
            "DB" | "STORE" => (500, None),
            _ => (409, Some(r.code)),
        },
        DomainError::AuditUnavailable(_) | DomainError::UsageTypeUnavailable(_) => (503, None),
    }
}

fn one_of_every_variant() -> Vec<DomainError> {
    let mut report = ValidationReport::new();
    report.violate("VALIDATION", "field", "invalid");
    vec![
        DomainError::Validation(report),
        DomainError::Conflict {
            code: "SKU_TYPE_FROZEN",
            detail: "references".into(),
        },
        DomainError::Forbidden {
            code: "NOT_SUBMITTER",
            detail: "hidden".into(),
        },
        DomainError::NotFound {
            what: "sku",
            id: uuid::Uuid::new_v4(),
        },
        DomainError::Approval(crate::domain::error::ApprovalRefusal {
            code: "DUPLICATE_VOTE",
            detail: "vote".into(),
        }),
        DomainError::StaleUnit { generation: 2 },
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
const DOMAIN_ERROR_VARIANTS: usize = 14;

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

#[test]
fn approval_refusals_preserve_their_status_code_and_generation() {
    use crate::domain::error::ApprovalRefusal;
    for (code, status) in [
        ("SOD_VIOLATION", 403),
        ("NOT_SUBMITTER", 403),
        ("NOTE_REQUIRED", 400),
        ("VALIDATION", 400),
        ("GENERATION_MISMATCH", 400),
        ("DB", 500),
        ("STORE", 500),
        ("UNIT_ALREADY_DECIDED", 409),
        ("DUPLICATE_VOTE", 409),
        ("UNIT_CONTENDED", 409),
        ("ROW_LOCKED_PENDING", 409),
        ("APPLY_REFUSED", 409),
    ] {
        let err = CanonicalError::from(DomainError::Approval(ApprovalRefusal {
            code,
            detail: "generation 7".into(),
        }));
        assert_eq!(err.status_code(), status, "{code}");
        assert_eq!(code_of(&err), if status == 500 { None } else { Some(code) });
    }
    let err = CanonicalError::from(DomainError::StaleUnit { generation: 7 });
    assert!(
        matches!(err, CanonicalError::FailedPrecondition {ctx,..} if ctx.violations[0].description.contains('7'))
    );
}

#[test]
fn actual_approval_errors_keep_custom_codes_fields_and_details() {
    use bss_approval::ApprovalError as A;
    let invalid = CanonicalError::from(DomainError::from(A::InvalidSubmit {
        code: "USAGE_NEEDS_METER",
        field: "unit".into(),
        detail: "a meter is required".into(),
    }));
    assert_eq!(invalid.status_code(), 400);
    assert_eq!(code_of(&invalid), Some("USAGE_NEEDS_METER"));
    assert!(
        matches!(invalid,CanonicalError::FailedPrecondition {ctx,..} if ctx.violations[0].subject=="unit" && ctx.violations[0].description=="a meter is required")
    );
    let apply = CanonicalError::from(DomainError::from(A::ApplyRefused {
        code: "SKU_REFERENCED",
        detail: "one live reference".into(),
    }));
    assert_eq!(apply.status_code(), 409);
    assert_eq!(code_of(&apply), Some("SKU_REFERENCED"));
    for (error, status, code) in [
        (A::SodViolation, 403, Some("SOD_VIOLATION")),
        (A::NotSubmitter, 403, Some("NOT_SUBMITTER")),
        (A::AlreadyDecided, 409, Some("UNIT_ALREADY_DECIDED")),
        (A::DuplicateVote, 409, Some("DUPLICATE_VOTE")),
        (A::Contended, 409, Some("UNIT_CONTENDED")),
        (
            A::Locked {
                item_type: "sku".into(),
                item_id: uuid::Uuid::new_v4(),
            },
            409,
            Some("ROW_LOCKED_PENDING"),
        ),
        (A::NoteRequired, 400, Some("NOTE_REQUIRED")),
        (A::Empty, 400, Some("VALIDATION")),
        (
            A::GenerationMismatch {
                seen: 1,
                current: 7,
            },
            400,
            Some("GENERATION_MISMATCH"),
        ),
        (A::Store("private detail".into()), 500, None),
        (
            A::Db(sea_orm::DbErr::Custom("private driver detail".into())),
            500,
            None,
        ),
    ] {
        let canonical = CanonicalError::from(DomainError::from(error));
        assert_eq!(canonical.status_code(), status);
        assert_eq!(code_of(&canonical), code);
        if code == Some("GENERATION_MISMATCH") {
            assert!(
                matches!(canonical,CanonicalError::FailedPrecondition {ctx,..} if ctx.violations[0].description.contains('7'))
            );
        }
    }
}
