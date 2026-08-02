//! The `RepoError` -> `DomainError` ladder, arm by arm.
//!
//! `repo_failure` is the single place a storage failure becomes a rejection the
//! rest of the system reasons about, so a wrong arm here is a wrong HTTP status
//! and a wrong wire code everywhere at once. Each case below pins the *category*
//! a refusal lands in, which is the part a consumer branches on.

use super::{RepoError, repo_failure};
use crate::domain::concurrency::{RowVersion, require_match};
use crate::domain::error::DomainError;

#[test]
fn an_absent_row_and_a_foreign_tenants_row_give_the_same_404() {
    // The repository already collapses the two cases; the ladder must not
    // re-open them. Subject and id survive verbatim so the response names what
    // the caller asked for, not what the store happens to call it.
    let err = RepoError::NotFound {
        subject: "plan revision".to_owned(),
        id: "3f2a/1".to_owned(),
    };

    assert_eq!(
        repo_failure(&err),
        DomainError::NotFound {
            subject: "plan revision".to_owned(),
            id: "3f2a/1".to_owned(),
        }
    );
}

#[test]
fn a_stale_write_is_a_retryable_conflict_naming_both_versions() {
    // 409 with both numbers: the caller refetches and retries, and an operator
    // reading the log can tell a caller that never refreshed from a bulk run
    // colliding with interactive editing.
    let err = RepoError::StaleRowVersion {
        subject: "plan revision".to_owned(),
        id: "3f2a/1".to_owned(),
        current: 7,
        submitted: 4,
    };

    let DomainError::StaleVersion(detail) = repo_failure(&err) else {
        panic!("a stale row version must map to STALE_VERSION");
    };
    assert!(detail.contains("current 7"), "got: {detail}");
    assert!(detail.contains("submitted 4"), "got: {detail}");
    assert!(detail.contains("plan revision 3f2a/1"), "got: {detail}");
}

#[test]
fn the_store_and_the_pre_check_spell_one_collision_one_way() {
    // The pre-check refuses from a version it has already read; the store
    // refuses from the row itself. They are two code paths reporting one
    // event, and an operator correlating a log line with a 409 body has to see
    // the same numbers in the same clause — otherwise a runbook grows two
    // entries for one failure. The store's line adds a `{subject} {id}:`
    // prefix, which is the only difference this asserts is allowed.
    let from_pre_check =
        require_match(RowVersion::new(7), RowVersion::new(4)).expect_err("7 != 4 is stale");
    let DomainError::StaleVersion(pre_check) = from_pre_check else {
        panic!("require_match must refuse with STALE_VERSION");
    };

    let DomainError::StaleVersion(from_store) = repo_failure(&RepoError::StaleRowVersion {
        subject: "plan revision".to_owned(),
        id: "3f2a/1".to_owned(),
        current: 7,
        submitted: 4,
    }) else {
        panic!("a stale row version must refuse with STALE_VERSION");
    };

    assert!(
        from_store.ends_with(&pre_check),
        "the store said {from_store:?}, the pre-check said {pre_check:?}"
    );
}

#[test]
fn editing_a_frozen_revision_is_forbidden_state_and_never_a_conflict() {
    // The distinction is the point of the variant. A conflict invites a retry;
    // no retry will make a published revision editable, and a caller told
    // "conflict" would loop until an operator read the logs.
    let err = RepoError::NotDraft {
        subject: "plan revision".to_owned(),
        id: "3f2a/0".to_owned(),
        state: "published".to_owned(),
    };

    let DomainError::LifecycleForbidden(detail) = repo_failure(&err) else {
        panic!("a frozen revision must map to LIFECYCLE_FORBIDDEN");
    };
    assert!(detail.contains("published"), "got: {detail}");
}

#[test]
fn a_plan_that_can_never_be_superseded_is_refused_in_its_own_words() {
    // Its own code as well as its own sentence (D-146). The caller asked for a
    // successor, not to edit content, so `NotDraft`'s "only draft content is
    // mutable" would answer a question nobody asked and would name as the remedy
    // — open the next revision — the very operation being refused. Sharing
    // `LIFECYCLE_FORBIDDEN` left the sentence carrying that distinction alone,
    // which is exactly the prose a consumer cannot branch on.
    let err = RepoError::NoSuccessorRevision {
        plan_id: "3f2a".to_owned(),
        state: "retired".to_owned(),
    };

    let DomainError::PlanRetiredNoSuccessor(detail) = repo_failure(&err) else {
        panic!("a plan that can never be superseded must map to PLAN_RETIRED_NO_SUCCESSOR");
    };
    assert!(detail.contains("3f2a"), "got: {detail}");
    assert!(detail.contains("retired"), "got: {detail}");
    assert!(
        detail.contains("never be superseded"),
        "the refusal must name the ground it actually rests on, got: {detail}"
    );
    assert!(
        !detail.contains("only draft content is mutable"),
        "the content-mutability sentence is the one this variant exists to \
         stop being told, got: {detail}"
    );
}

#[test]
fn a_second_open_draft_is_a_conflict_and_names_the_one_that_exists() {
    // A plan has at most one concurrently editable shape. The refusal points at
    // the revision holding the slot, so the caller edits it instead of guessing.
    // It is a uniqueness conflict on that slot rather than a transition the
    // state machine denies (D-146), which is why it leaves the lifecycle
    // category entirely.
    let err = RepoError::OpenDraftExists {
        plan_id: "3f2a".to_owned(),
        revision: 2,
    };

    let DomainError::OpenDraftRevisionExists(detail) = repo_failure(&err) else {
        panic!("a second open draft must map to OPEN_DRAFT_REVISION_EXISTS");
    };
    assert!(detail.contains("revision 2"), "got: {detail}");
}

#[test]
fn a_retired_plan_and_a_taken_draft_slot_are_told_apart_without_reading_prose() {
    // The test that would have passed before D-146 and must fail after it: both
    // refusals used to arrive as `LIFECYCLE_FORBIDDEN`, so a consumer branching
    // on the domain vocabulary saw one answer where the operator has two
    // actions. "Stop, this plan is retired" ends the attempt; "you already hold
    // an open draft at revision 2" sends the operator somewhere real.
    let retired = repo_failure(&RepoError::NoSuccessorRevision {
        plan_id: "3f2a".to_owned(),
        state: "retired".to_owned(),
    });
    let occupied = repo_failure(&RepoError::OpenDraftExists {
        plan_id: "3f2a".to_owned(),
        revision: 2,
    });

    assert_ne!(
        std::mem::discriminant(&retired),
        std::mem::discriminant(&occupied),
        "one code for two operator actions is the collapse D-146 exists to undo"
    );
    assert!(
        !matches!(retired, DomainError::LifecycleForbidden(_)),
        "the retired plan keeps the code it was narrowed into: {retired:?}"
    );
    assert!(
        !matches!(occupied, DomainError::LifecycleForbidden(_)),
        "and so does the occupied draft slot: {occupied:?}"
    );
    // What `LIFECYCLE_FORBIDDEN` keeps is the refusals with no alternative
    // action to describe: a frozen revision, and the projector's own frontier.
    assert!(matches!(
        repo_failure(&RepoError::NotDraft {
            subject: "plan revision".to_owned(),
            id: "3f2a/0".to_owned(),
            state: "published".to_owned(),
        }),
        DomainError::LifecycleForbidden(_)
    ));
    assert!(matches!(
        repo_failure(&RepoError::FrontierRegression {
            tenant: "7e11".to_owned(),
            current: 9,
            requested: 8,
        }),
        DomainError::LifecycleForbidden(_)
    ));
}

#[test]
fn an_occupied_scope_key_keeps_its_rendering_verbatim() {
    // The eight-axis rendering is the whole diagnostic: re-wrapping it in
    // another sentence, or dropping an axis, would report a collision between
    // rows that do not actually share a key.
    let key = "3f2a|USD|EU|base|9c1|all_subscriptions|recurring|none";
    let err = RepoError::DuplicateScopeKey(key.to_owned());

    assert_eq!(
        repo_failure(&err),
        DomainError::DuplicateScopeKey(key.to_owned())
    );
}

#[test]
fn a_reused_key_carrying_a_different_payload_is_its_own_refusal() {
    // Not a stale-version conflict, which would invite the one retry that can
    // never succeed: the stored response answers a different request, so
    // replaying it reports work nobody asked for and re-executing breaks the
    // at-most-once promise. The key and the operation survive into the detail
    // so an operator can find the caller that reused it.
    let err = RepoError::IdempotencyPayloadMismatch {
        operation: "create_price".to_owned(),
        client_key: "ck-9f2a".to_owned(),
    };

    let DomainError::IdempotencyPayloadMismatch(detail) = repo_failure(&err) else {
        panic!("a reused key with a different payload must map to IDEMPOTENCY_PAYLOAD_MISMATCH");
    };
    assert!(detail.contains("ck-9f2a"), "got: {detail}");
    assert!(detail.contains("create_price"), "got: {detail}");
}

#[test]
fn a_quantity_no_column_can_hold_is_the_callers_mistake_not_the_stores() {
    // The distinction this arm draws is the whole reason it is not
    // `CorruptRow`: a `package_size` past the `bigint` range arrived on a
    // request and the author can change it, so it is a 400-class rejection. The
    // identical mismatch read back *out* of the column would mean the table had
    // been written around, and that is an internal fault nobody can reshape a
    // request to avoid.
    let err = RepoError::ValueOutOfRange {
        field: "package_size".to_owned(),
        value: "18446744073709551615".to_owned(),
    };

    let DomainError::InvalidRequest(detail) = repo_failure(&err) else {
        panic!("an unstorable authored value must map to INVALID_REQUEST");
    };
    assert!(detail.contains("package_size"), "got: {detail}");
    assert!(detail.contains("18446744073709551615"), "got: {detail}");
}

#[test]
fn a_horizon_on_a_class_that_may_not_carry_one_carries_its_own_code() {
    // The arm that exists because a physical CHECK was reaching callers as a
    // driver error: `grandfather_until` is ordinary draft content, and pairing
    // it with the wrong eligibility class used to become `RepoError::Db` and
    // therefore `Internal` — a 500 for a request whose author only has to clear
    // one field. D-147 states the pairing as a rule, so the refusal no longer
    // borrows the generic bad-request answer, which named no field and told a
    // consumer nothing about which one to clear. The class is named so the
    // sentence says which key refused.
    let err = RepoError::GrandfatherHorizonOffClass {
        eligibility: "all_subscriptions".to_owned(),
    };

    let DomainError::GrandfatherUntilForbidden(detail) = repo_failure(&err) else {
        panic!("a horizon off its class must map to GRANDFATHER_UNTIL_FORBIDDEN");
    };
    assert!(detail.contains("all_subscriptions"), "got: {detail}");
    assert!(detail.contains("existing_grandfathered"), "got: {detail}");
    // Still not an internal fault, which is the property the arm was added for.
    assert!(!matches!(repo_failure(&err), DomainError::Internal(_)));
}

#[test]
fn an_instant_below_the_quantum_is_refused_where_it_would_have_been_stored() {
    // `timestamptz` holds microseconds, so nothing downstream refuses this: the
    // value persists and is then matched for equality against an instant another
    // gear produced at the quantum (D-144). The field is named because clearing
    // or rounding one value is the whole remedy.
    let err = RepoError::TimestampPrecisionExceeded {
        field: "grandfatherUntil".to_owned(),
        value: "2026-08-02T12:00:00.000500+00:00".to_owned(),
    };

    let DomainError::TimestampPrecisionExceeded(detail) = repo_failure(&err) else {
        panic!("a sub-millisecond instant must map to TIMESTAMP_PRECISION_EXCEEDED");
    };
    assert!(detail.contains("grandfatherUntil"), "got: {detail}");
    assert!(
        detail.contains("2026-08-02T12:00:00.000500"),
        "got: {detail}"
    );
}

#[test]
fn a_duplicate_meeting_an_unanswered_claim_is_its_own_refusal() {
    // Not the payload mismatch beside it, and not a fabricated reply: nothing
    // has been told to anybody yet, so the only honest answer is that the key is
    // held and a retry will find it either answered or mismatching (D-143). The
    // key and the operation survive so an operator can find the caller holding
    // it.
    let err = RepoError::IdempotencyKeyInFlight {
        operation: "create_price".to_owned(),
        client_key: "ck-9f2a".to_owned(),
    };

    let DomainError::IdempotencyKeyInFlight(detail) = repo_failure(&err) else {
        panic!("an unanswered claim must map to IDEMPOTENCY_KEY_IN_FLIGHT");
    };
    assert!(detail.contains("ck-9f2a"), "got: {detail}");
    assert!(detail.contains("create_price"), "got: {detail}");
}

#[test]
fn a_driver_fault_and_a_corrupt_reading_stay_internal() {
    // Neither is a caller mistake and neither is reshapeable into a request
    // that would succeed, which is exactly the line between a bad request and
    // an internal fault.
    for err in [
        RepoError::Db("connection reset".to_owned()),
        RepoError::CorruptRow("lifecycle_state holds 'pending'".to_owned()),
    ] {
        assert!(matches!(repo_failure(&err), DomainError::Internal(_)));
    }
}
