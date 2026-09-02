//! `SeaORM` entity for `bss.products_read_entity` — the browse projection's
//! serving row (`design/08` §3.1 `inst-ps-shape`, P-D-39).
//!
//! **No guard, by design.** Unlike every other entity in this module the
//! table behind this one carries no append-only or whitelist trigger: §4
//! calls the family *"rebuildable state, not records"* and records the
//! exemption as the point. See the migration's own doc before adding one.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_read_entity")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "entity_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// `product` or `sku` — part of the key, because the two id spaces are
    /// distinct and a row is addressed by both.
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_id: Uuid,
    /// The `productCode` or `skuCode`, nullable because a Product may carry
    /// none.
    pub entity_code: Option<String>,
    pub name: String,
    /// All five states, including the two no surface serves — the projector
    /// records what happened and `VisibilityFilter` decides what is seen.
    pub lifecycle_state: String,
    /// `inst-ps-shape`'s three flags.
    pub deprecated: bool,
    pub composition_pending: bool,
    /// Nullable: only a SKU carries it.
    pub sellable: Option<bool>,
    /// C6's head-read fields, carried so a browse response can render a
    /// deprecated row's successor without touching the head.
    pub deprecation_provenance: Option<String>,
    pub replaced_by_sku_id: Option<Uuid>,
    /// The query-build scope operands. **Empty means unrestricted**
    /// (P-D-39), so a predicate matches a row whose set is empty *or*
    /// contains the caller's claim.
    pub region_scope: String,
    pub brand_scope: String,
    /// The display fields `inst-ps-shape` names.
    pub sku_type: Option<String>,
    pub plan_tier_label: Option<String>,
    pub metering_unit: Option<String>,
    /// The resolved per-locale rendering, canonical.
    pub display_attributes: Option<String>,
    /// The assigned categories' full paths, canonical.
    pub category_paths: Option<String>,
    pub published_version: i64,
    /// This row's own last apply. The response-level floor is the per-tenant
    /// `StalenessStamp`, which has no table yet — see `features/read-models.md`
    /// §7.
    pub projected_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
