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

    /// A SKU publish (or re-publish) under a live parent that is not
    /// `published`. Owned slot: P-D-24 prices 409; P-D-96 admits the arm.
    /// Does **not** name children (§7 row 27 / P-D-96).
    #[error("parent is not published: {0}")]
    ParentNotPublished(String),

    /// Un-deprecation while a live retire intent exists on the subject or
    /// on a child the reversal would revive (P-D-96).
    #[error("retirement pending: {0}")]
    RetirementPending(String),

    /// Runner wrapping `STALE_REVISION` / `APPROVAL_REQUIRED`.
    #[error("schedule stale approval: {0}")]
    ScheduleStaleApproval(String),

    /// `replacedBy` named a SKU that is not `published`.
    #[error("replacedBy is not published: {0}")]
    ReplacedByNotPublished(String),

    /// Operator `effectiveAt` earlier than the configured lead.
    #[error("retirement lead time: {0}")]
    RetirementLeadTime(String),

    /// Product retirement over live SKUs without cascade confirmation.
    #[error("cascade confirmation required: {0}")]
    CascadeConfirmationRequired(String),

    /// `mustMigrateBy` while the v1 EOL flag is off.
    #[error("EOL disabled: {0}")]
    EolDisabled(String),

    /// A required field is absent at the state the entity is being moved to.
    #[error("incomplete entity: {0}")]
    IncompleteEntity(String),

    /// A `GovernedLiveOp`'s pinned state no longer matches the live row
    /// (`inst-gl-envelope`).
    ///
    /// **Not [`Self::StaleRevision`]**, and 02 §3.5 draws the line: a live
    /// row has no `internal_revision` to be stale against, so the envelope
    /// pins the target's **state** and this code names that mismatch. A third
    /// code — `STALE_CATEGORY_TOKEN` — belongs to the category live-value
    /// door's own precondition and is neither of these two.
    #[error("stale live op: {0}")]
    StaleLiveOp(String),
    /// A `MeterDeclaration` arrived with one half of its atomic pair —
    /// `metering_unit` and `usage_type_ref` together or not at all
    /// (`03 inst-mt-atomic-pair`; the paired `CHECK` refuses the same shape
    /// at the physical layer).
    #[error("incomplete meter declaration: {0}")]
    MeterDeclarationIncomplete(String),
    /// A declaration named a unit the recognized set does not carry —
    /// unknown, or `removed`, which is a tombstone outside the set
    /// (`03 inst-mt-recognized`). The path to a new unit is the recognized-set
    /// door's governed add, never an inline mint.
    #[error("unrecognized metering unit: {0}")]
    UnrecognizedUnit(String),
    /// A **new** declaration named a `deprecated` unit — including a draft
    /// whose unit was deprecated before its first publish, which the PRD
    /// treats as a new declaration (`03 inst-mt-recognized`). Existing
    /// published carriers keep resolving.
    #[error("deprecated metering unit: {0}")]
    UnitDeprecated(String),
    /// A unit removal was refused while a non-terminal published head still
    /// declares it, with the holders sampled in the detail
    /// (`03 inst-us-delist`). The admitted path is deprecate first, remove
    /// once unreferenced.
    #[error("unit delist blocked: {0}")]
    UnitDelistBlocked(String),
    /// [`Self::UnitDelistBlocked`]'s `plan_tier` sibling — the tier set has
    /// its own refusal code by design (`03 inst-pt-governed`), the retire
    /// being the set's `removed` state.
    #[error("plan tier retire blocked: {0}")]
    PlanTierRetireBlocked(String),
    /// [`Self::UnitDelistBlocked`]'s accounting-code sibling, covering the
    /// `tax_category` and `gl_code` sets (`03` §3.2's taxonomy).
    #[error("accounting code delist blocked: {0}")]
    AccountingCodeDelistBlocked(String),
    /// A usage SKU's `usageTypeRef` did not resolve in the collector
    /// (`03 inst-mt-resolve`).
    #[error("usage type unresolved: {0}")]
    UsageTypeUnresolved(String),
    /// The usage-type collector was unreachable — fail-closed 503 for
    /// usage SKUs only (**P-D-131**).
    #[error("usage type unavailable: {0}")]
    UsageTypeUnavailable(String),

    /// A Product reaching `published` carries no primary category
    /// (`inst-tx-primary-at-publish`).
    ///
    /// **A variant of its own rather than an [`Self::IncompleteEntity`]
    /// message.** `inst-fd-publish-revalidate` names
    /// *"`INCOMPLETE_ENTITY`/rule-named code"* — a disjunction, not one code
    /// — and `design/02` §3.3 declares `PRIMARY_CATEGORY_REQUIRED` as this
    /// slice's own under 01 §3.3's code-to-declaring-slice rule (**P-D-36**).
    /// Folding it into the generic message would lose the code a consumer
    /// matches on, which is the half of the refusal that is actionable: the
    /// caller assigns a primary category, and no other repair fixes it.
    #[error("primary category required: {0}")]
    PrimaryCategoryRequired(String),

    /// No satisfied, non-superseded approval record pinned to the door's
    /// expected revision. The door evaluates no materiality; that judgement is
    /// the governance feature's and reaches the door as a record's presence.
    #[error("approval required: {0}")]
    ApprovalRequired(String),

    /// The record's own submitter tried to decide it. Refused at every
    /// `N >= 1`, **by principal and never by role** (`design/05` C2): a human
    /// holding both `CatalogAdmin` and `FinanceReviewer` is still one
    /// principal, and `products_approval_decision`'s
    /// `UNIQUE (tenant_id, approval_id, approver_principal)` is the physical
    /// floor under the same rule. 403 per `design/05` §3.3 — the caller may
    /// not take this act, whatever the record's state.
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-self-approval:p1
    #[error("self-approval forbidden: {0}")]
    SelfApprovalForbidden(String),

    /// Two categories would share a normalized name inside one parent. The
    /// race is decided by `uq_products_category_name_in_parent` — and by
    /// `uq_products_category_root_name` for the roots, a plain unique index
    /// treating every `NULL` parent as distinct — never by a read-then-write
    /// check (`02 inst-tx-name-in-parent`, 409).
    ///
    /// **Re-checked on rename AND on re-parent**: a re-parent carries the
    /// node's existing name into a new sibling set, so it collides without
    /// the name changing.
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-name-in-parent:p1
    #[error("duplicate category name: {0}")]
    DuplicateCategoryName(String),

    /// A re-parent whose new ancestor chain contains the node itself
    /// (`02 inst-tx-walk`). 422 architectural, reaching the wire as a 400
    /// like every architectural 422 here — the request's content cannot be
    /// processed, whatever the tree's current state.
    #[error("taxonomy cycle: {0}")]
    TaxonomyCycle(String),

    /// A decision arrived on a record that is no longer open. `design/05`
    /// §2 puts it at **decide** — *"a decision arriving on a record already
    /// `superseded` is refused"* — and §3.3 gives it **409**: the record's
    /// current state refuses the act, which is the convention's own line
    /// between 409 and 403.
    ///
    /// The refusal exists because `products_approval_decision` is
    /// append-only outright: a verdict cast on a closed ceremony cannot be
    /// removed, and any evaluator counting distinct principals by
    /// `approval_id` would count it.
    #[error("approval superseded: {0}")]
    ApprovalSuperseded(String),

    /// A principal without the base role set attempted a decision
    /// (`design/05` C1, C8; **P-D-119** rows 13 and 30).
    ///
    /// **Raised at the decide door, never by the gate.** P-D-119 row 30 is
    /// explicit: the gate's one refusal is *"no satisfied record"* →
    /// `APPROVAL_REQUIRED`, and it never sees a principal's roles. The decide
    /// door is where a role is readable and the only place this code has a
    /// raise path.
    ///
    /// **403**, per row 13: the principal lacks standing to take the act at
    /// all, which is the convention's line against 409's *"the record's
    /// current state refuses it"*.
    ///
    /// Roles arrive as `token_scopes` claims (**P-D-134** row 25). Until the
    /// platform's PDP encodes them, a caller holds **no** role claim and this
    /// is what they are told — never that the role was held.
    #[error("approver role required: {0}")]
    ApproverRoleRequired(String),

    /// An approver whose brand and region claims do not cover the subject's
    /// scope (`inst-gv-scope`, on `01`'s **P-D-39** boundaries). 403 — the
    /// caller may not decide this subject, whatever the record's state — and
    /// audited like any scope violation.
    #[error("approver scope exceeded: {0}")]
    ApproverScopeExceeded(String),

    /// A write attempted under a break-glass elevation. **403 with no
    /// exception in v1** (`design/05` §2's break-glass flow, **P-D-133**
    /// row 18): the elevation substitutes a **read-only**
    /// `AccessScope::for_tenant(target)` for the caller's own scope, and v1
    /// is read and audit-export only.
    #[error("break-glass sessions are read-only: {0}")]
    BreakGlassWriteForbidden(String),

    /// A call past the elevation's window (`inst-bg-expiry`, **P-D-68**
    /// arm 2). 403.
    ///
    /// **Expiry gates admission, not completion**: a call admitted inside the
    /// window finishes even if the window closes under it, so this is raised
    /// at the pre-pipeline gate and nowhere else. The first post-expiry act
    /// also emits `BreakGlassExpired`, exactly once, through a CAS on the
    /// session's `expired_emitted` stamp in the same transaction as this
    /// refusal.
    #[error("break-glass session expired: {0}")]
    BreakGlassExpired(String),

    /// This record cannot take that decision — **P-D-119** row 37, the
    /// seventh code `design/05` §3.3's roster opens for.
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-governance-errors:p1
    ///
    /// Two refusals the ceremony raises, one user-facing fact. A **second
    /// verdict from one principal** collides with
    /// `uq_products_approval_decision_principal`, C2's physical floor under
    /// one-principal-one-decision; a **decision on a record that closed on no
    /// approver** is P-D-68 arm 1's zero-quorum record, which reaches
    /// `satisfied` at submission and has no open ceremony for a verdict to
    /// join. Both shipped as **500s** through `RepoError::Db` until this
    /// variant existed, and this gear's own rule is that error class follows
    /// provenance: a request-borne condition rendered as a server error is a
    /// defect, not a conservative choice.
    ///
    /// **409, not 403**: `design/05` §3.3's convention puts 409 where the
    /// record's current state refuses the act — which is exactly what both
    /// causes are — and 403 where the caller may not act at all. The detail
    /// says which cause; the code says the fact they share.
    ///
    /// One code rather than two: the alternative was rejected by P-D-119 as
    /// splitting a single user-facing fact, and a client that needs the cause
    /// reads the detail.
    #[error("decision already recorded: {0}")]
    DecisionAlreadyRecorded(String),

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

    /// Operator free text refused by the content-PII write block
    /// (`design/02` §3.3 `inst-av-pii-block`). 422 architectural, 400 on the
    /// wire.
    ///
    /// **Raised outside the pipeline**, which is why it is a variant at all
    /// and not a `ValidationReport` violation the way `02`'s seven content
    /// rules are: §3.3 says *"a code raised outside the pipeline needs no
    /// phase status and gets none"*, and `domain::taxonomy::content_pii_block`
    /// is its single raiser — a free function every door that accepts operator
    /// free text calls, in `01`, `04`, `05` and `07` as well as `02`.
    ///
    /// It is the **one** of slice `02`'s sixteen codes to gain a variant with
    /// this feature: the other fifteen either reach the wire through
    /// `Validation` carrying their own code, or have no raiser yet because
    /// their door's route is undeclared (§7 row 16).
    #[error("content blocked by the PII policy: {0}")]
    ContentPiiBlocked(String),

    /// A metadata write exceeded one of the three configured caps — key
    /// count, key byte length or value byte length (`inst-md-write`).
    ///
    /// **422 architectural, 400 on the wire**, transcribed from `design/02`
    /// §3.3's own `Problem responses` block and not re-derived from a class
    /// rule.
    ///
    /// It needs a variant of its own where the seven content rules do not,
    /// and the difference is where it is raised: those ride
    /// `DomainError::Validation`, whose mapping arm renders **each violation's
    /// own code**, while this cap is enforced **at the metadata door, outside
    /// any pipeline** — the store's own doc says so, *"the caps
    /// `METADATA_LIMIT` names are the door's, read from configuration"* — so
    /// there is no report for it to ride. Second of slice `02`'s sixteen codes
    /// to gain one, after `CONTENT_PII_BLOCKED`.
    #[error("metadata exceeds a configured cap: {0}")]
    MetadataLimit(String),

    /// A category retire or delete refused because something still holds the
    /// node. For a **retire**: a non-terminal Product filing under it or an
    /// `active` child. For a **delete**: **any** `products_product_category`
    /// row naming it and **any** child row, in any state (**P-D-116** row 21
    /// — a category with history is retired, never deleted).
    ///
    /// `design/02` §3.3 files `CATEGORY_REFERENCED` at **409**: the tree's
    /// current state refuses the act. Until 2026-09-03 it rode
    /// [`Self::Validation`], which renders **400** — the wrong status for a
    /// code the design puts at 409, and exactly the clause `dod-taxonomy-errors`
    /// measures (*"each carrying the RFC 9457 problem-response status the
    /// design slice assigns it"*). The domain refusal stays
    /// `taxonomy::CategoryReferenced`; this is its wire form, reached through
    /// `From`.
    #[error("category still referenced: {0}")]
    CategoryReferenced(String),

    /// A definition removal refused because a **non-terminal** head still
    /// carries one of its values (`inst-ad-deprecate-then-remove`; **P-D-116**
    /// row 11 makes this the one operand for removal and type change, and row 5
    /// keeps a `draft` head in it). 409 by `design/02` §3.3, for
    /// [`Self::CategoryReferenced`]'s reason. The wire form of
    /// `taxonomy::DefinitionInUse`.
    #[error("definition still in use: {0}")]
    DefinitionInUse(String),

    /// The category live-value door's `If-Match` mismatch on
    /// `products_category.mutation_seq` (**P-D-50**, `inst-av-category-branch`).
    /// 409 by `design/02` §3.3 — the door's own precondition, deliberately
    /// neither `STALE_REVISION` (the entity head's) nor `STALE_LIVE_OP` (the
    /// envelope's). The wire form of `taxonomy::StaleCategoryToken`.
    #[error("stale category token: {0}")]
    StaleCategoryToken(String),
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
            Self::StaleLiveOp(_) => "STALE_LIVE_OP",
            Self::MeterDeclarationIncomplete(_) => "METER_DECLARATION_INCOMPLETE",
            Self::UnrecognizedUnit(_) => "UNRECOGNIZED_UNIT",
            Self::UnitDeprecated(_) => "UNIT_DEPRECATED",
            Self::UnitDelistBlocked(_) => "UNIT_DELIST_BLOCKED",
            Self::PlanTierRetireBlocked(_) => "PLAN_TIER_RETIRE_BLOCKED",
            Self::AccountingCodeDelistBlocked(_) => "ACCOUNTING_CODE_DELIST_BLOCKED",
            Self::UsageTypeUnresolved(_) => "USAGE_TYPE_UNRESOLVED",
            Self::UsageTypeUnavailable(_) => "USAGE_TYPE_UNAVAILABLE",
            Self::PrimaryCategoryRequired(_) => "PRIMARY_CATEGORY_REQUIRED",
            Self::ApprovalRequired(_) => "APPROVAL_REQUIRED",
            Self::SelfApprovalForbidden(_) => "SELF_APPROVAL_FORBIDDEN",
            Self::ApprovalSuperseded(_) => "APPROVAL_SUPERSEDED",
            Self::ApproverRoleRequired(_) => "APPROVER_ROLE_REQUIRED",
            Self::ApproverScopeExceeded(_) => "APPROVER_SCOPE_EXCEEDED",
            Self::BreakGlassWriteForbidden(_) => "BREAKGLASS_WRITE_FORBIDDEN",
            Self::BreakGlassExpired(_) => "BREAKGLASS_EXPIRED",
            Self::DecisionAlreadyRecorded(_) => "DECISION_ALREADY_RECORDED",
            Self::DuplicateCategoryName(_) => "DUPLICATE_CATEGORY_NAME",
            Self::TaxonomyCycle(_) => "TAXONOMY_CYCLE",
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
            Self::ParentNotPublished(_) => "PARENT_NOT_PUBLISHED",
            Self::RetirementPending(_) => "RETIREMENT_PENDING",
            Self::ScheduleStaleApproval(_) => "SCHEDULE_STALE_APPROVAL",
            Self::ReplacedByNotPublished(_) => "REPLACED_BY_NOT_PUBLISHED",
            Self::RetirementLeadTime(_) => "RETIREMENT_LEAD_TIME",
            Self::CascadeConfirmationRequired(_) => "CASCADE_CONFIRMATION_REQUIRED",
            Self::EolDisabled(_) => "EOL_DISABLED",
            Self::ContentPiiBlocked(_) => "CONTENT_PII_BLOCKED",
            Self::MetadataLimit(_) => "METADATA_LIMIT",
            Self::CategoryReferenced(_) => "CATEGORY_REFERENCED",
            Self::DefinitionInUse(_) => "DEFINITION_IN_USE",
            Self::StaleCategoryToken(_) => "STALE_CATEGORY_TOKEN",
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod error_tests;
