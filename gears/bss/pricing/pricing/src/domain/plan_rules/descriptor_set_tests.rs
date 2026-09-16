#![allow(clippy::expect_used)]
use super::DescriptorSetComplete;
use crate::domain::instant::utc_ymd_hms;
use crate::domain::plan_shape::PlanShape;
use crate::domain::scope_key::PlanId;
use crate::domain::validation::{ValidationReport, ValidationRule};
use uuid::Uuid;

#[test]
fn extensions_are_additive_and_report_each_missing_plan_key() {
    let mut plan = PlanShape::new(
        PlanId::new(Uuid::from_u128(1)),
        1,
        utc_ymd_hms(2026, 9, 16, 0, 0, 0),
    );
    let rule = DescriptorSetComplete::extending_v1(
        ["costCentre", "segment", "costCentre"].map(str::to_owned),
    );
    assert_eq!(rule.required(), ["costCentre", "segment"]);
    let mut report = ValidationReport::default();
    rule.evaluate(&plan, &mut report);
    assert_eq!(report.violations.len(), 2);
    assert!(
        report
            .violations
            .iter()
            .all(|finding| finding.subject == plan.subject())
    );
    plan.descriptor_ext
        .insert("costCentre".to_owned(), "  ".to_owned());
    plan.descriptor_ext
        .insert("segment".to_owned(), "smb".to_owned());
    let mut report = ValidationReport::default();
    rule.evaluate(&plan, &mut report);
    assert_eq!(report.violations.len(), 1);
    assert!(report.violations[0].detail.contains("costCentre"));
    plan.descriptor_ext
        .insert("costCentre".to_owned(), "100".to_owned());
    let mut report = ValidationReport::default();
    rule.evaluate(&plan, &mut report);
    assert!(report.is_publishable());
}

#[test]
fn no_tenant_extension_requires_no_plan_descriptor_entity() {
    assert!(DescriptorSetComplete::default().required().is_empty());
}
