//! `products_read_inbox` — the projector's inbox, one row per consumed event in commit order.
//!
//! Rebuildable projection state (`design/08` §4): no append-only guard.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_read_inbox")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "tenant_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key)]
    pub inbox_id: i64,
    pub tenant_id: Uuid,
    pub partition: i32,
    pub aggregate_id: Uuid,
    pub payload_type: String,
    pub payload: String,
    pub actor_ref: Uuid,
    pub created_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
