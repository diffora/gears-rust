//! Gear-local `snake_case` wire types; SDK enums are represented by their stable tokens. A closed
//! set on a response is its `enum` (P-D-217); a request keeps `string`, so its door refuses.
use super::closed_sets::{
    ProductsApprovalKind, ProductsBillingTiming, ProductsCategoryStatus, ProductsDecisionKind,
    ProductsLifecycle, ProductsReferenceKind, ProductsReferenceState, ProductsSkuType,
    ProductsUnitState, ProductsVoteOutcome,
};
use crate::domain::sku::{NewSku, SkuPatch};
use crate::domain::validation::ValidationReport;
use crate::infra::storage::RepoError;
use bss_products_sdk::models::{
    BillingTiming, Category, Lifecycle, Sku, SkuContent, SkuType, SkuVersion,
};
use serde::Deserialize;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

// `Products`-prefixed like the gear's other shared names: settings-service serves its own
// `CategoryDto`, and the toolkit refuses two definitions under one component name at boot.
/// Wire representation of the registry Category.
#[toolkit_macros::api_dto(response)]
pub struct ProductsCategoryDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub code: String,
    pub name: String,
    pub is_default: bool,
    pub sort_order: i32,
    pub status: ProductsCategoryStatus,
    pub version: i64,
}
impl TryFrom<Category> for ProductsCategoryDto {
    type Error = RepoError;
    fn try_from(value: Category) -> Result<Self, RepoError> {
        Ok(Self {
            id: value.id,
            tenant_id: value.tenant_id,
            code: value.code,
            name: value.name,
            is_default: value.is_default,
            sort_order: value.sort_order,
            status: ProductsCategoryStatus::stored(
                &value.status,
                &format_args!("category {} status", value.id),
            )?,
            version: value.version,
        })
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
    pub r#type: ProductsSkuType,
    /// `null`: the SKU has no category (P-D-196).
    pub category_id: Option<Uuid>,
    pub description: String,
    pub sellable: bool,
    pub lifecycle: ProductsLifecycle,
    pub revision: i64,
    pub published_version: i64,
    pub gl_code: Option<String>,
    pub tax_category: Option<String>,
    pub invoice_line_template: Option<String>,
    pub billing_timing: Option<ProductsBillingTiming>,
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
            r#type: value.r#type.into(),
            category_id: value.category_id,
            description: value.description,
            sellable: value.sellable,
            lifecycle: value.lifecycle.into(),
            revision: value.revision,
            published_version: value.published_version,
            gl_code: value.gl_code,
            tax_category: value.tax_category,
            invoice_line_template: value.invoice_line_template,
            billing_timing: value.billing_timing.map(Into::into),
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
    pub r#type: ProductsSkuType,
    /// `null`: the SKU has no category (P-D-196).
    pub category_id: Option<Uuid>,
    pub description: String,
    pub sellable: bool,
    pub gl_code: Option<String>,
    pub tax_category: Option<String>,
    pub invoice_line_template: Option<String>,
    pub billing_timing: Option<ProductsBillingTiming>,
    pub usage_type_ref: Option<String>,
    pub unit: Option<String>,
}
impl From<SkuContent> for SkuContentDto {
    fn from(value: SkuContent) -> Self {
        Self {
            code: value.code,
            name: value.name,
            r#type: value.r#type.into(),
            category_id: value.category_id,
            description: value.description,
            sellable: value.sellable,
            gl_code: value.gl_code,
            tax_category: value.tax_category,
            invoice_line_template: value.invoice_line_template,
            billing_timing: value.billing_timing.map(Into::into),
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
#[expect(clippy::option_option, reason = "a PATCH field has three states")]
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
/// A category as its reads answer it (P-D-215): the category's fields and `sku_count`, the SKUs
/// that are not retired naming it — the ones that keep it in use (P-D-208), so a category with
/// `sku_count` 0 may be retired.
#[toolkit_macros::api_dto(response)]
pub struct ProductsCategoryItem {
    #[serde(flatten)]
    pub category: ProductsCategoryDto,
    pub sku_count: u64,
}
#[toolkit_macros::api_dto(response)]
pub struct SkuCard {
    pub sku: SkuDto,
    pub references: ReferencesDto,
    /// Pricing's usage of the SKU through the port pricing fills (P-D-197); `null` when no port is
    /// registered, when it refuses the caller, or when it cannot answer.
    pub usage: Option<SkuUsageDto>,
}
/// A SKU's prices by state, as pricing counts them (pricing D-428).
#[toolkit_macros::api_dto(response)]
pub struct SkuUsagePricesDto {
    pub approved: u64,
    pub pending: u64,
    pub draft: u64,
}
/// Pricing's usage of a SKU (P-D-197): its price-book entries, the distinct currencies of their
/// books (sorted), their prices by state, and the distinct plans naming them. Information for the
/// SKUs screen; never a fence input.
#[toolkit_macros::api_dto(response)]
pub struct SkuUsageDto {
    pub entries: u64,
    pub currencies: Vec<String>,
    pub prices: SkuUsagePricesDto,
    pub plans: u64,
}
impl From<bss_products_sdk::sku_usage::SkuUsage> for SkuUsageDto {
    fn from(u: bss_products_sdk::sku_usage::SkuUsage) -> Self {
        Self {
            entries: u.entries,
            currencies: u.currencies,
            prices: SkuUsagePricesDto {
                approved: u.prices.approved,
                pending: u.prices.pending,
                draft: u.prices.draft,
            },
            plans: u.plans,
        }
    }
}
#[toolkit_macros::api_dto(response)]
pub struct ReferencesDto {
    pub price_book_entries: u32,
    pub plans: u32,
    pub reserved: u32,
    pub by_owner: std::collections::BTreeMap<String, std::collections::BTreeMap<String, u32>>,
}
impl From<crate::domain::references::ReferenceSummary> for ReferencesDto {
    fn from(v: crate::domain::references::ReferenceSummary) -> Self {
        Self {
            price_book_entries: v.price_book_entries,
            plans: v.plans,
            reserved: v.reserved,
            by_owner: v.by_owner,
        }
    }
}
/// One item of the SKU list: the SKU's fields and pricing's `usage` (P-D-197), `null` when the
/// port is absent, refuses or cannot answer.
#[toolkit_macros::api_dto(response)]
pub struct SkuListItem {
    #[serde(flatten)]
    pub sku: SkuDto,
    pub usage: Option<SkuUsageDto>,
}
/// The SKU list's tab counts (P-D-211): every SKU the narrowing keeps, those in each lifecycle,
/// and those a pending approval unit locks (in any lifecycle).
#[toolkit_macros::api_dto(response)]
pub struct ProductsSkuCounts {
    pub all: u64,
    pub draft: u64,
    pub published: u64,
    pub deprecated: u64,
    pub retiring: u64,
    pub retired: u64,
    pub in_review: u64,
}
impl From<crate::infra::storage::repo::SkuCounts> for ProductsSkuCounts {
    fn from(c: crate::infra::storage::repo::SkuCounts) -> Self {
        Self {
            all: c.all,
            draft: c.draft,
            published: c.published,
            deprecated: c.deprecated,
            retiring: c.retiring,
            retired: c.retired,
            in_review: c.in_review,
        }
    }
}
/// One act in a SKU's history (P-D-213): when (`at`, the row's `written_at`: the submit, change and
/// draft doors take it before their transaction and keep it across a retry, the other writers inside
/// the attempt — never the commit; the history is in the order the acts wrote, by the audit row's id), who (`actor`, the nil uuid for the system's orphan-fence expiry), what
/// (`action`), the lifecycle it found and left (`null` on a row written before the audit log
/// carried them, on a create's `from`), the approval unit it concerned and its kind, and the note
/// it carried (a change's note, a decision's note, or the expiry's TTL).
#[toolkit_macros::api_dto(response)]
pub struct ProductsSkuHistoryEntry {
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub actor: Uuid,
    /// The audit row's action: a string, since no CHECK holds the column to a set (P-D-217).
    pub action: String,
    pub from_lifecycle: Option<ProductsLifecycle>,
    pub to_lifecycle: Option<ProductsLifecycle>,
    pub unit_id: Option<Uuid>,
    pub unit_kind: Option<String>,
    pub note: Option<String>,
}
impl From<crate::infra::storage::repo::SkuHistoryEntry> for ProductsSkuHistoryEntry {
    fn from(e: crate::infra::storage::repo::SkuHistoryEntry) -> Self {
        Self {
            at: e.at,
            actor: e.actor,
            action: e.action,
            from_lifecycle: e.from_lifecycle.map(Into::into),
            to_lifecycle: e.to_lifecycle.map(Into::into),
            unit_id: e.unit_id,
            unit_kind: e.unit_kind,
            note: e.note,
        }
    }
}
#[toolkit_macros::api_dto(request)]
pub struct SkuRequest {
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: String,
    /// Optional (P-D-196): omitted or `null` stays null, with no fallback to the default category.
    #[serde(default)]
    pub category_id: Option<Uuid>,
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
                .map(|s| parse_token(s, "billing_timing", BillingTiming::parse))
                .transpose()?,
            usage_type_ref: v.usage_type_ref,
            unit: v.unit,
        })
    }
}
#[toolkit_macros::api_dto(request)]
#[expect(
    clippy::option_option,
    reason = "None = omitted; Some(None) = clear; Some(Some(_)) = set"
)]
pub struct SkuPatchRequest {
    pub name: Option<String>,
    /// Omitted keeps the category, `null` clears it (P-D-196), a value sets it.
    #[serde(default, deserialize_with = "double_option")]
    pub category_id: Option<Option<Uuid>>,
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
                let parsed = BillingTiming::parse(&s);
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
    /// The submitter's reason: at most 2000 characters (400 `NOTE_TOO_LONG`), stored on the unit
    /// as `submit_note` and on the submit's history row, as sent (P-D-213, P-D-219).
    pub note: Option<String>,
}
/// The optional body of `POST /skus/{id}/submit` and `/retire` (P-D-219): the submitter's note,
/// at most 2000 characters (400 `NOTE_TOO_LONG`), stored on the unit as `submit_note` and on the
/// submit's history row, as sent. No body, `{}` and `note: null` carry none; any other field is a
/// 400.
#[toolkit_macros::api_dto(request)]
#[serde(deny_unknown_fields)]
pub struct ProductsSkuSubmitRequest {
    #[serde(default)]
    pub note: Option<String>,
}
#[toolkit_macros::api_dto(response)]
pub struct UnitDto {
    pub id: Uuid,
    /// The unit's kind, one products records: a stored unit of another kind is a corrupt row
    /// (500), never served.
    pub kind: ProductsApprovalKind,
    pub ref_type: String,
    pub ref_id: Uuid,
    pub state: ProductsUnitState,
    pub generation: i32,
    pub quorum_required: u32,
    #[serde(with = "crate::infra::serde_date::option")]
    pub common_effective_date: Option<Date>,
    pub submitted_by: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub submitted_at: OffsetDateTime,
    /// The submitter's note, as sent to the submit, change or retire door; null when none was
    /// sent, and on every unit submitted before the note was stored (P-D-219). Not content: the
    /// snapshot and its fingerprint do not carry it.
    pub submit_note: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub decided_at: Option<OffsetDateTime>,
    pub decided_note: Option<String>,
    pub snapshot: serde_json::Value,
    pub decisions: Vec<DecisionDto>,
    pub impact_live: Option<serde_json::Value>,
    /// Whether the caller may Approve this unit now (P-D-228): the approval engine's own rule
    /// (`bss_approval::approve_eligibility`, pricing D-459) over the unit's stored items and its
    /// decisions, with the caller as the voter. It is false for a decided unit, for its submitter
    /// and every author of its items (the SKU's creator: separation of duties) and for a caller who
    /// already voted in its current generation. It means Approve only: a reject judges no
    /// separation of duties, so the submitter and the SKU's creator may reject a unit whose flag
    /// is false. The grant is not judged here: without products approve the vote door still
    /// answers 403.
    pub caller_can_approve: bool,
}
/// `GET /approval-units/counts` (P-D-227): the units the list's narrowing keeps, by state and by
/// kind, every state and kind named (0 when none), and their total.
#[toolkit_macros::api_dto(response)]
pub struct ProductsApprovalUnitCounts {
    pub by_state: ProductsApprovalUnitStateCounts,
    pub by_kind: ProductsApprovalUnitKindCounts,
    pub total: u64,
}
/// The units in each state (P-D-227).
#[toolkit_macros::api_dto(response)]
#[derive(Default)]
pub struct ProductsApprovalUnitStateCounts {
    pub pending: u64,
    pub approved: u64,
    pub rejected: u64,
    pub withdrawn: u64,
}
/// The units of each kind products records (P-D-227).
#[toolkit_macros::api_dto(response)]
#[derive(Default)]
#[allow(
    clippy::struct_field_names,
    reason = "the fields are the stored kind names, sku_publish, sku_change and sku_retire"
)]
pub struct ProductsApprovalUnitKindCounts {
    pub sku_publish: u64,
    pub sku_change: u64,
    pub sku_retire: u64,
}
#[toolkit_macros::api_dto(response)]
pub struct DecisionDto {
    pub actor: Uuid,
    pub generation: i32,
    pub decision: ProductsDecisionKind,
    pub note: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub stale: bool,
}
/// One page of the unit list (P-D-224): its units, and the toolkit pager's `page_info`, whose
/// `next_cursor` continues it.
#[toolkit_macros::api_dto(response)]
pub struct UnitList {
    pub items: Vec<UnitDto>,
    pub page_info: toolkit_odata::PageInfo,
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
    pub outcome: ProductsVoteOutcome,
    pub unit: UnitDto,
}
#[toolkit_macros::api_dto(request)]
pub struct ApprovalPolicyRequest {
    pub kind: Option<String>,
    pub quorum: u32,
}
#[toolkit_macros::api_dto(response)]
pub struct ApprovalPolicyDto {
    pub default_quorum: u32,
    pub overrides: std::collections::BTreeMap<String, u32>,
}
#[cfg(test)]
#[path = "dto_tests.rs"]
mod dto_tests;

impl UnitDto {
    /// The unit as `reader` reads it: its decisions of every generation, and whether `reader` may
    /// approve it, judged by the engine's own predicate over the `authors` of the unit's stored
    /// (current generation) items and its `decisions` (P-D-228). `impact_live` is the caller's to
    /// fill.
    /// # Errors
    /// `CorruptRow` for a kind products does not record.
    pub fn of(
        u: bss_approval::Unit,
        authors: &[Uuid],
        decisions: Vec<bss_approval::Decision>,
        reader: Uuid,
    ) -> Result<Self, RepoError> {
        let caller_can_approve =
            bss_approval::approve_eligibility(&u, authors.iter().copied(), &decisions, reader)
                .refusal
                .is_none();
        Ok(Self {
            id: u.id,
            kind: ProductsApprovalKind::stored(&u.kind, &format_args!("approval unit {}", u.id))?,
            ref_type: u.ref_type,
            ref_id: u.ref_id,
            state: u.state.into(),
            generation: u.generation,
            quorum_required: u.quorum_required,
            common_effective_date: u.common_effective_date,
            submitted_by: u.submitted_by,
            submitted_at: u.submitted_at,
            submit_note: u.submit_note,
            decided_at: u.decided_at,
            decided_note: u.decided_note,
            snapshot: u.snapshot,
            decisions: decisions.into_iter().map(Into::into).collect(),
            impact_live: None,
            caller_can_approve,
        })
    }
}
impl From<bss_approval::Decision> for DecisionDto {
    fn from(d: bss_approval::Decision) -> Self {
        Self {
            actor: d.actor,
            generation: d.generation,
            decision: d.verdict.into(),
            note: d.note,
            at: d.at,
            stale: d.stale,
        }
    }
}
impl From<bss_approval::Policy> for ApprovalPolicyDto {
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
    pub kind: ProductsReferenceKind,
    pub ref_id: Uuid,
    pub state: ProductsReferenceState,
    pub forced: bool,
}
impl TryFrom<crate::infra::storage::repo::SkuReference> for ReferenceReceipt {
    type Error = RepoError;
    fn try_from(r: crate::infra::storage::repo::SkuReference) -> Result<Self, RepoError> {
        Ok(Self {
            reservation_id: r.id,
            sku_id: r.sku_id,
            owner: r.owner_gear,
            kind: ProductsReferenceKind::stored(
                &r.ref_kind,
                &format_args!("reference {} ref_kind", r.id),
            )?,
            ref_id: r.ref_id,
            state: ProductsReferenceState::stored(
                &r.state,
                &format_args!("reference {} state", r.id),
            )?,
            forced: r.forced,
        })
    }
}
