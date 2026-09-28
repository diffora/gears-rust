//! Pricing authoring wire contracts, with unique `OpenAPI` names and `snake_case` fields. A closed
//! set on a response is its `enum` (D-439); a request keeps `string`, so its door's code refuses.
use crate::api::rest::closed_sets::{
    PricingBillingTiming, PricingChargeKind, PricingDecisionKind, PricingEligibility,
    PricingEntryReferenceState, PricingItemReferenceState, PricingModel, PricingPeriod,
    PricingPriceState, PricingPriceStatus, PricingReferenceOpKind, PricingReferenceOpRefKind,
    PricingReferenceOpState, PricingRevisionState, PricingTreatment, PricingUnitState,
    PricingVoteOutcome,
};
use crate::infra::storage::{RepoError, entity};
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
    /// Free text, at most 2000 characters, or `null` (D-444).
    pub description: Option<String>,
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
            description: m.description,
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
    pub charge_kind: PricingChargeKind,
    pub period: Option<PricingPeriod>,
    /// The entry's model (D-427), fixed for its life and part of its key.
    pub model: PricingModel,
    pub dimension_key: Option<String>,
    pub invoice_line_override: Option<String>,
    pub reservation_id: Uuid,
    pub reference_state: PricingEntryReferenceState,
    pub version: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}
impl TryFrom<entity::price_book_entry::Model> for PricingPriceBookEntryDto {
    type Error = RepoError;
    fn try_from(m: entity::price_book_entry::Model) -> Result<Self, RepoError> {
        let id = m.id;
        Ok(Self {
            id,
            tenant_id: m.tenant_id,
            book_id: m.book_id,
            sku_id: m.sku_id,
            charge_kind: PricingChargeKind::stored(
                &m.charge_kind,
                &format_args!("entry {id} charge_kind"),
            )?,
            period: m
                .period
                .as_deref()
                .map(|p| PricingPeriod::stored(p, &format_args!("entry {id} period")))
                .transpose()?,
            model: PricingModel::stored(&m.model, &format_args!("entry {id} model"))?,
            dimension_key: m.dimension_key,
            invoice_line_override: m.invoice_line_override,
            reservation_id: m.reservation_id,
            reference_state: PricingEntryReferenceState::stored(
                &m.reference_state,
                &format_args!("entry {id} reference_state"),
            )?,
            version: m.version,
            created_at: m.created_at,
            updated_at: m.updated_at,
        })
    }
}
/// An entry's prices by state; a rejected price is not counted (D-428). The approved ones are
/// also counted by where their window stands today (D-440): `approved` = `scheduled + active +
/// superseded`.
#[toolkit_macros::api_dto(response)]
pub struct PricingEntryPriceCounts {
    pub approved: u64,
    pub pending: u64,
    pub draft: u64,
    /// Approved prices that start after today.
    pub scheduled: u64,
    /// Approved prices in force today.
    pub active: u64,
    /// Approved prices whose window ended on or before today.
    pub superseded: u64,
}
/// An entry's usage (D-428): its prices by state; `plans`, the distinct plans with a draft,
/// pending or published revision whose items name it; `plans_superseded_only`, the distinct plans
/// that name it only through superseded revisions (they still keep it `ENTRY_IN_USE`).
#[toolkit_macros::api_dto(response)]
pub struct PricingEntryUsage {
    pub prices: PricingEntryPriceCounts,
    pub plans: u64,
    pub plans_superseded_only: u64,
}
impl From<crate::infra::usage::EntryUsage> for PricingEntryUsage {
    fn from(u: crate::infra::usage::EntryUsage) -> Self {
        Self {
            prices: PricingEntryPriceCounts {
                approved: u.prices.approved,
                pending: u.prices.pending,
                draft: u.prices.draft,
                scheduled: u.prices.scheduled,
                active: u.prices.active,
                superseded: u.prices.superseded,
            },
            plans: u.plans,
            plans_superseded_only: u.plans_superseded_only,
        }
    }
}
/// What the two entry reads answer (D-428): the entry's fields, its `usage` and its
/// `current_price` (D-440): the default chain's approved price in force today, as D-434 chooses
/// and shows it — `null` when none is, or when the caller does not hold `price_book` read on the
/// entry's book. Every other answer that carries an entry (POST, PATCH, the stored receipt, the
/// export, publish-changes) keeps [`PricingPriceBookEntryDto`].
#[toolkit_macros::api_dto(response)]
pub struct PricingPriceBookEntryReadDto {
    #[serde(flatten)]
    pub entry: PricingPriceBookEntryDto,
    pub usage: PricingEntryUsage,
    pub current_price: Option<PricingPriceDto>,
}
impl PricingPriceBookEntryReadDto {
    /// # Errors
    /// `CorruptRow` for a stored token outside its closed set (D-439).
    pub fn of(
        m: entity::price_book_entry::Model,
        usage: crate::infra::usage::EntryUsage,
        current_price: Option<PricingPriceDto>,
    ) -> Result<Self, RepoError> {
        Ok(Self {
            entry: m.try_into()?,
            usage: usage.into(),
            current_price,
        })
    }
}
/// `GET /price-book-entries/{id}/prices` (D-440): every price of the entry in every state, the
/// default chain first, then each dimension value's chain in ascending order; each chain by
/// `effective_from`, then `version_no`.
#[toolkit_macros::api_dto(response)]
pub struct PricingEntryPriceList {
    pub items: Vec<PricingPriceDto>,
}
/// The query of `GET /price-book-entries/{id}/prices`: an optional `status`, one display status
/// or several comma-separated.
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PricingEntryPricesQuery {
    pub status: Option<String>,
}
/// A book's prices by state — a rejected price included — and its approved prices by where their
/// window stands today: `approved` = `scheduled + active + superseded` (D-441).
#[toolkit_macros::api_dto(response)]
pub struct PricingBookPriceCounts {
    pub draft: u64,
    pub pending: u64,
    pub approved: u64,
    pub scheduled: u64,
    pub active: u64,
    pub superseded: u64,
    pub rejected: u64,
}
/// A book's stats (D-441): its entries and their distinct SKUs; the distinct plans with a draft,
/// pending or published revision on the book (`plans`), and those that name it only through
/// superseded revisions (`plans_superseded_only`, what the delete refuses as
/// `BOOK_IN_PLAN_HISTORY`); its prices by state; its `prices` units in review; and the latest
/// change of the book, its entries, their prices and its units. `DELETE /price-books/{id}`
/// succeeds exactly when `entries`, `plans` and `plans_superseded_only` are 0 (D-444).
#[toolkit_macros::api_dto(response)]
pub struct PricingPriceBookStats {
    pub entries: u64,
    pub skus: u64,
    pub plans: u64,
    pub plans_superseded_only: u64,
    pub prices: PricingBookPriceCounts,
    pub pending_units: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub last_change_at: time::OffsetDateTime,
}
impl From<crate::infra::book_stats::BookStats> for PricingPriceBookStats {
    fn from(s: crate::infra::book_stats::BookStats) -> Self {
        let p = s.prices;
        Self {
            entries: s.entries,
            skus: s.skus,
            plans: s.plans,
            plans_superseded_only: s.plans_superseded_only,
            prices: PricingBookPriceCounts {
                draft: p.draft,
                pending: p.pending,
                approved: p.approved,
                scheduled: p.scheduled,
                active: p.active,
                superseded: p.superseded,
                rejected: p.rejected,
            },
            pending_units: s.pending_units,
            last_change_at: s.last_change_at,
        }
    }
}
/// What the two book reads answer (D-441): the book's fields and its `stats`. The write answers
/// (POST, PATCH, the stored receipt), the export and publish-changes keep [`PriceBookDto`].
#[toolkit_macros::api_dto(response)]
pub struct PricingPriceBookReadDto {
    #[serde(flatten)]
    pub book: PriceBookDto,
    pub stats: PricingPriceBookStats,
}
/// One entry of a SKU as `GET /price-book-entries?sku_id=` answers it (D-434): the entry, its
/// book's code, name and currency, its usage (D-428), and the default chain's price in force
/// today — `null` when none is, or when the caller does not hold `price_book` read (the export's
/// grant).
#[toolkit_macros::api_dto(response)]
pub struct PricingSkuEntryDto {
    #[serde(flatten)]
    pub entry: PricingPriceBookEntryDto,
    pub book_code: String,
    pub book_name: String,
    pub currency: String,
    pub usage: PricingEntryUsage,
    pub current_price: Option<PricingPriceDto>,
}
/// `GET /price-book-entries?sku_id=`: the SKU's entries in every book of the tenant.
#[toolkit_macros::api_dto(response)]
pub struct PricingSkuEntryList {
    pub items: Vec<PricingSkuEntryDto>,
}
/// The query of `GET /price-book-entries`: exactly one `sku_id`.
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PricingSkuEntryQuery {
    pub sku_id: Option<Uuid>,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingPriceDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub price_book_entry_id: Uuid,
    pub version_no: i32,
    pub dim_value: Option<String>,
    /// The entry's model, read-only (D-427): a price has no model of its own.
    pub model: PricingModel,
    pub price_json: serde_json::Value,
    pub min_fee: Option<String>,
    pub eligibility: PricingEligibility,
    pub effective_from: String,
    pub effective_to: Option<String>,
    pub keep_for_bound: bool,
    pub closed_explicitly: bool,
    pub temporary_until: Option<String>,
    pub paired_price_id: Option<Uuid>,
    pub return_of_price_id: Option<Uuid>,
    pub state: PricingPriceState,
    /// Display state of matrix row 10: an approved price shows where its window stands today.
    pub status: PricingPriceStatus,
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
impl PricingPriceDto {
    /// A stored price with its entry's model (D-427).
    /// # Errors
    /// `CorruptRow` for a stored token outside its closed set (D-439).
    pub fn of(m: entity::price::Model, model: &str) -> Result<Self, RepoError> {
        Self::at(m, model, time::OffsetDateTime::now_utc().date())
    }
    /// [`Self::of`] with its display status on `today`, the one day a whole read is dated on
    /// (D-440).
    /// # Errors
    /// `CorruptRow` for a stored token outside its closed set (D-439).
    pub fn at(m: entity::price::Model, model: &str, today: time::Date) -> Result<Self, RepoError> {
        let id = m.id;
        let state = PricingPriceState::stored(&m.state, &format_args!("price {id} state"))?;
        let status = crate::domain::price::window_display(
            state.into(),
            m.effective_from,
            m.effective_to,
            today,
        )
        .into();
        Ok(Self {
            id,
            tenant_id: m.tenant_id,
            price_book_entry_id: m.price_book_entry_id,
            version_no: m.version_no,
            dim_value: m.dim_value,
            model: PricingModel::stored(model, &format_args!("price {id} model"))?,
            price_json: m.price_json,
            min_fee: m.min_fee,
            eligibility: PricingEligibility::stored(
                &m.eligibility,
                &format_args!("price {id} eligibility"),
            )?,
            effective_from: m.effective_from.to_string(),
            effective_to: m.effective_to.map(|v| v.to_string()),
            keep_for_bound: m.keep_for_bound,
            closed_explicitly: m.closed_explicitly,
            temporary_until: m.temporary_until.map(|v| v.to_string()),
            paired_price_id: m.paired_price_id,
            return_of_price_id: m.return_of_price_id,
            state,
            status,
            pending_unit_id: m.pending_unit_id,
            approved_by_unit_id: m.approved_by_unit_id,
            note: m.note,
            created_by: m.created_by,
            approved_at: m.approved_at,
            version: m.version,
            created_at: m.created_at,
            updated_at: m.updated_at,
        })
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
    /// Free text, at most 2000 characters (D-444).
    pub description: Option<String>,
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
    /// Omitted keeps the description, `null` clears it; at most 2000 characters (D-444).
    #[serde(default, deserialize_with = "nullable_date")]
    pub description: Option<Option<String>>,
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
pub struct PricingPriceBookEntryList {
    pub items: Vec<PricingPriceBookEntryReadDto>,
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
/// One key of `PUT /dimension-keys`: the key and its whole list of values (a full replace).
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingDimensionEntry {
    pub key: String,
    pub values: Vec<String>,
}
/// `PUT /dimension-keys`: the tenant's whole registry.
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingDimensions {
    pub items: Vec<PricingDimensionEntry>,
}
/// `PATCH /dimension-keys` (D-436): the values of ONE declared key to add and to remove; keys
/// themselves are added and removed by the PUT.
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingDimensionKeyPatch {
    pub key: String,
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}
/// What uses one dimension value (D-436): the prices, of any state, whose entry names the key
/// and whose chain is the value. A value with prices is not removed (`DIM_VALUE_IN_USE`).
#[toolkit_macros::api_dto(response)]
pub struct PricingDimensionValueUsage {
    pub prices: u64,
}
/// One value of a key with its use.
#[toolkit_macros::api_dto(response)]
pub struct PricingDimensionValue {
    pub value: String,
    pub usage: PricingDimensionValueUsage,
}
/// One key of the registry as the reads and writes answer it.
#[toolkit_macros::api_dto(response)]
pub struct PricingDimensionKey {
    pub key: String,
    pub values: Vec<PricingDimensionValue>,
}
/// The registry as `GET`, `PUT` and `PATCH /dimension-keys` answer it (D-436).
#[toolkit_macros::api_dto(response)]
pub struct PricingDimensionRegistry {
    pub items: Vec<PricingDimensionKey>,
}
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingSettingsPut {
    pub default_timing: String,
    /// `half_up`, `half_even`, `half_down`, `up` or `down` (D-437).
    pub default_rounding: String,
    pub default_gl: Option<String>,
    pub default_tax_category: Option<String>,
    pub invoice_line_templates: std::collections::BTreeMap<String, String>,
    /// Required (D-438): the currencies a NEW book may take, a full replace; `[]` is any.
    pub currencies: Vec<String>,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingSettingsDto {
    pub default_timing: PricingBillingTiming,
    /// One of the five modes (D-437) once written through the door; a string, not an enum: no
    /// CHECK guards the column, and a legacy value reads back as stored (D-439).
    pub default_rounding: String,
    pub default_gl: Option<String>,
    pub default_tax_category: Option<String>,
    pub invoice_line_templates: serde_json::Value,
    /// The currencies a new book may take; empty means any (D-438).
    pub currencies: Vec<String>,
    pub version: i64,
    /// When the settings were last written; null before the first write (version 0).
    #[serde(with = "time::serde::rfc3339::option")]
    pub updated_at: Option<time::OffsetDateTime>,
    /// Who wrote them last; null before the first write and on a row written before D-438.
    pub updated_by: Option<Uuid>,
}

#[toolkit_macros::api_dto(request)]
#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PricingPriceBookEntryCreate {
    pub sku_id: Uuid,
    /// Required and fixed for the entry's life (D-427): `flat`, `per_unit`, `graduated`,
    /// `volume` or `package`, one the SKU's charge kind allows.
    pub model: String,
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

/// `POST /plan-revisions/{id}/items`: one item, one op with its own key (D-407). A null entry
/// is an included item with no charge.
#[toolkit_macros::api_dto(request)]
#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PricingPlanItemCreate {
    pub sku_id: Uuid,
    pub price_book_entry_id: Option<Uuid>,
    /// `paid`, `optional` or `included`.
    pub treatment: String,
    /// Canonical decimal text, for an included usage item.
    pub included_qty: Option<String>,
    pub qty_min: Option<i32>,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingPlanItemDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub revision_id: Uuid,
    pub sku_id: Uuid,
    pub price_book_entry_id: Option<Uuid>,
    pub treatment: PricingTreatment,
    pub included_qty: Option<String>,
    pub qty_min: Option<i32>,
    /// None until a reserve answers: a copied item attaches after its write (D-413).
    pub reservation_id: Option<Uuid>,
    pub reference_state: PricingItemReferenceState,
    pub version: i64,
    pub created_by: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}
impl TryFrom<entity::plan_item::Model> for PricingPlanItemDto {
    type Error = RepoError;
    fn try_from(m: entity::plan_item::Model) -> Result<Self, RepoError> {
        let id = m.id;
        Ok(Self {
            id,
            tenant_id: m.tenant_id,
            revision_id: m.revision_id,
            sku_id: m.sku_id,
            price_book_entry_id: m.price_book_entry_id,
            treatment: PricingTreatment::stored(
                &m.treatment,
                &format_args!("plan item {id} treatment"),
            )?,
            included_qty: m.included_qty,
            qty_min: m.qty_min,
            reservation_id: m.reservation_id,
            reference_state: PricingItemReferenceState::stored(
                &m.reference_state,
                &format_args!("plan item {id} reference_state"),
            )?,
            version: m.version,
            created_by: m.created_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
        })
    }
}
/// `GET /plan-items/{id}` (D-434): the item with its revision's number and state and its plan.
#[toolkit_macros::api_dto(response)]
pub struct PricingPlanItemReadDto {
    #[serde(flatten)]
    pub item: PricingPlanItemDto,
    pub plan_id: Uuid,
    pub rev_no: i32,
    /// The revision's state.
    pub state: PricingRevisionState,
}
/// The query of `GET /plans`: an optional `sku_id` (D-434).
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PricingPlanQuery {
    pub sku_id: Option<Uuid>,
}
/// `POST /plans`: a plan and its draft rev 1 on `book_id`.
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingPlanCreate {
    pub code: String,
    pub name: String,
    pub book_id: Uuid,
}
/// `POST /plans/{id}/clone`: the new plan's own code and name; its draft rev 1 copies the source's
/// published revision.
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingPlanClone {
    pub code: String,
    pub name: String,
}
/// `PATCH /plans/{id}`: the plan's name, under If-Match.
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
pub struct PricingPlanPatch {
    pub name: String,
}
/// One revision of a plan as its plan lists it: the header, without items.
#[toolkit_macros::api_dto(response)]
pub struct PricingPlanRevisionHeader {
    pub id: Uuid,
    pub rev_no: i32,
    pub book_id: Uuid,
    pub state: PricingRevisionState,
    pub available_from: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub published_at: Option<time::OffsetDateTime>,
}
impl TryFrom<&entity::plan_revision::Model> for PricingPlanRevisionHeader {
    type Error = RepoError;
    fn try_from(m: &entity::plan_revision::Model) -> Result<Self, RepoError> {
        Ok(Self {
            id: m.id,
            rev_no: m.rev_no,
            book_id: m.book_id,
            state: PricingRevisionState::stored(
                &m.state,
                &format_args!("revision {} state", m.id),
            )?,
            available_from: m.available_from.map(|d| d.to_string()),
            published_at: m.published_at,
        })
    }
}
/// A plan with the headers of its revisions in revision order.
#[toolkit_macros::api_dto(response)]
pub struct PricingPlanDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub code: String,
    pub name: String,
    /// The revision number the last applied `plan_revision` unit published.
    pub published_rev: Option<i32>,
    pub version: i64,
    pub created_by: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
    pub revisions: Vec<PricingPlanRevisionHeader>,
}
impl PricingPlanDto {
    /// # Errors
    /// `CorruptRow` for a stored token outside its closed set (D-439).
    pub fn of(
        m: entity::plan::Model,
        revisions: &[entity::plan_revision::Model],
    ) -> Result<Self, RepoError> {
        Ok(Self {
            id: m.id,
            tenant_id: m.tenant_id,
            code: m.code,
            name: m.name,
            published_rev: m.published_rev,
            version: m.version,
            created_by: m.created_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
            revisions: revisions
                .iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}
#[toolkit_macros::api_dto(response)]
pub struct PricingPlanList {
    pub items: Vec<PricingPlanDto>,
}
/// A revision with its items (D-407: items are a sub-resource, read with their revision).
#[toolkit_macros::api_dto(response)]
pub struct PricingPlanRevisionDto {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub plan_id: Uuid,
    pub rev_no: i32,
    pub book_id: Uuid,
    pub state: PricingRevisionState,
    /// The sale date; null means "at publish".
    pub available_from: Option<String>,
    pub pending_unit_id: Option<Uuid>,
    pub approved_by_unit_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub published_at: Option<time::OffsetDateTime>,
    pub version: i64,
    /// The draft's author: the one principal who edits it and its items (D-404).
    pub created_by: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
    pub items: Vec<PricingPlanItemDto>,
}
impl PricingPlanRevisionDto {
    /// # Errors
    /// `CorruptRow` for a stored token outside its closed set (D-439).
    pub fn of(
        m: &entity::plan_revision::Model,
        items: Vec<entity::plan_item::Model>,
    ) -> Result<Self, RepoError> {
        Ok(Self {
            id: m.id,
            tenant_id: m.tenant_id,
            plan_id: m.plan_id,
            rev_no: m.rev_no,
            book_id: m.book_id,
            state: PricingRevisionState::stored(
                &m.state,
                &format_args!("revision {} state", m.id),
            )?,
            available_from: m.available_from.map(|d| d.to_string()),
            pending_unit_id: m.pending_unit_id,
            approved_by_unit_id: m.approved_by_unit_id,
            published_at: m.published_at,
            version: m.version,
            created_by: m.created_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
            items: items
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}
/// `PATCH /plan-revisions/{id}`, draft only: the book and the sale date, never an item list
/// (D-407). A book change remaps each item to the new book's entry of the same (SKU, charge kind,
/// period); an unmatched item keeps its entry and the checks show it foreign.
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::option_option,
    reason = "PATCH distinguishes omission, null clearing and a new date"
)]
pub struct PricingPlanRevisionPatch {
    pub book_id: Option<Uuid>,
    #[serde(default, deserialize_with = "nullable_date")]
    pub available_from: Option<Option<String>>,
}
/// `PATCH /plan-items/{id}`, draft only: never a SKU change (the SKU is the item's reference).
#[toolkit_macros::api_dto(request)]
#[derive(Clone)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::option_option,
    reason = "PATCH distinguishes omission, null clearing and a new value"
)]
pub struct PricingPlanItemPatch {
    pub treatment: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub included_qty: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub qty_min: Option<Option<i32>>,
    #[serde(default, deserialize_with = "nullable")]
    pub price_book_entry_id: Option<Option<Uuid>>,
}
#[allow(
    clippy::option_option,
    reason = "PATCH distinguishes omission, null clearing and a new value"
)]
fn nullable<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    <Option<T> as serde::Deserialize>::deserialize(d).map(Some)
}
/// One row of a revision's checks (D-408). An `info` row is always ok and never blocks.
#[toolkit_macros::api_dto(response)]
pub struct PricingPlanCheckDto {
    pub code: String,
    pub ok: bool,
    pub label: String,
    pub detail: String,
    pub info: bool,
    /// The approval units whose pending prices would cover what is uncovered: computed on every
    /// read, never stored (spec §6).
    pub blocked_by: Vec<Uuid>,
}
impl From<crate::domain::plan::Check> for PricingPlanCheckDto {
    fn from(c: crate::domain::plan::Check) -> Self {
        Self {
            code: c.code.into(),
            ok: c.ok,
            label: c.label,
            detail: c.detail,
            info: c.info,
            blocked_by: c.blocked_by,
        }
    }
}
/// `GET /plan-revisions/{id}/checks`: every check on the sale date, from fresh SKU reads.
#[toolkit_macros::api_dto(response)]
pub struct PricingPlanChecksDto {
    pub checks: Vec<PricingPlanCheckDto>,
    /// Every check is ok: the revision may be submitted.
    pub ready: bool,
    /// `available_from`, or today for a revision sold from its publication.
    pub sale_date: String,
}
#[toolkit_macros::api_dto(response)]
pub struct PricingReferenceOpDto {
    pub op_id: Uuid,
    pub kind: PricingReferenceOpKind,
    pub state: PricingReferenceOpState,
    /// The reference the op works for (D-407).
    pub ref_kind: PricingReferenceOpRefKind,
    pub ref_id: Uuid,
    pub sku_id: Uuid,
    pub reservation_id: Option<Uuid>,
    pub attempts: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub next_attempt_at: time::OffsetDateTime,
    pub last_error: Option<String>,
}
impl TryFrom<entity::reference_op::Model> for PricingReferenceOpDto {
    type Error = RepoError;
    fn try_from(op: entity::reference_op::Model) -> Result<Self, RepoError> {
        let id = op.op_id;
        Ok(Self {
            op_id: id,
            kind: PricingReferenceOpKind::stored(&op.kind, &format_args!("op {id} kind"))?,
            state: PricingReferenceOpState::stored(&op.state, &format_args!("op {id} state"))?,
            ref_kind: PricingReferenceOpRefKind::stored(
                &op.ref_kind,
                &format_args!("op {id} ref_kind"),
            )?,
            ref_id: op.ref_id,
            sku_id: op.sku_id,
            reservation_id: op.reservation_id,
            attempts: op.attempts,
            next_attempt_at: op.next_attempt_at,
            last_error: op.last_error,
        })
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
    /// Money in the entry's model (D-427); a price carries no model of its own.
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
    pub price: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "nullable_date")]
    pub min_fee: Option<Option<String>>,
    pub eligibility: Option<String>,
    /// A temporary draft's start moves with its end; a return's start is its pair's end.
    pub effective_from: Option<String>,
    /// The end of the temporary half of a draft; its pair is re-derived over the new dates. Any
    /// other price, and `null`, is 400 `TEMPORARY_PRICE_FIXED`.
    #[serde(default, deserialize_with = "nullable_date")]
    pub temporary_until: Option<Option<String>>,
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
    pub decision: PricingDecisionKind,
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
            decision: d.verdict.into(),
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
    pub state: PricingUnitState,
    pub generation: i32,
    pub quorum_required: u32,
    pub common_effective_date: Option<String>,
    pub submitted_by: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub submitted_at: time::OffsetDateTime,
    /// The submitter's note (D-445). Pricing's submit doors take none, so it is null on every
    /// pricing unit; the field keeps the unit shape products shares (P-D-219).
    pub submit_note: Option<String>,
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
            state: u.state.into(),
            generation: u.generation,
            quorum_required: u.quorum_required,
            common_effective_date: u.common_effective_date.map(|d| d.to_string()),
            submitted_by: u.submitted_by,
            submitted_at: u.submitted_at,
            submit_note: u.submit_note,
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
    pub outcome: PricingVoteOutcome,
    pub unit: PricingApprovalUnitDto,
}
/// The unit a submission recorded and its prices after the transaction.
#[toolkit_macros::api_dto(response)]
pub struct PricingSubmitReceipt {
    pub applied: bool,
    pub unit: PricingApprovalUnitDto,
    pub prices: Vec<PricingPriceDto>,
}
/// The `plan_revision` unit a submission recorded and the revision after the transaction:
/// pending under the unit, or published when quorum zero applied it at once.
#[toolkit_macros::api_dto(response)]
pub struct PricingPlanRevisionSubmitReceipt {
    pub applied: bool,
    pub unit: PricingApprovalUnitDto,
    pub revision: PricingPlanRevisionDto,
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
    /// Prices and entries the listed drafts touch, the plan revisions naming those entries, and
    /// subscriptions (unavailable until the Subscriptions integration).
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
