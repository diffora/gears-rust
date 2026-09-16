#![allow(clippy::expect_used)]
use super::{GlCodeResolved, LineTemplateResolved};
use crate::domain::concurrency::RowVersion;
use crate::domain::instant::utc_ymd_hms;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::line_template::DefaultLineTemplates;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::plan_shape::PlanShape;
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::PriceRow;
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey, SkuId,
};
use crate::domain::taxonomy::GlCodeDeclared;
use crate::domain::validation::{ValidationReport, ValidationRule};
use uuid::Uuid;
fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(1))
}
fn now() -> time::OffsetDateTime {
    utc_ymd_hms(2026, 9, 16, 0, 0, 0)
}
fn region(value: &str) -> Region {
    Region::new(value).expect("region")
}
fn row_in(price_id: u128, in_region: &str) -> PriceRecord {
    let scope_key = ScopeKey::new(
        plan(),
        CurrencyCode::new("EUR").expect("three letters"),
        region(in_region),
        PhaseId::new(Uuid::from_u128(0xf1)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
        SkuId::new(Uuid::from_u128(5)),
    )
    .expect("all_subscriptions pairs with cohort none");

    let mut row = PriceRow::new(ChargeKind::Recurring, None);
    row.amount_minor = Some(MinorAmount::new(1000).expect("non-negative"));

    PriceRecord {
        resolved_invoice_line_template: None,
        resolved_gl_code: None,
        price_id: Uuid::from_u128(price_id),
        scope_key,
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: None,
        proration_contract: None,
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
        lifecycle_state: LifecycleState::Draft,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: now(),
        row_version: RowVersion::new(0),
    }
}

#[test]
fn row_absences_name_each_price_and_blank_override_does_not_inherit() {
    let mut shape = PlanShape::new(plan(), 1, now());
    shape.rows = vec![row_in(10, "eu"), row_in(11, "eu")];
    let mut defaults = DefaultLineTemplates::default().to_map();
    defaults.insert("recurring".to_owned(), String::new());
    let mut report = ValidationReport::default();
    LineTemplateResolved {
        tenant_defaults: DefaultLineTemplates::from_map(defaults).expect("empty is valid"),
    }
    .evaluate(&shape, &mut report);
    GlCodeResolved {
        tenant_default: None,
    }
    .evaluate(&shape, &mut report);
    assert_eq!(report.violations.len(), 4);
    for row in &shape.rows {
        assert_eq!(
            report
                .violations
                .iter()
                .filter(|finding| finding.subject == row.price_id.to_string())
                .count(),
            2
        );
    }
    shape.rows[0].row.invoice_line_template = Some(" ".to_owned());
    shape.rows[0].row.gl_code_ref = Some(" ".to_owned());
    let mut report = ValidationReport::default();
    LineTemplateResolved {
        tenant_defaults: DefaultLineTemplates::default(),
    }
    .evaluate(&shape, &mut report);
    GlCodeResolved {
        tenant_default: Some("4000".to_owned()),
    }
    .evaluate(&shape, &mut report);
    assert_eq!(report.violations.len(), 2);
    assert!(
        report
            .violations
            .iter()
            .all(|finding| finding.subject == shape.rows[0].price_id.to_string())
    );
}
#[test]
fn effective_gl_is_validated_once_per_row_and_empty_vocabulary_constrains_nothing() {
    let mut shape = PlanShape::new(plan(), 1, now());
    shape.rows = vec![row_in(10, "eu"), row_in(11, "eu")];
    shape.rows[0].row.gl_code_ref = Some("bad".to_owned());
    let mut rule = GlCodeDeclared {
        tenant_default: Some("bad".to_owned()),
        declared: ["4000".to_owned()].into_iter().collect(),
    };
    let mut report = ValidationReport::default();
    rule.evaluate(&shape, &mut report);
    assert_eq!(report.violations.len(), 2);
    assert!(
        report
            .violations
            .iter()
            .all(|finding| finding.code == "GL_CODE_UNKNOWN")
    );
    rule.declared.clear();
    let mut report = ValidationReport::default();
    rule.evaluate(&shape, &mut report);
    assert!(report.is_publishable());
}

#[test]
fn immutable_rows_use_frozen_descriptors_after_tenant_defaults_change() {
    let mut shape = PlanShape::new(plan(), 2, now());
    let mut row = row_in(10, "eu");
    row.lifecycle_state = LifecycleState::Published;
    row.resolved_invoice_line_template = Some("Original {sku}".to_owned());
    row.resolved_gl_code = Some("4000".to_owned());
    shape.rows.push(row);
    let mut empty = DefaultLineTemplates::default().to_map();
    empty.insert("recurring".to_owned(), String::new());
    let mut report = ValidationReport::default();
    LineTemplateResolved {
        tenant_defaults: DefaultLineTemplates::from_map(empty).expect("blank allowed"),
    }
    .evaluate(&shape, &mut report);
    GlCodeResolved {
        tenant_default: None,
    }
    .evaluate(&shape, &mut report);
    GlCodeDeclared {
        tenant_default: Some("changed".to_owned()),
        declared: ["4000".to_owned()].into_iter().collect(),
    }
    .evaluate(&shape, &mut report);
    assert!(report.is_publishable(), "{report:?}");
    shape.rows[0].resolved_gl_code = Some("unknown-frozen".to_owned());
    GlCodeDeclared {
        tenant_default: Some("4000".to_owned()),
        declared: ["4000".to_owned()].into_iter().collect(),
    }
    .evaluate(&shape, &mut report);
    assert_eq!(report.violations[0].code, "GL_CODE_UNKNOWN");
    shape.rows[0].resolved_invoice_line_template = None;
    LineTemplateResolved {
        tenant_defaults: DefaultLineTemplates::default(),
    }
    .evaluate(&shape, &mut report);
    assert_eq!(report.violations[1].code, "DESCRIPTOR_INCOMPLETE");
}
