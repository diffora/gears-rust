//! Exhaustive `DomainError` → `CanonicalError` ladder check: every variant maps
//! to its expected canonical category, and the fail-closed rejections carry the
//! agreed wire code. This locks the machine-readable error contract — the codes
//! are what a consumer branches on once the coarse category is known.

use toolkit::api::canonical_prelude::CanonicalError;

use crate::domain::coverage::{AVAILABILITY_OUTSIDE_COVERAGE, WINDOW_COVERAGE_MISSING, WINDOW_GAP};
use crate::domain::error::DomainError;
use crate::domain::validation::ValidationReport;

fn status(err: DomainError) -> u16 {
    CanonicalError::from(err).status_code()
}

fn rendered(err: DomainError) -> String {
    // The wire code lives in the problem body, not the status line; rendering
    // through Debug is enough to assert it survives the ladder.
    format!("{:?}", CanonicalError::from(err))
}

/// The `reason` of a 403, read off the typed context rather than out of `Debug`.
///
/// Exact rather than a substring match, and that is the point: `reason` is the
/// **only** free field a `permission_denied()` has, so it is where a consumer
/// reads the code — and a code with anything else appended to it is a code no
/// exact match finds. See the ladder's module note.
///
/// # Panics
/// When the arm is not a permission denial at all, which is the other half of
/// the same guard.
fn permission_reason(err: DomainError) -> String {
    match CanonicalError::from(err) {
        CanonicalError::PermissionDenied { ctx, .. } => ctx.reason,
        other => panic!("expected a 403 permission denial, got {other:?}"),
    }
}

/// The `reason` of a 409, read off the typed context rather than out of `Debug`.
///
/// The sibling of [`permission_reason`], and it exists because that function's
/// argument is **not** special to 403s: a `contains` assertion over the rendered
/// `Debug` string cannot see a code with something *appended* to it. Corrupting
/// `WINDOW_OVERLAP` into `WINDOW_OVERLAPX` left `the_wire_codes_survive_the_ladder`
/// green, because the corrupted string contains the correct one — found while
/// proving that arm by removal. `reason` is the field a consumer branches on, so the
/// assertion has to be an equality against it.
///
/// G0 reported the other fifteen rather than changing lines it did not own, and
/// that report was acted on the same day — but the first pass claimed more than
/// it did, and G0's own independent review caught the claim: four governance
/// codes were still asserted by containment, and the sentence here said none
/// were. Corrected in the same session. **Every positive code assertion in this
/// module now reads its code off the typed context and compares it by
/// equality** — 403 here, 409 here, the 400 class and the validation envelope's
/// fan-out through [`precondition_codes`] — and each is proved by corruption:
/// appending one character to any of them reddens its test.
///
/// The `!…contains(…)` assertions that remain are sound as containment and stay:
/// a document that does not contain a substring cannot contain a code built on
/// it either, so appending a character cannot defeat a negative.
///
/// # Panics
/// When the arm is not a conflict at all, which is the other half of the same guard.
fn aborted_reason(err: DomainError) -> String {
    match CanonicalError::from(err) {
        CanonicalError::Aborted { ctx, .. } => ctx.reason,
        other => panic!("expected a 409 conflict, got {other:?}"),
    }
}

/// The `type_` of every precondition violation on a 400, read off the typed
/// context rather than out of `Debug`.
///
/// The third sibling, and the one that closes the hole for the whole 400 class —
/// which is most of the ladder. [`aborted_reason`]'s note explains the mechanism
/// on one code; the same measurement holds here for **fifteen**: until
/// 2026-08-04 `the_wire_codes_survive_the_ladder` asserted every 400 code with
/// `rendered(..).contains("CODE")` over the whole problem document, so a code
/// with anything **appended** to it satisfied its own assertion. That is not a
/// hypothetical: `WINDOW_OVERLAP` corrupted to `WINDOW_OVERLAPX` left the suite
/// green, which is how the hole was found.
///
/// Reading the field a consumer discriminates on, and comparing it by equality,
/// is what makes the assertion evidence about the code rather than about the
/// document containing something like it.
fn precondition_codes(err: DomainError) -> Vec<String> {
    match CanonicalError::from(err) {
        CanonicalError::FailedPrecondition { ctx, .. } => {
            ctx.violations.into_iter().map(|v| v.type_).collect()
        }
        other => panic!("expected a 400 failed precondition, got {other:?}"),
    }
}

/// The `subject` of a 400 that carries exactly one violation — the field the
/// caller is told to move.
///
/// Read off the typed context and asserted by equality, for `precondition_code`'s
/// reason: the code says *what rule* was broken and the subject says *where*, and a
/// violation naming a field the request does not carry is a refusal an operator
/// cannot act on.
///
/// # Panics
/// When the arm is not a 400, or renders more than one violation.
fn precondition_field(err: DomainError) -> String {
    match CanonicalError::from(err) {
        CanonicalError::FailedPrecondition { ctx, .. } => {
            assert_eq!(ctx.violations.len(), 1, "expected exactly one violation");
            ctx.violations
                .into_iter()
                .next()
                .expect("one violation")
                .subject
        }
        other => panic!("expected a 400 failed precondition, got {other:?}"),
    }
}

/// The single precondition code of a 400 that carries exactly one violation.
fn precondition_code(err: DomainError) -> String {
    let codes = precondition_codes(err);
    assert_eq!(
        codes.len(),
        1,
        "this arm must render exactly one violation, got {codes:?}"
    );
    codes.into_iter().next().expect("one violation")
}

#[test]
fn conflicts_are_409() {
    // The five Foundation-owned conflicts and the two `05-governance.md` §5
    // adds. A caller resolves them by refetching and retrying, which is exactly
    // what 409 tells it to do. Two joined the class in 2026-08-02's wave and
    // were classified by what they are rather than by the section they were
    // found in: an unanswered claim is resolved by retrying (D-143), and a
    // second draft revision is a uniqueness conflict on the plan's one editable
    // slot, not a state-machine edge (D-146).
    let detail = || "detail".to_owned();

    assert_eq!(status(DomainError::DuplicateScopeKey(detail())), 409);
    assert_eq!(status(DomainError::StaleVersion(detail())), 409);
    assert_eq!(
        status(DomainError::IdempotencyPayloadMismatch(detail())),
        409
    );
    assert_eq!(status(DomainError::IdempotencyKeyInFlight(detail())), 409);
    assert_eq!(status(DomainError::OpenDraftRevisionExists(detail())), 409);
    // The two governance conflicts. §5 types both 409 outright, and each is a
    // race an entitled reviewer lost rather than a request to correct: the
    // record moved under a decision, or the subject did.
    assert_eq!(status(DomainError::ApprovalNotPending(detail())), 409);
    assert_eq!(status(DomainError::ApprovalContentMismatch(detail())), 409);
    // The third, added when the submit path got a writer for it. Its class is
    // `OPEN_DRAFT_REVISION_EXISTS`'s: a slot the subject has one of is taken.
    assert_eq!(status(DomainError::PendingChangeUnitExists(detail())), 409);
    // The window store's own conflict, and the one code of S7 §5's eight that
    // section types 409 rather than as an architectural 422: two intervals on
    // one canonical scope key claim one instant.
    assert_eq!(status(DomainError::WindowOverlap(detail())), 409);
    // Its two siblings, which §5 types 409 outright as well. Both tell a caller
    // the clock moved under a request that was right when it was written, which
    // is the class 409 names — and neither is the fourth window refusal, whose
    // request was never right and which is asserted with the 400s below.
    assert_eq!(
        status(DomainError::WindowHistoricalImmutable(detail())),
        409
    );
    assert_eq!(status(DomainError::WindowNotCancellable(detail())), 409);
    // D-09's own two codes, `WindowOverlap`'s reason one plane over: the request
    // was well-formed and what refused it is a sibling membership on the payer
    // their own request never named. §5 types both 409 outright.
    assert_eq!(status(DomainError::MembershipOverlap(detail())), 409);
    assert_eq!(status(DomainError::MembershipConflict(detail())), 409);
}

/// The two authority refusals of §5, **403 and carrying only their code**.
///
/// Both halves are one guard and neither was executable until this test: the
/// category, because a `SELF_APPROVAL_FORBIDDEN` rendered 409 with a retry hint
/// invites a client to retry the self-approval it was just refused; and the
/// exactness of `reason`, because the code is what a consumer branches on and a
/// `"CODE: detail"` string is not that code.
#[test]
fn the_two_authority_refusals_are_403_carrying_exactly_their_code() {
    let detail = || "approval 3f2a: the submitter may not decide it".to_owned();

    assert_eq!(status(DomainError::SelfApprovalForbidden(detail())), 403);
    assert_eq!(status(DomainError::RegionScopeDenied(detail())), 403);

    // Spelled against the domain's own constants: the ladder holds a literal, so
    // this is the line that fails if the two ever part company.
    assert_eq!(
        permission_reason(DomainError::SelfApprovalForbidden(detail())),
        crate::domain::approval::SELF_APPROVAL_FORBIDDEN
    );
    assert_eq!(
        permission_reason(DomainError::RegionScopeDenied(detail())),
        crate::domain::approval::REGION_SCOPE_DENIED
    );
}

#[test]
fn fail_closed_publish_rejections_render_400_not_the_documented_422() {
    // The design set's 422s are architectural: the platform's canonical family
    // has no 422 (FailedPrecondition renders 400), so the code — not the status
    // — is the discriminator (Foundation 3.3). Pinned so the rendering is a
    // checked fact rather than a surprise at the first consumer.
    let detail = || "detail".to_owned();

    assert_eq!(status(DomainError::RoundingPolicyUnresolved(detail())), 400);
    assert_eq!(status(DomainError::PrecisionExceeded(detail())), 400);
    assert_eq!(
        status(DomainError::TimestampPrecisionExceeded(detail())),
        400
    );
    assert_eq!(status(DomainError::AmountNegative(detail())), 400);
    assert_eq!(status(DomainError::CurrencyInvalid(detail())), 400);
    assert_eq!(status(DomainError::FixtureMissing(detail())), 400);
    assert_eq!(status(DomainError::LifecycleForbidden(detail())), 400);
    assert_eq!(status(DomainError::PlanRetiredNoSuccessor(detail())), 400);
    assert_eq!(
        status(DomainError::GrandfatherUntilForbidden(detail())),
        400
    );
    // `05-governance.md` §5 types this one 422, which is the same architectural
    // 422 the nine above are: the body parsed and a field is missing, so it is a
    // precondition failure carrying its code at 400 — never a 422, because no
    // path can produce one.
    assert_eq!(status(DomainError::ReasonRequired(detail())), 400);
    // D-63's future-only start, `07-pricewindow-linkage.md` §5's fourth window
    // refusal and the only one of the four that is not a conflict: the instant
    // parses, and a retry of the same request only puts the start further behind.
    assert_eq!(status(DomainError::WindowStartInPast(detail())), 400);
    // `inst-fg-trailing`'s refusal, fail-closed under D-182. §5 types it 422, so
    // it lands here with the nine above and not in the conflict class: unlike
    // `WINDOW_OVERLAP` and `WINDOW_HISTORICAL_IMMUTABLE`, nothing moved under the
    // caller — the coverage it declines to remove is the coverage they composed
    // the request against, and a retry unchanged is refused identically.
    assert_eq!(status(DomainError::WindowTrailingVoid(detail())), 400);
    // `inst-su-instant`'s changeover floor. §5 types it 422, so it lands here for
    // the same reason as the two above and is not a conflict: the clock moved, not
    // the world. It is not a retry either — the design set's remedy is a
    // recomposition — but "not retryable" is not "a conflict", and 409 would
    // falsely promise that a refresh resolves it.
    assert_eq!(
        status(DomainError::SupersessionInstantPassed(detail())),
        400
    );
}

/// `inst-su-instant`'s code survives the ladder.
///
/// **Its own test rather than a line in the window list**, for the reason the
/// trailing void has its own: that list is closed at four by
/// [`WindowRefusal`](crate::domain::window::WindowRefusal), and appending a
/// supersession refusal to it would invite the next reader to append it to that
/// *type* — which would break the partition test below over something that is not a
/// window refusal at all. This one is about a **row-plane act's** instant; the
/// window it goes on to schedule is a consequence.
///
/// There is no `repo_failure` half to assert: the floor compares two instants the
/// caller and the clock supply, so it is raised in `domain` and no `RepoError`
/// variant corresponds. `repo_failure`'s `match` carries no wildcard, so one added
/// later fails to compile until it is mapped.
#[test]
fn the_changeover_floor_code_survives_the_ladder() {
    assert_eq!(
        precondition_code(DomainError::SupersessionInstantPassed("detail".to_owned())),
        crate::domain::supersession::SUPERSESSION_INSTANT_PASSED
    );
}

#[test]
fn the_wire_codes_survive_the_ladder() {
    let detail = || "detail".to_owned();

    assert_eq!(
        precondition_code(DomainError::RoundingPolicyUnresolved(detail())),
        "ROUNDING_POLICY_UNRESOLVED"
    );
    assert_eq!(
        precondition_code(DomainError::PrecisionExceeded(detail())),
        "PRECISION_EXCEEDED"
    );
    assert_eq!(
        precondition_code(DomainError::AmountNegative(detail())),
        "AMOUNT_NEGATIVE"
    );
    assert_eq!(
        precondition_code(DomainError::CurrencyInvalid(detail())),
        "CURRENCY_INVALID"
    );
    assert_eq!(
        precondition_code(DomainError::FixtureMissing(detail())),
        "FIXTURE_MISSING"
    );
    assert_eq!(
        aborted_reason(DomainError::DuplicateScopeKey(detail())),
        "DUPLICATE_SCOPE_KEY"
    );
    assert_eq!(
        aborted_reason(DomainError::StaleVersion(detail())),
        "STALE_VERSION"
    );
    assert_eq!(
        aborted_reason(DomainError::IdempotencyPayloadMismatch(detail())),
        "IDEMPOTENCY_PAYLOAD_MISMATCH"
    );
    assert_eq!(
        aborted_reason(DomainError::IdempotencyKeyInFlight(detail())),
        "IDEMPOTENCY_KEY_IN_FLIGHT"
    );
    assert_eq!(
        aborted_reason(DomainError::OpenDraftRevisionExists(detail())),
        "OPEN_DRAFT_REVISION_EXISTS"
    );
    assert_eq!(
        precondition_code(DomainError::TimestampPrecisionExceeded(detail())),
        "TIMESTAMP_PRECISION_EXCEEDED"
    );
    assert_eq!(
        precondition_code(DomainError::PlanRetiredNoSuccessor(detail())),
        "PLAN_RETIRED_NO_SUCCESSOR"
    );
    assert_eq!(
        precondition_code(DomainError::GrandfatherUntilForbidden(detail())),
        "GRANDFATHER_UNTIL_FORBIDDEN"
    );
    // The governance codes, spelled against the domain's own constants rather
    // than second literals: the ladder holds the literal, `domain::approval`
    // holds the constant, and the two must render one string. Until these lines
    // existed the reason on the ladder could be renamed with the whole crate
    // green, which is the coverage their Foundation neighbours above already
    // had. Their statuses are asserted with the class each belongs to —
    // `conflicts_are_409`, `fail_closed_publish_rejections_…` and
    // `the_two_authority_refusals_…` — so one aspect of one arm reddens one
    // test.
    assert_eq!(
        aborted_reason(DomainError::ApprovalNotPending(detail())),
        crate::domain::approval::APPROVAL_NOT_PENDING
    );
    assert_eq!(
        aborted_reason(DomainError::ApprovalContentMismatch(detail())),
        crate::domain::approval::APPROVAL_CONTENT_MISMATCH
    );
    assert_eq!(
        precondition_code(DomainError::ReasonRequired(detail())),
        crate::domain::approval::REASON_REQUIRED
    );
    assert_eq!(
        aborted_reason(DomainError::PendingChangeUnitExists(detail())),
        crate::domain::approval::PENDING_CHANGE_UNIT_EXISTS
    );
    // D-09's two codes. Bare literals, `WindowOverlap`'s own convention beside it:
    // no `domain::membership` module exists to hold a constant, so — like
    // `DUPLICATE_SCOPE_KEY` and `STALE_VERSION` above — the ladder's literal is
    // the only spelling and this is what pins it against a silent rename.
    assert_eq!(
        aborted_reason(DomainError::MembershipOverlap(detail())),
        "MEMBERSHIP_OVERLAP"
    );
    assert_eq!(
        aborted_reason(DomainError::MembershipConflict(detail())),
        "MEMBERSHIP_CONFLICT"
    );
}

/// The four window codes, each spelled against `domain::window`'s own constant
/// rather than a second literal: the ladder holds the literal, the domain holds
/// the constant, and a rename of either alone must be the failure.
///
/// Asserted **exactly** rather than by `contains`, and [`aborted_reason`]'s doc
/// says what that buys: `contains` cannot see a code with a character appended to
/// it. Split out of [`the_wire_codes_survive_the_ladder`] rather than added to it
/// because these four are one section's set and read as one.
#[test]
fn the_window_codes_survive_the_ladder() {
    let detail = || "detail".to_owned();

    assert_eq!(
        aborted_reason(DomainError::WindowOverlap(detail())),
        crate::domain::window::WINDOW_OVERLAP
    );
    assert_eq!(
        aborted_reason(DomainError::WindowHistoricalImmutable(detail())),
        crate::domain::window::WINDOW_HISTORICAL_IMMUTABLE
    );
    assert_eq!(
        aborted_reason(DomainError::WindowNotCancellable(detail())),
        crate::domain::window::WINDOW_NOT_CANCELLABLE
    );
    assert_eq!(
        precondition_code(DomainError::WindowStartInPast(detail())),
        crate::domain::window::WINDOW_START_IN_PAST
    );
}

/// `inst-fg-trailing`'s code survives the ladder, spelled against
/// `domain::coverage`'s constant.
///
/// **Its own test rather than a fifth line in the one above**, and the reason is
/// the reason the constant lives in `domain::coverage`: those four are the state
/// machine's set, closed at four by
/// [`WindowRefusal`](crate::domain::window::WindowRefusal), and appending this one
/// to that list would invite the next reader to append it to that *type* — which
/// would break the partition test one function down for a refusal that is not
/// about a window at all.
///
/// **There is no `repo_failure` half to assert, and that is a fact about the
/// producer rather than a gap in the assertions.** The floor this rule compares
/// against is `max(current coverage end, now + longest_cycle_sold(...))`, and
/// `longest_cycle_sold` takes a [`PlanShape`](crate::domain::plan_shape::PlanShape)
/// — a value assembled from four repositories, which no repository holds while it
/// is executing. So the refusal is raised by `infra::window` and `RepoError` has no
/// corresponding variant to map: an arm asserted here would be an arm nothing can
/// reach. What keeps that honest is `repo_failure`'s own `match`, which carries **no
/// wildcard arm** — a `RepoError::WindowTrailingVoid` added later fails to compile
/// until it is mapped, and `src/infra/storage_tests.rs` is where its rendering would
/// then be asserted, one case per test, as every arm there already is. (The count is
/// not repeated: this sentence said "fifteen" against eighteen, in the same diff that
/// corrected the identical fault two files over.)
#[test]
fn the_trailing_void_code_survives_the_ladder() {
    assert_eq!(
        precondition_code(DomainError::WindowTrailingVoid("detail".to_owned())),
        crate::domain::coverage::WINDOW_TRAILING_VOID
    );
}

/// Every §5 window refusal reaches the wire as **its own** code, and the set is
/// closed by the type rather than by this list.
///
/// The list-shaped assertion above pins each arm; this one pins the **partition**.
/// [`WindowRefusal::ALL`](crate::domain::window::WindowRefusal::ALL) is walked, so
/// a fifth refusal added to the domain fails here until it is mapped, and two
/// refusals collapsed onto one code fail here even though each of them still
/// renders "a" code. That is the failure mode the four collapsed
/// `LIFECYCLE_FORBIDDEN` refusals of D-146 had, one plane over: every arm
/// rendered a code and a consumer could tell none of them apart.
#[test]
fn the_four_window_refusals_are_four_codes_and_the_type_closes_the_set() {
    use crate::domain::window::WindowRefusal;

    let mut seen: Vec<&str> = Vec::new();
    for refusal in WindowRefusal::ALL {
        let code = refusal.code();
        let rendered = match CanonicalError::from(refusal.refuse("detail".to_owned())) {
            CanonicalError::Aborted { ctx, .. } => ctx.reason,
            CanonicalError::FailedPrecondition { ctx, .. } => ctx
                .violations
                .into_iter()
                .map(|v| v.type_)
                .next()
                .expect("one violation"),
            other => {
                panic!("{code} must reach the wire as a conflict or a precondition: {other:?}")
            }
        };
        assert_eq!(rendered, code, "{code} must reach the wire as itself");
        assert!(!seen.contains(&code), "{code} is claimed twice");
        seen.push(code);
    }
    assert_eq!(
        seen.len(),
        4,
        "section 5 declares four refusals about one window"
    );
}

#[test]
fn the_narrowed_lifecycle_refusals_are_three_distinct_codes_not_one() {
    // The test that would have passed before D-146 and must fail after it. Every
    // refusal the authoring plane collapsed onto `LIFECYCLE_FORBIDDEN` used to
    // render that one code, so a consumer branching on the code string could
    // tell none of them apart. Two of them ask an operator for different things
    // — stop, versus go and edit the draft you already have — and one of them is
    // not even a state-machine edge.
    // The one that kept the code, read off the typed context; the two that were
    // freed from it, read off the rendered document because the claim about them is
    // an absence — the code they must NOT carry appears nowhere in it.
    let detail = || "detail".to_owned();
    let retired = rendered(DomainError::PlanRetiredNoSuccessor(detail()));
    let open_draft = rendered(DomainError::OpenDraftRevisionExists(detail()));

    assert_eq!(
        precondition_code(DomainError::LifecycleForbidden(detail())),
        "LIFECYCLE_FORBIDDEN"
    );
    assert!(
        !retired.contains("LIFECYCLE_FORBIDDEN"),
        "a retired plan must not be told the code it was collapsed into: {retired}"
    );
    assert!(
        !open_draft.contains("LIFECYCLE_FORBIDDEN"),
        "a second open draft must not be told it either: {open_draft}"
    );
    // And the conflict half of the narrowing is visible without reading the
    // body at all: the retired plan is a stop, the open draft is retriable.
    assert_eq!(status(DomainError::PlanRetiredNoSuccessor(detail())), 400);
    assert_eq!(status(DomainError::OpenDraftRevisionExists(detail())), 409);
}

#[test]
fn the_validation_envelope_carries_every_blocking_violation() {
    // One round trip must be enough to remediate a plan: if the envelope
    // truncated, an author would publish N times to discover N problems.
    let mut report = ValidationReport::default();
    report.violate("TIER_BANDS_OVERLAP", "price-1", "bands 2 and 3 overlap");
    report.violate("TIER_TOP_BAND_CLOSED", "price-1", "top band must be open");
    report.warn("ADVISORY", "price-1", "not blocking");

    let blocking = precondition_codes(DomainError::ValidationFailed(report.clone()));
    let body = rendered(DomainError::ValidationFailed(report));

    assert_eq!(
        blocking,
        vec![
            "TIER_BANDS_OVERLAP".to_owned(),
            "TIER_TOP_BAND_CLOSED".to_owned()
        ],
        "the envelope carries every blocking violation, and only those"
    );
    // The advisory rides the success path's report, never the error envelope.
    assert!(!body.contains("ADVISORY"));
}

/// The three Slice-7 coverage codes reach the wire, as **violations** and not as
/// variants of their own.
///
/// The phase plan asserts that these three need "no new mapping code, which is
/// why these three codes are not `DomainError` variants". That claim is
/// **verified here rather than inherited**: `WINDOW_COVERAGE_MISSING`,
/// `WINDOW_GAP` and `AVAILABILITY_OUTSIDE_COVERAGE` are raised by registered
/// `ValidationRule`s, so they travel inside `ValidationFailed` and the ladder
/// renders each as one `PreconditionViolation` on a **400** with the code in
/// `type_`. There is no arm to add and none was added.
///
/// Read off the typed context and compared by **equality**, per this module's own
/// measurement: a code with one character appended satisfies a `contains`
/// assertion over the rendered document, and exactly that shipped green here for
/// `WINDOW_OVERLAP`.
#[test]
fn the_three_coverage_codes_travel_as_precondition_violations() {
    let mut report = ValidationReport::default();
    report.violate(WINDOW_COVERAGE_MISSING, "key-1", "no window on the key");
    report.violate(WINDOW_GAP, "key-1", "uncovered between two windows");
    report.violate(AVAILABILITY_OUTSIDE_COVERAGE, "key-1", "sold past coverage");

    assert_eq!(status(DomainError::ValidationFailed(report.clone())), 400);
    assert_eq!(
        precondition_codes(DomainError::ValidationFailed(report)),
        vec![
            WINDOW_COVERAGE_MISSING.to_owned(),
            WINDOW_GAP.to_owned(),
            AVAILABILITY_OUTSIDE_COVERAGE.to_owned()
        ]
    );
}

#[test]
fn unavailability_is_503_and_keeps_its_diagnostic_server_side() {
    // An operator needs to know which dependency is down; a caller only needs
    // to know to retry, and the registry's internals are not its business.
    let body = rendered(DomainError::CatalogVersionUnavailable(
        "registry refused connection at 10.0.0.7".to_owned(),
    ));

    assert_eq!(
        status(DomainError::CatalogVersionUnavailable("x".to_owned())),
        503
    );
    assert_eq!(
        status(DomainError::ReadModelUnavailable("x".to_owned())),
        503
    );
    assert!(!body.contains("10.0.0.7"));
}

#[test]
fn absent_and_scoped_out_share_one_answer() {
    assert_eq!(
        status(DomainError::NotFound {
            subject: "plan".to_owned(),
            id: "3f2a".to_owned(),
        }),
        404
    );
}

#[test]
fn a_bad_request_is_400_and_an_internal_fault_is_500() {
    assert_eq!(status(DomainError::InvalidRequest("x".to_owned())), 400);
    assert_eq!(status(DomainError::Internal("x".to_owned())), 500);
}

#[test]
fn a_registry_failure_becomes_the_fail_closed_domain_variant() {
    use crate::domain::ports::{CatalogVersionRegistryError, registry_failure};

    // Every registry failure lands on the same answer: no version, no publish.
    // Inventing one locally would make this gear a second incrementer.
    for err in [
        CatalogVersionRegistryError::Unconfigured,
        CatalogVersionRegistryError::Unreachable("down".to_owned()),
        CatalogVersionRegistryError::Rejected("unknown sku".to_owned()),
        CatalogVersionRegistryError::Internal("boom".to_owned()),
    ] {
        assert!(matches!(
            registry_failure(&err),
            DomainError::CatalogVersionUnavailable(_)
        ));
    }
}

#[test]
fn a_same_aggregate_contention_is_a_409_with_its_own_code() {
    // D-159. A loser at a per-aggregate serialization point used to reach the
    // caller as `Internal` -> 500, indistinguishable from a dead connection, for
    // a request whose entire remedy is to retry.
    assert_eq!(
        status(DomainError::ConcurrentMutation("plan p".to_owned())),
        409
    );
    assert_eq!(
        aborted_reason(DomainError::ConcurrentMutation("plan p".to_owned())),
        "CONCURRENT_MUTATION"
    );
}

#[test]
fn a_contention_is_not_a_stale_version_and_the_two_are_tellable_apart() {
    // The distinction is the reason the code exists. Nothing the caller presented
    // was stale, so a caller told `STALE_VERSION` would refresh a version that
    // was never wrong - and one whose precondition genuinely failed must still be
    // told to refresh.
    let contention_code = aborted_reason(DomainError::ConcurrentMutation("plan p".to_owned()));
    let stale_code = aborted_reason(DomainError::StaleVersion(
        "current 3, submitted 2".to_owned(),
    ));
    let contention = rendered(DomainError::ConcurrentMutation("plan p".to_owned()));
    let stale = rendered(DomainError::StaleVersion(
        "current 3, submitted 2".to_owned(),
    ));

    assert_eq!(contention_code, "CONCURRENT_MUTATION");
    assert!(!contention.contains("STALE_VERSION"), "{contention}");
    assert_eq!(stale_code, "STALE_VERSION");
    assert!(!stale.contains("CONCURRENT_MUTATION"), "{stale}");
    // Both are 409: each is a conflict a retry resolves, and the code is the
    // discriminator rather than the status (sec 3.3).
    assert_eq!(
        status(DomainError::ConcurrentMutation("plan p".to_owned())),
        status(DomainError::StaleVersion("s".to_owned()))
    );
}

#[test]
fn a_contention_is_not_the_callers_own_duplicate_request() {
    // `IDEMPOTENCY_KEY_IN_FLIGHT`'s subject is the caller's own retry;
    // `CONCURRENT_MUTATION`'s is somebody else's write. Same status, different
    // sentence, and a bulk run branches on which.
    let contention = rendered(DomainError::ConcurrentMutation("plan p".to_owned()));
    assert_eq!(
        aborted_reason(DomainError::ConcurrentMutation("plan p".to_owned())),
        "CONCURRENT_MUTATION"
    );
    assert!(
        !contention.contains("IDEMPOTENCY_KEY_IN_FLIGHT"),
        "{contention}"
    );
}

#[test]
fn a_genuine_storage_failure_is_still_an_internal_fault() {
    // The recognition NARROWS `Internal` rather than swallowing it: a dead
    // connection must not be answered "retry, somebody else got here first".
    assert_eq!(
        status(DomainError::Internal("the disk is gone".to_owned())),
        500
    );
}

/// `THRESHOLD_INVALID` reaches the wire as a **400** carrying its own code.
///
/// Both halves, in one test and asserted by equality off the typed context: §5
/// types it 422 and the canonical family has no such category, so the status is
/// 400 and the code is the whole of the discriminator. A test asserting only the
/// status would pass against every other `failed_precondition` arm in the ladder,
/// which is the failure mode this file exists for.
///
/// The subject is `entries` on every producing path, and that is asserted too:
/// all five of §6's shape rules are about the entry set, so a violation naming any
/// other field would be naming a field the caller cannot move.
#[test]
fn the_threshold_invalid_code_survives_the_ladder() {
    assert_eq!(
        status(DomainError::ThresholdInvalid("detail".to_owned())),
        400
    );
    assert_eq!(
        precondition_code(DomainError::ThresholdInvalid("detail".to_owned())),
        "THRESHOLD_INVALID"
    );
    assert_eq!(
        precondition_field(DomainError::ThresholdInvalid("detail".to_owned())),
        "entries"
    );
}
