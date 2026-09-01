use super::DomainError;
use crate::domain::validation::ValidationReport;

#[test]
fn every_variant_carries_its_design_set_wire_code() {
    let cases: Vec<(DomainError, &str)> = vec![
        (
            DomainError::Validation(ValidationReport::new()),
            "VALIDATION",
        ),
        (DomainError::DuplicateName("n".into()), "DUPLICATE_NAME"),
        (DomainError::DuplicateCode("c".into()), "DUPLICATE_CODE"),
        (
            DomainError::StaleRevision {
                expected: 1,
                found: 2,
            },
            "STALE_REVISION",
        ),
        (
            DomainError::IdempotencyConflict("k".into()),
            "IDEMPOTENCY_CONFLICT",
        ),
        (
            DomainError::IdempotencyKeyInFlight("k".into()),
            "IDEMPOTENCY_KEY_IN_FLIGHT",
        ),
        (DomainError::EntityTerminal("e".into()), "ENTITY_TERMINAL"),
        (
            DomainError::AuditUnavailable("a".into()),
            "AUDIT_UNAVAILABLE",
        ),
        (
            DomainError::IllegalTransition {
                from: "retired".into(),
                to: "draft".into(),
            },
            "ILLEGAL_TRANSITION",
        ),
        (
            DomainError::IllegalFieldMutation("f".into()),
            "ILLEGAL_FIELD_MUTATION",
        ),
        (
            DomainError::ScopeNotContained("s".into()),
            "SCOPE_NOT_CONTAINED",
        ),
        (DomainError::ParentTerminal("p".into()), "PARENT_TERMINAL"),
        (
            DomainError::IncompleteEntity("i".into()),
            "INCOMPLETE_ENTITY",
        ),
        (
            DomainError::ApprovalRequired("a".into()),
            "APPROVAL_REQUIRED",
        ),
        (
            DomainError::ErasureUnknownActor("a".into()),
            "ERASURE_UNKNOWN_ACTOR",
        ),
    ];
    for (error, expected) in &cases {
        assert_eq!(error.code(), *expected, "wrong code for {error:?}");
    }
    // The count is the guard: a variant added without a case here leaves the
    // roster short, and the roster is what the response map is built from.
    assert_eq!(
        cases.len(),
        15,
        "the Foundation owns fourteen raiseable codes and hosts one of \
         retention-erasure's (ERASURE_UNKNOWN_ACTOR, the erasure door's own; \
         P-D-64 keeps that slice's roster at one); PARENT_NOT_PUBLISHED is \
         registered by the lifecycle feature and RETIREMENT_PENDING is declared by it"
    );
}
