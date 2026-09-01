//! `SeaORM` entity for `bss.products_category` — the governed tree
//! (`design/02` §4.1, P-D-50, P-D-88).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_category")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "category_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub category_id: Uuid,
    /// `None` is a root. Root-name uniqueness is its own partial index
    /// (P-D-88 arm 1): the declared in-parent UNIQUE cannot hold over NULL.
    pub parent_id: Option<Uuid>,
    pub name: String,
    /// The Foundation's operand: NFKC, full casefold, trim + collapse —
    /// `domain::name::normalize`, computed application-side.
    pub name_normalized: String,
    /// `active` or `retired`; deletion is physical only through the retire
    /// guard (`inst-tx-retire-guard`).
    pub state: String,
    /// The live-value door's `If-Match` operand (P-D-50). Counts **acts**,
    /// not row writes.
    pub mutation_seq: i64,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
