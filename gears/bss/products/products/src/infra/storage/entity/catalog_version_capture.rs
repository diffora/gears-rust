//! `SeaORM` entity for `bss.products_catalog_version_capture` — one stored
//! canonical copy of a live set (`design/06-catalog-version.md` §4, H3:
//! live content is copied, never referenced; frozen by the migration's
//! guard). The admitted `capture_kind` set is the snapshot builder's to
//! enforce (P-D-74, P-D-83).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_catalog_version_capture")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "capture_kind",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub catalog_version_id: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub capture_kind: String,
    /// The canonical rendering of the captured set — the bytes inside the
    /// manifest checksum.
    pub content: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
