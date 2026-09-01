//! `domain::recognized` — each rule probed on the case whose absence would
//! ship the defect its instruction names.

use super::{MemberState, SetKind, declaration_verdict, member_edge, meter_pair_complete};
use crate::domain::error::DomainError;

/// The four admitted edges, by name.
#[test]
fn the_member_machine_admits_exactly_four_edges() {
    for (from, to) in [
        (MemberState::Active, MemberState::Deprecated),
        (MemberState::Deprecated, MemberState::Removed),
        (MemberState::Deprecated, MemberState::Active),
        (MemberState::Removed, MemberState::Active),
    ] {
        member_edge(from, to).expect("an admitted edge");
    }
}

/// **`active → removed` is refused** — the machine's whole safety property
/// is that deprecation blocks new declarations first.
#[test]
fn a_direct_removal_is_refused() {
    let err = member_edge(MemberState::Active, MemberState::Removed)
        .expect_err("de-listing runs active -> deprecated -> removed, never in one step");
    assert_eq!(err.code(), "ILLEGAL_TRANSITION");
}

/// The diagonal and the remaining pairs are refused too — the list is
/// closed, not a default.
#[test]
fn the_edge_list_is_closed() {
    for from in [
        MemberState::Active,
        MemberState::Deprecated,
        MemberState::Removed,
    ] {
        member_edge(from, from).expect_err("no self-edge is admitted");
    }
    member_edge(MemberState::Removed, MemberState::Deprecated)
        .expect_err("a tombstone re-enters the set as active or not at all");
}

/// A new declaration's three verdicts: active admits, deprecated refuses
/// with its own code, removed and unknown are one refusal — the tombstone is
/// outside the set.
#[test]
fn a_new_declaration_reads_the_set_not_the_column() {
    declaration_verdict("gib_month", Some(MemberState::Active)).expect("an active unit admits");
    let dep = declaration_verdict("gib_month", Some(MemberState::Deprecated))
        .expect_err("a deprecated unit refuses NEW declarations");
    assert_eq!(dep.code(), "UNIT_DEPRECATED");
    let gone = declaration_verdict("gib_month", Some(MemberState::Removed))
        .expect_err("a removed member is outside the set");
    assert_eq!(gone.code(), "UNRECOGNIZED_UNIT");
    let unknown =
        declaration_verdict("gib_month", None).expect_err("an unknown code is not in the set");
    assert_eq!(unknown.code(), "UNRECOGNIZED_UNIT");
}

/// The atomic pair, in both directions of incompleteness — and the refusal
/// names the half that is missing, not merely that one is.
#[test]
fn the_meter_pair_travels_together_or_not_at_all() {
    meter_pair_complete(None, None).expect("no declaration is a complete non-declaration");
    meter_pair_complete(Some("gib_month"), Some("usage:storage")).expect("the whole pair");
    let missing_usage =
        meter_pair_complete(Some("gib_month"), None).expect_err("half a declaration is refused");
    assert_eq!(missing_usage.code(), "METER_DECLARATION_INCOMPLETE");
    assert!(
        matches!(missing_usage, DomainError::MeterDeclarationIncomplete(ref d) if d.contains("without usage_type_ref")),
        "got {missing_usage}"
    );
    let missing_unit = meter_pair_complete(None, Some("usage:storage"))
        .expect_err("the other half is refused the same way");
    assert!(
        matches!(missing_unit, DomainError::MeterDeclarationIncomplete(ref d) if d.contains("without metering_unit")),
        "got {missing_unit}"
    );
}

/// The kind roster round-trips and refuses everything outside it, and each
/// kind's blocked-removal code is the design's own.
#[test]
fn the_kind_roster_and_its_refusal_codes() {
    for kind in [
        SetKind::MeteringUnit,
        SetKind::TaxCategory,
        SetKind::GlCode,
        SetKind::PlanTier,
    ] {
        assert_eq!(SetKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(SetKind::parse("units"), None, "no alias, no default");
    assert_eq!(
        SetKind::MeteringUnit.delist_blocked(String::new()).code(),
        "UNIT_DELIST_BLOCKED"
    );
    assert_eq!(
        SetKind::PlanTier.delist_blocked(String::new()).code(),
        "PLAN_TIER_RETIRE_BLOCKED"
    );
    assert_eq!(
        SetKind::TaxCategory.delist_blocked(String::new()).code(),
        "ACCOUNTING_CODE_DELIST_BLOCKED"
    );
    assert_eq!(
        SetKind::GlCode.delist_blocked(String::new()).code(),
        "ACCOUNTING_CODE_DELIST_BLOCKED"
    );
}
