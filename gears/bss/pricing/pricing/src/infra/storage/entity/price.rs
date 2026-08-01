//! `SeaORM` entity for `bss.pricing_price` — the price rows and, in the same
//! table, the price **history** (`design/01-foundation.md` §3.7).
//!
//! The eight canonical scope-key columns come first and in normative order
//! (§4.1). `cohort` is a `String` holding the domain token (`none`, or the
//! cutover instant) rather than an `Option<DateTime<Utc>>`, because it is a
//! column of a partial `UNIQUE` index and distinct `NULL`s do not collide.
//!
//! Money is integer minor units. `amount_minor` is nullable because its
//! placement is per-kind (Slice 3): required on `flat` / `per_unit`, and NULL on
//! the band and package kinds whose money lives elsewhere — so no row ever
//! carries two competing prices.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_price")]
#[secure(tenant_col = "tenant_id", resource_col = "price_id", no_owner, no_type)]
#[allow(
    clippy::struct_field_names,
    reason = "`model_kind` is the normative column name (design/03-price-structure.md 6); \
              the struct is called `Model` only because SeaORM's DeriveEntityModel requires it"
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub price_id: Uuid,
    pub tenant_id: Uuid,
    // --- the canonical scope key, normative order (design 4.1) ---
    pub plan_id: Uuid,
    pub currency: String,
    pub region: String,
    /// Always `base` on a row this gear authors; partner / orgTier / brand
    /// overlays are separate overlay documents, not a value of this axis.
    pub price_overlay: String,
    pub phase: Uuid,
    pub price_eligibility: String,
    pub charge_kind: String,
    /// `none`, or the UTC cutover instant that created the grandfathering
    /// generation, rendered by `domain::scope_key::Cohort`.
    pub cohort: String,
    // --- price / model ---
    pub amount_minor: Option<i64>,
    pub model_kind: Option<String>,
    pub tax_inclusive: bool,
    pub billing_timing: Option<String>,
    // --- evaluation policy (usage rows) ---
    pub meter: Option<String>,
    /// Dimension discriminator. `NOT NULL DEFAULT ''` — the empty string is the
    /// empty-tuple sentinel, so the Slice-2 injectivity index collides
    /// undimensioned rows instead of treating them as distinct `NULL`s.
    pub dimension_key: String,
    pub billing_granularity: Option<String>,
    pub aggregation_function: Option<String>,
    pub aggregation_granularity: Option<String>,
    pub tier_aggregation_window: Option<String>,
    pub tier_qualification_window: Option<String>,
    pub max_hold_granules: Option<i32>,
    /// The named rounding-policy id resolved at publish (row-level, else the
    /// tenant default, else `ROUNDING_POLICY_UNRESOLVED`).
    pub rounding_policy_ref: Option<String>,
    /// Grandfathering horizon. Monotonically tightenable only; the table
    /// trigger rejects loosening it.
    pub grandfather_until: Option<DateTime<Utc>>,
    /// Set on a successor row; it is what gives the supersession unit guard its
    /// comparison referent (D-127).
    pub supersedes_price_id: Option<Uuid>,
    pub lifecycle_state: String,
    /// Pseudonymous authoring principal (the Slice-12 history-export actor).
    pub created_by: Uuid,
    pub created_at_utc: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
