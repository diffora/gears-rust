//! Read-only `SeaORM` entity for the `tenant_closure` system table.
//!
//! `#[secure(unrestricted)]` — this is a platform-managed hierarchy table,
//! not a tenant-scoped resource. Queried only to check descendant membership.

use modkit_db::secure::Scopable;
use sea_orm::entity::prelude::*;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "tenant_closure")]
#[secure(unrestricted)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub ancestor_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub descendant_id: Uuid,
    pub barrier: i16,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
