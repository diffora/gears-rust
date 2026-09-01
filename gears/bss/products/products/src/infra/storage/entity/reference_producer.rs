//! `SeaORM` entity for `bss.products_reference_producer` — the registered
//! producer set (`design/07` §4, P-D-03, P-D-87).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_reference_producer")]
#[secure(tenant_col = "tenant_id", resource_col = "producer", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub producer: String,
    /// `registered` or `retired` — the predicate quantifies over the
    /// first only.
    pub state: String,
    pub registered_at: ChronoDateTimeUtc,
    /// The ceremony that admitted the registration, where one ran.
    pub ceremony_ref: Option<Uuid>,
    /// The reserved declaration field (`PRD` §15).
    pub declaration_payload: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
