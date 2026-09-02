//! `SeaORM` entity for `bss.products_deferred_retirement` — the leave-and-list
//! snapshot of a Product cascade that could not finish
//! (`design/04-lifecycle.md` §4).
//!
//! **Never deleted.** Resolved rows flip `resolved_at` and freeze; the
//! migration's guard is the physical floor under that continuity.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_deferred_retirement")]
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
    /// The parent's `ScheduledTransition` id.
    #[sea_orm(primary_key, auto_increment = false)]
    pub cascade_ref: Uuid,
    /// Leave-and-list snapshot (children + reasons), JSON text.
    pub children_snapshot: String,
    pub created_by: Uuid,
    pub resolved_at: Option<ChronoDateTimeUtc>,
    /// `children_cleared` or `cascade_cancelled` when resolved; NULL while live.
    pub resolution: Option<String>,
    pub created_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
