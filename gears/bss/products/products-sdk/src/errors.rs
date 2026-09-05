//! The error-code vocabulary — `inst-sdk-surface`'s eighth row, as
//! `features/consumer-contracts.md`'s `dod-sdk-surface` states it: **a
//! documented vocabulary, not a second error type on the port**. Every SDK
//! method returns `CanonicalError`; what this enum adds is the closed set of
//! `code` values a consumer may match on, one variant per code the gear's
//! `DomainError` declares (`design/01` §3.3 plus every slice's own §3.x
//! taxonomy), so a rename is a **breaking** change here as the PRD §9 policy
//! says it is. The gear's own test suite holds this roster against
//! `DomainError`'s in both directions, so a code added to the gear without a
//! variant here fails **in the change that added it**.
//!
//! The one thing this module does not do is classify: which status a code
//! rides, which channel carries it (`context.reason`, the audit row) and what
//! a refusal means are the gear's `From<DomainError>` ladder's, and a second
//! classification on this port would be a second place for a rejection to be
//! categorised.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-sdk-surface:p1

/// One registered refusal code. The wire spelling is [`ErrorCode::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ErrorCode {
    /// `ACCOUNTING_CODE_DELIST_BLOCKED`.
    AccountingCodeDelistBlocked,
    /// `ACCOUNTING_CODE_DEPRECATED`.
    AccountingCodeDeprecated,
    /// `ACCOUNTING_CODE_REQUIRED`.
    AccountingCodeRequired,
    /// `ACCOUNTING_CODE_UNKNOWN`.
    AccountingCodeUnknown,
    /// `APPROVAL_REQUIRED`.
    ApprovalRequired,
    /// `APPROVAL_SUPERSEDED`.
    ApprovalSuperseded,
    /// `APPROVER_ROLE_REQUIRED`.
    ApproverRoleRequired,
    /// `APPROVER_SCOPE_EXCEEDED`.
    ApproverScopeExceeded,
    /// `AUDIT_UNAVAILABLE`.
    AuditUnavailable,
    /// `BREAKGLASS_CORRECTION_DISABLED`.
    BreakglassCorrectionDisabled,
    /// `BREAKGLASS_EXPIRED`.
    BreakglassExpired,
    /// `BREAKGLASS_WRITE_FORBIDDEN`.
    BreakglassWriteForbidden,
    /// `BULK_DEPENDENCY_FAILED`.
    BulkDependencyFailed,
    /// `BULK_LIMIT`.
    BulkLimit,
    /// `BULK_OVERRIDE_UNACKNOWLEDGED`.
    BulkOverrideUnacknowledged,
    /// `BUNDLE_OVERRIDE_REQUIRED`.
    BundleOverrideRequired,
    /// `CASCADE_CONFIRMATION_REQUIRED`.
    CascadeConfirmationRequired,
    /// `CATALOG_VERSION_UNKNOWN`.
    CatalogVersionUnknown,
    /// `CATEGORY_REFERENCED`.
    CategoryReferenced,
    /// `CLONE_SOURCE_DISCARDED`.
    CloneSourceDiscarded,
    /// `CONTENT_PII_BLOCKED`.
    ContentPiiBlocked,
    /// `CORRECTION_APPROVAL_OPEN`.
    CorrectionApprovalOpen,
    /// `CORRECTION_DIRTY_HEAD`.
    CorrectionDirtyHead,
    /// `CORRECTION_REFERENCED`.
    CorrectionReferenced,
    /// `CORRECTION_SIGNAL_AVAILABLE`.
    CorrectionSignalAvailable,
    /// `DECISION_ALREADY_RECORDED`.
    DecisionAlreadyRecorded,
    /// `DEFINITION_IN_USE`.
    DefinitionInUse,
    /// `DUPLICATE_CATEGORY_NAME`.
    DuplicateCategoryName,
    /// `DUPLICATE_CODE`.
    DuplicateCode,
    /// `DUPLICATE_NAME`.
    DuplicateName,
    /// `ENTITY_TERMINAL`.
    EntityTerminal,
    /// `EOL_DISABLED`.
    EolDisabled,
    /// `ERASURE_UNKNOWN_ACTOR`.
    ErasureUnknownActor,
    /// `FREEZE_INCOMPLETE`.
    FreezeIncomplete,
    /// `IDEMPOTENCY_CONFLICT`.
    IdempotencyConflict,
    /// `IDEMPOTENCY_KEY_IN_FLIGHT`.
    IdempotencyKeyInFlight,
    /// `ILLEGAL_FIELD_MUTATION`.
    IllegalFieldMutation,
    /// `ILLEGAL_TRANSITION`.
    IllegalTransition,
    /// `INCOMPLETE_ENTITY`.
    IncompleteEntity,
    /// `INTENT_REQUIRED`.
    IntentRequired,
    /// `METADATA_LIMIT`.
    MetadataLimit,
    /// `METER_DECLARATION_INCOMPLETE`.
    MeterDeclarationIncomplete,
    /// `PARENT_NOT_PUBLISHED`.
    ParentNotPublished,
    /// `PARENT_TERMINAL`.
    ParentTerminal,
    /// `PARTICIPANT_UNKNOWN`.
    ParticipantUnknown,
    /// `PLAN_TIER_DEPRECATED`.
    PlanTierDeprecated,
    /// `PLAN_TIER_RETIRE_BLOCKED`.
    PlanTierRetireBlocked,
    /// `PLAN_TIER_UNKNOWN`.
    PlanTierUnknown,
    /// `PRIMARY_CATEGORY_REQUIRED`.
    PrimaryCategoryRequired,
    /// `PRODUCER_RETIREMENT_WOULD_FREE`.
    ProducerRetirementWouldFree,
    /// `PRODUCER_SET_EMPTY_FORBIDDEN`.
    ProducerSetEmptyForbidden,
    /// `PRODUCER_UNREGISTERED`.
    ProducerUnregistered,
    /// `PROMOTION_DIRTY_HEAD`.
    PromotionDirtyHead,
    /// `PROMOTION_IDENTITY_CONFLICT`.
    PromotionIdentityConflict,
    /// `READ_MODEL_OVERLOADED`.
    ReadModelOverloaded,
    /// `REPLACED_BY_NOT_PUBLISHED`.
    ReplacedByNotPublished,
    /// `REQUEST_SOURCE_UNKNOWN`.
    RequestSourceUnknown,
    /// `RETIREMENT_LEAD_TIME`.
    RetirementLeadTime,
    /// `RETIREMENT_PENDING`.
    RetirementPending,
    /// `SCHEDULE_STALE_APPROVAL`.
    ScheduleStaleApproval,
    /// `SCOPE_NOT_CONTAINED`.
    ScopeNotContained,
    /// `SELF_APPROVAL_FORBIDDEN`.
    SelfApprovalForbidden,
    /// `SKU_TYPE_UNKNOWN`.
    SkuTypeUnknown,
    /// `STAGED_ENTITY_CHANGED`.
    StagedEntityChanged,
    /// `STALE_CATEGORY_TOKEN`.
    StaleCategoryToken,
    /// `STALE_LIVE_OP`.
    StaleLiveOp,
    /// `STALE_REVISION`.
    StaleRevision,
    /// `TAXONOMY_CYCLE`.
    TaxonomyCycle,
    /// `UNIT_DELIST_BLOCKED`.
    UnitDelistBlocked,
    /// `UNIT_DEPRECATED`.
    UnitDeprecated,
    /// `UNRECOGNIZED_UNIT`.
    UnrecognizedUnit,
    /// `USAGE_TYPE_UNAVAILABLE`.
    UsageTypeUnavailable,
    /// `USAGE_TYPE_UNRESOLVED`.
    UsageTypeUnresolved,
    /// `VALIDATION`.
    Validation,
    /// `VERSION_FORCED_INCOMPLETE`.
    VersionForcedIncomplete,
    /// `WATERMARK_CONFLICT`.
    WatermarkConflict,
    /// `WATERMARK_FUTURE`.
    WatermarkFuture,
    /// `WATERMARK_REGRESSION`.
    WatermarkRegression,
}

impl ErrorCode {
    /// Every registered code, in the vocabulary's canonical (alphabetical)
    /// order. **78** at this revision; the gear's roster test pins the
    /// number too, so the two counts move together.
    pub const ALL: &'static [ErrorCode] = &[
        ErrorCode::AccountingCodeDelistBlocked,
        ErrorCode::AccountingCodeDeprecated,
        ErrorCode::AccountingCodeRequired,
        ErrorCode::AccountingCodeUnknown,
        ErrorCode::ApprovalRequired,
        ErrorCode::ApprovalSuperseded,
        ErrorCode::ApproverRoleRequired,
        ErrorCode::ApproverScopeExceeded,
        ErrorCode::AuditUnavailable,
        ErrorCode::BreakglassCorrectionDisabled,
        ErrorCode::BreakglassExpired,
        ErrorCode::BreakglassWriteForbidden,
        ErrorCode::BulkDependencyFailed,
        ErrorCode::BulkLimit,
        ErrorCode::BulkOverrideUnacknowledged,
        ErrorCode::BundleOverrideRequired,
        ErrorCode::CascadeConfirmationRequired,
        ErrorCode::CatalogVersionUnknown,
        ErrorCode::CategoryReferenced,
        ErrorCode::CloneSourceDiscarded,
        ErrorCode::ContentPiiBlocked,
        ErrorCode::CorrectionApprovalOpen,
        ErrorCode::CorrectionDirtyHead,
        ErrorCode::CorrectionReferenced,
        ErrorCode::CorrectionSignalAvailable,
        ErrorCode::DecisionAlreadyRecorded,
        ErrorCode::DefinitionInUse,
        ErrorCode::DuplicateCategoryName,
        ErrorCode::DuplicateCode,
        ErrorCode::DuplicateName,
        ErrorCode::EntityTerminal,
        ErrorCode::EolDisabled,
        ErrorCode::ErasureUnknownActor,
        ErrorCode::FreezeIncomplete,
        ErrorCode::IdempotencyConflict,
        ErrorCode::IdempotencyKeyInFlight,
        ErrorCode::IllegalFieldMutation,
        ErrorCode::IllegalTransition,
        ErrorCode::IncompleteEntity,
        ErrorCode::IntentRequired,
        ErrorCode::MetadataLimit,
        ErrorCode::MeterDeclarationIncomplete,
        ErrorCode::ParentNotPublished,
        ErrorCode::ParentTerminal,
        ErrorCode::ParticipantUnknown,
        ErrorCode::PlanTierDeprecated,
        ErrorCode::PlanTierRetireBlocked,
        ErrorCode::PlanTierUnknown,
        ErrorCode::PrimaryCategoryRequired,
        ErrorCode::ProducerRetirementWouldFree,
        ErrorCode::ProducerSetEmptyForbidden,
        ErrorCode::ProducerUnregistered,
        ErrorCode::PromotionDirtyHead,
        ErrorCode::PromotionIdentityConflict,
        ErrorCode::ReadModelOverloaded,
        ErrorCode::ReplacedByNotPublished,
        ErrorCode::RequestSourceUnknown,
        ErrorCode::RetirementLeadTime,
        ErrorCode::RetirementPending,
        ErrorCode::ScheduleStaleApproval,
        ErrorCode::ScopeNotContained,
        ErrorCode::SelfApprovalForbidden,
        ErrorCode::SkuTypeUnknown,
        ErrorCode::StagedEntityChanged,
        ErrorCode::StaleCategoryToken,
        ErrorCode::StaleLiveOp,
        ErrorCode::StaleRevision,
        ErrorCode::TaxonomyCycle,
        ErrorCode::UnitDelistBlocked,
        ErrorCode::UnitDeprecated,
        ErrorCode::UnrecognizedUnit,
        ErrorCode::UsageTypeUnavailable,
        ErrorCode::UsageTypeUnresolved,
        ErrorCode::Validation,
        ErrorCode::VersionForcedIncomplete,
        ErrorCode::WatermarkConflict,
        ErrorCode::WatermarkFuture,
        ErrorCode::WatermarkRegression,
    ];

    /// The wire spelling — the `code` a canonical error carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AccountingCodeDelistBlocked => "ACCOUNTING_CODE_DELIST_BLOCKED",
            Self::AccountingCodeDeprecated => "ACCOUNTING_CODE_DEPRECATED",
            Self::AccountingCodeRequired => "ACCOUNTING_CODE_REQUIRED",
            Self::AccountingCodeUnknown => "ACCOUNTING_CODE_UNKNOWN",
            Self::ApprovalRequired => "APPROVAL_REQUIRED",
            Self::ApprovalSuperseded => "APPROVAL_SUPERSEDED",
            Self::ApproverRoleRequired => "APPROVER_ROLE_REQUIRED",
            Self::ApproverScopeExceeded => "APPROVER_SCOPE_EXCEEDED",
            Self::AuditUnavailable => "AUDIT_UNAVAILABLE",
            Self::BreakglassCorrectionDisabled => "BREAKGLASS_CORRECTION_DISABLED",
            Self::BreakglassExpired => "BREAKGLASS_EXPIRED",
            Self::BreakglassWriteForbidden => "BREAKGLASS_WRITE_FORBIDDEN",
            Self::BulkDependencyFailed => "BULK_DEPENDENCY_FAILED",
            Self::BulkLimit => "BULK_LIMIT",
            Self::BulkOverrideUnacknowledged => "BULK_OVERRIDE_UNACKNOWLEDGED",
            Self::BundleOverrideRequired => "BUNDLE_OVERRIDE_REQUIRED",
            Self::CascadeConfirmationRequired => "CASCADE_CONFIRMATION_REQUIRED",
            Self::CatalogVersionUnknown => "CATALOG_VERSION_UNKNOWN",
            Self::CategoryReferenced => "CATEGORY_REFERENCED",
            Self::CloneSourceDiscarded => "CLONE_SOURCE_DISCARDED",
            Self::ContentPiiBlocked => "CONTENT_PII_BLOCKED",
            Self::CorrectionApprovalOpen => "CORRECTION_APPROVAL_OPEN",
            Self::CorrectionDirtyHead => "CORRECTION_DIRTY_HEAD",
            Self::CorrectionReferenced => "CORRECTION_REFERENCED",
            Self::CorrectionSignalAvailable => "CORRECTION_SIGNAL_AVAILABLE",
            Self::DecisionAlreadyRecorded => "DECISION_ALREADY_RECORDED",
            Self::DefinitionInUse => "DEFINITION_IN_USE",
            Self::DuplicateCategoryName => "DUPLICATE_CATEGORY_NAME",
            Self::DuplicateCode => "DUPLICATE_CODE",
            Self::DuplicateName => "DUPLICATE_NAME",
            Self::EntityTerminal => "ENTITY_TERMINAL",
            Self::EolDisabled => "EOL_DISABLED",
            Self::ErasureUnknownActor => "ERASURE_UNKNOWN_ACTOR",
            Self::FreezeIncomplete => "FREEZE_INCOMPLETE",
            Self::IdempotencyConflict => "IDEMPOTENCY_CONFLICT",
            Self::IdempotencyKeyInFlight => "IDEMPOTENCY_KEY_IN_FLIGHT",
            Self::IllegalFieldMutation => "ILLEGAL_FIELD_MUTATION",
            Self::IllegalTransition => "ILLEGAL_TRANSITION",
            Self::IncompleteEntity => "INCOMPLETE_ENTITY",
            Self::IntentRequired => "INTENT_REQUIRED",
            Self::MetadataLimit => "METADATA_LIMIT",
            Self::MeterDeclarationIncomplete => "METER_DECLARATION_INCOMPLETE",
            Self::ParentNotPublished => "PARENT_NOT_PUBLISHED",
            Self::ParentTerminal => "PARENT_TERMINAL",
            Self::ParticipantUnknown => "PARTICIPANT_UNKNOWN",
            Self::PlanTierDeprecated => "PLAN_TIER_DEPRECATED",
            Self::PlanTierRetireBlocked => "PLAN_TIER_RETIRE_BLOCKED",
            Self::PlanTierUnknown => "PLAN_TIER_UNKNOWN",
            Self::PrimaryCategoryRequired => "PRIMARY_CATEGORY_REQUIRED",
            Self::ProducerRetirementWouldFree => "PRODUCER_RETIREMENT_WOULD_FREE",
            Self::ProducerSetEmptyForbidden => "PRODUCER_SET_EMPTY_FORBIDDEN",
            Self::ProducerUnregistered => "PRODUCER_UNREGISTERED",
            Self::PromotionDirtyHead => "PROMOTION_DIRTY_HEAD",
            Self::PromotionIdentityConflict => "PROMOTION_IDENTITY_CONFLICT",
            Self::ReadModelOverloaded => "READ_MODEL_OVERLOADED",
            Self::ReplacedByNotPublished => "REPLACED_BY_NOT_PUBLISHED",
            Self::RequestSourceUnknown => "REQUEST_SOURCE_UNKNOWN",
            Self::RetirementLeadTime => "RETIREMENT_LEAD_TIME",
            Self::RetirementPending => "RETIREMENT_PENDING",
            Self::ScheduleStaleApproval => "SCHEDULE_STALE_APPROVAL",
            Self::ScopeNotContained => "SCOPE_NOT_CONTAINED",
            Self::SelfApprovalForbidden => "SELF_APPROVAL_FORBIDDEN",
            Self::SkuTypeUnknown => "SKU_TYPE_UNKNOWN",
            Self::StagedEntityChanged => "STAGED_ENTITY_CHANGED",
            Self::StaleCategoryToken => "STALE_CATEGORY_TOKEN",
            Self::StaleLiveOp => "STALE_LIVE_OP",
            Self::StaleRevision => "STALE_REVISION",
            Self::TaxonomyCycle => "TAXONOMY_CYCLE",
            Self::UnitDelistBlocked => "UNIT_DELIST_BLOCKED",
            Self::UnitDeprecated => "UNIT_DEPRECATED",
            Self::UnrecognizedUnit => "UNRECOGNIZED_UNIT",
            Self::UsageTypeUnavailable => "USAGE_TYPE_UNAVAILABLE",
            Self::UsageTypeUnresolved => "USAGE_TYPE_UNRESOLVED",
            Self::Validation => "VALIDATION",
            Self::VersionForcedIncomplete => "VERSION_FORCED_INCOMPLETE",
            Self::WatermarkConflict => "WATERMARK_CONFLICT",
            Self::WatermarkFuture => "WATERMARK_FUTURE",
            Self::WatermarkRegression => "WATERMARK_REGRESSION",
        }
    }

    /// Parse a wire code; `None` for a spelling this vocabulary does not
    /// carry (a consumer built against an older SDK reading a newer gear).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ACCOUNTING_CODE_DELIST_BLOCKED" => Some(Self::AccountingCodeDelistBlocked),
            "ACCOUNTING_CODE_DEPRECATED" => Some(Self::AccountingCodeDeprecated),
            "ACCOUNTING_CODE_REQUIRED" => Some(Self::AccountingCodeRequired),
            "ACCOUNTING_CODE_UNKNOWN" => Some(Self::AccountingCodeUnknown),
            "APPROVAL_REQUIRED" => Some(Self::ApprovalRequired),
            "APPROVAL_SUPERSEDED" => Some(Self::ApprovalSuperseded),
            "APPROVER_ROLE_REQUIRED" => Some(Self::ApproverRoleRequired),
            "APPROVER_SCOPE_EXCEEDED" => Some(Self::ApproverScopeExceeded),
            "AUDIT_UNAVAILABLE" => Some(Self::AuditUnavailable),
            "BREAKGLASS_CORRECTION_DISABLED" => Some(Self::BreakglassCorrectionDisabled),
            "BREAKGLASS_EXPIRED" => Some(Self::BreakglassExpired),
            "BREAKGLASS_WRITE_FORBIDDEN" => Some(Self::BreakglassWriteForbidden),
            "BULK_DEPENDENCY_FAILED" => Some(Self::BulkDependencyFailed),
            "BULK_LIMIT" => Some(Self::BulkLimit),
            "BULK_OVERRIDE_UNACKNOWLEDGED" => Some(Self::BulkOverrideUnacknowledged),
            "BUNDLE_OVERRIDE_REQUIRED" => Some(Self::BundleOverrideRequired),
            "CASCADE_CONFIRMATION_REQUIRED" => Some(Self::CascadeConfirmationRequired),
            "CATALOG_VERSION_UNKNOWN" => Some(Self::CatalogVersionUnknown),
            "CATEGORY_REFERENCED" => Some(Self::CategoryReferenced),
            "CLONE_SOURCE_DISCARDED" => Some(Self::CloneSourceDiscarded),
            "CONTENT_PII_BLOCKED" => Some(Self::ContentPiiBlocked),
            "CORRECTION_APPROVAL_OPEN" => Some(Self::CorrectionApprovalOpen),
            "CORRECTION_DIRTY_HEAD" => Some(Self::CorrectionDirtyHead),
            "CORRECTION_REFERENCED" => Some(Self::CorrectionReferenced),
            "CORRECTION_SIGNAL_AVAILABLE" => Some(Self::CorrectionSignalAvailable),
            "DECISION_ALREADY_RECORDED" => Some(Self::DecisionAlreadyRecorded),
            "DEFINITION_IN_USE" => Some(Self::DefinitionInUse),
            "DUPLICATE_CATEGORY_NAME" => Some(Self::DuplicateCategoryName),
            "DUPLICATE_CODE" => Some(Self::DuplicateCode),
            "DUPLICATE_NAME" => Some(Self::DuplicateName),
            "ENTITY_TERMINAL" => Some(Self::EntityTerminal),
            "EOL_DISABLED" => Some(Self::EolDisabled),
            "ERASURE_UNKNOWN_ACTOR" => Some(Self::ErasureUnknownActor),
            "FREEZE_INCOMPLETE" => Some(Self::FreezeIncomplete),
            "IDEMPOTENCY_CONFLICT" => Some(Self::IdempotencyConflict),
            "IDEMPOTENCY_KEY_IN_FLIGHT" => Some(Self::IdempotencyKeyInFlight),
            "ILLEGAL_FIELD_MUTATION" => Some(Self::IllegalFieldMutation),
            "ILLEGAL_TRANSITION" => Some(Self::IllegalTransition),
            "INCOMPLETE_ENTITY" => Some(Self::IncompleteEntity),
            "INTENT_REQUIRED" => Some(Self::IntentRequired),
            "METADATA_LIMIT" => Some(Self::MetadataLimit),
            "METER_DECLARATION_INCOMPLETE" => Some(Self::MeterDeclarationIncomplete),
            "PARENT_NOT_PUBLISHED" => Some(Self::ParentNotPublished),
            "PARENT_TERMINAL" => Some(Self::ParentTerminal),
            "PARTICIPANT_UNKNOWN" => Some(Self::ParticipantUnknown),
            "PLAN_TIER_DEPRECATED" => Some(Self::PlanTierDeprecated),
            "PLAN_TIER_RETIRE_BLOCKED" => Some(Self::PlanTierRetireBlocked),
            "PLAN_TIER_UNKNOWN" => Some(Self::PlanTierUnknown),
            "PRIMARY_CATEGORY_REQUIRED" => Some(Self::PrimaryCategoryRequired),
            "PRODUCER_RETIREMENT_WOULD_FREE" => Some(Self::ProducerRetirementWouldFree),
            "PRODUCER_SET_EMPTY_FORBIDDEN" => Some(Self::ProducerSetEmptyForbidden),
            "PRODUCER_UNREGISTERED" => Some(Self::ProducerUnregistered),
            "PROMOTION_DIRTY_HEAD" => Some(Self::PromotionDirtyHead),
            "PROMOTION_IDENTITY_CONFLICT" => Some(Self::PromotionIdentityConflict),
            "READ_MODEL_OVERLOADED" => Some(Self::ReadModelOverloaded),
            "REPLACED_BY_NOT_PUBLISHED" => Some(Self::ReplacedByNotPublished),
            "REQUEST_SOURCE_UNKNOWN" => Some(Self::RequestSourceUnknown),
            "RETIREMENT_LEAD_TIME" => Some(Self::RetirementLeadTime),
            "RETIREMENT_PENDING" => Some(Self::RetirementPending),
            "SCHEDULE_STALE_APPROVAL" => Some(Self::ScheduleStaleApproval),
            "SCOPE_NOT_CONTAINED" => Some(Self::ScopeNotContained),
            "SELF_APPROVAL_FORBIDDEN" => Some(Self::SelfApprovalForbidden),
            "SKU_TYPE_UNKNOWN" => Some(Self::SkuTypeUnknown),
            "STAGED_ENTITY_CHANGED" => Some(Self::StagedEntityChanged),
            "STALE_CATEGORY_TOKEN" => Some(Self::StaleCategoryToken),
            "STALE_LIVE_OP" => Some(Self::StaleLiveOp),
            "STALE_REVISION" => Some(Self::StaleRevision),
            "TAXONOMY_CYCLE" => Some(Self::TaxonomyCycle),
            "UNIT_DELIST_BLOCKED" => Some(Self::UnitDelistBlocked),
            "UNIT_DEPRECATED" => Some(Self::UnitDeprecated),
            "UNRECOGNIZED_UNIT" => Some(Self::UnrecognizedUnit),
            "USAGE_TYPE_UNAVAILABLE" => Some(Self::UsageTypeUnavailable),
            "USAGE_TYPE_UNRESOLVED" => Some(Self::UsageTypeUnresolved),
            "VALIDATION" => Some(Self::Validation),
            "VERSION_FORCED_INCOMPLETE" => Some(Self::VersionForcedIncomplete),
            "WATERMARK_CONFLICT" => Some(Self::WatermarkConflict),
            "WATERMARK_FUTURE" => Some(Self::WatermarkFuture),
            "WATERMARK_REGRESSION" => Some(Self::WatermarkRegression),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ErrorCode;

    #[test]
    fn the_roster_round_trips_and_is_sorted() {
        assert_eq!(ErrorCode::ALL.len(), 78);
        let mut seen = std::collections::BTreeSet::new();
        let mut previous: Option<&str> = None;
        for code in ErrorCode::ALL {
            let wire = code.as_str();
            assert_eq!(ErrorCode::parse(wire), Some(*code), "{wire} parses back");
            assert!(seen.insert(wire), "{wire} is listed once");
            if let Some(p) = previous {
                assert!(p < wire, "{p} precedes {wire}: the roster is alphabetical");
            }
            previous = Some(wire);
        }
        assert_eq!(ErrorCode::parse("NOT_A_CODE"), None);
    }
}
