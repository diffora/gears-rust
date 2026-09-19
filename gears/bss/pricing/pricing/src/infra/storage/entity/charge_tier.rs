//! `SeaORM` entity for `bss.pricing_charge_tier` — shared tier geometry of a
//! line version. Market rates live on [`super::price_tier_band`].

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_charge_tier")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "line_version_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub line_version_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub band_ordinal: i32,
    pub from_qty: i64,
    pub to_qty: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
