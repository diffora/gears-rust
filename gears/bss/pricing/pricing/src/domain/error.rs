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
    /// A window was created with a start that is not strictly in the future
    /// (`07-pricewindow-linkage.md` §5, D-63, `inst-ws-future-start`).
    ///
    /// §5 types it 422, which is this crate's architectural 422 rendered **400**
    /// with the code as the discriminator. A precondition failure and not a
    /// malformed argument for [`DomainError::TimestampPrecisionExceeded`]'s reason:
    /// the instant parses and is a perfectly good instant — it is simply behind a
    /// clock the request cannot see.
    ///
    /// Deliberately **not** a conflict, unlike the three window refusals that are.
    /// Those three tell a caller the world moved under a request that was right
    /// when written, and a retry with fresh state resolves them. This one tells a
    /// caller their request was never right: retrying it unchanged only puts the
    /// start further into the past.
    ///
    /// **What it closes is a backdating bypass**, which is why it is not
    /// aesthetic. `inst-ws-immutable` guards only the mutation of a past start, so
    /// without this rule a `plan × write` holder could schedule a window starting
    /// sixty days ago, have the `WindowActivationJob` activate it, and reprice open
    /// unposted arrears periods — round the reason-mandatory two-person
    /// `BackdateGrant` S2 calls the only sanctioned backdating, and without
    /// tripping S5's `BACKDATE_SIDE_EFFECT` predicate.
    #[error("window start is not in the future: {0}")]
    WindowStartInPast(String),
    /// A cancel or an `effectiveTo` shortening would leave the canonical scope key
    /// uncovered with no successor (`07-pricewindow-linkage.md` §5,
    /// `inst-fg-trailing`, architectural **422** rendered 400). Names the key and
    /// the first uncovered instant.
    ///
    /// # It is `inst-fg-detect`'s blind spot, not a second spelling of it
    ///
    /// Gap detection compares each window against its **successor**, so by
    /// construction it cannot see a void with no successor. The two rules overlap on
    /// most statements each refuses and neither subsumes the other: the statement
    /// only *this* rule can refuse is a key whose windows are contiguous with no
    /// interior hole, whose last coverage is then removed.
    ///
    /// # Fail-closed with no exemption, and that is D-182 rather than a gap
    ///
    /// `inst-fg-trailing`'s only exemption (D-51, narrowed by D-80 and D-94) turns
    /// on whether the key has in-flight subscribers, a predicate that resolves
    /// through the **D-79 Subscriptions inbound lane**. That lane has no client, no
    /// contract type and no counterpart gear in this repository, and the
    /// counterparty's own SUB-P8 still specifies the pre-D-131 union count rather
    /// than the per-price-id presence map every consumer of the lane needs. D-131
    /// makes a lane that cannot answer the **subscribers-present** case, and D-182
    /// records that an *absent* lane is that case too — fail-closed is about the
    /// predicate's evaluability, not the mechanism of its silence. So this refusal
    /// covers the exempt cancels as well, **no exemption branch is written**, and no
    /// second code is minted for the absent-lane case.
    ///
    /// Not a conflict, and for [`DomainError::WindowStartInPast`]'s reason with the
    /// sign reversed: the world did not move under the caller. The coverage this
    /// refuses to remove is the coverage that was there when they composed the
    /// request, and a retry unchanged is refused identically — the check consumes
    /// nothing and clears nothing, so re-entry is stable (D-182 clause 5).
    #[error("window trailing void: {0}")]
    WindowTrailingVoid(String),
    /// A supersession's changeover instant has fallen behind the floor
    /// `inst-su-instant` holds it to (`07-pricewindow-linkage.md`, architectural
    /// **422** rendered 400). Names the instant, the floor and the remedy.
    ///
    /// **Two floors, one code**, and that is the design set's choice rather than a
    /// conflation: strictly future at submit, and at least D-47's max batching
    /// delay ahead at approval commit. The refusal names which moment it was asked
    /// at, because the same instant can pass one and fail the other and a caller
    /// told only the code cannot tell those apart.
    ///
    /// Not a conflict, for [`DomainError::WindowStartInPast`]'s reason — the world
    /// did not move under the caller, the clock did — but unlike that one it is
    /// **not** answered by a retry either. The design set's remedy is that the unit
    /// is *recomposed* against a fresh instant, so the message says so: resubmitting
    /// the same instant is refused identically and lands further behind the floor
    /// each time. That is why this carries its own code rather than joining
    /// `WINDOW_START_IN_PAST`, which it superficially resembles: the field is
    /// different, the floor is different at one of the two moments, and the remedy
    /// is a recomposition rather than a correction.
    #[error("supersession changeover instant passed: {0}")]
    SupersessionInstantPassed(String),
    /// A proposed approval-threshold policy breaks one of §6's shape rules
    /// (`design/05-governance.md` §5, which types it 422).
    ///
    /// The rules, and which layer owns each — because three of the five are held
    /// somewhere else and this variant must not be read as the only guard:
    ///
    /// * **keys are ISO 4217 codes** — [`CurrencyCode::new`] owns the shape and
    ///   answers [`DomainError::CurrencyInvalid`]; the surface folds that into this
    ///   code, because a caller of the policy PUT broke *this* rule and a
    ///   `CURRENCY_INVALID` on a route with no price row on it names nothing they
    ///   can find;
    /// * **`percent > 0`** — held by `ThresholdBasis::Percent`'s constructor path at
    ///   the surface and by `chk_pricing_approval_threshold_percent_positive` in the
    ///   store;
    /// * **`absolute_minor >= 0`** — the same pair one CHECK over;
    /// * **at least one entry, and no currency twice** —
    ///   [`ThresholdVersion::new`](crate::domain::materiality::ThresholdVersion::new),
    ///   whose two refusals are the ones only this variant carries.
    ///
    /// §6's *"`absolute_minor` ≥ 0 in minor units at the currency's ISO 4217
    /// precision"* adds **nothing checkable** and is not checked: the value is
    /// already an integer count of minor units, so every integer satisfies it at
    /// every precision. Stated rather than silently dropped, because a rule with no
    /// test looks like a rule somebody forgot.
    ///
    /// A precondition failure and not a conflict: nothing moved under the caller,
    /// and a retry unchanged is refused identically.
    ///
    /// [`CurrencyCode::new`]: crate::domain::money::CurrencyCode::new
    #[error("threshold policy invalid: {0}")]
    ThresholdInvalid(String),

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
    /// A decision was asked of an approval record that is no longer pending
    /// (`design/05-governance.md` §5, `inst-as-immutable`).
    ///
    /// A conflict on mutable state and classified as one: the request was
    /// well-formed and the reviewer's authority sufficient, but another decision
    /// — an approve, a reject, or the TOCTOU guard's void — reached the record
    /// first. Re-reading it is the whole remedy, which is what a 409 asks for.
    ///
    /// Deliberately **not** [`DomainError::LifecycleForbidden`], though it is a
    /// state-machine edge and that variant carries the others. Every refusal on
    /// that line is a 400 telling a caller their request was wrong about the
    /// world; this one tells them the world moved, and a reviewer sent to check
    /// their own request would find nothing to fix.
    #[error("approval not pending: {0}")]
    ApprovalNotPending(String),
    /// A second change unit was submitted over a subject a pending unit already
    /// holds (`07-pricewindow-linkage.md` §5 / `inst-co-single-pending`, **409**).
    ///
    /// The conflict class, beside [`DomainError::OpenDraftRevisionExists`] and
    /// for the same reading: it is a uniqueness conflict on a slot the subject
    /// has exactly one of, and the operator's next action is real — decide the
    /// unit the refusal names, or withdraw it (`inst-as-void`). The refusal
    /// therefore names it; a refusal that did not would leave the caller holding
    /// a lock they cannot find.
    ///
    /// Declared by the design set (S7 §5's problem responses, S12
    /// `inst-bk-approval-subset`, D-35) and referenced here rather than minted.
    #[error("pending change unit exists: {0}")]
    PendingChangeUnitExists(String),
    /// The submitter of an approval unit tried to decide it
    /// (`inst-tp-distinct`, §5, **403**).
    ///
    /// A **permission** refusal and not a precondition one, which is what §5
    /// types it and what it is: the request was well-formed, the record was
    /// pending, the content had not moved — the principal simply may not be the
    /// second person. Nothing about the request can be edited to make it
    /// succeed, so a 400 telling them to fix it would be a loop, and a 409
    /// inviting a retry would be worse.
    ///
    /// The attempt is written to `pricing_audit_log` as a `deny` record before
    /// this is returned (`inst-tp-selfaudit`); see
    /// [`crate::infra::approval`] for why that record is committed in its own
    /// transaction.
    #[error("self-approval forbidden: {0}")]
    SelfApprovalForbidden(String),
    /// The approver's pricing-region grant does not cover every region the
    /// pinned change set touches (`inst-ap-scope`, §5, **403**).
    ///
    /// Beside [`DomainError::SelfApprovalForbidden`] and for the same reason:
    /// it is an authority refusal, and it is audited.
    #[error("region scope denied: {0}")]
    RegionScopeDenied(String),
    /// The pinned content no longer matches the subject at decision time
    /// (`inst-ap-pin`, §5, **409**).
    ///
    /// A conflict, with the conflicts: somebody mutated the subject after the
    /// reviewer opened it. Re-reading — and, since the TOCTOU guard will have
    /// voided the record, re-submitting — is the remedy, which is what a 409
    /// asks for. Deliberately not a precondition failure: the reviewer's request
    /// was right about the world as they were shown it.
    #[error("approval content mismatch: {0}")]
    ApprovalContentMismatch(String),
    /// A rejection arrived with no reason (`inst-as-reject`, §5).
    ///
    /// §5 types it 422. The canonical family has no 422 category, so it reaches
    /// the wire as a **400** carrying `REASON_REQUIRED` and the code is the
    /// discriminator — the Foundation §3.3 rule this crate applies to every
    /// architectural 422.
    #[error("reason required: {0}")]
    ReasonRequired(String),
    /// A scheduled or adjusted window intersects one already on the same
    /// canonical scope key (`07-pricewindow-linkage.md` §5, **409**).
    ///
    /// A conflict rather than a precondition failure, and §5 types it 409
    /// outright — the only one of the eight window codes it does. The reading
    /// holds: the caller's request was well-formed and their authority was
    /// sufficient, and what refused it is the state of a *sibling* row on the key
    /// they cannot see from their own request. Re-reading the key's windows is
    /// the remedy, which is what a 409 asks for.
    ///
    /// Deliberately not [`DomainError::DuplicateScopeKey`], which it superficially
    /// resembles: that one says two rows claim one key, while this says two
    /// intervals on **one** key claim one instant. The key here is legitimately
    /// shared — a supersession's predecessor and successor are both on it — so a
    /// caller told the key was duplicated would go looking for a row to retire.
    ///
    /// It is **not** raised by any constraint. §6 puts non-overlap "inside every
    /// mutation" because the key is eight columns of `pricing_price` and none of
    /// them is on the window row, so the sole producer is
    /// [`window_repo`](crate::infra::storage::repo::window_repo) — see that
    /// module and the migration's own note.
    #[error("window overlap: {0}")]
    WindowOverlap(String),
    /// A mutation reached for something §4 froze — a past `effectiveFrom`, a
    /// terminal window, the window↔price binding, or an `effectiveTo` that has
    /// already passed (`07-pricewindow-linkage.md` §5, `inst-ws-immutable`,
    /// **409**).
    ///
    /// A conflict for [`DomainError::WindowOverlap`]'s reason and one of its own:
    /// what refused the request is **where the clock now stands**. A `PATCH`
    /// composed against a window whose end was five minutes away is a request that
    /// was right when it was written, and re-reading the window is the whole
    /// remedy — which is what a 409 asks for. §5 types it 409 outright.
    ///
    /// Deliberately **not** [`DomainError::LifecycleForbidden`], which carried this
    /// ground until the code had a producer. That variant is D-146's "refusals that
    /// describe no alternative action", and this one describes a very specific one
    /// on two of its three grounds: schedule a *new* window instead of moving this
    /// one's start, or shorten to an instant that is still ahead.
    ///
    /// Deliberately **not** [`DomainError::WindowNotCancellable`] either, though
    /// both refuse an operation on a window that has moved on. That one is the
    /// answer to a *cancellation* and names a different next action — shorten it —
    /// while this is the answer to a mutation of frozen content.
    #[error("window historical immutability: {0}")]
    WindowHistoricalImmutable(String),
    /// A cancellation was asked of a window that has taken effect or is history
    /// (`07-pricewindow-linkage.md` §5, `inst-ws-cancel`, **409**).
    ///
    /// §4's third edge leaves `scheduled` and nothing else, and §5 types the
    /// refusal 409. A conflict rather than a precondition failure because the
    /// caller's request was well-formed and their authority sufficient: the window
    /// activated between the read that showed it as `scheduled` and the `DELETE`.
    ///
    /// It is worth its own code beside
    /// [`DomainError::WindowHistoricalImmutable`] precisely because the operator's
    /// next action differs. Told the window is immutable an operator stops; told it
    /// is not *cancellable* they shorten it through `effectiveTo`, which is the
    /// operation §4 leaves them and the one a cutover's own shorten uses.
    #[error("window not cancellable: {0}")]
    WindowNotCancellable(String),

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
