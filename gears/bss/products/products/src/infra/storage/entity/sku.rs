//! Tenant-scoped storage model for `products_sku`.
use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_sku")]
#[secure(tenant_col = "tenant_id", resource_col = "id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub code: String,
    pub name: String,
    #[sea_orm(column_name = "type")]
    pub r#type: String,
    pub category_id: Uuid,
    pub description: String,
    pub sellable: bool,
    pub lifecycle: String,
    pub fence_prior_lifecycle: Option<String>,
    pub fenced_at: Option<TimeDateTimeWithTimeZone>,
    pub fence_op_id: Option<Uuid>,
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
    pub created_at: TimeDateTimeWithTimeZone,
    pub updated_at: TimeDateTimeWithTimeZone,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
