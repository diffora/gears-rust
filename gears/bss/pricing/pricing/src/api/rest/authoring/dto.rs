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
pub struct PricingPriceBookEntryDto {
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
impl From<entity::price_book_entry::Model> for PricingPriceBookEntryDto {
    fn from(m: entity::price_book_entry::Model) -> Self {
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
pub struct PricingPriceDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub price_book_entry_id: Uuid,
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
    pub paired_price_id: Option<Uuid>,
    pub return_of_price_id: Option<Uuid>,
    pub state: String,
    /// Display state of matrix row 10: draft, pending, rejected, scheduled, active or superseded.
    pub status: String,
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
impl From<entity::price::Model> for PricingPriceDto {
    fn from(m: entity::price::Model) -> Self {
        let status = m
            .state
            .parse::<crate::domain::price::PriceState>()
            .map_or_else(
                |_| m.state.clone(),
                |state| {
                    crate::domain::price::window_status(
                        state,
                        m.effective_from,
                        m.effective_to,
                        time::OffsetDateTime::now_utc().date(),
                    )
                    .to_owned()
                },
            );
        Self {
            id: m.id,
            tenant_id: m.tenant_id,
            price_book_entry_id: m.price_book_entry_id,
            version_no: m.version_no,
            dim_value: m.dim_value,
            model: m.model,
            price_json: m.price_json,
            min_fee: m.min_fee,
            eligibility: m.eligibility,
            effective_from: m.effective_from.to_string(),
            effective_to: m.effective_to.map(|v| v.to_string()),
            keep_for_bound: m.keep_for_bound,
            closed_explicitly: m.closed_explicitly,
            temporary_until: m.temporary_until.map(|v| v.to_string()),
            paired_price_id: m.paired_price_id,
            return_of_price_id: m.return_of_price_id,
            state: m.state,
            status,
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
pub struct PricingPriceBookEntryList {
    pub items: Vec<PricingPriceBookEntryDto>,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingExportEntry {
    pub entry: PricingPriceBookEntryDto,
    pub prices: Vec<PricingPriceDto>,
}
#[toolkit_macros::api_dto(response)]
pub struct PriceBookExport {
    pub book: PriceBookDto,
    pub entries: Vec<PricingExportEntry>,
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

#[toolkit_macros::api_dto(request)]
#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PricingPriceBookEntryCreate {
    pub sku_id: Uuid,
    pub period: Option<String>,
    pub dimension_key: Option<String>,
    pub invoice_line_override: Option<String>,
}
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::option_option,
    reason = "PATCH distinguishes omission and clearing"
)]
pub struct PricingPriceBookEntryPatch {
    #[serde(default, deserialize_with = "nullable_date")]
    pub dimension_key: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_date")]
    pub invoice_line_override: Option<Option<String>>,
}

#[toolkit_macros::api_dto(response)]
pub struct PricingReferenceOpDto {
    pub op_id: Uuid,
    pub kind: String,
    pub state: String,
    /// The reference the op works for: `price_book_entry` or `plan_item` (D-407).
    pub ref_kind: String,
    pub ref_id: Uuid,
    pub sku_id: Uuid,
    pub reservation_id: Option<Uuid>,
    pub attempts: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub next_attempt_at: time::OffsetDateTime,
    pub last_error: Option<String>,
}
impl From<entity::reference_op::Model> for PricingReferenceOpDto {
    fn from(op: entity::reference_op::Model) -> Self {
        Self {
            op_id: op.op_id,
            kind: op.kind,
            state: op.state,
            ref_kind: op.ref_kind,
            ref_id: op.ref_id,
            sku_id: op.sku_id,
            reservation_id: op.reservation_id,
            attempts: op.attempts,
            next_attempt_at: op.next_attempt_at,
            last_error: op.last_error,
        }
    }
}
#[toolkit_macros::api_dto(response)]
pub struct PricingReferenceOpPage {
    pub items: Vec<PricingReferenceOpDto>,
    pub next_cursor: Option<Uuid>,
}
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PricingReferenceOpQuery {
    pub state: Option<String>,
    pub limit: Option<u64>,
    pub cursor: Option<Uuid>,
}

#[toolkit_macros::api_dto(request)]
#[derive(Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PricingPriceCreate {
    pub dim_value: Option<String>,
    pub model: String,
    pub price: serde_json::Value,
    pub min_fee: Option<String>,
    pub eligibility: String,
    pub effective_from: String,
    pub temporary_until: Option<String>,
    pub note: Option<String>,
}
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::option_option,
    reason = "PATCH distinguishes omission, null clearing and a new value"
)]
pub struct PricingPricePatch {
    #[serde(default, deserialize_with = "nullable_date")]
    pub dim_value: Option<Option<String>>,
    pub model: Option<String>,
    pub price: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "nullable_date")]
    pub min_fee: Option<Option<String>>,
    pub eligibility: Option<String>,
    pub effective_from: Option<String>,
    #[serde(default, deserialize_with = "nullable_date")]
    pub note: Option<Option<String>>,
}
/// The draft price, and its return partner when the request made a temporary pair.
#[toolkit_macros::api_dto(response)]
pub struct PricingPriceCreated {
    pub items: Vec<PricingPriceDto>,
}

/// One reviewer decision; decisions of earlier generations are kept and marked stale.
#[toolkit_macros::api_dto(response)]
pub struct PricingDecisionDto {
    pub actor: Uuid,
    pub generation: i32,
    pub decision: String,
    pub note: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: time::OffsetDateTime,
    pub stale: bool,
}
impl From<bss_approval::Decision> for PricingDecisionDto {
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
/// An approval unit with its snapshot, decisions and, on the card, the live impact.
#[toolkit_macros::api_dto(response)]
pub struct PricingApprovalUnitDto {
    pub id: Uuid,
    pub kind: String,
    pub ref_type: String,
    pub ref_id: Uuid,
    pub state: String,
    pub generation: i32,
    pub quorum_required: u32,
    pub common_effective_date: Option<String>,
    pub submitted_by: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub submitted_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub decided_at: Option<time::OffsetDateTime>,
    pub decided_note: Option<String>,
    pub snapshot: serde_json::Value,
    pub decisions: Vec<PricingDecisionDto>,
    pub impact: Option<serde_json::Value>,
}
impl From<bss_approval::Unit> for PricingApprovalUnitDto {
    fn from(u: bss_approval::Unit) -> Self {
        Self {
            id: u.id,
            kind: u.kind,
            ref_type: u.ref_type,
            ref_id: u.ref_id,
            state: u.state.as_str().into(),
            generation: u.generation,
            quorum_required: u.quorum_required,
            common_effective_date: u.common_effective_date.map(|d| d.to_string()),
            submitted_by: u.submitted_by,
            submitted_at: u.submitted_at,
            decided_at: u.decided_at,
            decided_note: u.decided_note,
            snapshot: u.snapshot,
            decisions: Vec::new(),
            impact: None,
        }
    }
}
#[toolkit_macros::api_dto(response)]
pub struct PricingApprovalUnitList {
    pub items: Vec<PricingApprovalUnitDto>,
}
/// A vote names the generation its reviewer saw; a reject needs a note.
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingVoteRequest {
    pub generation: i32,
    pub note: Option<String>,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingVoteReceipt {
    pub have: Option<u32>,
    pub need: Option<u32>,
    pub outcome: String,
    pub unit: PricingApprovalUnitDto,
}
/// The unit a submission recorded and its prices after the transaction.
#[toolkit_macros::api_dto(response)]
pub struct PricingSubmitReceipt {
    pub applied: bool,
    pub unit: PricingApprovalUnitDto,
    pub prices: Vec<PricingPriceDto>,
}
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingPublishChangesRequest {
    pub price_ids: Option<Vec<Uuid>>,
    pub common_effective_date: Option<String>,
}
/// One draft price as the operator sees it before publishing: the entry key, the chain,
/// the approved predecessor it follows, its pair partner and the default selection.
#[toolkit_macros::api_dto(response)]
pub struct PricingProposedPrice {
    pub price: PricingPriceDto,
    pub entry: PricingPriceBookEntryDto,
    pub chain: String,
    pub before: Option<PricingPriceDto>,
    pub pair_partner_id: Option<Uuid>,
    pub selected: bool,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingPublishChanges {
    pub book: PriceBookDto,
    pub prices: Vec<PricingProposedPrice>,
    /// Prices and entries the listed drafts touch; plans and subscriptions from phase 3.
    pub impact: serde_json::Value,
}
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingApprovalPolicyPut {
    pub kind: Option<String>,
    pub quorum: u32,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingApprovalPolicyDto {
    pub default_quorum: u32,
    pub overrides: std::collections::BTreeMap<String, u32>,
}
impl From<bss_approval::Policy> for PricingApprovalPolicyDto {
    fn from(p: bss_approval::Policy) -> Self {
        Self {
            default_quorum: p.default_quorum,
            overrides: p.overrides,
        }
    }
}
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PricingApprovalUnitQuery {
    pub state: Option<String>,
    pub kind: Option<String>,
    pub ref_id: Option<Uuid>,
    pub book_id: Option<Uuid>,
}
