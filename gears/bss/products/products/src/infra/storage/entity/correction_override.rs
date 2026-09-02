//! `SeaORM` entity for `bss.products_correction_override` — the break-glass
//! correction's evidence rows (`design/07` §4, P-D-16).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_correction_override")]
// The resource is the row's **own** id, not its subject. Two reasons, and
// the second is a fail-open: this entity's own field doc says evidence rows
// are never addressed by their subject, and the nearest analogue — the audit
// plane — keys on `audit_id`; and `correction_overrides_since` is a
// tenant-wide count, so with `sku_id` as the resource a SKU-addressed
// caller's scope (`sku x correct` compiles one) would render
// `sku_id IN (...)` and count 1 where the tripwire needs 6 — the escalation
// the table exists to raise, silently not firing.
#[secure(
    tenant_col = "tenant_id",
    resource_col = "override_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// The row's own id. Evidence rows are never addressed by their subject:
    /// one SKU can carry many overrides, and each is its own fact.
    #[sea_orm(primary_key, auto_increment = false)]
    pub override_id: Uuid,
    /// The SKU the correction touched. **Not** the scope column — see the
    /// `#[secure]` note above — and tenant containment over it is the
    /// door's precondition, not the scope layer's: the `FK` resolves
    /// `products_sku (sku_id)` globally.
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
    /// write path does not ship. The `DoD`'s join — the audit row carrying
    /// **the same value** — is owed on both sides: `products_audit_log`'s
    /// roster carries no `ceremony_ref` column either.
    pub ceremony_ref: Uuid,
    /// The instant the evidence landed — the tripwire's **window** operand.
    /// The count itself is over this **table**, scoped by `tenant_id` and
    /// windowed on this column (the `DoD`: *"a windowed count over this
    /// table"*), so there is no counter state to drift from the rows. A
    /// filter on this column alone would count every tenant's overrides into
    /// one window.
    pub recorded_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
