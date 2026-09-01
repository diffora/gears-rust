//! `SeaORM` entity for `bss.products_attribute_value` — the value plane
//! (`design/02` §4.1, P-D-88 arm 2).
//!
//! For Product and SKU rows this table holds the **current head state only**
//! — history lives in the frozen version rows. For **category** rows it IS
//! the live state, with no freeze-copy (H2's fix).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_attribute_value")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "entity_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// `product`, `sku` or `category`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub definition_id: Uuid,
    /// The three locale coordinates ship `NOT NULL` with `""` as the stated
    /// absence value (P-D-88 arm 2), so the key is total and the `global`
    /// coordinate is spelled `("", "", "")`. What `global` MEANS to the
    /// resolver is still `design/02` §6's.
    #[sea_orm(primary_key, auto_increment = false)]
    pub locale: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub region: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub brand: String,
    pub value: String,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
