//! `SeaORM` entity for `bss.products_catalog_version` — the version row
//! (`design/06-catalog-version.md` §4; storage shape only, the guards live
//! in the migration). `freeze_state` is the only column the guard's UPDATE
//! arm admits; everything else is frozen at insert.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_catalog_version")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "catalog_version_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// Gapless per tenant by the allocator's contract (`m20260901_000008`).
    #[sea_orm(primary_key, auto_increment = false)]
    pub catalog_version_id: i64,
    /// Hex `SHA-256` over the canonical manifest rendering.
    pub checksum: String,
    /// The digest rule the checksum was computed under (P-D-73 arm 1).
    pub digest_version: i32,
    /// The commit instant.
    pub published_at: ChronoDateTimeUtc,
    /// Derived cache of the participant set; the authoritative copy is the
    /// capture store's, inside the checksum (P-D-67).
    pub participant_set_snapshot: String,
    /// `open`, `complete` or `complete(forced)` — the ledger's derived
    /// cache, refreshed in-transaction by ack, release and force-completion
    /// (P-D-73 arm 2).
    pub freeze_state: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
