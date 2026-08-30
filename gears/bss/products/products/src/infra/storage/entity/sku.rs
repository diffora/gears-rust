//! `SeaORM` entity for `bss.products_sku` — a SKU's identity, its parent link,
//! its lifecycle and its two version counters.
//!
//! The capability columns a SKU carries — `type`, `sellable`, `plan_tier`, the
//! accounting code refs, the metering unit — are **carried** on this row rather
//! than in a side table, because a side table keyed the same way as its parent
//! is a join nobody needs and a second place for one row's facts to live. Their
//! write rules belong to the features that own them: the split is by validator,
//! not by table. They arrive with those features.
//!
//! @cpt-cf-bss-products-fr-identifier-contract
//! @cpt-cf-bss-products-fr-define-sku
//! @cpt-dod:cpt-cf-bss-products-dod-entity-tables:p1

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_sku")]
#[secure(tenant_col = "tenant_id", resource_col = "sku_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub sku_id: Uuid,
    pub tenant_id: Uuid,
    /// The parent Product. A bucket-i column: re-parenting changes whose SKU it
    /// is, not how it is described, so it is refused after first publish.
    pub product_id: Uuid,
    /// Tenant-unique among non-discarded rows, reserved by the insert itself.
    pub sku_code: String,
    /// `draft | published | deprecated | retired | discarded`, constrained by
    /// `chk_products_sku_lifecycle_state`.
    pub lifecycle_state: String,
    /// Moves on every admitted write.
    pub internal_revision: i64,
    /// Moves only on publish.
    pub published_version: i64,
    /// The unresolved-composition flag (`design/01-foundation.md` §4.2,
    /// **P-D-35**). `NOT NULL DEFAULT false`, and system-owned: the migration's
    /// guard admits a change to it **only** in the same statement as a
    /// `published_version` bump, so no operator save can move it. `bool`
    /// rather than a nullable third reading, because the create flow writes it
    /// nowhere and the unraised state is the default.
    pub composition_pending: bool,
    /// A flat value set, contained in the parent's. `NOT NULL`, default empty,
    /// where **empty means unrestricted**.
    pub region_scope: String,
    /// The same shape and the same reading as `region_scope`.
    pub brand_scope: String,
    /// The pseudonymous ref of whoever created the row.
    pub created_by: String,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
