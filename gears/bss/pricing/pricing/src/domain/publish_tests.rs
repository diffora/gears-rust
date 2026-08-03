//! What the publish vocabulary promises the audit trail and the surface.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::{PlanPublishUnit, PublishAuthorization, PublishReceipt};
use crate::domain::evaluation_policy::EVALUATION_POLICY_GENERATION;
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

#[test]
fn a_receipt_composes_the_three_segment_snapshot_ref() {
    // The commit's three artifacts as one composite (D-162): the pending
    // handle, the rows it published, and the generation their content is to be
    // read under. Before D-162 the third had no producer and the composite
    // could not be built at all outside a test.
    let receipt = PublishReceipt::new(
        unit(),
        "pend-9".to_owned(),
        vec![Uuid::from_u128(31), Uuid::from_u128(32)],
        2,
    );
    let snapshot = receipt.snapshot_ref();

    assert_eq!(
        snapshot.version_ref(),
        &VersionRef::Pending("pend-9".into())
    );
    assert_eq!(
        snapshot.price_ids(),
        [Uuid::from_u128(31), Uuid::from_u128(32)]
    );
    assert_eq!(
        snapshot.evaluation_policy_version(),
        EVALUATION_POLICY_GENERATION
    );
}

#[test]
fn a_receipts_snapshot_ref_carries_no_version_the_registry_has_not_given() {
    // The receipt's ref is structurally pending, and so is the composite it
    // builds: finalizing is `CatalogVersionPublished`'s, never the commit's.
    let snapshot = PublishReceipt::new(unit(), "pend-9".to_owned(), Vec::new(), 0).snapshot_ref();

    assert!(!snapshot.version_ref().is_committed());
}
