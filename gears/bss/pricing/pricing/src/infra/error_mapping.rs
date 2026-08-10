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
//! `GRANDFATHER_UNTIL_FORBIDDEN`, `CONCURRENT_MUTATION`, the five
//! `05-governance.md` §5 declares — `APPROVAL_NOT_PENDING`,
//! `APPROVAL_CONTENT_MISMATCH`, `SELF_APPROVAL_FORBIDDEN`,
//! `REGION_SCOPE_DENIED`, `REASON_REQUIRED` — and
//! `PENDING_CHANGE_UNIT_EXISTS`, which `07-pricewindow-linkage.md` §5 declares
//! and this gear's submit path is the first writer of); a consumer matches the
//! category coarsely and the code **exactly**.
//!
//! *Exactly* is why the two 403 arms carry a **bare code** in `reason` and drop
//! their detail. `permission_denied()` has no detail slot — its detail is the
//! fixed platform sentence and `reason` is the only free field — so a detail
//! carried there would have to ride the code as `"CODE: detail"`, and a consumer
//! comparing `reason` to `SELF_APPROVAL_FORBIDDEN` would never match. What is
//! given up is nothing the caller lacks and nothing an operator loses: the
//! detail names the approval record, which is the id the caller addressed the
//! request to, and the attempt itself is already on `pricing_audit_log` as a
//! `deny` record carrying that id (`inst-tp-selfaudit`, `inst-rb-audit`) —
//! a durable trail rather than a log line.
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
use crate::domain::migration;

#[resource_error(gts_id!("cf.bss.pricing.plan.v1~"))]
struct PlanResource;

/// One architectural 422 (rendered 400, the code the discriminator).
///
/// Added with Slice 11's three and used **only** by them, rather than folding the
/// arms above into it: those spell the builder out inline, and rewriting them to
/// save lines here would be a re-organisation of a file two strands are appending
/// to. What this is for is keeping the three new arms one line each, because
/// `clippy::too_many_lines` bounds [`From<DomainError>`] at 200 and the inline
/// spelling put it at 206.
fn precondition(field: &'static str, detail: &str, code: &'static str) -> CanonicalError {
    PlanResource::failed_precondition()
        .with_precondition_violation(field, detail, code)
        .create()
}

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
            // §5's batch-level refusal, the same shape: architectural 422,
            // rendered 400, the code the discriminator. The subject is `rows`
            // because that is what the operator edits — the refusal is about the
            // file, not about any one row, which is what all-or-nothing means.
            // Through the helper for its stated reason: this ladder is bounded at
            // 200 lines and an inline builder is five.
            D::BulkValidationFailed(detail) => precondition(
                "rows",
                &detail,
                crate::api::rest::bulk_imports::BULK_VALIDATION_FAILED,
            ),
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
            // The sibling of the line above and refused for the same structural
            // reason — no further revision will ever open — but kept apart from
            // it deliberately: that code would assert a retirement that never
            // happened, and a retired plan still has a current revision, a warm
            // delta and a clone route forward where this one has none (D-145 as
            // amended, `02-plan-definition.md` §5).
            D::PlanAbandonedNoSuccessor(detail) => PlanResource::failed_precondition()
                .with_precondition_violation(
                    "lifecycle_state",
                    detail,
                    "PLAN_ABANDONED_NO_SUCCESSOR",
                )
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
            // D-196's axis pair, and the same treatment for the same reason: a
            // usage line is a key axis, so a pair in the wrong place is a
            // precondition failure about where the row is filed rather than a
            // malformed field. The subject is the pair rather than one field —
            // the two halves are only wrong *together*.
            D::UsageLineAxisMismatch(detail) => PlanResource::failed_precondition()
                .with_precondition_violation(
                    "meter,dimension_key",
                    detail,
                    "USAGE_LINE_AXIS_MISMATCH",
                )
                .create(),
            // `SupersessionInstantPassed`'s twin, mapped the same way for the same
            // reason: §5 types both 422, which this platform renders 400, and the
            // instant parses perfectly well — it is simply behind a clock the
            // request cannot see. Two codes over one floor, because the operator is
            // told which act they were performing.
            D::CutoverInstantPassed(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("cutover_at", detail, "CUTOVER_INSTANT_PASSED")
                .create(),
            // A composed unit that would leave the key uncovered. §5 types it 422 and
            // declares the code; the supersession's identical refusal has none, which
            // is why that one still travels as `LIFECYCLE_FORBIDDEN`.
            D::CutoverGap(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("cutover_at", detail, "CUTOVER_GAP")
                .create(),
            // Slice 11's three architectural 422s (§5), rendered 400 with the code
            // as the discriminator; see the module note.
            //
            // All three are precondition failures rather than malformed arguments,
            // and each names the field an operator actually edits: the body parsed
            // in every case, and what is wrong is a relationship between the request
            // and the world (a target's lifecycle state, a tenant's notice policy,
            // a delta set) rather than the shape of a value.
            D::MigrationTargetInvalid(d) => {
                precondition("target_plan_id", &d, migration::MIGRATION_TARGET_INVALID)
            }
            // D-49. The field is `effective_at` and not the policy, because the
            // policy is a different object under a different grant - an operator
            // who cannot wait must go and change that one, audited, first.
            D::MigrationNoticeTooShort(d) => {
                precondition("effective_at", &d, migration::MIGRATION_NOTICE_TOO_SHORT)
            }
            // The deltas ride the detail, enumerated: "blocked" alone is not an
            // actionable sentence when the remedy is to change the target, scope a
            // named subscription out, or fix a named add-on.
            D::MigrationBlocked(d) => precondition("scope", &d, migration::MIGRATION_BLOCKED),
            // D-63's future-only start. An architectural 422 (§5) rendered 400,
            // and a precondition failure rather than a malformed argument for
            // `TIMESTAMP_PRECISION_EXCEEDED`'s reason: the instant parses and is a
            // perfectly good instant, it is simply behind a clock the request
            // cannot see. It is the one of the four window refusals that is **not**
            // a conflict — a retry unchanged only puts the start further into the
            // past.
            D::WindowStartInPast(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("effective_from", detail, "WINDOW_START_IN_PAST")
                .create(),
            // `inst-su-instant`'s two floors. An architectural 422 (§5) rendered
            // 400, and a precondition failure rather than a conflict for the reason
            // its neighbour above is one — the clock moved, not the world. It is
            // **not** a retry, though: the design set's remedy is that the unit is
            // recomposed against a fresh instant, and the detail carries that word,
            // because a caller who resubmits the same instant is refused identically
            // and lands further behind the floor each time.
            D::SupersessionInstantPassed(detail) => PlanResource::failed_precondition()
                .with_precondition_violation(
                    "changeover_instant",
                    detail,
                    crate::domain::supersession::SUPERSESSION_INSTANT_PASSED,
                )
                .create(),
            // `inst-fg-trailing`, fail-closed under D-182. An architectural 422 (§5)
            // rendered 400, and a precondition failure rather than a conflict for
            // the reason its sibling above is one, with the sign reversed: nothing
            // moved under the caller. The coverage this refuses to remove is the
            // coverage they composed the request against, so a retry unchanged is
            // refused identically — which is exactly what a 409 would falsely
            // promise a remedy for.
            //
            // The subject is `effective_to` on both producing paths, including the
            // cancel: a cancellation *is* the removal of an interval's coverage, and
            // naming the field the operator can still move — shorten to an instant
            // inside the floor, or schedule a successor — is D-146's line.
            D::WindowTrailingVoid(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("effective_to", detail, "WINDOW_TRAILING_VOID")
                .create(),
            // §5's `THRESHOLD_INVALID`, an architectural 422 rendered 400 with the
            // code as the discriminator. The subject is `entries` on every producing
            // path: all five shape rules are about the entry set - its size, its
            // keys, and the basis inside each - and there is no second field on the
            // request an operator could move instead. `effective_from` has its own
            // refusal one layer down (`TIMESTAMP_PRECISION_EXCEEDED`), so folding it
            // in here would give one field two codes.
            D::ThresholdInvalid(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("entries", detail, "THRESHOLD_INVALID")
                .create(),

            // -- Aborted (409) -- conflicts the caller can resolve and retry.
            D::DuplicateScopeKey(detail) => PlanResource::aborted(detail)
                .with_reason(crate::domain::import::DUPLICATE_SCOPE_KEY)
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
            // The same class one construct over (D-159): somebody else's write
            // reached the aggregate's serialization point first. It belongs with
            // the four above and not with `Internal`, because "retry" is the
            // whole remedy - and a 500 tells a client to page an operator about
            // a race.
            D::ConcurrentMutation(detail) => PlanResource::aborted(detail)
                .with_reason("CONCURRENT_MUTATION")
                .create(),
            // A uniqueness conflict on the plan's one draft slot (D-146), the
            // `DUPLICATE_SCOPE_KEY` class rather than a state-machine edge —
            // which is why it is here and not beside `LIFECYCLE_FORBIDDEN`.
            D::OpenDraftRevisionExists(detail) => PlanResource::aborted(detail)
                .with_reason("OPEN_DRAFT_REVISION_EXISTS")
                .create(),
            // One bundle per plan. The conflict class for the line above's
            // reason — a uniqueness conflict on a slot the plan has one of — and
            // the reason string is `BUNDLE_EXISTS_ON_PLAN` because §5 declares no
            // code for it and a caller still needs a discriminator that is not
            // the message. Owed-register entry B-9 asks the design set to
            // declare one; until it does this is the gear naming a refusal it
            // owns rather than borrowing a code that names something else.
            D::BundleExistsOnPlan(detail) => PlanResource::aborted(detail)
                .with_reason("BUNDLE_EXISTS_ON_PLAN")
                .create(),
            // A decision that lost a race with another decision
            // (`design/05-governance.md` §5, which types it 409 outright rather
            // than as one of the section's architectural 422s). It joins the
            // conflict class and not the precondition one: re-reading the record
            // is the remedy, and the caller's own request was never wrong.
            // §5's `PRECEDENCE_DUPLICATE`, one of the two overlay codes it
            // types **409 outright** rather than as an architectural 422. It
            // belongs in the conflict class for `WINDOW_OVERLAP`'s reason: what
            // refused the request is the state of a sibling row on a slot the
            // caller's own request never named.
            D::PrecedenceDuplicate(detail) => PlanResource::aborted(detail)
                .with_reason(crate::domain::overlay_rules::PRECEDENCE_DUPLICATE)
                .create(),
            // §5's second 409: the overlay analogue of `WINDOW_OVERLAP`, and
            // classified with it. Two intervals on one line key claim one
            // instant, and re-reading the key's overlays is the remedy.
            D::OverlayIntervalOverlap(detail) => PlanResource::aborted(detail)
                .with_reason(crate::domain::overlay_rules::OVERLAY_INTERVAL_OVERLAP)
                .create(),
            D::TaxonomyValueInUse(detail) => PlanResource::aborted(detail)
                .with_reason(crate::domain::taxonomy::TAXONOMY_VALUE_IN_USE)
                .create(),
            D::PriceRowAbsent(detail) => PlanResource::not_found(detail)
                .with_resource(crate::api::rest::preview::PRICE_ROW_ABSENT)
                .create(),
            // A 404 that carries a code, for the line above's reason: the plan in
            // the path may exist and be perfectly editable, holding only a draft.
            // "Not found" alone sends an operator looking for a missing id.
            D::CloneSourceNotFound(detail) => PlanResource::not_found(detail)
                .with_resource(crate::api::rest::plans::CLONE_SOURCE_NOT_FOUND)
                .create(),
            D::RegionUnknown(detail) => PlanResource::failed_precondition()
                .with_precondition_violation(
                    "scope_key.region",
                    detail,
                    crate::domain::taxonomy::REGION_UNKNOWN,
                )
                .create(),
            D::ApprovalNotPending(detail) => PlanResource::aborted(detail)
                .with_reason("APPROVAL_NOT_PENDING")
                .create(),
            // The one-pending-unit-per-subject conflict
            // (`07-pricewindow-linkage.md` `inst-co-single-pending`). It sits in
            // the conflict class with `OPEN_DRAFT_REVISION_EXISTS`, which is the
            // same shape one construct over: a slot the subject has exactly one
            // of is taken, the caller's request was never malformed, and the
            // remedy is to act on the unit the detail names.
            D::PendingChangeUnitExists(detail) => PlanResource::aborted(detail)
                .with_reason("PENDING_CHANGE_UNIT_EXISTS")
                .create(),
            // The TOCTOU pin's own refusal (`inst-ap-pin`). It joins the
            // conflict class rather than the precondition one for the reason
            // `APPROVAL_NOT_PENDING` does: the reviewer's request was right
            // about the world they were shown, and somebody else moved it.
            D::ApprovalContentMismatch(detail) => PlanResource::aborted(detail)
                .with_reason("APPROVAL_CONTENT_MISMATCH")
                .create(),
            // Two intervals on one canonical scope key claim one instant
            // (`07-pricewindow-linkage.md` §5, which types this 409 outright
            // rather than as one of the section's architectural 422s). It joins
            // the conflict class for `APPROVAL_NOT_PENDING`'s reason: the
            // request was right about what the caller could see, and what
            // refused it is a sibling window on a key their own request never
            // named.
            D::WindowOverlap(detail) => PlanResource::aborted(detail)
                .with_reason("WINDOW_OVERLAP")
                .create(),
            // A mutation of what §4 froze (`inst-ws-immutable`), 409 in §5's own
            // words. It joins the conflict class rather than the precondition one
            // because what refused it is where the clock stands: a `PATCH`
            // composed against a window whose end was five minutes away was right
            // when it was written, and re-reading the window is the remedy.
            D::WindowHistoricalImmutable(detail) => PlanResource::aborted(detail)
                .with_reason("WINDOW_HISTORICAL_IMMUTABLE")
                .create(),
            // The cancel path's own refusal (`inst-ws-cancel`), 409 in §5's own
            // words and kept apart from the line above deliberately: an operator
            // told the window is immutable stops, and one told it is not
            // *cancellable* shortens it through `effectiveTo` instead, which is
            // the operation §4 leaves them.
            D::WindowNotCancellable(detail) => PlanResource::aborted(detail)
                .with_reason("WINDOW_NOT_CANCELLABLE")
                .create(),
            // `inst-re-references`, 409 in §5's own words. The conflict class for
            // `TaxonomyValueInUse`'s reason and it is the same fact one plane
            // over: what refused the retirement is a bundle or an add-on
            // override the caller's own request never named, and going to look
            // at the enumerated referrers is the whole remedy.
            D::RetirePlanReferenced(detail) => PlanResource::aborted(detail)
                .with_reason(crate::domain::retirement::RETIRE_PLAN_REFERENCED)
                .create(),
            // The line above's sibling, and a conflict for the same reason: what
            // refuses the retirement is a live schedule aimed at this plan, which
            // the caller's own request never named. Kept apart from it because the
            // remedies are different acts on different objects - one goes and
            // re-composes a bundle, the other cancels a migration or waits for it.
            D::RetireTargetOfMigration(detail) => PlanResource::aborted(detail)
                .with_reason(crate::domain::migration::RETIRE_TARGET_OF_MIGRATION)
                .create(),
            // D-34's one named refusal, 409 in §5's own words. A conflict on
            // mutable state and not a 400: the request was well-formed and the
            // authority sufficient, the run simply finished first. The code names
            // **completion** rather than the pre-D-34 `MIGRATION_ALREADY_EFFECTIVE`
            // because an `in_progress` run past its effective date is still
            // cancellable, so "already effective" named the wrong fact.
            D::MigrationCompleted(detail) => PlanResource::aborted(detail)
                .with_reason(crate::domain::migration::MIGRATION_COMPLETED)
                .create(),

            // -- PermissionDenied (403) -- the two audited authority refusals.
            //
            // Both are **403 in the design set's own words** (§5), and both are
            // written to `pricing_audit_log` as a `deny` record before they
            // reach here. Neither is retriable and neither names a field to fix,
            // which is why they are not in the 400 bucket with the malformed
            // requests: there is nothing about the request to correct, only
            // somebody else to ask.
            //
            // The detail is **dropped** rather than folded into `reason`; see
            // the module note on why the code has to stand alone there.
            D::SelfApprovalForbidden(_detail) => PlanResource::permission_denied()
                .with_reason("SELF_APPROVAL_FORBIDDEN")
                .create(),
            D::RegionScopeDenied(_detail) => PlanResource::permission_denied()
                .with_reason("REGION_SCOPE_DENIED")
                .create(),

            // An architectural 422 (§5) rendered 400, with the code as the
            // discriminator; see the module note. It is a precondition failure
            // and not a malformed argument because the body parsed — the field
            // is simply absent, and `reason` is the field an author fills in.
            D::ReasonRequired(detail) => PlanResource::failed_precondition()
                .with_precondition_violation("reason", detail, "REASON_REQUIRED")
                .create(),

            // -- The aggregate validation envelope (architectural 422, rendered 400) --
            //
            // Every blocking violation is carried as its own precondition
            // violation, so the author sees the whole remediation list in one
            // response. Advisory warnings are deliberately NOT rendered here:
            // this envelope exists only on the rejecting path, and a warning must
            // reach the author on the succeeding path too.
            //
            // **What this used to claim next is false and is withdrawn** (D-197,
            // 2026-08-05): it said the warning "rides the validation report on the
            // response body". No response body carries it — all three publish
            // success views are `{ outcome, materiality, approval, receipt }`, and
            // the pre-check's report is read only to build *this* envelope. So
            // `PLAN_SIZE_SOFT_CAP_EXCEEDED` and `TIER_BAND_PRICE_INCREASE` are
            // computed and discarded, and D-160's reason for making the caps
            // advisory rather than blocking has no channel to be advisory through.
            // Leaving warnings out of this envelope is still right; what is owed is
            // the field on the succeeding path, and it is owed by the group that
            // owns the publish response DTO.
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
