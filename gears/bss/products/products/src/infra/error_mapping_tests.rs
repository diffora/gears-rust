use std::collections::HashSet;

use toolkit::api::canonical_prelude::CanonicalError;

use crate::authz::labels;
use crate::domain::error::DomainError;
use crate::domain::validation::ValidationReport;

/// The wire code a mapped `CanonicalError` carries, read the way a consumer
/// would: `reason` on the 409/403 categories, the first precondition
/// violation's `type` on the 422-rendered-400 ones. `None` for the 503,
/// which the module doc says carries neither a code nor a resource type.
#[allow(
    clippy::match_same_arms,
    reason = "`Aborted`'s and `PermissionDenied`'s `ctx` are two different context types that \
              happen to share a `reason: String` field; the two arms read alike but cannot be \
              merged with `|` since the bound `ctx` would need one type across both."
)]
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

/// The status and wire code the ladder is contracted to answer for each
/// variant, transcribing `design/01-foundation.md` §3.3's Problem-response
/// line — not a second copy of the ladder's builders, so re-pointing an arm
/// from `aborted` to `precondition` changes the real answer and not this
/// one, which is what turns this test red.
///
/// Exhaustive by construction: a variant added to `DomainError` without a
/// line added here is a compile error, the same property `DomainError::code`
/// itself has.
fn declared_status_and_code(err: &DomainError) -> (u16, Option<&'static str>) {
    use DomainError as D;
    match err {
        D::Validation(_) => (400, Some("VALIDATION")),
        D::DuplicateName(_) => (409, Some("DUPLICATE_NAME")),
        D::DuplicateCode(_) => (409, Some("DUPLICATE_CODE")),
        D::StaleRevision { .. } => (409, Some("STALE_REVISION")),
        D::IdempotencyConflict(_) => (409, Some("IDEMPOTENCY_CONFLICT")),
        D::IdempotencyKeyInFlight(_) => (409, Some("IDEMPOTENCY_KEY_IN_FLIGHT")),
        D::EntityTerminal(_) => (409, Some("ENTITY_TERMINAL")),
        D::AuditUnavailable(_) => (503, None),
        D::IllegalTransition { .. } => (409, Some("ILLEGAL_TRANSITION")),
        D::IllegalFieldMutation(_) => (409, Some("ILLEGAL_FIELD_MUTATION")),
        D::ScopeNotContained(_) => (400, Some("SCOPE_NOT_CONTAINED")),
        D::ParentTerminal(_) => (409, Some("PARENT_TERMINAL")),
        D::IncompleteEntity(_) => (400, Some("INCOMPLETE_ENTITY")),
        D::ApprovalRequired(_) => (403, Some("APPROVAL_REQUIRED")),
        D::ErasureUnknownActor(_) => (400, Some("ERASURE_UNKNOWN_ACTOR")),
        D::CloneSourceDiscarded(_) => (409, Some("CLONE_SOURCE_DISCARDED")),
        // FailedPrecondition renders 400 on the wire; the discriminator the
        // consumer matches on is the violation TYPE (CATALOG_VERSION_REJECTED),
        // asserted by its own case below, while the audit channel carries the
        // domain code.
        D::RequestSourceUnknown(_) => (400, Some("CATALOG_VERSION_REJECTED")),
        D::IntentRequired(_) => (400, Some("INTENT_REQUIRED")),
        D::FreezeIncomplete(_) => (409, Some("FREEZE_INCOMPLETE")),
        D::VersionForcedIncomplete(_) => (409, Some("VERSION_FORCED_INCOMPLETE")),
        D::StagedEntityChanged(_) => (409, Some("STAGED_ENTITY_CHANGED")),
        // The 404 shape carries no code channel on the wire; the code rides
        // the audit row through `DomainError::code()`.
        D::CatalogVersionUnknown(_) => (404, None),
        D::ParticipantUnknown(_) => (403, Some("PARTICIPANT_UNKNOWN")),
    }
}

/// One value of every variant, in [`declared_status_and_code`]'s order.
fn one_of_every_variant() -> Vec<DomainError> {
    use DomainError as D;
    let d = || "detail".to_owned();
    let mut report = ValidationReport::new();
    report.violate(
        "VALIDATION",
        "name",
        "must contain a non-whitespace character",
    );
    vec![
        D::Validation(report),
        D::DuplicateName(d()),
        D::DuplicateCode(d()),
        D::StaleRevision {
            expected: 3,
            found: 4,
        },
        D::IdempotencyConflict(d()),
        D::IdempotencyKeyInFlight(d()),
        D::EntityTerminal(d()),
        D::AuditUnavailable(d()),
        D::IllegalTransition {
            from: "draft".to_owned(),
            to: "published".to_owned(),
        },
        D::IllegalFieldMutation(d()),
        D::ScopeNotContained(d()),
        D::ParentTerminal(d()),
        D::IncompleteEntity(d()),
        D::ApprovalRequired(d()),
        D::ErasureUnknownActor(d()),
        D::CloneSourceDiscarded(d()),
        D::RequestSourceUnknown(d()),
        D::IntentRequired(d()),
        D::FreezeIncomplete(d()),
        D::VersionForcedIncomplete(d()),
        D::StagedEntityChanged(d()),
        D::CatalogVersionUnknown(d()),
        D::ParticipantUnknown(d()),
    ]
}

/// **The count the roster and the enum must agree on.**
///
/// A literal because there is no way to ask the enum, and it is the half of
/// the gate [`declared_status_and_code`]'s exhaustive match cannot give: a
/// new variant makes that match fail to compile, and this makes the roster
/// that is *missing* the value fail the case. Bump it in the same edit that
/// adds the variant to both.
const DOMAIN_ERROR_VARIANTS: usize = 23;

/// Covers all 14 variants (§3.3's own count, `DomainError::code`'s own
/// exhaustiveness note): every one lands on the status the design ladder
/// names, and — where the design says the code is the attribution channel —
/// the mapped `Problem` carries `DomainError::code()`'s own value, not a
/// second literal that could drift from it.
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

/// `AUDIT_UNAVAILABLE` and `APPROVAL_REQUIRED` are the lone members of their
/// status classes among the fourteen — a 503 and a 403 in a ladder that is
/// otherwise 409s and 400s — which is exactly what makes each the likeliest
/// to be miscopied into the 409 block the census above would still catch,
/// but a dedicated assertion says so without needing the table read.
#[test]
fn audit_unavailable_is_503_and_approval_required_is_403() {
    let audit = CanonicalError::from(DomainError::AuditUnavailable(
        "audit row insert failed".to_owned(),
    ));
    assert_eq!(audit.status_code(), 503);
    assert_eq!(
        audit.resource_type(),
        None,
        "the audit plane is neither a Product nor a SKU refusing a write"
    );

    let approval = CanonicalError::from(DomainError::ApprovalRequired(
        "no satisfied approval record".to_owned(),
    ));
    assert_eq!(approval.status_code(), 403);
    assert_eq!(code_of(&approval), Some("APPROVAL_REQUIRED"));
}

/// The two containment refusals are `SKU`-resource — the module doc's
/// reason: a `Product` has no parent to check and no containment to prove,
/// so a `Product` door can never raise either.
#[test]
fn parent_terminal_and_scope_not_contained_carry_the_sku_resource_type() {
    let parent_terminal = CanonicalError::from(DomainError::ParentTerminal(
        "parent product is retired".to_owned(),
    ));
    let scope_not_contained = CanonicalError::from(DomainError::ScopeNotContained(
        "child region_scope is not a subset of the parent's".to_owned(),
    ));

    for err in [&parent_terminal, &scope_not_contained] {
        assert_eq!(err.resource_type(), Some(labels::SKU));
    }
}

/// A Foundation-generic refusal — one every door raises identically,
/// `ENTITY_TERMINAL` standing in for the class — lands on the default
/// `ProductResource` marker, distinguishing it from the two `SKU`-only
/// refusals above.
#[test]
fn a_foundation_generic_refusal_carries_the_product_resource_type_by_default() {
    let entity_terminal =
        CanonicalError::from(DomainError::EntityTerminal("head is retired".to_owned()));
    assert_eq!(entity_terminal.resource_type(), Some(labels::PRODUCT));
}

/// The three products-owned 422 codes stay wire **400** by design, not by
/// omission. `design/01-foundation.md` §3.3's "Status rendering — the 422s
/// in this set are architectural, not wire" section is explicit that a
/// transport override to 422 is *available* — `Http::status_code(422)` via
/// `.with_override(...)` reaches it, staying within `FailedPrecondition`'s
/// 4xx class — and was rejected as an **owner's call, 2026-08-27**, to keep
/// one wire shape per registry code (the code is the discriminator, not the
/// status). This test is what would catch a reintroduction of that
/// override: it goes red the day `error_mapping.rs` ships one of these
/// three at 422, which is exactly the day it should.
#[test]
fn the_products_owned_422_codes_stay_wire_400_by_design() {
    let mut report = ValidationReport::new();
    report.violate(
        "VALIDATION",
        "name",
        "must contain a non-whitespace character",
    );

    let validation = CanonicalError::from(DomainError::Validation(report));
    let scope_not_contained = CanonicalError::from(DomainError::ScopeNotContained(
        "child region_scope is not a subset of the parent's".to_owned(),
    ));
    let incomplete_entity = CanonicalError::from(DomainError::IncompleteEntity(
        "a required field is absent at the target state".to_owned(),
    ));
    let erasure_unknown_actor = CanonicalError::from(DomainError::ErasureUnknownActor(
        "the named principal resolves to no actor_ref in this tenant".to_owned(),
    ));

    for (err, code) in [
        (&validation, "VALIDATION"),
        (&scope_not_contained, "SCOPE_NOT_CONTAINED"),
        (&incomplete_entity, "INCOMPLETE_ENTITY"),
        (&erasure_unknown_actor, "ERASURE_UNKNOWN_ACTOR"),
    ] {
        assert_eq!(
            err.status_code(),
            400,
            "{code} is an architectural 422 (§3.3) that this gear renders as 400 by owner's \
             call, not by omission"
        );
        assert_eq!(code_of(err), Some(code));
    }
}
