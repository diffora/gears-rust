//! `SeaORM` entity for `bss.products_metadata` — the ungoverned map, outside
//! frozen version content (`design/02` §4.1, P-D-06).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_metadata")]
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
    pub entity_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub value: String,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
