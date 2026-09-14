//! Tests for the governance gate's host contract.
//!
//! Two things are measured here and they are not the same thing. The first is
//! the **contract**: what a verdict is allowed to say, and which of its
//! answers a caller may spend. The second is the **default host** this slice
//! ships — the one that holds no record store — and what it answers under
//! each mode, together with the reason it answers that.
//!
//! The default host never refuses under [`GateMode::Gate`], so nothing it
//! does can prove the refusal path is wired at all. That path is measured
//! against a test double declared in this file
//! ([`AlwaysRefusesGate`]), which is the only thing standing between
//! `inst-fd-gate-mode-gate` and a code path no test has ever taken.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use uuid::Uuid;

use bss_products_sdk::models::EntityKind;

use super::{
    ApprovalDisposition, ApprovalId, EntityRef, GateMode, GateSubject, GateVerdict, GovernanceGate,
    NoMaterialityPolicyGate,
};
use crate::domain::concurrency::InternalRevision;
use crate::domain::error::DomainError;

/// A fixed subject, so no case has to invent one and no two cases differ by
/// an identity that is not what they are measuring.
fn entity() -> EntityRef {
    EntityRef {
        tenant_id: Uuid::from_u128(0x11),
        entity_kind: EntityKind::Sku,
        entity_id: Uuid::from_u128(0x22),
    }
}

/// The same subject as the seam now expresses it (P-D-67 arm 4), built
/// through the entity constructor the arm keeps.
fn subject() -> GateSubject {
    GateSubject::entity_publish(entity(), InternalRevision::new(1))
}

/// A host that refuses every question, whatever the mode.
///
/// The default host authorizes under [`GateMode::Gate`], so the refusal arm
/// of the seam has no production caller yet. Without a double the mapping
/// from a `no` to `APPROVAL_REQUIRED` would be untested code, and the first
/// real host — slice 05's — would be the thing that discovered whether it
/// worked.
struct AlwaysRefusesGate;

impl GovernanceGate for AlwaysRefusesGate {
    fn evaluate(&self, _subject: GateSubject, _mode: GateMode) -> Result<GateVerdict, DomainError> {
        Ok(GateVerdict::Refused {
            reason: "no satisfied approval record pinned to this revision".to_owned(),
        })
    }
}

/// A host that authorizes and names a record to be consumed, standing in for
/// the shape slice 05 will ship under [`GateMode::Gate`].
struct AuthorizesAndNamesARecord {
    approval: ApprovalId,
    uncomposed_bundle_override: bool,
}

impl GovernanceGate for AuthorizesAndNamesARecord {
    fn evaluate(&self, _subject: GateSubject, mode: GateMode) -> Result<GateVerdict, DomainError> {
        let disposition = match mode {
            GateMode::Gate => ApprovalDisposition::Consume(self.approval),
            GateMode::PreAuthorized(id) => ApprovalDisposition::Verified(id),
        };
        Ok(GateVerdict::authorized(
            disposition,
            self.uncomposed_bundle_override,
            "the case's own host authorized this act".to_owned(),
        ))
    }
}

/// Under `Gate`, the default host authorizes — and the record it names is
/// **none**, so the publish transaction has nothing to flip `consumed`.
///
/// The reason matters as much as the verdict: this is not "an approval was
/// granted", it is "no materiality policy is registered, so this act needs no
/// approval ceremony at all". Slice 05 registers the policy that turns the
/// same question into `APPROVAL_REQUIRED`.
#[test]
fn the_default_host_authorizes_under_gate_with_no_record_to_consume() {
    let verdict = NoMaterialityPolicyGate
        .evaluate(subject(), GateMode::Gate)
        .expect("the default host reaches an answer without a store");

    let authorization = verdict
        .into_authorization()
        .expect("the default host authorizes under Gate");
    assert_eq!(authorization.disposition, ApprovalDisposition::NoRecord);
    assert_eq!(authorization.approval_to_consume(), None);
    assert_eq!(authorization.approval_ref(), None);
    assert!(!authorization.uncomposed_bundle_override);
}

/// Under `PreAuthorized`, the default host **refuses**.
///
/// The mode's whole contract is to verify that the named record authorized
/// this subject at this revision. A host with no record store can verify
/// nothing, and accepting an unverifiable approval id would be fail-open.
#[test]
fn the_default_host_refuses_under_preauthorized_because_it_can_verify_nothing() {
    let id = ApprovalId::new(Uuid::from_u128(0x33));
    let verdict = NoMaterialityPolicyGate
        .evaluate(subject(), GateMode::PreAuthorized(id))
        .expect("refusing is an answer, not a host failure");

    let error = verdict
        .into_authorization()
        .expect_err("an unverifiable approval id is never authorized");
    assert!(matches!(error, DomainError::ApprovalRequired(_)));
    assert_eq!(error.code(), "APPROVAL_REQUIRED");
}

/// A `no` from any host becomes `APPROVAL_REQUIRED`, carrying the host's own
/// reason.
///
/// This is the arm the default host never takes; see [`AlwaysRefusesGate`].
#[test]
fn a_host_that_refuses_under_gate_produces_approval_required() {
    let verdict = AlwaysRefusesGate
        .evaluate(subject(), GateMode::Gate)
        .expect("a refusal is an answer, not a host failure");

    let error = verdict
        .into_authorization()
        .expect_err("a refusing host must not authorize");
    match error {
        DomainError::ApprovalRequired(reason) => {
            assert_eq!(
                reason,
                "no satisfied approval record pinned to this revision"
            );
        }
        other => panic!("expected APPROVAL_REQUIRED, got {}", other.code()),
    }
}

/// Under `Gate` a named record is spendable: the publish transaction is told,
/// by the type, which id to flip `consumed` (`inst-fd-publish-consume`).
#[test]
fn an_authorized_gate_verdict_names_the_record_the_transaction_must_consume() {
    let id = ApprovalId::new(Uuid::from_u128(0x44));
    let host = AuthorizesAndNamesARecord {
        approval: id,
        uncomposed_bundle_override: true,
    };

    let authorization = host
        .evaluate(subject(), GateMode::Gate)
        .expect("the double reaches an answer")
        .into_authorization()
        .expect("the double authorizes");

    assert_eq!(authorization.disposition, ApprovalDisposition::Consume(id));
    assert_eq!(authorization.approval_to_consume(), Some(id));
    assert_eq!(authorization.approval_ref(), Some(id));
    assert!(authorization.uncomposed_bundle_override);
}

/// Under `PreAuthorized` the same record is **not** spendable: the id is
/// still what `approval_ref` stores, but nothing is consumed.
///
/// The distinction lives in the type rather than in a caller's memory —
/// `approval_to_consume` is the only way to reach an id for the consume flip,
/// and it answers `None` here.
#[test]
fn a_preauthorized_verdict_names_a_record_but_offers_nothing_to_consume() {
    let id = ApprovalId::new(Uuid::from_u128(0x55));
    let host = AuthorizesAndNamesARecord {
        approval: id,
        uncomposed_bundle_override: false,
    };

    let authorization = host
        .evaluate(subject(), GateMode::PreAuthorized(id))
        .expect("the double reaches an answer")
        .into_authorization()
        .expect("the double authorizes");

    assert_eq!(authorization.disposition, ApprovalDisposition::Verified(id));
    assert_eq!(authorization.approval_to_consume(), None);
    assert_eq!(authorization.approval_ref(), Some(id));
}

/// A verdict with no record offers nothing to consume and nothing to store in
/// `approval_ref`, which is the shape every act under the default host takes.
#[test]
fn a_verdict_with_no_record_is_spendable_nowhere() {
    let authorization = GateVerdict::authorized(
        ApprovalDisposition::NoRecord,
        false,
        "no ceremony applied".to_owned(),
    )
    .into_authorization()
    .expect("an authorized verdict authorizes");

    assert_eq!(authorization.approval_to_consume(), None);
    assert_eq!(authorization.approval_ref(), None);
}

/// The reason survives on a `yes` as well as on a `no` — `inst-fd-gate-verdict`
/// puts the reason on both answers, and an audit row for an authorized act
/// records why it was authorized.
#[test]
fn an_authorized_verdict_still_carries_its_reason() {
    let authorization = NoMaterialityPolicyGate
        .evaluate(subject(), GateMode::Gate)
        .expect("the default host reaches an answer")
        .into_authorization()
        .expect("the default host authorizes under Gate");

    assert!(
        authorization.reason.contains("no materiality policy"),
        "the default host must say why it authorized, not merely that it did: {}",
        authorization.reason
    );
}
