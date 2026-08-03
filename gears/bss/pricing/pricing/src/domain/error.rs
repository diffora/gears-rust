//! `DomainError` — the gear's internal rejection vocabulary.
//!
//! Every variant is mapped to the AIP-193
//! [`toolkit_canonical_errors::CanonicalError`] envelope by the SINGLE
//! authoritative ladder in [`crate::infra::error_mapping`], so a domain
//! rejection is assigned a canonical category in exactly one place and both
//! surfaces — REST and the in-process client — agree by construction.
//!
//! The Foundation owns the problem types listed in `design/01-foundation.md`
//! §3.3; slices **reference** them and never redefine them, which is why they
//! live here rather than beside the surface that raises them. The wire codes
//! (`DUPLICATE_SCOPE_KEY`, `AMOUNT_NEGATIVE`, …) are the machine-readable
//! discriminators a consumer matches after the coarse category; they are the
//! names the design set uses, verbatim.

use toolkit_macros::domain_model;

use crate::domain::validation::ValidationReport;

/// A catalog operation rejection.
///
/// Fail-closed is the rule the taxonomy encodes: there is no variant that means
/// "proceeded with a default". An absent required field is a publish failure
/// ([`DomainError::ValidationFailed`]), never a downstream substitution.
#[domain_model]
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    // -- InvalidArgument (bad request shape / value) --
    /// A request the gear cannot interpret at all — malformed ids, an unknown
    /// enum value, a missing required body field.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    // -- FailedPrecondition (state forbids the operation) --
    /// A published row resolves neither a row-level `rounding_policy_ref` nor a
    /// tenant default. Rounding decides the last minor unit of every charge, so
    /// an unresolved policy fails publish rather than picking one.
    #[error("rounding policy unresolved: {0}")]
    RoundingPolicyUnresolved(String),
    /// An amount carries more precision than the currency's ISO 4217 minor
    /// unit. There is no flat two-decimal rule: JPY takes 0, BHD takes 3.
    #[error("precision exceeds the currency minor unit: {0}")]
    PrecisionExceeded(String),
    /// An authored instant carries finer precision than the millisecond quantum
    /// every instant this gear publishes or compares is expressed at (D-144).
    ///
    /// Refused rather than truncated, which is what an unstated quantum
    /// degenerates into. Truncation moves the instant a scope-key axis is
    /// matched on for equality across a gear boundary, so a truncating producer
    /// and a non-truncating consumer agree until the day they do not, with no
    /// failure in between — and the generation is then unfindable by exactly the
    /// subscribers grandfathering retains. The money-side sibling
    /// [`DomainError::PrecisionExceeded`] takes the same posture for the same
    /// reason.
    #[error("timestamp precision exceeds the millisecond quantum: {0}")]
    TimestampPrecisionExceeded(String),
    /// A negative amount. Typed credit rows are deliberately Future scope, so a
    /// negative price is a mistake, not an unsupported feature.
    #[error("amount must be >= 0: {0}")]
    AmountNegative(String),
    /// A currency code that is not valid ISO 4217.
    #[error("invalid ISO 4217 currency: {0}")]
    CurrencyInvalid(String),
    /// Publish of a `modelKind` (or a non-`sum` aggregation row) that no green
    /// joint fixture gates. The corpus is the conformance contract with Rating;
    /// publishing past a missing fixture would ship a shape no one has agreed
    /// how to evaluate.
    #[error("no green conformance fixture: {0}")]
    FixtureMissing(String),
    /// The state machine forbids the transition — mutating a published row's
    /// content, superseding a grandfathered row.
    ///
    /// Narrowed by D-146 to exactly the refusals that describe **no alternative
    /// action**. The two it used to swallow are back out:
    /// [`DomainError::PlanRetiredNoSuccessor`] and
    /// [`DomainError::OpenDraftRevisionExists`] are refusals an operator acts on
    /// differently, and a consumer that had to parse prose to tell them apart
    /// could not act on either.
    #[error("lifecycle transition forbidden: {0}")]
    LifecycleForbidden(String),
    /// A revision was opened on, or a publish attempted for, a plan that is
    /// retired.
    ///
    /// A stop, not a retry and not an alternative: retirement is terminal, so a
    /// successor would be unpublishable from the moment it was opened. Told as
    /// [`DomainError::LifecycleForbidden`] this was indistinguishable from
    /// refusals the operator can work around, and the only remedy the shared
    /// sentence implied was the very call being refused.
    #[error("plan is retired and takes no successor revision: {0}")]
    PlanRetiredNoSuccessor(String),
    /// An authoring call named a plan whose **every** revision is `abandoned`.
    ///
    /// It holds no current revision and no open draft and can acquire neither:
    /// a first draft is minted at revision `0` outright, and a successor
    /// presupposes a current revision to succeed from — so a plan created and
    /// discarded before its first publish has spent its id (D-145 as amended
    /// 2026-08-02; `02-plan-definition.md` §5, `01-foundation.md` §3.3).
    ///
    /// Deliberately not [`DomainError::PlanRetiredNoSuccessor`], which would
    /// assert a retirement that never happened and which describes a plan that
    /// still has a current revision, a warm delta and a clone route forward.
    /// Deliberately not [`DomainError::LifecycleForbidden`] either, by D-146's
    /// own line: that code holds the refusals describing **no** alternative
    /// action, and this one describes a specific one — the id is spent, so mint
    /// a new plan and stop retrying this one.
    #[error("plan holds only abandoned revisions and takes no further revision: {0}")]
    PlanAbandonedNoSuccessor(String),
    /// A grandfathering horizon on a row whose eligibility class is not
    /// `existing_grandfathered`.
    ///
    /// The horizon expires a *retained generation*, and the other two classes
    /// retain nobody, so a horizon there names a moment nothing observes. The
    /// pairing was a physical column check with no rule behind it, which reached
    /// the caller as an internal fault — a 500 for a request whose author only
    /// has to clear one field.
    #[error("grandfathering horizon forbidden off the grandfathered class: {0}")]
    GrandfatherUntilForbidden(String),

    // -- Aborted (conflict; the caller may retry with fresh state) --
    /// Another current row already occupies the canonical scope key.
    #[error("duplicate canonical scope key: {0}")]
    DuplicateScopeKey(String),
    /// The submitted `ETag` / row version is stale: an interactive edit and a
    /// bulk run collided, or the caller is working from a read it did not
    /// refresh. Neither change is silently overwritten.
    #[error("stale version: {0}")]
    StaleVersion(String),
    /// The same idempotency key arrived with a different payload. Never
    /// replayed and never re-executed — the two requests disagree about what
    /// they are.
    #[error("idempotency payload mismatch: {0}")]
    IdempotencyPayloadMismatch(String),
    /// The idempotency key is claimed and not yet answered, so there is no
    /// stored response to replay (D-143).
    ///
    /// A retry is the whole remedy, and it is one a client can act on: shortly
    /// after, the answer is either the stored response or
    /// [`DomainError::IdempotencyPayloadMismatch`]. Without a code of its own
    /// the surface had to answer a caller with a response nobody had produced —
    /// the one thing the dedup row exists to make impossible.
    #[error("idempotency key claimed and not yet answered: {0}")]
    IdempotencyKeyInFlight(String),
    /// Another mutation of this aggregate committed while yours was in flight,
    /// and the store's per-aggregate serialization point refused the loser
    /// (D-159).
    ///
    /// **Retriable, and it is nobody's mistake.** Three constructs serialize
    /// writes per aggregate: the audit chain head `(tenant_id, chain_id, seq)`,
    /// the outbox's per-`(tenant_id, aggregate_id)` sequence, and the
    /// current-revision partial `UNIQUE`. A loser at any of them used to reach
    /// the caller as `Internal` -> **500**, indistinguishable from a dead
    /// connection, for a request whose entire remedy is to try again.
    ///
    /// Deliberately **not** [`DomainError::StaleVersion`]: nothing the caller
    /// presented was stale, and a caller told their `If-Match` failed would
    /// refresh a version that was never wrong. Deliberately **not**
    /// [`DomainError::IdempotencyKeyInFlight`] either, whose subject is the
    /// caller's own duplicate request rather than somebody else's write.
    #[error("concurrent mutation: {0}")]
    ConcurrentMutation(String),
    /// The plan already holds an open draft revision, named by the refusal.
    ///
    /// A uniqueness conflict on the plan's one editable slot, not a state
    /// machine edge. The operator's next action is a real one — go and edit that
    /// revision — which is unreachable from a refusal that does not say which
    /// revision holds the slot.
    #[error("open draft revision exists: {0}")]
    OpenDraftRevisionExists(String),

    // -- The aggregate validation envelope --
    /// The fail-closed validation pipeline rejected the publish. Carries the
    /// whole report rather than the first failure: authoring remediates a plan
    /// in one pass, and a truncated report turns one publish into N round
    /// trips.
    #[error("publish validation failed: {0} blocking violation(s)")]
    ValidationFailed(ValidationReport),

    // -- NotFound --
    /// The named subject does not exist, or lies outside the caller's scope —
    /// deliberately the same answer either way, so the surface leaks no
    /// existence. `subject` is the noun (`plan`, `price`, `overlay`), `id` the
    /// reference the caller supplied.
    #[error("{subject} {id} not found")]
    NotFound {
        /// The kind of thing that was not found.
        subject: String,
        /// The identifier the caller asked for.
        id: String,
    },

    // -- Unavailable (fail closed, retry later) --
    /// The `CatalogVersion` registry could not be reached or has no registered
    /// client. Publish stops here: addressability is not optional, and the
    /// registry is the sole incrementer.
    #[error("catalog-version registry unavailable: {0}")]
    CatalogVersionUnavailable(String),
    /// The read model is unavailable. The read path fails closed rather than
    /// serving a stale or partial version.
    #[error("read model unavailable: {0}")]
    ReadModelUnavailable(String),

    // -- Internal --
    /// An infrastructure fault with no domain meaning.
    #[error("internal: {0}")]
    Internal(String),
}
