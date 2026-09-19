//! Invoice label syntax (D-373). Double braces `{{` and `}}` escape literal
//! braces. Pricing validates and freezes the source; Billing renders it later.
use std::collections::BTreeMap;
use toolkit_macros::domain_model;

use crate::domain::price_row::PriceRow;
use crate::domain::scope_key::ChargeKind;
use crate::domain::validation::{ValidationReport, ValidationRule};

/// A placeholder or brace sequence outside the supported grammar.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownPlaceholder {
    /// Byte offset in the authored source.
    pub offset: usize,
    /// Unknown name or malformed brace sequence.
    pub name: String,
}

/// Validated template source. No localized labels are resolved by Pricing.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template(String);
impl Template {
    /// Original authored source, including escaped braces.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Invalid template syntax at a row or policy write.
pub const LINE_TEMPLATE_INVALID: &str = "LINE_TEMPLATE_INVALID";

/// Validate all placeholders, reporting every malformed sequence encountered.
/// Empty and whitespace-only templates are syntactically valid; publish checks
/// their completeness against the tenant defaults.
///
/// # Errors
/// Returns unknown names and unmatched braces with their source byte offsets.
pub fn parse(source: &str) -> Result<Template, Vec<UnknownPlaceholder>> {
    let mut errors = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        if ch != '{' && ch != '}' {
            continue;
        }
        if chars.peek().is_some_and(|(_, next)| *next == ch) {
            chars.next();
            continue;
        }
        if ch == '}' {
            errors.push(UnknownPlaceholder {
                offset,
                name: "}".to_owned(),
            });
            continue;
        }
        let start = offset + 1;
        let mut end = None;
        for (index, next) in chars.by_ref() {
            if next == '}' {
                end = Some(index);
                break;
            }
        }
        match end {
            Some(end)
                if matches!(
                    &source[start..end],
                    "sku" | "sku_code" | "unit" | "plan" | "phase" | "dimension" | "period"
                ) => {}
            Some(end) => errors.push(UnknownPlaceholder {
                offset,
                name: source[start..end].to_owned(),
            }),
            None => errors.push(UnknownPlaceholder {
                offset,
                name: source[offset..].to_owned(),
            }),
        }
    }
    if errors.is_empty() {
        Ok(Template(source.to_owned()))
    } else {
        Err(errors)
    }
}

/// Complete tenant template policy, keyed by the live charge kinds.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultLineTemplates(BTreeMap<String, String>);
impl Default for DefaultLineTemplates {
    fn default() -> Self {
        Self(
            [
                ("recurring", "{sku} - {period}"),
                ("usage", "{sku}, {unit}"),
                ("one_time", "{sku}"),
            ]
            .into_iter()
            .map(|(kind, source)| (kind.to_owned(), source.to_owned()))
            .collect(),
        )
    }
}
impl DefaultLineTemplates {
    /// Validate the exact policy key set and every source template.
    ///
    /// # Errors
    /// Returns a description of missing/extra keys or malformed templates.
    pub fn from_map(values: BTreeMap<String, String>) -> Result<Self, String> {
        let expected = ["recurring", "usage", "one_time"];
        if values.len() != expected.len() || expected.iter().any(|key| !values.contains_key(*key)) {
            return Err("default_line_templates requires exactly recurring, usage, one_time".to_owned());
        }
        for (kind, source) in &values {
            if let Err(errors) = parse(source) {
                return Err(format!("{LINE_TEMPLATE_INVALID}: {kind}: {errors:?}"));
            }
        }
        Ok(Self(values))
    }
    /// Template for a kind; all three entries are guaranteed by construction.
    #[must_use]
    pub fn get(&self, kind: ChargeKind) -> &str {
        self.0.get(kind.as_str()).map_or("", String::as_str)
    }
    /// Persistable policy representation.
    #[must_use]
    pub fn to_map(&self) -> BTreeMap<String, String> {
        self.0.clone()
    }
}

/// Every syntax operand is in the submitted row, so malformed templates refuse
/// at write; an incomplete draft remains authorable.
pub struct LineTemplateValid;
impl ValidationRule<PriceRow> for LineTemplateValid {
    fn name(&self) -> &'static str {
        "inst-ds-template-syntax"
    }
    fn evaluate(&self, row: &PriceRow, report: &mut ValidationReport) {
        if let Some(source) = &row.invoice_line_template
            && let Err(errors) = parse(source)
        {
            report.violate_at_write(
                LINE_TEMPLATE_INVALID,
                row.subject(),
                format!("invalid invoiceLineTemplate: {errors:?}"),
            );
        }
    }
}

#[cfg(test)]
#[path = "line_template_tests.rs"]
mod tests;
