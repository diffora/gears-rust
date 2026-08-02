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
    // Same category as a frozen revision, deliberately different sentence. The
    // caller asked for a successor, not to edit content, so `NotDraft`'s "only
    // draft content is mutable" would answer a question nobody asked and would
    // name as the remedy — open the next revision — the very operation being
    // refused. What an operator branches on is the category; what an operator
    // reads is the sentence, and only one of the two was ever right.
    let err = RepoError::NoSuccessorRevision {
        plan_id: "3f2a".to_owned(),
        state: "retired".to_owned(),
    };

    let DomainError::LifecycleForbidden(detail) = repo_failure(&err) else {
        panic!("a plan that can never be superseded must map to LIFECYCLE_FORBIDDEN");
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
fn a_second_open_draft_is_forbidden_state_and_names_the_one_that_exists() {
    // A plan has at most one concurrently editable shape. The refusal points at
    // the revision holding the slot, so the caller edits it instead of guessing.
    let err = RepoError::OpenDraftExists {
        plan_id: "3f2a".to_owned(),
        revision: 2,
    };

    let DomainError::LifecycleForbidden(detail) = repo_failure(&err) else {
        panic!("a second open draft must map to LIFECYCLE_FORBIDDEN");
    };
    assert!(detail.contains("revision 2"), "got: {detail}");
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
fn a_horizon_on_a_class_that_may_not_carry_one_is_a_bad_request_not_a_500() {
    // The arm that exists because a physical CHECK was reaching callers as a
    // driver error: `grandfather_until` is ordinary draft content, and pairing
    // it with the wrong eligibility class used to become `RepoError::Db` and
    // therefore `Internal` — a 500 for a request whose author only has to clear
    // one field. The class is named so the sentence says which key refused.
    let err = RepoError::GrandfatherHorizonOffClass {
        eligibility: "all_subscriptions".to_owned(),
    };

    let DomainError::InvalidRequest(detail) = repo_failure(&err) else {
        panic!("a horizon off its class must map to INVALID_REQUEST");
    };
    assert!(detail.contains("all_subscriptions"), "got: {detail}");
    assert!(detail.contains("existing_grandfathered"), "got: {detail}");
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
