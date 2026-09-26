//! Tenant-scoped storage for `pricing_price_book_entry`.
use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_price_book_entry")]
#[secure(tenant_col = "tenant_id", resource_col = "id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
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
    pub created_at: TimeDateTimeWithTimeZone,
    pub updated_at: TimeDateTimeWithTimeZone,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
