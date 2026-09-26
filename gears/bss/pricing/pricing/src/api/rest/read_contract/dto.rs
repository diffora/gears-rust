//! The consumer read contract's wire shapes (D-419…D-422): `snake_case`, money and quantities as
//! exact decimal text, dates as `YYYY-MM-DD`. The consumer goldens freeze them.
use uuid::Uuid;

/// `GET /resolve`: one plan revision resolved on one date with the caller's pins (D-419). It
/// carries no totals and no promotion (D-409, D-415).
#[toolkit_macros::api_dto(response)]
pub struct PricingResolveDto {
    pub plan_revision_id: Uuid,
    pub plan_id: Uuid,
    pub rev_no: i32,
    /// `published` or `superseded`: no other revision resolves.
    pub state: String,
    /// The revision's book.
    pub book_id: Uuid,
    /// The book's currency.
    pub currency: String,
    /// The currency's scale: the number of minor digits money carries.
    pub currency_minor_digits: u32,
    /// The tenant's `default_rounding`.
    pub rounding_policy: String,
    /// The date resolved, `YYYY-MM-DD`.
    pub date: String,
    /// Every item of the revision, or the one `item_id` names.
    pub items: Vec<PricingResolveItemDto>,
}
/// One item of the revision on the date: its stored fields, its SKU version and resolved invoice
/// inputs (D-421), and its chain matrix (D-420).
#[toolkit_macros::api_dto(response)]
pub struct PricingResolveItemDto {
    pub item_id: Uuid,
    pub sku_id: Uuid,
    /// `paid`, `optional` or `included`.
    pub treatment: String,
    /// Exact decimal text.
    pub included_qty: Option<String>,
    pub qty_min: Option<i32>,
    /// Null for an included item without an entry, which has no chains.
    pub price_book_entry_id: Option<Uuid>,
    /// `recurring`, `usage` or `one_time`; null without an entry.
    pub charge_kind: Option<String>,
    pub period: Option<String>,
    /// The SKU version in force on the date; null when Products has no version on that date or
    /// does not know the SKU.
    pub sku_version: Option<PricingResolveSkuVersionDto>,
    /// The entry's override, else the SKU version's template, else the tenant template for the
    /// charge kind.
    pub invoice_line_template: PricingResolveInputDto,
    /// The SKU version's, else the tenant `default_gl`.
    pub gl_code: PricingResolveInputDto,
    /// The SKU version's, else the tenant `default_tax_category`.
    pub tax_category: PricingResolveInputDto,
    /// The SKU version's, else the tenant `default_timing` (PRD AC #13).
    pub billing_timing: PricingResolveInputDto,
    /// The meter of that SKU version; both null without a version.
    pub meter: PricingResolveMeterDto,
    /// The default chain, then each value registered today in the registry's order, then any
    /// other value a pin names.
    pub chains: Vec<PricingResolveChainDto>,
}
/// The SKU version an item reads on the date.
#[toolkit_macros::api_dto(response)]
pub struct PricingResolveSkuVersionDto {
    pub published_version: i64,
    /// `YYYY-MM-DD`.
    pub effective_from: String,
}
/// A resolved invoice input and where it came from; both null when no source holds it.
#[toolkit_macros::api_dto(response)]
pub struct PricingResolveInputDto {
    pub value: Option<String>,
    /// `entry`, `sku` or `tenant`.
    pub source: Option<String>,
}
/// The meter of the SKU version in force on the date.
#[toolkit_macros::api_dto(response)]
pub struct PricingResolveMeterDto {
    pub usage_type_ref: Option<String>,
    pub unit: Option<String>,
}
/// One row of an item's matrix: the default chain (`dim_value` null) or one value.
#[toolkit_macros::api_dto(response)]
pub struct PricingResolveChainDto {
    pub dim_value: Option<String>,
    /// Neither the value's own chain nor the default binds on the date: `binding` is null. Never
    /// a refusal and never an invented price.
    pub uncovered: bool,
    pub binding: Option<PricingResolveBindingDto>,
}
/// The price bound for the period, as stored.
#[toolkit_macros::api_dto(response)]
pub struct PricingResolveBindingDto {
    pub price_id: Uuid,
    /// The chain the bound price belongs to: the value, or null for the default chain.
    pub dim_used: Option<String>,
    /// The pin the renewal walk started from; null for a signup.
    pub pinned_from: Option<Uuid>,
    /// `flat`, `per_unit`, `graduated`, `volume` or `package`.
    pub model: String,
    /// The price's money as stored, amounts as exact decimal text.
    pub price: serde_json::Value,
    /// Exact decimal text.
    pub min_fee: Option<String>,
    /// `all` or `new`.
    pub eligibility: String,
    pub effective_from: String,
    pub effective_to: Option<String>,
    pub temporary_until: Option<String>,
    /// The price is kept for pinned subscriptions (the predecessor of a `new` price).
    pub keep_for_bound: bool,
}
#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PricingResolveQuery {
    pub plan_revision_id: Option<String>,
    pub date: Option<String>,
    pub item_id: Option<String>,
    pub pins: Option<String>,
}
