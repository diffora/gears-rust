//! `SeaORM` entity for `bss.products_approval_decision` — one principal's
//! verdict (`design/05` §4; the UNIQUE is C2's physical floor).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_approval_decision")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "approval_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub approval_id: Uuid,
    /// The principal as an `actor_ref` — pseudonymous, never a raw
    /// identifier: these rows are append-only, so one raw identifier written
    /// is unreachable by erasure forever.
    #[sea_orm(primary_key, auto_increment = false)]
    pub approver_principal: Uuid,
    pub verdict: String,
    pub reason: Option<String>,
    pub override_acknowledgments: Option<String>,
    pub decided_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
