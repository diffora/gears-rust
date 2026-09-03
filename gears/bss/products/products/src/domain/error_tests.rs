use super::DomainError;
use crate::domain::validation::ValidationReport;

/// One value of every `DomainError` variant, paired with the wire code
/// `DomainError::code` must answer for it.
///
/// A function rather than a `let` inside the test, for the reason
/// `infra::error_mapping_tests::declared_status_and_code` is one: the roster
/// is long enough that holding it inline puts the test over
/// `clippy::too_many_lines`, which this crate denies.
fn wire_code_roster() -> Vec<(DomainError, &'static str)> {
    vec![
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
            DomainError::ParentNotPublished("p".into()),
            "PARENT_NOT_PUBLISHED",
        ),
        (
            DomainError::RetirementPending("r".into()),
            "RETIREMENT_PENDING",
        ),
        (
            DomainError::ScheduleStaleApproval("s".into()),
            "SCHEDULE_STALE_APPROVAL",
        ),
        (
            DomainError::ReplacedByNotPublished("r".into()),
            "REPLACED_BY_NOT_PUBLISHED",
        ),
        (
            DomainError::RetirementLeadTime("e".into()),
            "RETIREMENT_LEAD_TIME",
        ),
        (
            DomainError::CascadeConfirmationRequired("c".into()),
            "CASCADE_CONFIRMATION_REQUIRED",
        ),
        (DomainError::EolDisabled("e".into()), "EOL_DISABLED"),
        (
            DomainError::IncompleteEntity("i".into()),
            "INCOMPLETE_ENTITY",
        ),
        (
            DomainError::PrimaryCategoryRequired("p".into()),
            "PRIMARY_CATEGORY_REQUIRED",
        ),
        (DomainError::StaleLiveOp("s".into()), "STALE_LIVE_OP"),
        (
            DomainError::ApprovalRequired("a".into()),
            "APPROVAL_REQUIRED",
        ),
        (
            DomainError::ErasureUnknownActor("a".into()),
            "ERASURE_UNKNOWN_ACTOR",
        ),
        (
            DomainError::CloneSourceDiscarded("s".into()),
            "CLONE_SOURCE_DISCARDED",
        ),
        (
            DomainError::RequestSourceUnknown("s".into()),
            "REQUEST_SOURCE_UNKNOWN",
        ),
        (DomainError::IntentRequired("s".into()), "INTENT_REQUIRED"),
        (
            DomainError::FreezeIncomplete("s".into()),
            "FREEZE_INCOMPLETE",
        ),
        (
            DomainError::VersionForcedIncomplete("s".into()),
            "VERSION_FORCED_INCOMPLETE",
        ),
        (
            DomainError::StagedEntityChanged("s".into()),
            "STAGED_ENTITY_CHANGED",
        ),
        (
            DomainError::CatalogVersionUnknown("s".into()),
            "CATALOG_VERSION_UNKNOWN",
        ),
        (
            DomainError::ParticipantUnknown("s".into()),
            "PARTICIPANT_UNKNOWN",
        ),
        (
            DomainError::BulkDependencyFailed("s".into()),
            "BULK_DEPENDENCY_FAILED",
        ),
        (
            DomainError::PromotionIdentityConflict("s".into()),
            "PROMOTION_IDENTITY_CONFLICT",
        ),
        (
            DomainError::PromotionDirtyHead("s".into()),
            "PROMOTION_DIRTY_HEAD",
        ),
        (
            DomainError::BulkOverrideUnacknowledged("s".into()),
            "BULK_OVERRIDE_UNACKNOWLEDGED",
        ),
        (DomainError::BulkLimit("s".into()), "BULK_LIMIT"),
        (
            DomainError::ProducerUnregistered("s".into()),
            "PRODUCER_UNREGISTERED",
        ),
        (
            DomainError::WatermarkRegression("s".into()),
            "WATERMARK_REGRESSION",
        ),
        (
            DomainError::WatermarkConflict("s".into()),
            "WATERMARK_CONFLICT",
        ),
        (DomainError::WatermarkFuture("s".into()), "WATERMARK_FUTURE"),
        // -- The ten this roster was short of, added 2026-09-02 on strand A's
        // A-OWED-11. Measured `DomainError::code`'s arms against this array:
        // 51 against 41, and the shortfall predates every strand's work. --
        (
            DomainError::MeterDeclarationIncomplete("m".into()),
            "METER_DECLARATION_INCOMPLETE",
        ),
        (
            DomainError::UnrecognizedUnit("u".into()),
            "UNRECOGNIZED_UNIT",
        ),
        (DomainError::UnitDeprecated("u".into()), "UNIT_DEPRECATED"),
        (
            DomainError::UnitDelistBlocked("u".into()),
            "UNIT_DELIST_BLOCKED",
        ),
        (
            DomainError::PlanTierRetireBlocked("p".into()),
            "PLAN_TIER_RETIRE_BLOCKED",
        ),
        (
            DomainError::AccountingCodeDelistBlocked("a".into()),
            "ACCOUNTING_CODE_DELIST_BLOCKED",
        ),
        (
            DomainError::SelfApprovalForbidden("s".into()),
            "SELF_APPROVAL_FORBIDDEN",
        ),
        (
            DomainError::ApprovalSuperseded("a".into()),
            "APPROVAL_SUPERSEDED",
        ),
        (
            DomainError::DuplicateCategoryName("d".into()),
            "DUPLICATE_CATEGORY_NAME",
        ),
        (DomainError::TaxonomyCycle("t".into()), "TAXONOMY_CYCLE"),
        (
            DomainError::ContentPiiBlocked("s".into()),
            "CONTENT_PII_BLOCKED",
        ),
        (DomainError::MetadataLimit("m".into()), "METADATA_LIMIT"),
    ]
}

#[test]
fn every_variant_carries_its_design_set_wire_code() {
    let cases = wire_code_roster();
    for (error, expected) in &cases {
        assert_eq!(error.code(), *expected, "wrong code for {error:?}");
    }
    // The count is a guard, and the WEAKER of the two this enum has. Nothing
    // ties this array to `DomainError` itself: adding a variant forces a
    // `code()` arm, because that match is exhaustive, but it does not force a
    // case here — which is how the roster sat **ten** short of the enum from
    // before any strand's work until 2026-09-02 (strand A's A-OWED-11 measured
    // it; the ten are marked in the array above).
    //
    // **The strong guard is `infra::error_mapping_tests`**: its
    // `declared_status_and_code` is an exhaustive `match` over `DomainError`,
    // so a new variant does not compile until it is handled there, and its
    // `DOMAIN_ERROR_VARIANTS` pins the count. **That constant and this literal
    // are the same number and must move together** — they disagreed 51 to 41
    // until today. Read that file's own note before changing either.
    assert_eq!(
        cases.len(),
        53,
        "the Foundation owns fourteen raiseable codes and hosts two guests \
         (retention-erasure's ERASURE_UNKNOWN_ACTOR, P-D-64 keeping that \
         roster at one, and the clone door's CLONE_SOURCE_DISCARDED, \
         P-D-75's mint), and the catalog-version feature declares its seven \
         (dod-cv-error-taxonomy): REQUEST_SOURCE_UNKNOWN, INTENT_REQUIRED, \
         FREEZE_INCOMPLETE, VERSION_FORCED_INCOMPLETE, STAGED_ENTITY_CHANGED, \
         CATALOG_VERSION_UNKNOWN and PARTICIPANT_UNKNOWN, and bulk-promotion \
         its five (dod-bulk-errors): BULK_DEPENDENCY_FAILED, \
         PROMOTION_IDENTITY_CONFLICT, PROMOTION_DIRTY_HEAD, \
         BULK_OVERRIDE_UNACKNOWLEDGED and BULK_LIMIT, and reference-signal \
         the watermark door's four: PRODUCER_UNREGISTERED, \
         WATERMARK_REGRESSION, WATERMARK_CONFLICT and WATERMARK_FUTURE; \
         PARENT_NOT_PUBLISHED, RETIREMENT_PENDING, SCHEDULE_STALE_APPROVAL, \
         REPLACED_BY_NOT_PUBLISHED, RETIREMENT_LEAD_TIME, \
         CASCADE_CONFIRMATION_REQUIRED and EOL_DISABLED are 04's seven \
         (dod-lifecycle-errors, P-D-96); taxonomy-attributes declares \
         PRIMARY_CATEGORY_REQUIRED and STALE_LIVE_OP, the first registered publish validator \
         that belongs to neither 04 nor 05"
    );
}
