//! `SeaORM` entity for `bss.pricing_charge_line` — stable logical charge identity.
//!
//! Currency and region are not axes of this row; they live on
//! [`super::market_price`]. Shared authorable content lives on
//! [`super::charge_line_version`].

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_charge_line")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "charge_line_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub charge_line_id: Uuid,
    pub plan_id: Uuid,
    pub phase: Uuid,
    pub price_overlay: String,
    pub price_eligibility: String,
    pub charge_kind: String,
    pub cohort: String,
    pub sku_id: Uuid,
    pub dimension_key: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
