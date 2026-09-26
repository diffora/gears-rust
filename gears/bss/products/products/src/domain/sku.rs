//! @cpt-dod:cpt-cf-bss-products-dod-lifecycle-edges:p1
//! Pure SKU content validation, patching and lifecycle rules.
//! @cpt-dod:cpt-cf-bss-products-dod-bundle-unpriced:p1
use crate::domain::error::DomainError;
use crate::domain::recognized::UsageTypeAnswer;
use crate::domain::validation::ValidationReport;
use bss_products_sdk::models::{BillingTiming, Lifecycle, SkuContent, SkuType};
use serde::Deserialize;
use uuid::Uuid;

/// Input for a new draft SKU.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone)]
pub struct NewSku {
    pub code: String,
    pub name: String,
    pub r#type: SkuType,
    pub category_id: Uuid,
    pub description: String,
    pub sellable: bool,
    pub gl_code: Option<String>,
    pub tax_category: Option<String>,
    pub invoice_line_template: Option<String>,
    pub billing_timing: Option<BillingTiming>,
    pub usage_type_ref: Option<String>,
    pub unit: Option<String>,
}

/// Preserve explicit null as a present patch value.
#[allow(clippy::option_option)] // The specified PATCH contract distinguishes three states.
fn double_option<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

/// Omitted nullable fields are unchanged; explicit null clears them.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::option_option)] // None = omitted; Some(None) = clear; Some(Some(_)) = set.
pub struct SkuPatch {
    pub name: Option<String>,
    pub category_id: Option<Uuid>,
    pub description: Option<String>,
    pub sellable: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub gl_code: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub tax_category: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub invoice_line_template: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub billing_timing: Option<Option<BillingTiming>>,
    #[serde(default, deserialize_with = "double_option")]
    pub usage_type_ref: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub unit: Option<Option<String>>,
    pub lifecycle: Option<Lifecycle>,
    pub r#type: Option<SkuType>,
}

/// Apply business fields without changing lifecycle or concurrency metadata.
#[must_use]
pub fn apply_patch(c: &SkuContent, p: &SkuPatch) -> SkuContent {
    let mut out = c.clone();
    if let Some(v) = &p.name {
        out.name.clone_from(v);
    }
    if let Some(v) = p.category_id {
        out.category_id = v;
    }
    if let Some(v) = &p.description {
        out.description.clone_from(v);
    }
    if let Some(v) = p.sellable {
        out.sellable = v;
    }
    if let Some(v) = &p.gl_code {
        out.gl_code.clone_from(v);
    }
    if let Some(v) = &p.tax_category {
        out.tax_category.clone_from(v);
    }
    if let Some(v) = &p.invoice_line_template {
        out.invoice_line_template.clone_from(v);
    }
    if let Some(v) = p.billing_timing {
        out.billing_timing = v;
    }
    if let Some(v) = &p.usage_type_ref {
        out.usage_type_ref.clone_from(v);
    }
    if let Some(v) = &p.unit {
        out.unit.clone_from(v);
    }
    if let Some(v) = p.r#type {
        out.r#type = v;
    }
    out
}

/// @cpt-cf-bss-products-fr-sku-define
#[must_use]
pub fn validate_new(new: &NewSku) -> ValidationReport {
    let mut r = ValidationReport::new();
    if new.code.trim().is_empty() {
        r.violate("VALIDATION", "code", "code must not be blank");
    }
    if new.code.chars().count() > 64 {
        r.violate("VALIDATION", "code", "code is at most 64 characters");
    }
    if new.name.trim().is_empty() {
        r.violate("VALIDATION", "name", "name must not be blank");
    }
    r
}

/// @cpt-cf-bss-products-fr-sku-metering · @cpt-cf-bss-products-fr-sku-bundle
#[must_use]
pub fn validate_publish(c: &SkuContent, usage_type: Option<&UsageTypeAnswer>) -> ValidationReport {
    let mut r = ValidationReport::new();
    match c.r#type {
        SkuType::Usage => {
            if c.usage_type_ref.as_deref().unwrap_or("").trim().is_empty() {
                r.violate(
                    "USAGE_NEEDS_METER",
                    "usage_type_ref",
                    "a usage SKU names its usage type before publish",
                );
            }
            if c.unit.as_deref().unwrap_or("").trim().is_empty() {
                r.violate(
                    "USAGE_NEEDS_METER",
                    "unit",
                    "a usage SKU names its unit before publish",
                );
            }
            match usage_type {
                Some(UsageTypeAnswer::Unavailable) => r.violate(
                    "USAGE_TYPE_UNAVAILABLE",
                    "usage_type_ref",
                    "the usage type catalog did not answer",
                ),
                Some(UsageTypeAnswer::Unresolved) => r.violate(
                    "USAGE_TYPE_UNRESOLVED",
                    "usage_type_ref",
                    "the usage type catalog does not know this ref",
                ),
                None if c
                    .usage_type_ref
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()) =>
                {
                    r.violate(
                        "USAGE_TYPE_UNRESOLVED",
                        "usage_type_ref",
                        "the usage type was not resolved",
                    );
                }
                Some(UsageTypeAnswer::Resolved(_)) | None => {}
            }
        }
        SkuType::Bundle => {
            if c.usage_type_ref.is_some() {
                r.violate(
                    "BUNDLE_HAS_NO_METER",
                    "usage_type_ref",
                    "a bundle SKU is never metered",
                );
            }
            if c.unit.is_some() {
                r.violate(
                    "BUNDLE_HAS_NO_METER",
                    "unit",
                    "a bundle SKU is never metered",
                );
            }
        }
        SkuType::Recurring | SkuType::OneTime => {}
    }
    r
}

/// @cpt-cf-bss-products-fr-sku-type-frozen
/// Refuse a type change while any reference is live.
///
/// # Errors
/// Returns `SKU_TYPE_FROZEN` for a nonzero live count.
pub fn validate_type_change(references: u32) -> Result<(), DomainError> {
    if references > 0 {
        return Err(DomainError::Conflict {
            code: "SKU_TYPE_FROZEN",
            detail: format!(
                "{references} live reference(s) point to this SKU; its type cannot change"
            ),
        });
    }
    Ok(())
}

/// @cpt-cf-bss-products-fr-sku-lifecycle
#[must_use]
pub const fn lifecycle_edge(from: Lifecycle, to: Lifecycle) -> bool {
    use Lifecycle::{Deprecated, Draft, Published, Retired, Retiring};
    matches!(
        (from, to),
        (Draft | Deprecated | Retiring, Published)
            | (Published | Retiring, Deprecated)
            | (Published | Deprecated, Retiring)
            | (Retiring, Retired)
    )
}

/// Return sorted wire field names whose business values changed.
#[must_use]
pub fn changed_fields(a: &SkuContent, b: &SkuContent) -> Vec<String> {
    let mut v = Vec::new();
    macro_rules! diff {
        ($f:ident) => {
            if a.$f != b.$f {
                v.push(stringify!($f).to_owned());
            }
        };
    }
    diff!(code);
    diff!(name);
    if a.r#type != b.r#type {
        v.push("type".to_owned());
    }
    diff!(category_id);
    diff!(description);
    diff!(sellable);
    diff!(gl_code);
    diff!(tax_category);
    diff!(invoice_line_template);
    diff!(billing_timing);
    diff!(usage_type_ref);
    diff!(unit);
    v.sort();
    v
}

#[cfg(test)]
#[path = "sku_tests.rs"]
mod sku_tests;
