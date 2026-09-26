//! Usage-type and meter checks.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::{UsageTypeAnswer, judge_usage_type, meter_pair_complete};
use crate::domain::error::DomainError;

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

#[test]
fn a_binding_snapshot_renders_sorted_and_flat() {
    let binding = crate::test_support::probe_binding();
    assert_eq!(
        crate::domain::recognized::binding_snapshot_json(&binding),
        r#"{"gts_id":"usage:storage","kind":"counter","metadata_fields":["region","zone"]}"#
    );
}
