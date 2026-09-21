//! `SeaORM` entity for `bss.pricing_price_tier_band` — one market's ladder: each
//! band's bounds beside the rate that prices it.

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
    /// The structure version the price names. Carried so the band can answer
    /// for its line's `model_kind` without a join through `pricing_price`.
    pub line_version_id: Uuid,
    /// Inclusive lower bound. A band's identity within its price.
    pub from_qty: i64,
    /// Exclusive upper bound; `NULL` on the open top band.
    pub to_qty: Option<i64>,
    /// Rate in 10^-9 minor units (D-311). Column name is the existing typed
    /// `unit_price_nano` rather than a second scale.
    pub unit_price_nano: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
