//! The three pure pieces of the workflow, proven where they are pure.
//!
//! The storage-backed half — that a real pending unit is really voided by a real
//! mutation, that a self-approval really lands a `deny` row — is
//! `tests/sqlite_approval_service.rs`, where there is a database to put rows in.
//! What is here is what needs none: how a subject ref resolves to a plan, which
//! regions a change set touches, and which wire refusal each judgement becomes.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use chrono::{TimeZone, Utc};
use serde_json::json;
use uuid::Uuid;

use super::{RegionGrant, grant_against, independent_approver, refusal_to_domain, regions_of};
use crate::domain::approval::{ApprovalState, DecisionRefusal};
use crate::domain::audit::AuditSubjectKind;
use crate::domain::concurrency::RowVersion;
use crate::domain::error::DomainError;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::CurrencyCode;
use crate::domain::plan_shape::PlanShape;
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{ModelKind, PriceRow};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use crate::infra::storage::repo::approval_repo::ApprovalRecord;

const PLAN: Uuid = Uuid::from_u128(0x91a4);

fn record_with_subject(subject_ref: &str) -> ApprovalRecord {
    ApprovalRecord {
        approval_id: Uuid::from_u128(0xa1),
        tenant_id: Uuid::from_u128(0x7e11),
        subject_ref: subject_ref.to_owned(),
        subject_kind: AuditSubjectKind::PlanRevision,
        content_hash: vec![0xde, 0xad],
        state: ApprovalState::Submitted,
        submitter_principal: Uuid::from_u128(0x5b01),
        approver_principal: None,
        reason: None,
        materiality: json!({}),
        submitted_at: Utc.with_ymd_and_hms(2026, 8, 3, 9, 0, 0).unwrap(),
        decided_at: None,
    }
}

// ---------------------------------------------------------------------------
// Which plan a unit is about
// ---------------------------------------------------------------------------

/// The ref `audit_repo::plan_revision_ref` mints round-trips.
///
/// Written against a ref that function **produced**, not against a literal: the
/// two are one contract, and a literal here would keep passing the day the
/// renderer changed its separator.
/// Through `approval_repo::subject_plan` — the **parse**, which is what stayed pure
/// when `plan_of` grew a store read. A `window` subject's plan comes off the
/// window's scope key rather than out of its ref, so the resolution as a whole now
/// needs a runner and is asserted in `tests/sqlite_window_service.rs`; the parse
/// does not, and this is it.
#[test]
fn a_plan_revision_ref_resolves_to_its_plan() {
    let plan = PlanId::new(PLAN);
    let rendered = crate::infra::storage::repo::audit_repo::plan_revision_ref(plan, 7);
    assert_eq!(
        crate::infra::storage::repo::approval_repo::subject_plan(&record_with_subject(&rendered))
            .unwrap(),
        plan
    );
}

/// A ref this crate cannot have written is an invariant breach, not a caller's
/// mistake: the CHECKs admit any text, so an unparseable ref means the table was
/// written around.
///
/// **Both halves**: the parse answers `CorruptRow`, and the rendering an operator
/// meets is a 500 rather than a 400. The second assertion is what makes this test
/// about the *classification* — a `CorruptRow` quietly re-mapped onto the
/// bad-request family would tell a caller to fix a request they cannot fix.
#[test]
fn a_subject_ref_that_is_not_a_plan_revision_is_an_internal_fault() {
    for malformed in ["", "not-a-uuid/3", "3", &PLAN.to_string()] {
        let err = crate::infra::storage::repo::approval_repo::subject_plan(&record_with_subject(
            malformed,
        ))
        .expect_err("this is not a plan revision ref");
        assert!(
            matches!(err, crate::infra::storage::RepoError::CorruptRow(_)),
            "{malformed:?} gave {err:?}"
        );
        assert!(
            matches!(
                crate::infra::storage::repo_failure(&err),
                DomainError::Internal(_)
            ),
            "{malformed:?} must reach the wire as a 500, not a 400"
        );
    }
}

// ---------------------------------------------------------------------------
// Which regions a change set touches
// ---------------------------------------------------------------------------

fn row(market: &str) -> PriceRecord {
    let scope_key = ScopeKey::new(
        PlanId::new(PLAN),
        CurrencyCode::new("USD").expect("three letters"),
        Region::new(market).expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0x11)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("all_subscriptions pairs with cohort none");
    PriceRecord {
        price_id: Uuid::new_v4(),
        scope_key,
        row: PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat)),
        tax_inclusive: false,
        billing_timing: None,
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
        lifecycle_state: LifecycleState::Draft,
        created_by: Uuid::from_u128(0xac10),
        created_at_utc: Utc.with_ymd_and_hms(2026, 8, 3, 9, 0, 0).unwrap(),
        row_version: RowVersion::new(0),
    }
}

fn shape_over(markets: &[&str]) -> PlanShape {
    let mut shape = PlanShape::new(
        PlanId::new(PLAN),
        3,
        Utc.with_ymd_and_hms(2026, 8, 3, 12, 0, 0).unwrap(),
    );
    shape.rows = markets.iter().map(|market| row(market)).collect();
    shape
}

#[test]
fn the_change_sets_regions_are_the_union_over_its_rows() {
    let regions = regions_of(&shape_over(&["EU", "US", "EU"]));
    let expected: BTreeSet<Region> = ["EU", "US"]
        .iter()
        .map(|value| Region::new(value).expect("a non-blank region"))
        .collect();
    assert_eq!(regions, expected);
}

/// A pure plan-shape revision touches no region at all.
///
/// Not a degenerate case: D-115 makes shape-only revisions — a phase duration, a
/// GL code, an add-on rule — always material, so they reach the approval
/// workflow routinely and carry zero price rows when they do.
#[test]
fn a_revision_with_no_price_rows_touches_no_region() {
    assert!(regions_of(&shape_over(&[])).is_empty());
}

// ---------------------------------------------------------------------------
// Which set the scope rule measures the approver against
// ---------------------------------------------------------------------------

/// **The untransported grant is resolved here, off the argument, and never off a
/// second read.**
///
/// That is the whole content of the 2026-08-04 fix. The value is what the
/// surface used to compute for itself; the difference is *when* — the surface's
/// read happened before the judgement transaction, so a mutation adding a row in
/// a new region between the two made `is_subset` fail and answered `OutOfScope`,
/// which `DecisionRefusal::is_an_audited_violation` files as a `deny` record
/// against a reviewer who exceeded no authority. `judge` passes the change set it
/// re-derived itself, so there is no second world for the two halves of the rule
/// to disagree about.
#[test]
fn an_untransported_grant_is_the_change_set_it_is_measured_against() {
    let change_set = regions_of(&shape_over(&["EU", "US"]));

    assert_eq!(
        grant_against(&RegionGrant::Untransported, &change_set),
        change_set,
        "an untransported grant covers exactly what it is measured against, so the rule holds"
    );
    assert!(
        change_set.is_subset(&grant_against(&RegionGrant::Untransported, &change_set)),
        "which is the property `authorize_decision` then evaluates"
    );
}

/// An explicit grant is taken exactly as given, **narrower included**.
///
/// The variant exists so the rule stays falsifiable: collapsing the field into
/// the synthesis above would leave `inst-ap-scope`'s refusing direction with no
/// caller anywhere in the crate, and `REGION_SCOPE_DENIED` would be a code no
/// test could reach.
#[test]
fn an_explicit_grant_is_taken_exactly_as_given() {
    let change_set = regions_of(&shape_over(&["EU", "US"]));
    let narrow: BTreeSet<Region> = BTreeSet::from([Region::new("EU").expect("a region")]);

    assert_eq!(
        grant_against(&RegionGrant::Explicit(narrow.clone()), &change_set),
        narrow
    );
    assert!(
        !change_set.is_subset(&narrow),
        "and it is narrow enough to refuse, or this asserts nothing"
    );
}

// ---------------------------------------------------------------------------
// Which wire refusal each judgement becomes
// ---------------------------------------------------------------------------

/// Every refusal maps to a distinct domain variant, and the detail names the
/// record.
///
/// The mapping is exhaustive by construction — `refusal_to_domain` matches on
/// the enum — but *which* variant each becomes is a classification decision
/// (403 vs 409 vs 400), and this is where it is written down once.
#[test]
fn each_refusal_becomes_the_variant_its_status_needs() {
    let id = Uuid::from_u128(0xa1);
    for refusal in DecisionRefusal::ALL {
        let err = refusal_to_domain(*refusal, id);
        let matched = matches!(
            (refusal, &err),
            (
                DecisionRefusal::SelfApproval,
                DomainError::SelfApprovalForbidden(_)
            ) | (
                DecisionRefusal::OutOfScope,
                DomainError::RegionScopeDenied(_)
            ) | (
                DecisionRefusal::ContentMismatch,
                DomainError::ApprovalContentMismatch(_)
            ) | (
                DecisionRefusal::NotPending,
                DomainError::ApprovalNotPending(_)
            ) | (
                DecisionRefusal::ReasonRequired,
                DomainError::ReasonRequired(_)
            )
        );
        assert!(matched, "{refusal:?} became {err:?}");
        assert!(
            err.to_string().contains(&id.to_string()),
            "the refusal must name the record: {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// Which approved records actually authorize
// ---------------------------------------------------------------------------

/// An approved record built to order, so the two invariant breaches below can be
/// staged at all.
///
/// **They cannot be staged through the store, and that is the point of testing the
/// predicate rather than a route.** `chk_pricing_approval_distinct_principals`
/// (`approver_principal IS NULL OR approver_principal <> submitter_principal`) and
/// `chk_pricing_approval_approver` (`state IN ('submitted','voided') OR
/// approver_principal IS NOT NULL`) are in both backends, so no writer in this crate
/// can produce either row. What this guards is the case those CHECKs did not run in
/// — a migration, a restore, a later slice's writer — which is exactly the argument
/// `authorization_of` already carries for the same rule on the publish path.
fn approved_by(submitter: Uuid, approver: Option<Uuid>) -> ApprovalRecord {
    ApprovalRecord {
        state: ApprovalState::Approved,
        submitter_principal: submitter,
        approver_principal: approver,
        decided_at: Some(Utc.with_ymd_and_hms(2026, 8, 3, 10, 0, 0).unwrap()),
        ..record_with_subject("019fd000-0000-7000-8000-000000000000/0")
    }
}

/// The ordinary case: two principals, so the record authorizes and names the second.
#[test]
fn an_approved_record_decided_by_a_second_principal_authorizes() {
    let submitter = Uuid::from_u128(0x5b01);
    let approver = Uuid::from_u128(0xa99a);

    let answer = independent_approver(&approved_by(submitter, Some(approver)));

    assert_eq!(answer.expect("two principals authorize"), approver);
}

/// One principal twice authorizes nothing — the whole of `inst-tp-distinct`.
#[test]
fn an_approved_record_naming_one_principal_twice_authorizes_nothing() {
    let one = Uuid::from_u128(0x5b01);

    let answer = independent_approver(&approved_by(one, Some(one)));

    assert!(
        matches!(answer, Err(DomainError::SelfApprovalForbidden(_))),
        "a record naming {one} as both submitter and approver is not a second signature: {answer:?}"
    );
}

/// An `approved` record with no approver at all is an invariant breach, not an
/// approval — and it is a *different* answer from the self-approval above, because
/// the two name different faults to whoever reads the log.
#[test]
fn an_approved_record_with_no_approver_is_an_invariant_breach() {
    let answer = independent_approver(&approved_by(Uuid::from_u128(0x5b01), None));

    assert!(
        matches!(answer, Err(DomainError::Internal(_))),
        "an approved record carrying no approver is a broken invariant: {answer:?}"
    );
}
