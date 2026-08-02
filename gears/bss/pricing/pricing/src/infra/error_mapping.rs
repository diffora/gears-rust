//! The SINGLE authoritative `DomainError` → AIP-193 `CanonicalError` ladder.
//!
//! Both surfaces consume it: REST handlers via `?` / an explicit map, and the
//! in-process client via `.map_err(CanonicalError::from)`. A domain variant is
//! therefore assigned a canonical category — and an HTTP status — in exactly
//! one place, which is what keeps the wire contract the design set states from
//! drifting per handler.
//!
//! The wire codes are the names the design set uses verbatim
//! (`DUPLICATE_SCOPE_KEY`, `STALE_VERSION`, `IDEMPOTENCY_PAYLOAD_MISMATCH`,
//! `IDEMPOTENCY_KEY_IN_FLIGHT`, `OPEN_DRAFT_REVISION_EXISTS`,
//! `ROUNDING_POLICY_UNRESOLVED`, `PRECISION_EXCEEDED`,
//! `TIMESTAMP_PRECISION_EXCEEDED`, `AMOUNT_NEGATIVE`, `CURRENCY_INVALID`,
//! `FIXTURE_MISSING`, `PLAN_RETIRED_NO_SUCCESSOR`,
//! `GRANDFATHER_UNTIL_FORBIDDEN`); a consumer matches the category coarsely and
//! the code exactly.
//!
//! **The design set's 422s are architectural, not wire** (normative:
//! `design/01-foundation.md` §3.3). They say *unprocessable content*; the
//! platform's canonical family has no 422 category at all —
//! `FailedPrecondition`, `InvalidArgument` and `OutOfRange` all render **400**
//! (`toolkit_canonical_errors::CanonicalError::status_code`) — so each reaches
//! the wire as a 400 carrying its code, and the **code is the discriminator**,
//! not the status. Two rules follow and are honoured below: a rejection is
//! classified by what it is, so a retriable conflict on mutable state stays a
//! **409** rather than collapsing into the 400 bucket; and no route may declare
//! a 422 in its `OpenAPI` registration, because no path can produce one.

use toolkit::api::canonical_prelude::{CanonicalError, resource_error};

use crate::domain::error::DomainError;

#[resource_error(gts_id!("cf.bss.pricing.plan.v1~"))]
struct PlanResource;

impl From<DomainError> for CanonicalError {
    fn from(err: DomainError) -> Self {
        use DomainError as D;
        match err {
            // -- InvalidArgument (400) --
            D::InvalidRequest(detail) => PlanResource::invalid_argument()
                .with_constraint(detail)
                .create(),

            // -- FailedPrecondition -- the fail-closed publish rejections
            // (architectural 422, rendered 400; see the module note).
            D::RoundingPolicyUnresolved(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("price", detail, "ROUNDING_POLICY_UNRESOLVED")
                .create(),
            D::PrecisionExceeded(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("amount_minor", detail, "PRECISION_EXCEEDED")
                .create(),
            // The temporal sibling of the line above, and classified with it
            // rather than as a malformed argument: the instant parses, it is
            // simply finer than the quantum the catalog compares instants at
            // (D-144).
            D::TimestampPrecisionExceeded(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("timestamp", detail, "TIMESTAMP_PRECISION_EXCEEDED")
                .create(),
            D::AmountNegative(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("amount_minor", detail, "AMOUNT_NEGATIVE")
                .create(),
            D::CurrencyInvalid(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("currency", detail, "CURRENCY_INVALID")
                .create(),
            D::FixtureMissing(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("model_kind", detail, "FIXTURE_MISSING")
                .create(),
            D::LifecycleForbidden(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("lifecycle_state", detail, "LIFECYCLE_FORBIDDEN")
                .create(),
            // Narrowed out of the line above (D-146). It stays in the
            // precondition class because retirement really is state forbidding
            // the operation — what it must not stay is indistinguishable from
            // the refusals an operator can work around.
            D::PlanRetiredNoSuccessor(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("lifecycle_state", detail, "PLAN_RETIRED_NO_SUCCESSOR")
                .create(),
            // The `cohort` rule's sibling (D-147): one axis-conditioned field,
            // one code. It is a publish refusal about the row's eligibility
            // class, not a malformed request, which is why it lands here and no
            // longer on the generic bad-request answer.
            D::GrandfatherUntilForbidden(detail) => PlanResource::failed_precondition()
                .with_precondition_violation(
                    "grandfather_until",
                    detail,
                    "GRANDFATHER_UNTIL_FORBIDDEN",
                )
                .create(),

            // -- Aborted (409) -- conflicts the caller can resolve and retry.
            D::DuplicateScopeKey(detail) => PlanResource::aborted(detail)
                .with_reason("DUPLICATE_SCOPE_KEY")
                .create(),
            D::StaleVersion(detail) => PlanResource::aborted(detail)
                .with_reason("STALE_VERSION")
                .create(),
            D::IdempotencyPayloadMismatch(detail) => PlanResource::aborted(detail)
                .with_reason("IDEMPOTENCY_PAYLOAD_MISMATCH")
                .create(),
            // A conflict on mutable state that a retry resolves, so it keeps the
            // 409 the three above hold rather than collapsing into the 400
            // bucket (D-143): retry, and the answer is the stored response or
            // the mismatch refusal.
            D::IdempotencyKeyInFlight(detail) => PlanResource::aborted(detail)
                .with_reason("IDEMPOTENCY_KEY_IN_FLIGHT")
                .create(),
            // A uniqueness conflict on the plan's one draft slot (D-146), the
            // `DUPLICATE_SCOPE_KEY` class rather than a state-machine edge —
            // which is why it is here and not beside `LIFECYCLE_FORBIDDEN`.
            D::OpenDraftRevisionExists(detail) => PlanResource::aborted(detail)
                .with_reason("OPEN_DRAFT_REVISION_EXISTS")
                .create(),

            // -- The aggregate validation envelope (architectural 422, rendered 400) --
            //
            // Every blocking violation is carried as its own precondition
            // violation, so the author sees the whole remediation list in one
            // response. Advisory warnings are deliberately NOT rendered here:
            // this envelope exists only on the rejecting path, and a warning
            // must reach the author on the succeeding path too — it rides the
            // validation report on the response body, not the error.
            D::ValidationFailed(report) => {
                let mut violations = report.violations.into_iter();
                let Some(first) = violations.next() else {
                    // A rejection with nothing to remediate is a bug in the
                    // pipeline, not a client error: reporting it as a
                    // precondition failure would tell the author to fix
                    // something the response does not name.
                    return Self::internal(
                        "pricing: publish validation failed with an empty report",
                    )
                    .create();
                };
                let mut builder = PlanResource::failed_precondition().with_precondition_violation(
                    first.subject,
                    first.detail,
                    first.code,
                );
                for violation in violations {
                    builder = builder.with_precondition_violation(
                        violation.subject,
                        violation.detail,
                        violation.code,
                    );
                }
                builder.create()
            }

            // -- NotFound (404) -- absent and scoped-out are the same answer.
            D::NotFound { subject, id } => {
                PlanResource::not_found(format!("{subject} {id} not found"))
                    .with_resource(id)
                    .create()
            }

            // -- Unavailable (503) -- fail closed, retry later.
            //
            // The detail stays server-side: an operator needs to know the
            // registry is down, a caller only needs to know to retry.
            D::CatalogVersionUnavailable(detail) => {
                tracing::error!(detail, "catalog-version registry unavailable");
                CanonicalError::service_unavailable().create()
            }
            D::ReadModelUnavailable(detail) => {
                tracing::error!(detail, "read model unavailable");
                CanonicalError::service_unavailable().create()
            }

            // -- Internal (500) --
            D::Internal(detail) => CanonicalError::internal(format!("pricing: {detail}")).create(),
        }
    }
}

#[cfg(test)]
#[path = "error_mapping_tests.rs"]
mod error_mapping_tests;
