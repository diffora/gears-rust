//! `products_read_freeze_status` — the freeze-status dashboard, polled from 06's ledger.
//!
//! Rebuildable projection state (`design/08` §4): no append-only guard.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_read_freeze_status")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "tenant_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub catalog_version_id: i64,
    pub freeze_state: String,
    pub pending: i32,
    pub acked: i32,
    pub released: i32,
    pub forced: i32,
    pub published_at: ChronoDateTimeUtc,
    pub polled_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
