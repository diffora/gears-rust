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
        D::ParentNotPublished(_) => (409, Some("PARENT_NOT_PUBLISHED")),
        D::RetirementPending(_) => (409, Some("RETIREMENT_PENDING")),
        D::ScheduleStaleApproval(_) => (409, Some("SCHEDULE_STALE_APPROVAL")),
        D::ReplacedByNotPublished(_) => (400, Some("REPLACED_BY_NOT_PUBLISHED")),
        D::RetirementLeadTime(_) => (400, Some("RETIREMENT_LEAD_TIME")),
        D::CascadeConfirmationRequired(_) => (400, Some("CASCADE_CONFIRMATION_REQUIRED")),
        D::EolDisabled(_) => (400, Some("EOL_DISABLED")),
        D::IncompleteEntity(_) => (400, Some("INCOMPLETE_ENTITY")),
        D::PrimaryCategoryRequired(_) => (400, Some("PRIMARY_CATEGORY_REQUIRED")),
        D::StaleLiveOp(_) => (409, Some("STALE_LIVE_OP")),
        // 03's meter refusals: 422 architectural, 400 on the wire (the slice's
        // own Problem-responses note) — the precondition shape.
        D::MeterDeclarationIncomplete(_) => (400, Some("METER_DECLARATION_INCOMPLETE")),
        D::UnrecognizedUnit(_) => (400, Some("UNRECOGNIZED_UNIT")),
        D::UnitDeprecated(_) => (400, Some("UNIT_DEPRECATED")),
        // The three delist blocks are 409s in the same note.
        D::UnitDelistBlocked(_) => (409, Some("UNIT_DELIST_BLOCKED")),
        D::PlanTierRetireBlocked(_) => (409, Some("PLAN_TIER_RETIRE_BLOCKED")),
        D::AccountingCodeDelistBlocked(_) => (409, Some("ACCOUNTING_CODE_DELIST_BLOCKED")),
        D::ApprovalRequired(_) => (403, Some("APPROVAL_REQUIRED")),
        D::SelfApprovalForbidden(_) => (403, Some("SELF_APPROVAL_FORBIDDEN")),
        D::ApprovalSuperseded(_) => (409, Some("APPROVAL_SUPERSEDED")),
        D::DuplicateCategoryName(_) => (409, Some("DUPLICATE_CATEGORY_NAME")),
        D::TaxonomyCycle(_) => (400, Some("TAXONOMY_CYCLE")),
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
        D::BulkDependencyFailed(_) => (400, Some("BULK_DEPENDENCY_FAILED")),
        D::PromotionIdentityConflict(_) => (409, Some("PROMOTION_IDENTITY_CONFLICT")),
        D::PromotionDirtyHead(_) => (409, Some("PROMOTION_DIRTY_HEAD")),
        D::BulkOverrideUnacknowledged(_) => (400, Some("BULK_OVERRIDE_UNACKNOWLEDGED")),
        D::BulkLimit(_) => (409, Some("BULK_LIMIT")),
        D::ProducerUnregistered(_) => (403, Some("PRODUCER_UNREGISTERED")),
        D::WatermarkRegression(_) => (409, Some("WATERMARK_REGRESSION")),
        D::WatermarkConflict(_) => (409, Some("WATERMARK_CONFLICT")),
        D::WatermarkFuture(_) => (400, Some("WATERMARK_FUTURE")),
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
        D::ParentNotPublished(d()),
        D::RetirementPending(d()),
        D::ScheduleStaleApproval(d()),
        D::ReplacedByNotPublished(d()),
        D::RetirementLeadTime(d()),
        D::CascadeConfirmationRequired(d()),
        D::EolDisabled(d()),
        D::IncompleteEntity(d()),
        D::PrimaryCategoryRequired(d()),
        D::StaleLiveOp(d()),
        D::ApprovalRequired(d()),
        D::SelfApprovalForbidden(d()),
        D::ApprovalSuperseded(d()),
        D::DuplicateCategoryName(d()),
        D::TaxonomyCycle(d()),
        // The six 03 variants the roster never carried. Each has had an arm
        // in `declared_status_and_code` since it landed, so the exhaustive
        // match compiled and the *ladder* went unchecked for all six — the
        // count below was wrong by six, not by one.
        D::MeterDeclarationIncomplete(d()),
        D::UnrecognizedUnit(d()),
        D::UnitDeprecated(d()),
        D::UnitDelistBlocked(d()),
        D::PlanTierRetireBlocked(d()),
        D::AccountingCodeDelistBlocked(d()),
        D::ErasureUnknownActor(d()),
        D::CloneSourceDiscarded(d()),
        D::RequestSourceUnknown(d()),
        D::IntentRequired(d()),
        D::FreezeIncomplete(d()),
        D::VersionForcedIncomplete(d()),
        D::StagedEntityChanged(d()),
        D::CatalogVersionUnknown(d()),
        D::ParticipantUnknown(d()),
        D::BulkDependencyFailed(d()),
        D::PromotionIdentityConflict(d()),
        D::PromotionDirtyHead(d()),
        D::BulkOverrideUnacknowledged(d()),
        D::BulkLimit(d()),
        D::ProducerUnregistered(d()),
        D::WatermarkRegression(d()),
        D::WatermarkConflict(d()),
        D::WatermarkFuture(d()),
    ]
}

/// **The count the roster and the enum must agree on.**
///
/// A literal because there is no way to ask the enum, and it is the half of
/// the gate [`declared_status_and_code`]'s exhaustive match cannot give: a
/// new variant makes that match fail to compile, and this makes the roster
/// that is *missing* the value fail the case. Bump it in the same edit that
/// adds the variant to both.
///
/// **It read 35 against 41 real variants until 2026-09-02**, and the gap was
/// invisible in exactly the way this constant exists to prevent: six
/// `03`-owned variants had arms in [`declared_status_and_code`] — so the
/// exhaustive match compiled — and no roster entry, so the ladder was never
/// checked on any of them. Bumping the literal by one per added variant
/// keeps a pre-existing shortfall forever; the only safe move is to
/// re-derive it against the enum.
/// **Paired with `domain::error_tests`'s own `cases.len()` literal.** The two
/// count the same enum and must move together; they disagreed 51 to 41 until
/// 2026-09-02, when strand A's A-OWED-11 measured the sibling ten short. This
/// file's guard is the strong one — `declared_status_and_code`'s match is
/// exhaustive, so a new variant does not compile until it is handled — and the
/// sibling's is a roster nothing ties to the enum. Move both.
const DOMAIN_ERROR_VARIANTS: usize = 51;

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
