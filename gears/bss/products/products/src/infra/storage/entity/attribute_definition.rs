//! `SeaORM` entity for `bss.products_attribute_definition` — the governed
//! definition roster (`design/02` §4.1, P-D-47).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_attribute_definition")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "definition_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub definition_id: Uuid,
    /// Unique per tenant.
    pub key: String,
    /// **No roster CHECK**: no document enumerates the admitted types, so the
    /// DDL pins non-emptiness only and the set stays the door's once decided
    /// (P-D-74's shape, registered in `design/02` §6).
    pub value_type: String,
    pub localized: bool,
    pub region_scope: String,
    pub brand_scope: String,
    /// `active`, `deprecated` or `removed` — the last reachable only as a
    /// flip, never a DELETE (P-D-47), which a trigger enforces.
    pub state: String,
    /// The well-known marker; a seeded definition is deprecatable but not
    /// removable.
    pub seeded_by: Option<String>,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
