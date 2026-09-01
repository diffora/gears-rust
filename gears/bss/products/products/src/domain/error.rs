//! `DomainError` — the Foundation's rejection vocabulary.
//!
//! A code belongs to the rule that raises it, and the rule belongs to a feature
//! (P-D-36). The variants here are the Foundation-owned codes of
//! `design/01-foundation.md` §3.3; capability features reference them and never
//! redefine them, which is why they live here rather than beside each surface.
//!
//! The wire codes are the machine-readable discriminators a consumer matches on
//! after the coarse category, and they are the names the design set uses,
//! verbatim.

use toolkit_macros::domain_model;

use crate::domain::validation::ValidationReport;

/// A registry operation rejection.
///
/// @cpt-cf-bss-products-fr-expected-failure-behavior
/// @cpt-dod:cpt-cf-bss-products-dod-error-taxonomy:p1
///
/// Fail-closed is what the taxonomy encodes: no variant means "proceeded with a
/// default". An absent required field is a refusal, never a substitution.
#[domain_model]
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    /// The per-field envelope. Carries every violation the failing phase
    /// collected; the audit row records one code (P-D-37).
    #[error("validation failed: {0}")]
    Validation(ValidationReport),

    /// The normalized name collides within `(tenant_id, brand_id)` on a
    /// non-discarded row. The message names the holder.
    #[error("duplicate name: {0}")]
    DuplicateName(String),

    /// Either reservation lost its race — `skuCode` or `productCode`. One code
    /// covers both (P-D-25).
    #[error("duplicate code: {0}")]
    DuplicateCode(String),

    /// `If-Match` did not match the head's current internal revision.
    #[error("stale revision: expected {expected}, found {found}")]
    StaleRevision {
        /// What the caller pinned.
        expected: i64,
        /// What the head actually carries.
        found: i64,
    },

    /// The same idempotency key arrived with a different payload.
    #[error("idempotency conflict on key {0}")]
    IdempotencyConflict(String),

    /// A matching-payload hit on a claimed but unanswered key.
    #[error("idempotency key in flight: {0}")]
    IdempotencyKeyInFlight(String),

    /// Any head write on a `retired` or `discarded` row — save, publish or
    /// correction alike (P-D-25, widened by P-D-32).
    #[error("entity is terminal: {0}")]
    EntityTerminal(String),

    /// A refusal's own audit row could not be written, so the door does not
    /// report the domain refusal. One of the gear's three 503s.
    #[error("audit unavailable: {0}")]
    AuditUnavailable(String),

    /// An edge outside the admitted list.
    #[error("illegal transition from {from} to {to}")]
    IllegalTransition {
        /// The state the head is in.
        from: String,
        /// The state the caller asked for.
        to: String,
    },

    /// A write the head door may not take: bucket-i after first publish, any
    /// update of `cloned_from`, or bucket-ii after first publish — the last
    /// belonging to the correction door, which the reason names.
    #[error("illegal field mutation: {0}")]
    IllegalFieldMutation(String),

    /// A child's scope is not provably contained in its parent's. Containment
    /// is defined over restrictions, not over raw sets (P-D-39).
    #[error("scope not contained: {0}")]
    ScopeNotContained(String),

    /// The parent's own state refuses the write, as distinct from the payload
    /// being wrong.
    #[error("parent is terminal: {0}")]
    ParentTerminal(String),

    /// A required field is absent at the state the entity is being moved to.
    #[error("incomplete entity: {0}")]
    IncompleteEntity(String),

    /// No satisfied, non-superseded approval record pinned to the door's
    /// expected revision. The door evaluates no materiality; that judgement is
    /// the governance feature's and reaches the door as a record's presence.
    #[error("approval required: {0}")]
    ApprovalRequired(String),

    /// The erasure door's own refusal: the named principal resolves to no
    /// `actor_ref` in this tenant. The one code `10-retention-erasure` owns
    /// (P-D-64 kept the roster at one) — 422 architectural, reaching the wire
    /// as a 400 like every architectural 422 here.
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-retention-error-taxonomy:p1
    #[error("erasure names an unknown actor: {0}")]
    ErasureUnknownActor(String),

    /// The clone door's own refusal: the source is `discarded`. Minted by
    /// **P-D-75** on P-D-52's test — `ENTITY_TERMINAL` means a head *write*
    /// and the clone writes nothing to the source (a `retired` source is
    /// explicitly admitted), while the bare 404 carries no code channel.
    /// 409: the source's state refuses the act.
    #[error("clone source is discarded: {0}")]
    CloneSourceDiscarded(String),

    /// An increment request whose `source` is outside the registered
    /// trigger set (P-D-52; `design/06` §2 rule 1). Raised **after** the
    /// `catalog_version x request` grant passes — a precondition on the
    /// request's content, not an authorization fact — and mapped to a
    /// `FailedPrecondition` carrying a violation of type
    /// `CATALOG_VERSION_REJECTED`, the discriminator the consumer's
    /// `Rejected` arm matches on.
    #[error("unregistered increment source: {0}")]
    RequestSourceUnknown(String),

    /// A resolution request with no `intent` (`inst-rv-intent`). 422
    /// architectural, 400 on the wire, carrying its code — the registry
    /// declares no transport override (`dod-cv-error-taxonomy`).
    #[error("intent is required: {0}")]
    IntentRequired(String),

    /// `posted` resolution of a version whose freeze ledger still holds a
    /// `pending` row (C5's fail-closed default). 409: the version's state
    /// refuses the act.
    #[error("freeze incomplete: {0}")]
    FreezeIncomplete(String),

    /// `posted` resolution of a force-completed version, naming each
    /// `not_frozen(forced)` participant until every one has since frozen or
    /// released through its own door (`inst-rv-intent`, P-D-19). 409.
    #[error("version forced incomplete: {0}")]
    VersionForcedIncomplete(String),

    /// The operator lane's stage-vs-commit refusal (`inst-sn-revalidate`,
    /// P-D-09): an entity moved between collect and commit, named. The
    /// mechanical lanes restage internally and never raise it; the variant
    /// ships now because `dod-cv-error-taxonomy` pins the whole seven, and
    /// its raiser arrives with the operator catalog-publish door. 409.
    #[error("staged entity changed: {0}")]
    StagedEntityChanged(String),

    /// A path segment names a catalog version this tenant has none of —
    /// raised by the shared version lookup, resolve and diff alike
    /// (`dod-intentful-resolver`). 404.
    #[error("catalog version unknown: {0}")]
    CatalogVersionUnknown(String),

    /// An ack or release from a principal outside the version's snapshotted
    /// set (`inst-fz-ack`) — a membership check, not authentication. 403
    /// rather than 404, because the caller's identity is the subject of the
    /// refusal and a 404 would leak whether the version exists.
    ///
    /// With the six variants above this completes the catalog-version
    /// seven, `REQUEST_SOURCE_UNKNOWN` included.
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-cv-error-taxonomy:p1
    #[error("participant unknown: {0}")]
    ParticipantUnknown(String),

    /// A bulk row whose in-batch dependency failed, refused **without
    /// touching the store** (`dod-stage-phase`'s dependency order). A
    /// per-row ledger outcome: the import door answers 202 and the row
    /// carries this, so its status is what the ledger reader returns.
    /// 422 architectural, 400 on the wire.
    #[error("bulk dependency failed: {0}")]
    BulkDependencyFailed(String),

    /// The promotion resolver's identity collision — two rows, or a row and
    /// a live entity, claiming one promotion identity. 409; a per-row
    /// ledger outcome, its raiser arriving with the resolver.
    #[error("promotion identity conflict: {0}")]
    PromotionIdentityConflict(String),

    /// An update-as-draft row whose target head carries an unpublished
    /// edit. 409; a per-row ledger outcome, its raiser arriving with the
    /// resolver.
    #[error("promotion dirty head: {0}")]
    PromotionDirtyHead(String),

    /// A batch override the ceremony left unacknowledged. 422
    /// architectural, 400 on the wire; a per-row ledger outcome, its raiser
    /// arriving with the override ceremony.
    #[error("bulk override unacknowledged: {0}")]
    BulkOverrideUnacknowledged(String),

    /// The one of the five that is the **door's** own refusal: a batch over
    /// the configured bounds — max rows per batch, or the tenant's
    /// concurrent-batch ceiling. Two operands, one code
    /// (`inst-bm-limits`). 409: the tenant's current state refuses the act.
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-bulk-errors:p1
    #[error("bulk limit: {0}")]
    BulkLimit(String),

    /// A watermark posted by a name outside the tenant's registered
    /// producer set. **403**: the caller's identity is the refusal's
    /// subject.
    #[error("producer unregistered: {0}")]
    ProducerUnregistered(String),

    /// A watermark older than the one stored — the set would move
    /// backwards, and a producer's completeness claim is monotone. 409.
    #[error("watermark regression: {0}")]
    WatermarkRegression(String),

    /// An equal `watermark_at` carrying a **different** set, told apart
    /// from the admitted idempotent replay by the stored `set_hash`
    /// (**P-D-71**). 409.
    #[error("watermark conflict: {0}")]
    WatermarkConflict(String),

    /// A `watermark_at` above the receiving clock plus the configured
    /// skew, refused **and alerted**. 422 architectural, 400 on the wire.
    /// The bound is `p1` rather than hygiene: one accepted future-dated
    /// post makes its producer read permanently fresh, freezes its member
    /// set behind `WATERMARK_REGRESSION`, and leaves every SKU outside
    /// that frozen set reading fresh-zero.
    #[error("watermark in the future: {0}")]
    WatermarkFuture(String),
}

impl DomainError {
    /// The stable wire code, which is what a consumer matches on and what the
    /// audit row records.
    ///
    /// Deliberately exhaustive rather than a catch-all: a variant added without
    /// a code is a compile error here, which is the only thing that keeps the
    /// taxonomy and the vocabulary from drifting.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match *self {
            Self::Validation(_) => "VALIDATION",
            Self::DuplicateName(_) => "DUPLICATE_NAME",
            Self::DuplicateCode(_) => "DUPLICATE_CODE",
            Self::StaleRevision { .. } => "STALE_REVISION",
            Self::IdempotencyConflict(_) => "IDEMPOTENCY_CONFLICT",
            Self::IdempotencyKeyInFlight(_) => "IDEMPOTENCY_KEY_IN_FLIGHT",
            Self::EntityTerminal(_) => "ENTITY_TERMINAL",
            Self::AuditUnavailable(_) => "AUDIT_UNAVAILABLE",
            Self::IllegalTransition { .. } => "ILLEGAL_TRANSITION",
            Self::IllegalFieldMutation(_) => "ILLEGAL_FIELD_MUTATION",
            Self::ScopeNotContained(_) => "SCOPE_NOT_CONTAINED",
            Self::ParentTerminal(_) => "PARENT_TERMINAL",
            Self::IncompleteEntity(_) => "INCOMPLETE_ENTITY",
            Self::ApprovalRequired(_) => "APPROVAL_REQUIRED",
            Self::ErasureUnknownActor(_) => "ERASURE_UNKNOWN_ACTOR",
            Self::CloneSourceDiscarded(_) => "CLONE_SOURCE_DISCARDED",
            Self::RequestSourceUnknown(_) => "REQUEST_SOURCE_UNKNOWN",
            Self::IntentRequired(_) => "INTENT_REQUIRED",
            Self::FreezeIncomplete(_) => "FREEZE_INCOMPLETE",
            Self::VersionForcedIncomplete(_) => "VERSION_FORCED_INCOMPLETE",
            Self::StagedEntityChanged(_) => "STAGED_ENTITY_CHANGED",
            Self::CatalogVersionUnknown(_) => "CATALOG_VERSION_UNKNOWN",
            Self::ParticipantUnknown(_) => "PARTICIPANT_UNKNOWN",
            Self::BulkDependencyFailed(_) => "BULK_DEPENDENCY_FAILED",
            Self::PromotionIdentityConflict(_) => "PROMOTION_IDENTITY_CONFLICT",
            Self::PromotionDirtyHead(_) => "PROMOTION_DIRTY_HEAD",
            Self::BulkOverrideUnacknowledged(_) => "BULK_OVERRIDE_UNACKNOWLEDGED",
            Self::BulkLimit(_) => "BULK_LIMIT",
            Self::ProducerUnregistered(_) => "PRODUCER_UNREGISTERED",
            Self::WatermarkRegression(_) => "WATERMARK_REGRESSION",
            Self::WatermarkConflict(_) => "WATERMARK_CONFLICT",
            Self::WatermarkFuture(_) => "WATERMARK_FUTURE",
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod error_tests;
