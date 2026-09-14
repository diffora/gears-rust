//! `SeaORM` entity for `bss.products_catalog_version_entry` — one manifest
//! reference into immutable `products_entity_version`
//! (`design/06-catalog-version.md` §4, P-D-60; frozen by the migration's
//! guard).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_catalog_version_entry")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "entity_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub catalog_version_id: i64,
    /// `product` or `sku`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_id: Uuid,
    /// The frozen version this manifest pins for the entity.
    pub published_version: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
