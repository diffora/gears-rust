//! `SeaORM` entity for `bss.pricing_composite_meter` — one derived-meter
//! definition of **one plan revision** (`design/10-advanced-primitives.md` §6),
//! keyed `(composite_id, plan_revision)`.
//!
//! [`plan_phase`](super::plan_phase)'s arrangement exactly, and for D-106's
//! reason rather than by imitation: `plan_revision` says *which copy* of the
//! definition this row is, and `composite_id` is the composite itself, stable
//! for the life of the plan. Opening a draft revision copies the rows under its
//! own number without re-minting an id, so a formula edit on the draft leaves the
//! published revision's rows byte-identical.
//!
//! There is no `lifecycle_state` here for the same reason there is none on a
//! phase row: the definition is frozen when **its** revision publishes, so the
//! parent `pricing_plan` row is the referent and the table's append-only
//! triggers read it.

use sea_orm::entity::prelude::*;
use uuid::Uuid;

use toolkit_db_macros::Scopable;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_composite_meter")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "composite_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub composite_id: Uuid,
    /// The revision this copy belongs to — the second half of the key, and the
    /// row it is frozen with.
    #[sea_orm(primary_key, auto_increment = false)]
    pub plan_revision: i64,
    /// Copied from the parent revision by the repository, never taken from a
    /// request: the foreign key covers `(plan_id, plan_revision)` alone, so
    /// nothing in the schema stops a child carrying a foreign tenant.
    pub tenant_id: Uuid,
    pub plan_id: Uuid,
    /// The registry-declared `meteringUnit` this composite rates as
    /// (`inst-cm-output`). Declaring it is the registry's act and not this
    /// gear's (D-32) — what is persisted here is the id and the binding.
    pub output_unit: String,
    /// The constituent `meteringUnit` ids, as a JSON array of strings.
    ///
    /// `≥ 2` is a **publish rule** and not a column constraint
    /// (`COMPOSITE_TOO_FEW_CONSTITUENTS`); whether each is *published* is not
    /// checked at all, because this gear has no registry client — see the
    /// migration's module doc.
    pub constituent_units: Json,
    /// The formula **as data** (A4): operands plus operator/weights, a
    /// declarative schema and never executable code. The catalog persists and
    /// freezes it; Rating evaluates it (`inst-cm-frozen`).
    pub formula: Json,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
