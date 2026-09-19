//! `SeaORM` entity for `bss.pricing_price_tier_band` — market rates of a price
//! against shared [`super::charge_tier`] geometry.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_price_tier_band")]
#[secure(tenant_col = "tenant_id", resource_col = "price_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub band_id: Uuid,
    pub tenant_id: Uuid,
    pub price_id: Uuid,
    pub line_version_id: Uuid,
    pub band_ordinal: i32,
    /// Rate in 10^-9 minor units (D-311). Column name is the existing typed
    /// `unit_price_nano` rather than a second scale.
    pub unit_price_nano: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
