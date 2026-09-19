//! `SeaORM` entity for `bss.pricing_price` — one monetary version of a market
//! price, bound to an immutable line-version reference.
//!
//! Shared authorable structure lives on [`super::charge_line_version`]. Identity
//! of the logical line lives on [`super::charge_line`]. Currency/region live on
//! [`super::market_price`]. `plan_id` is a derived, FK-checked copy of the
//! line's plan so list/history surfaces can filter without a join.

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_price")]
#[secure(tenant_col = "tenant_id", resource_col = "price_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub price_id: Uuid,
    pub tenant_id: Uuid,
    pub market_price_id: Uuid,
    pub line_version_id: Uuid,
    pub charge_line_id: Uuid,
    /// Derived from [`super::charge_line::Model::plan_id`]; compound FK-checked.
    pub plan_id: Uuid,
    pub plan_revision: i64,
    pub amount_minor: Option<i64>,
    pub unit_rate_nano: Option<i64>,
    pub package_price_minor: Option<i64>,
    pub reserved_rate_nano: Option<i64>,
    pub tax_inclusive: bool,
    pub tax_category_ref: Option<String>,
    pub resolved_tax_category: Option<String>,
    pub rounding_policy_ref: Option<String>,
    pub resolved_rounding_policy: Option<String>,
    pub grandfather_until: Option<OffsetDateTime>,
    pub supersedes_price_id: Option<Uuid>,
    pub lifecycle_state: String,
    pub created_by: Uuid,
    pub created_at_utc: OffsetDateTime,
    pub row_version: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
