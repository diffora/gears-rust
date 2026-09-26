//! Tenant-scoped storage for `pricing_price`.
use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_price")]
#[secure(tenant_col = "tenant_id", resource_col = "id", no_owner, no_type)]
#[allow(
    clippy::struct_field_names,
    reason = "SeaORM requires Model; the schema names its pricing discriminator model"
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub price_book_entry_id: Uuid,
    pub version_no: i32,
    pub dim_value: Option<String>,
    pub model: String,
    pub price_json: Json,
    /// Canonical decimal text ("30.00"): exact on both dialects. sea-orm decodes a `SQLite`
    /// `Decimal` through `f64`, which drops the scale and every digit past `f64`'s precision.
    pub min_fee: Option<String>,
    pub eligibility: String,
    pub effective_from: TimeDate,
    pub effective_to: Option<TimeDate>,
    pub keep_for_bound: bool,
    pub closed_explicitly: bool,
    pub temporary_until: Option<TimeDate>,
    pub paired_price_id: Option<Uuid>,
    pub return_of_price_id: Option<Uuid>,
    pub state: String,
    pub pending_unit_id: Option<Uuid>,
    pub approved_by_unit_id: Option<Uuid>,
    pub note: Option<String>,
    pub created_by: Uuid,
    pub approved_at: Option<TimeDateTimeWithTimeZone>,
    pub version: i64,
    pub created_at: TimeDateTimeWithTimeZone,
    pub updated_at: TimeDateTimeWithTimeZone,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
