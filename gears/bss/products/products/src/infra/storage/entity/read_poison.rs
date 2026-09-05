//! `products_read_poison` — a parked poison message (P-D-126 rows 9 and 12).
//!
//! Rebuildable projection state (`design/08` §4): no append-only guard.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_read_poison")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "tenant_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub inbox_id: i64,
    pub tenant_id: Uuid,
    pub payload_type: String,
    pub attempts: i32,
    pub last_error: String,
    pub parked_at: ChronoDateTimeUtc,
    pub released_at: Option<ChronoDateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
