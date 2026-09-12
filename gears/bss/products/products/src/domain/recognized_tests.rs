//! `domain::recognized` — each rule probed on the case whose absence would
//! ship the defect its instruction names.

use super::{
    FINANCE_MATERIAL_COLUMNS, MemberState, SetKind, UsageTypeAnswer, declaration_is_new,
    declaration_verdict, is_finance_material, judge_usage_type, member_edge, meter_pair_complete,
};
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

/// P-D-121 row 8: the check runs on a new or changed declaration only.
#[test]
fn the_recognized_and_active_check_judges_only_a_new_or_changed_declaration() {
    assert!(
        declaration_is_new(None, Some("gold"), true),
        "first publish is a new declaration"
    );
    assert!(
        !declaration_is_new(Some("gold"), Some("gold"), false),
        "a carried-forward value is not re-judged"
    );
    assert!(
        declaration_is_new(Some("gold"), Some("silver"), false),
        "a changed declaration is judged against the current set"
    );
}

/// The usage-type resolver's three answers, judged before the transaction.
#[test]
fn the_usage_type_resolver_has_three_answers() {
    let binding = crate::test_support::probe_binding();
    assert_eq!(
        judge_usage_type(UsageTypeAnswer::Resolved(binding.clone()), "usage:ok")
            .expect("resolved admits"),
        binding,
        "the binding rides through to the freeze (dod-binding-snapshot)"
    );
    let unknown = judge_usage_type(UsageTypeAnswer::Unresolved, "usage:gone")
        .expect_err("unknown is USAGE_TYPE_UNRESOLVED");
    assert_eq!(unknown.code(), "USAGE_TYPE_UNRESOLVED");
    let down = judge_usage_type(UsageTypeAnswer::Unavailable, "usage:x")
        .expect_err("unreachable is USAGE_TYPE_UNAVAILABLE");
    assert_eq!(down.code(), "USAGE_TYPE_UNAVAILABLE");
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

/// The stored form of a binding is one JSON object with sorted keys and
/// sorted metadata fields, so equal bindings store equal bytes
/// (`dod-binding-snapshot`).
#[test]
fn a_binding_snapshot_renders_sorted_and_flat() {
    let binding = crate::test_support::probe_binding();
    assert_eq!(
        binding.snapshot_json(),
        r#"{"gts_id":"usage:storage","kind":"counter","metadata_fields":["region","zone"]}"#
    );
}

/// `dod-finance-materiality`'s operand: the two accounting codes and only
/// them — `plan_tier` is Product's — computed from the touched set the submit
/// door already builds.
#[test]
fn finance_materiality_is_the_two_accounting_codes_and_nothing_else() {
    let touched = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
    assert!(is_finance_material(&touched(&["tax_category_ref"])));
    assert!(is_finance_material(&touched(&["name", "gl_code_ref"])));
    assert!(!is_finance_material(&touched(&[
        "plan_tier",
        "sellable",
        "sku_type"
    ])));
    assert!(!is_finance_material(&touched(&[])));
    assert_eq!(
        FINANCE_MATERIAL_COLUMNS,
        ["tax_category_ref", "gl_code_ref"]
    );
}

/// Every set kind names the `products_sku` column its members are declared
/// in — the removal guard's population, uniform across the four
/// (`dod-recognized-set-mechanics`), and each of those columns is registered
/// in the bucket roster.
#[test]
fn every_set_kind_has_a_registered_carrier_column() {
    for kind in [
        SetKind::MeteringUnit,
        SetKind::PlanTier,
        SetKind::TaxCategory,
        SetKind::GlCode,
    ] {
        let column = kind.carrier_column();
        assert!(
            crate::domain::bucket::SKU_COLUMNS
                .iter()
                .any(|tag| tag.column == column),
            "{}'s carrier `{column}` is not a registered SKU column",
            kind.as_str()
        );
    }
}
