//! Gear-local `snake_case` wire types; SDK enums are represented by their stable tokens.
use crate::domain::sku::{NewSku, SkuPatch};
use crate::domain::validation::ValidationReport;
use bss_products_sdk::models::{
    BillingTiming, Category, Lifecycle, Sku, SkuContent, SkuType, SkuVersion,
};
use serde::Deserialize;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

/// Wire representation of the registry Category.
#[toolkit_macros::api_dto(response)]
pub struct CategoryDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub code: String,
    pub name: String,
    pub is_default: bool,
    pub sort_order: i32,
    pub status: String,
    pub version: i64,
}
impl From<Category> for CategoryDto {
    fn from(value: Category) -> Self {
        Self {
            id: value.id,
            tenant_id: value.tenant_id,
            code: value.code,
            name: value.name,
            is_default: value.is_default,
            sort_order: value.sort_order,
            status: value.status,
            version: value.version,
        }
    }
}
/// Wire representation of the registry Sku.
#[toolkit_macros::api_dto(response)]
pub struct SkuDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub category_id: Uuid,
    pub description: String,
    pub sellable: bool,
    pub lifecycle: String,
    pub revision: i64,
    pub published_version: i64,
    pub gl_code: Option<String>,
    pub tax_category: Option<String>,
    pub invoice_line_template: Option<String>,
    pub billing_timing: Option<String>,
    pub usage_type_ref: Option<String>,
    pub unit: Option<String>,
    pub type_change_pending: bool,
    pub pending_unit_id: Option<Uuid>,
    pub approved_by_unit_id: Option<Uuid>,
    pub created_by: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}
impl From<Sku> for SkuDto {
    fn from(value: Sku) -> Self {
        Self {
            id: value.id,
            tenant_id: value.tenant_id,
            code: value.code,
            name: value.name,
            r#type: value.r#type.as_str().to_owned(),
            category_id: value.category_id,
            description: value.description,
            sellable: value.sellable,
            lifecycle: value.lifecycle.as_str().to_owned(),
            revision: value.revision,
            published_version: value.published_version,
            gl_code: value.gl_code,
            tax_category: value.tax_category,
            invoice_line_template: value.invoice_line_template,
            billing_timing: value
                .billing_timing
                .map(|v| billing_timing_token(v).to_owned()),
            usage_type_ref: value.usage_type_ref,
            unit: value.unit,
            type_change_pending: value.type_change_pending,
            pending_unit_id: value.pending_unit_id,
            approved_by_unit_id: value.approved_by_unit_id,
            created_by: value.created_by,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}
/// Wire representation of the registry `SkuContent`.
#[toolkit_macros::api_dto(response)]
pub struct SkuContentDto {
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub category_id: Uuid,
    pub description: String,
    pub sellable: bool,
    pub gl_code: Option<String>,
    pub tax_category: Option<String>,
    pub invoice_line_template: Option<String>,
    pub billing_timing: Option<String>,
    pub usage_type_ref: Option<String>,
    pub unit: Option<String>,
}
impl From<SkuContent> for SkuContentDto {
    fn from(value: SkuContent) -> Self {
        Self {
            code: value.code,
            name: value.name,
            r#type: value.r#type.as_str().to_owned(),
            category_id: value.category_id,
            description: value.description,
            sellable: value.sellable,
            gl_code: value.gl_code,
            tax_category: value.tax_category,
            invoice_line_template: value.invoice_line_template,
            billing_timing: value
                .billing_timing
                .map(|v| billing_timing_token(v).to_owned()),
            usage_type_ref: value.usage_type_ref,
            unit: value.unit,
        }
    }
}
/// Wire representation of the registry `SkuVersion`.
#[toolkit_macros::api_dto(response)]
pub struct SkuVersionDto {
    pub sku_id: Uuid,
    pub published_version: i64,
    #[serde(with = "crate::infra::serde_date")]
    pub effective_from: Date,
    pub content: SkuContentDto,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}
impl From<SkuVersion> for SkuVersionDto {
    fn from(value: SkuVersion) -> Self {
        Self {
            sku_id: value.sku_id,
            published_version: value.published_version,
            effective_from: value.effective_from,
            content: value.content.into(),
            created_at: value.created_at,
        }
    }
}

/// Stable billing token (the SDK enum has no `as_str` method).
pub(crate) const fn billing_timing_token(value: BillingTiming) -> &'static str {
    match value {
        BillingTiming::Advance => "advance",
        BillingTiming::Arrears => "arrears",
    }
}
fn parse_billing(value: &str) -> Option<BillingTiming> {
    match value {
        "advance" => Some(BillingTiming::Advance),
        "arrears" => Some(BillingTiming::Arrears),
        _ => None,
    }
}
/// Parse one enum token, retaining the wire field in validation failures.
pub(crate) fn parse_token<T>(
    value: &str,
    field: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<T, ValidationReport> {
    parse(value).ok_or_else(|| {
        let mut r = ValidationReport::new();
        r.violate("VALIDATION", field, format!("unknown {field}: {value}"));
        r
    })
}
/// Preserve explicit null as a present patch value.
#[allow(clippy::option_option)] // PATCH has three states.
fn double_option<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}
#[toolkit_macros::api_dto(request)]
pub struct CategoryRequest {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub sort_order: i32,
}
#[toolkit_macros::api_dto(request)]
pub struct CategoryPatchRequest {
    pub name: Option<String>,
    pub is_default: Option<bool>,
    pub sort_order: Option<i32>,
}
#[toolkit_macros::api_dto(response)]
pub struct CategoryList {
    pub items: Vec<CategoryDto>,
}
#[toolkit_macros::api_dto(response)]
pub struct SkuCard {
    pub sku: SkuDto,
    pub references: ReferencesDto,
}
#[toolkit_macros::api_dto(response)]
pub struct ReferencesDto {
    pub prices: u32,
    pub plans: u32,
    pub reserved: u32,
    pub by_owner: std::collections::BTreeMap<String, std::collections::BTreeMap<String, u32>>,
}
impl From<crate::domain::references::ReferenceSummary> for ReferencesDto {
    fn from(v: crate::domain::references::ReferenceSummary) -> Self {
        Self {
            prices: v.prices,
            plans: v.plans,
            reserved: v.reserved,
            by_owner: v.by_owner,
        }
    }
}
#[toolkit_macros::api_dto(response)]
pub struct SkuList {
    pub items: Vec<SkuDto>,
    pub next: Option<String>,
}
#[toolkit_macros::api_dto(request)]
pub struct SkuRequest {
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub category_id: Uuid,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_sellable")]
    pub sellable: bool,
    pub gl_code: Option<String>,
    pub tax_category: Option<String>,
    pub invoice_line_template: Option<String>,
    pub billing_timing: Option<String>,
    pub usage_type_ref: Option<String>,
    pub unit: Option<String>,
}
const fn default_sellable() -> bool {
    true
}
impl TryFrom<SkuRequest> for NewSku {
    type Error = ValidationReport;
    fn try_from(v: SkuRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            code: v.code.trim().to_owned(),
            name: v.name.trim().to_owned(),
            r#type: parse_token(&v.r#type, "type", SkuType::parse)?,
            category_id: v.category_id,
            description: v.description,
            sellable: v.sellable,
            gl_code: v.gl_code,
            tax_category: v.tax_category,
            invoice_line_template: v.invoice_line_template,
            billing_timing: v
                .billing_timing
                .as_deref()
                .map(|s| parse_token(s, "billing_timing", parse_billing))
                .transpose()?,
            usage_type_ref: v.usage_type_ref,
            unit: v.unit,
        })
    }
}
#[toolkit_macros::api_dto(request)]
#[allow(clippy::option_option)] // None = omitted; Some(None) = clear; Some(Some(_)) = set.
pub struct SkuPatchRequest {
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
    pub billing_timing: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub usage_type_ref: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub unit: Option<Option<String>>,
    pub lifecycle: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
}
impl TryFrom<SkuPatchRequest> for SkuPatch {
    type Error = ValidationReport;
    fn try_from(v: SkuPatchRequest) -> Result<Self, Self::Error> {
        let mut report = ValidationReport::new();
        let mut parse = |value: &str, field: &str| {
            report.violate("VALIDATION", field, format!("unknown {field}: {value}"));
        };
        let kind = v.r#type.as_deref().and_then(|s| {
            let parsed = SkuType::parse(s);
            if parsed.is_none() {
                parse(s, "type");
            }
            parsed
        });
        let lifecycle = v.lifecycle.as_deref().and_then(|s| {
            let parsed = Lifecycle::parse(s);
            if parsed.is_none() {
                parse(s, "lifecycle");
            }
            parsed
        });
        let billing_timing = v.billing_timing.map(|o| {
            o.and_then(|s| {
                let parsed = parse_billing(&s);
                if parsed.is_none() {
                    parse(&s, "billing_timing");
                }
                parsed
            })
        });
        if !report.is_empty() {
            return Err(report);
        }
        Ok(Self {
            name: v.name.map(|s| s.trim().to_owned()),
            category_id: v.category_id,
            description: v.description,
            sellable: v.sellable,
            gl_code: v.gl_code,
            tax_category: v.tax_category,
            invoice_line_template: v.invoice_line_template,
            billing_timing,
            usage_type_ref: v.usage_type_ref,
            unit: v.unit,
            lifecycle,
            r#type: kind,
        })
    }
}
#[toolkit_macros::api_dto(request)]
pub struct SkuChangeRequest {
    #[serde(flatten)]
    pub patch: SkuPatchRequest,
    #[serde(default, with = "crate::infra::serde_date::option")]
    pub effective_from: Option<Date>,
    pub note: Option<String>,
}
#[toolkit_macros::api_dto(response)]
pub struct UnitDto {
    pub id: Uuid,
    pub kind: String,
    pub ref_type: String,
    pub ref_id: Uuid,
    pub state: String,
    pub generation: i32,
    pub quorum_required: u32,
    #[serde(with = "crate::infra::serde_date::option")]
    pub common_effective_date: Option<Date>,
    pub submitted_by: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub submitted_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub decided_at: Option<OffsetDateTime>,
    pub decided_note: Option<String>,
    pub snapshot: serde_json::Value,
    pub decisions: Vec<DecisionDto>,
    pub impact_live: Option<serde_json::Value>,
}
#[toolkit_macros::api_dto(response)]
pub struct DecisionDto {
    pub actor: Uuid,
    pub generation: i32,
    pub decision: String,
    pub note: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub stale: bool,
}
#[toolkit_macros::api_dto(response)]
pub struct UnitList {
    pub items: Vec<UnitDto>,
}
#[toolkit_macros::api_dto(request)]
pub struct VoteRequest {
    pub generation: i32,
    pub note: Option<String>,
}
#[toolkit_macros::api_dto(response)]
pub struct SubmitReceipt {
    pub applied: bool,
    pub unit: UnitDto,
    pub sku: SkuDto,
}
#[toolkit_macros::api_dto(response)]
pub struct VoteReceipt {
    pub have: Option<u32>,
    pub need: Option<u32>,
    pub outcome: String,
    pub unit: UnitDto,
}
#[toolkit_macros::api_dto(request)]
pub struct PolicyRequest {
    pub kind: Option<String>,
    pub quorum: u32,
}
#[toolkit_macros::api_dto(response)]
pub struct PolicyDto {
    pub default_quorum: u32,
    pub overrides: std::collections::BTreeMap<String, u32>,
}
#[cfg(test)]
#[path = "dto_tests.rs"]
mod dto_tests;

#[toolkit_macros::api_dto(request)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::empty_structs_with_brackets,
    reason = "Serde must accept an empty JSON object, not null"
)]
pub struct EmptyRequest {}
impl From<bss_approval::Unit> for UnitDto {
    fn from(u: bss_approval::Unit) -> Self {
        Self {
            id: u.id,
            kind: u.kind,
            ref_type: u.ref_type,
            ref_id: u.ref_id,
            state: u.state.as_str().into(),
            generation: u.generation,
            quorum_required: u.quorum_required,
            common_effective_date: u.common_effective_date,
            submitted_by: u.submitted_by,
            submitted_at: u.submitted_at,
            decided_at: u.decided_at,
            decided_note: u.decided_note,
            snapshot: u.snapshot,
            decisions: Vec::new(),
            impact_live: None,
        }
    }
}
impl From<bss_approval::Decision> for DecisionDto {
    fn from(d: bss_approval::Decision) -> Self {
        Self {
            actor: d.actor,
            generation: d.generation,
            decision: d.verdict.as_str().into(),
            note: d.note,
            at: d.at,
            stale: d.stale,
        }
    }
}
impl From<bss_approval::Policy> for PolicyDto {
    fn from(p: bss_approval::Policy) -> Self {
        Self {
            default_quorum: p.default_quorum,
            overrides: p.overrides,
        }
    }
}
#[toolkit_macros::api_dto(request)]
pub struct ReserveRequest {
    pub owner: String,
    pub kind: String,
    pub ref_id: Uuid,
}
#[toolkit_macros::api_dto(request)]
pub struct ReleaseRequest {
    #[serde(default)]
    pub force: bool,
    pub reason: Option<String>,
}
#[toolkit_macros::api_dto(response)]
pub struct ReferenceReceipt {
    pub reservation_id: Uuid,
    pub sku_id: Uuid,
    pub owner: String,
    pub kind: String,
    pub ref_id: Uuid,
    pub state: String,
    pub forced: bool,
}
impl From<crate::infra::storage::repo::SkuReference> for ReferenceReceipt {
    fn from(r: crate::infra::storage::repo::SkuReference) -> Self {
        Self {
            reservation_id: r.id,
            sku_id: r.sku_id,
            owner: r.owner_gear,
            kind: r.ref_kind,
            ref_id: r.ref_id,
            state: r.state,
            forced: r.forced,
        }
    }
}
