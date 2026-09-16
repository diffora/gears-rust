//! D-152 extension completeness. D-373 moved the pinned descriptor elements
//! to rows and derived itemization; this rule owns only plan extension keys.
use crate::domain::plan_rules::DESCRIPTOR_INCOMPLETE;
use crate::domain::plan_shape::PlanShape;
use crate::domain::validation::{ValidationReport, ValidationRule};
use std::collections::BTreeSet;
use toolkit_macros::domain_model;

/// Additive tenant-required plan extensions (`inst-ds-required`).
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DescriptorSetComplete {
    required: Vec<String>,
}
impl DescriptorSetComplete {
    /// Add tenant-required extensions to the pinned billing contract. Pinned
    /// row elements are checked by their own rules, never satisfied from ext.
    #[must_use]
    pub fn extending_v1(additional: impl IntoIterator<Item = String>) -> Self {
        let mut seen = BTreeSet::new();
        Self {
            required: additional
                .into_iter()
                .filter(|key| seen.insert(key.clone()))
                .collect(),
        }
    }
    /// Extension names in deterministic report order.
    #[must_use]
    pub fn required(&self) -> &[String] {
        &self.required
    }
}
impl ValidationRule<PlanShape> for DescriptorSetComplete {
    fn name(&self) -> &'static str {
        "inst-ds-required"
    }
    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        for field in &self.required {
            if subject
                .descriptor_ext
                .get(field)
                .is_none_or(|value| value.trim().is_empty())
            {
                report.violate(
                    DESCRIPTOR_INCOMPLETE,
                    subject.subject(),
                    format!("{field} is required in billing.ext before publish"),
                );
            }
        }
    }
}
#[cfg(test)]
#[path = "descriptor_set_tests.rs"]
mod descriptor_set_tests;
