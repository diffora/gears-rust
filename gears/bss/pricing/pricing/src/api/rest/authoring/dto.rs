//! Pricing authoring wire contracts, with unique `OpenAPI` names and `snake_case` fields.
use crate::infra::storage::entity;
use uuid::Uuid;
#[toolkit_macros::api_dto(response)]
pub struct PriceBookDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub code: String,
    pub name: String,
    pub currency: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub version: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}
impl From<entity::price_book::Model> for PriceBookDto {
    fn from(m: entity::price_book::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            code: m.code,
            name: m.name,
            currency: m.currency,
            valid_from: m.valid_from.map(|v| v.to_string()),
            valid_until: m.valid_until.map(|v| v.to_string()),
            version: m.version,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}
#[toolkit_macros::api_dto(response)]
pub struct PricingPriceDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub book_id: Uuid,
    pub sku_id: Uuid,
    pub charge_kind: String,
    pub period: Option<String>,
    pub dimension_key: Option<String>,
    pub invoice_line_override: Option<String>,
    pub reservation_id: Uuid,
    pub reference_state: String,
    pub version: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}
impl From<entity::price::Model> for PricingPriceDto {
    fn from(m: entity::price::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            book_id: m.book_id,
            sku_id: m.sku_id,
            charge_kind: m.charge_kind,
            period: m.period,
            dimension_key: m.dimension_key,
            invoice_line_override: m.invoice_line_override,
            reservation_id: m.reservation_id,
            reference_state: m.reference_state,
            version: m.version,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}
#[toolkit_macros::api_dto(response)]
pub struct PricingPriceRowDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub price_id: Uuid,
    pub version_no: i32,
    pub dim_value: Option<String>,
    pub model: String,
    pub price_json: serde_json::Value,
    pub min_fee: Option<String>,
    pub eligibility: String,
    pub effective_from: String,
    pub effective_to: Option<String>,
    pub keep_for_bound: bool,
    pub closed_explicitly: bool,
    pub temporary_until: Option<String>,
    pub paired_row_id: Option<Uuid>,
    pub return_of_row_id: Option<Uuid>,
    pub state: String,
    pub pending_unit_id: Option<Uuid>,
    pub approved_by_unit_id: Option<Uuid>,
    pub note: Option<String>,
    pub created_by: Uuid,
    #[serde(with = "time::serde::rfc3339::option")]
    pub approved_at: Option<time::OffsetDateTime>,
    pub version: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}
impl From<entity::price_row::Model> for PricingPriceRowDto {
    fn from(m: entity::price_row::Model) -> Self {
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            price_id: m.price_id,
            version_no: m.version_no,
            dim_value: m.dim_value,
            model: m.model,
            price_json: m.price_json,
            min_fee: m.min_fee.map(|v| v.to_string()),
            eligibility: m.eligibility,
            effective_from: m.effective_from.to_string(),
            effective_to: m.effective_to.map(|v| v.to_string()),
            keep_for_bound: m.keep_for_bound,
            closed_explicitly: m.closed_explicitly,
            temporary_until: m.temporary_until.map(|v| v.to_string()),
            paired_row_id: m.paired_row_id,
            return_of_row_id: m.return_of_row_id,
            state: m.state,
            pending_unit_id: m.pending_unit_id,
            approved_by_unit_id: m.approved_by_unit_id,
            note: m.note,
            created_by: m.created_by,
            approved_at: m.approved_at,
            version: m.version,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PriceBookCreate {
    pub code: String,
    pub name: String,
    pub currency: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::option_option,
    reason = "PATCH distinguishes omission, null clearing, and a new date"
)]
pub struct PriceBookPatch {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "nullable_date")]
    pub valid_from: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_date")]
    pub valid_until: Option<Option<String>>,
}
#[allow(
    clippy::option_option,
    reason = "PATCH distinguishes omission, null clearing, and a new date"
)]
fn nullable_date<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<String>>, D::Error> {
    <Option<String> as serde::Deserialize>::deserialize(d).map(Some)
}
#[toolkit_macros::api_dto(response)]
pub struct PriceBookList {
    pub items: Vec<PriceBookDto>,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingPriceList {
    pub items: Vec<PricingPriceDto>,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingExportPrice {
    pub price: PricingPriceDto,
    pub rows: Vec<PricingPriceRowDto>,
}
#[toolkit_macros::api_dto(response)]
pub struct PriceBookExport {
    pub book: PriceBookDto,
    pub prices: Vec<PricingExportPrice>,
}
#[toolkit_macros::api_dto(request, response)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingDimensionEntry {
    pub key: String,
    pub values: Vec<String>,
}
#[toolkit_macros::api_dto(request, response)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingDimensions {
    pub items: Vec<PricingDimensionEntry>,
}
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingSettingsPut {
    pub default_timing: String,
    pub default_rounding: String,
    pub default_gl: Option<String>,
    pub default_tax_category: Option<String>,
    pub invoice_line_templates: std::collections::BTreeMap<String, String>,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingSettingsDto {
    pub default_timing: String,
    pub default_rounding: String,
    pub default_gl: Option<String>,
    pub default_tax_category: Option<String>,
    pub invoice_line_templates: serde_json::Value,
    pub version: i64,
}
