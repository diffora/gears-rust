//! `SeaORM` entity for `bss.products_product_category` — the assignment
//! table, the **single source of truth** for category membership
//! (`design/02` §4.1; 01's head tables carry no inline category columns).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_product_category")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "product_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub product_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub category_id: Uuid,
    /// `primary` or `secondary`. At-most-one-primary is a partial index,
    /// never a convention; the *required* half is
    /// `inst-tx-primary-at-publish`'s validator.
    #[sea_orm(primary_key, auto_increment = false)]
    pub role: String,
    pub assigned_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
