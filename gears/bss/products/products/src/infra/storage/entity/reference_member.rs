//! `SeaORM` entity for `bss.products_reference_member` — one SKU of one
//! producer's posted set (`design/07` §4).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_reference_member")]
#[secure(tenant_col = "tenant_id", resource_col = "producer", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub producer: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub sku_id: Uuid,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
