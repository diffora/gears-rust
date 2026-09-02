//! `SeaORM` entity for `bss.products_correction_override` — the break-glass
//! correction's evidence rows (`design/07` §4, P-D-16).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_correction_override")]
#[secure(tenant_col = "tenant_id", resource_col = "sku_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// The row's own id. Evidence rows are never addressed by their subject:
    /// one SKU can carry many overrides, and each is its own fact.
    #[sea_orm(primary_key, auto_increment = false)]
    pub override_id: Uuid,
    /// The SKU the correction touched — the scope column too.
    pub sku_id: Uuid,
    /// The immutable field the ceremony admitted a write to.
    pub field: String,
    /// The ceremony's reason. **Mandatory**, and the `CHECK` refuses an
    /// empty one: an override with no stated reason is not evidence.
    pub reason: String,
    /// Which arm admitted the override — `producer_unavailable` (a) or
    /// `unresolvable_target` (b).
    pub admitting_arm: String,
    /// Arm (a)'s evidence: the per-producer unavailability snapshot, as a
    /// canonical rendering. `NULL` on arm (b), which the `CHECK` pins.
    pub unavailability_snapshot: Option<String>,
    /// Arm (b)'s evidence. `NULL` on arm (a), same `CHECK`.
    pub unresolvable_target: Option<String>,
    /// The `05-governance` ceremony this override rode. No FK — that slice's
    /// write path does not ship — and the audit row carries the same value,
    /// so the two are joinable from either side.
    pub ceremony_ref: Uuid,
    /// The instant the evidence landed. **The tripwire's whole operand**:
    /// the counter is a windowed count over this column, so there is no
    /// counter state to drift from the rows.
    pub recorded_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
