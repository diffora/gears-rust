//! `SeaORM` entities for `bss.products_reference_watermark` and
//! `bss.products_reference_member` — one producer's posted watermark and
//! the complete SKU set it covers (`design/07` §4, P-D-71).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

/// The watermark head: one row per `(tenant, producer)`.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_reference_watermark")]
#[secure(tenant_col = "tenant_id", resource_col = "producer", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub producer: String,
    /// The instant the producer's set is complete **as of** — the
    /// freshness verdict's operand.
    pub watermark_at: ChronoDateTimeUtc,
    /// When the post arrived, which is not the same instant.
    pub posted_at: ChronoDateTimeUtc,
    /// The hex digest of the posted set — the equal-`watermark_at`
    /// comparison's operand (**P-D-71**), which is what tells an idempotent
    /// replay from a `WATERMARK_CONFLICT`.
    pub set_hash: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
