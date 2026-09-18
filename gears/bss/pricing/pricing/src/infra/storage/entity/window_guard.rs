//! `SeaORM` entity for `bss.pricing_window_guard` — lock identity for one plan
//! (D-374).
//!
//! Keyed `(tenant_id, plan_id)`: one row per plan, not per revision. `serial`
//! is the value a scoped `UPDATE … serial = serial + 1` advances to hold the
//! write lock until transaction end.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_window_guard")]
#[secure(tenant_col = "tenant_id", resource_col = "plan_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub plan_id: Uuid,
    pub serial: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
