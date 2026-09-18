//! `SeaORM` entity for `bss.pricing_window_baseline` — one captured live window
//! reference on a plan revision (D-374).
//!
//! Keyed `(tenant_id, plan_id, plan_revision, window_id)`. Frozen with the
//! owner revision; abandoned revisions keep these rows.

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_window_baseline")]
#[secure(tenant_col = "tenant_id", resource_col = "plan_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub plan_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub plan_revision: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub window_id: Uuid,
    pub price_id: Uuid,
    pub mutation_seq: i64,
    pub effective_from: OffsetDateTime,
    pub effective_to: Option<OffsetDateTime>,
    pub cancelled: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
