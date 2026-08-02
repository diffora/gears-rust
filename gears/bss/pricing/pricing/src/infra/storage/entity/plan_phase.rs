//! `SeaORM` entity for `bss.pricing_plan_phase` — one phase of **one plan
//! revision** (`design/02-plan-definition.md` §6), keyed
//! `(phase_id, plan_revision)`.
//!
//! The pair is not a composite id: `plan_revision` says *which copy* of the
//! phase this row is, and `phase_id` is the phase itself, stable for the life
//! of the plan. A new revision copies these rows under its own number without
//! re-minting an id (D-83), because the `phase` axis of the canonical scope key
//! holds a bare `phase_id` (D-19) and same-key supersession compares it (D-56).
//!
//! There is no `lifecycle_state` here. A phase row is frozen when **its**
//! revision publishes, so the parent `pricing_plan` row is the referent and the
//! table's append-only triggers read it — the same arrangement
//! `pricing_price_tier_band` has with its price row.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_plan_phase")]
#[secure(tenant_col = "tenant_id", resource_col = "phase_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub phase_id: Uuid,
    /// The revision this copy belongs to — the second half of the key, and the
    /// row it is frozen with.
    #[sea_orm(primary_key, auto_increment = false)]
    pub plan_revision: i64,
    /// Copied from the parent revision by the repository, never taken from a
    /// request: the foreign key covers `(plan_id, plan_revision)` alone, so
    /// nothing in the schema stops a child carrying a foreign tenant.
    pub tenant_id: Uuid,
    pub plan_id: Uuid,
    /// `trial` | `intro` | `evergreen`, `CHECK`-constrained to those three; the
    /// value set is `domain::plan_shape::PhaseKind`. It is **not** terminality
    /// (C-4) — see [`Model::converts_to_phase_id`].
    pub kind: String,
    /// The phase's position in the chain. The lowest ordinal is the entry
    /// phase (`inst-ph-graph`).
    pub ordinal: i32,
    /// Where this phase converts to. NULL **is** terminality, which is why the
    /// partial `UNIQUE` that admits one terminal phase per revision is written
    /// over this column's nullity rather than over [`Model::kind`].
    pub converts_to_phase_id: Option<Uuid>,
    /// How long the phase lasts. Required `> 0` on a non-terminal phase and
    /// forbidden on the terminal one — a pipeline rule
    /// (`inst-ph-duration`, `PHASE_DURATION_INVALID`), never a CHECK here.
    pub phase_duration_days: Option<i32>,
    /// The PRD-named projection of [`Model::phase_duration_days`] on a `trial`
    /// phase (`inst-ph-trial`). `chk_pricing_plan_phase_display_trial_days`
    /// guards drift between the two persisted columns.
    pub display_trial_days: Option<i32>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
