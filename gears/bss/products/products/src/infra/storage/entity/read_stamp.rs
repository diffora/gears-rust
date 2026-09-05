//! `SeaORM` entity for `bss.products_read_stamp` — the `StalenessStamp`'s
//! per-tenant row (P-D-70 arm 6, P-D-07).
//!
//! **One row per tenant, no guard.** The projector overwrites it on every
//! apply, so the rebuildable-family exemption of `design/08` §4 applies here
//! as it does to `products_read_entity`.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_read_stamp")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "tenant_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// The last catalog version projected. **`NULL` for a tenant that has
    /// published none** — the anchorless arm, which a sentinel would make
    /// indistinguishable from a dropped stamp.
    pub catalog_version_id: Option<i64>,
    /// The last apply's instant. Advances on **every** apply, version or
    /// none (P-D-70 arm 3), so the sole freshness signal always has a
    /// writer.
    pub projected_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
