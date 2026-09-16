//! Row-borne billing contract completeness (D-373 R2/R3).
use crate::domain::line_template::DefaultLineTemplates;
use crate::domain::plan_rules::DESCRIPTOR_INCOMPLETE;
use crate::domain::plan_shape::PlanShape;
use crate::domain::publish::rules::GL_CODE_UNRESOLVED;
use crate::domain::validation::{ValidationReport, ValidationRule};

/// Resolve each candidate row's line template against the tenant policy.
pub struct LineTemplateResolved {
    /// Complete tenant defaults by charge kind.
    pub tenant_defaults: DefaultLineTemplates,
}
impl ValidationRule<PlanShape> for LineTemplateResolved {
    fn name(&self) -> &'static str {
        "inst-ds-template"
    }
    fn evaluate(&self, shape: &PlanShape, report: &mut ValidationReport) {
        for record in &shape.rows {
            let value = record.effective_invoice_line_template(&self.tenant_defaults);
            if value.is_none_or(|source| source.trim().is_empty()) {
                report.violate(DESCRIPTOR_INCOMPLETE, record.price_id.to_string(), "invoiceLineTemplate is required: author a row override or a tenant default for this charge kind");
            }
        }
    }
}
/// Resolve each candidate row's GL code against the tenant policy.
pub struct GlCodeResolved {
    /// Optional tenant default; a blank authored override remains blank.
    pub tenant_default: Option<String>,
}
impl ValidationRule<PlanShape> for GlCodeResolved {
    fn name(&self) -> &'static str {
        "inst-ds-glresolve"
    }
    fn evaluate(&self, shape: &PlanShape, report: &mut ValidationReport) {
        for record in &shape.rows {
            let value = record.effective_gl_code(self.tenant_default.as_deref());
            if value.is_none_or(|code| code.trim().is_empty()) {
                report.violate(
                    GL_CODE_UNRESOLVED,
                    record.price_id.to_string(),
                    "glCode is required: author a row glCodeRef or a tenant default",
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "row_descriptor_rules_tests.rs"]
mod tests;
