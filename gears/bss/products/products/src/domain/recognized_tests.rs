//! `domain::recognized` — each rule probed on the case whose absence would
//! ship the defect its instruction names.

use super::{
    MemberOp, MemberState, SetKind, SkuType, UsageTypeAnswer, declaration_is_new,
    declaration_verdict, judge_usage_type, member_edge, meter_pair_complete, type_profile,
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
    for kind in [SetKind::MeteringUnit, SetKind::PlanTier] {
        assert_eq!(SetKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(SetKind::parse("units"), None, "no alias, no default");
    // The two accounting kinds left the roster with P-D-169, and a stored row
    // or a path segment naming one must now be refused like any other token
    // outside it.
    assert_eq!(SetKind::parse("tax_category"), None);
    assert_eq!(SetKind::parse("gl_code"), None);
    assert_eq!(
        SetKind::MeteringUnit.delist_blocked(String::new()).code(),
        "UNIT_DELIST_BLOCKED"
    );
    assert_eq!(
        SetKind::PlanTier.delist_blocked(String::new()).code(),
        "PLAN_TIER_RETIRE_BLOCKED"
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

/// Every set kind names the `products_sku` column its members are declared
/// in — the removal guard's population, uniform across both
/// (`dod-recognized-set-mechanics`), and each of those columns is registered
/// in the bucket roster.
#[test]
fn every_set_kind_has_a_registered_carrier_column() {
    for kind in [SetKind::MeteringUnit, SetKind::PlanTier] {
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

/// **Every member op is in the roster, round-trips its token, and exactly one
/// of them is the exception** (**P-D-171**).
///
/// The `match` is what makes it total: a fourth variant will not compile
/// until it is named here, and a bare `ALL.len()` assertion would prove
/// nothing because the array's type carries its own length. The
/// exactly-one clause is the guard that matters — `is_display_label_rename`
/// answering `true` for a second op would hand `min(N, 1)` to an act
/// `design/05` §4 registers material.
#[test]
fn every_member_op_is_in_the_roster_and_exactly_one_is_the_exception() {
    for op in [MemberOp::Add, MemberOp::Transition, MemberOp::Relabel] {
        assert!(MemberOp::ALL.contains(&op), "{} is outside ALL", op.token());
        assert_eq!(
            MemberOp::parse(op.token()),
            Some(op),
            "{} does not round-trip",
            op.token()
        );
    }
    let exceptions: Vec<&str> = MemberOp::ALL
        .into_iter()
        .filter(|op| op.is_display_label_rename())
        .map(MemberOp::token)
        .collect();
    assert_eq!(exceptions, vec!["recognized_set.label"]);
}

/// A token outside the roster parses to nothing, including `02`'s own
/// spelling of the same edit — the slices share the exception, not the
/// vocabulary.
#[test]
fn a_token_outside_the_roster_declares_no_op() {
    for outside in [
        "attribute_definition.label",
        "category.rename",
        "recognized_set",
        "recognized_set.remove",
        "",
    ] {
        assert_eq!(MemberOp::parse(outside), None, "`{outside}` parsed");
    }
}

/// The same closed set the SDK pins, proved on this crate's own enum: the two
/// definitions are independent, so one may be ported while the other is not,
/// and `type_profile`'s refusal is what an unported writer actually meets.
#[test]
fn sku_role_vocabulary_is_closed() {
    for token in ["offer", "component", "bundle"] {
        assert_eq!(SkuType::parse(token).map(SkuType::as_str), Some(token));
    }
    for token in ["product", "service", "resource", "", "Offer"] {
        assert_eq!(SkuType::parse(token), None, "`{token}` parsed");
    }
    assert_eq!(SkuType::ALL.len(), 3);
    assert!(matches!(
        type_profile(Some("product")),
        Err(DomainError::SkuTypeUnknown(_))
    ));
}
