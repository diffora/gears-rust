//! What the publish vocabulary promises the audit trail and the surface.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::{PlanPublishUnit, PublishAuthorization, PublishReceipt};
use crate::domain::scope_key::PlanId;
use crate::domain::snapshot::VersionRef;
use uuid::Uuid;

fn unit() -> PlanPublishUnit {
    PlanPublishUnit::new(PlanId::new(Uuid::from_u128(7)), 3)
}

#[test]
fn an_approved_publish_carries_its_record_and_both_principals() {
    let approval = Uuid::from_u128(11);
    let submitter = Uuid::from_u128(12);
    let approver = Uuid::from_u128(13);

    let auth = PublishAuthorization::approved(approval, submitter, approver);

    assert_eq!(auth.approval_ref(), Some(approval));
    assert_eq!(auth.principals(), Some((submitter, approver)));
}

#[test]
fn an_auto_publishable_change_carries_neither() {
    let auth = PublishAuthorization::auto_publishable();

    assert_eq!(auth.approval_ref(), None);
    // Not "the actor approved itself": there is no second principal, and
    // saying so is what keeps the trail honest about which publishes had one.
    assert_eq!(auth.principals(), None);
}

#[test]
fn the_two_person_check_is_deliberately_not_enforced_here() {
    // `inst-tp-distinct` is enforceable in one line and is Slice 5's, because
    // `inst-tp-selfaudit` binds its refusal to an audit record this group
    // cannot write. Half the rule enforced here would give it two owners.
    let principal = Uuid::from_u128(21);
    let auth = PublishAuthorization::approved(Uuid::from_u128(20), principal, principal);

    assert_eq!(auth.principals(), Some((principal, principal)));
}

#[test]
fn a_receipt_holds_a_pending_handle_and_never_a_version() {
    let receipt = PublishReceipt::new(
        unit(),
        "pend-42".to_owned(),
        vec![Uuid::from_u128(31), Uuid::from_u128(32)],
        0,
    );

    assert_eq!(
        receipt.version_ref(),
        &VersionRef::Pending("pend-42".into())
    );
    assert!(!receipt.version_ref().is_committed());
    assert_eq!(receipt.version_ref().committed(), None);
    assert_eq!(receipt.version_ref().pending_ref(), Some("pend-42"));
}

#[test]
fn a_receipt_names_the_unit_and_what_it_moved() {
    let receipt = PublishReceipt::new(unit(), "pend-1".to_owned(), vec![Uuid::from_u128(31)], 4);

    assert_eq!(receipt.plan_id(), PlanId::new(Uuid::from_u128(7)));
    assert_eq!(receipt.revision(), 3);
    assert_eq!(receipt.published_price_ids(), [Uuid::from_u128(31)]);
    assert_eq!(receipt.audit_seq(), 4);
}
