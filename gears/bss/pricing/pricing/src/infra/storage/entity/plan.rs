//! `SeaORM` entity for `bss.pricing_plan` — one **revision** of a plan
//! (`design/01-foundation.md` §3.7, D-56), keyed `(plan_id, revision)`.
//!
//! At most one revision is the plan's **current** one (`published` or
//! `retired`, D-128) and at most one is an open `draft`; both are partial
//! `UNIQUE` indexes on the table, and a discarded draft's `abandoned` tombstone
//! is outside both, so a plan may hold any number of them (D-145). A published
//! revision's content is frozen — the only permitted UPDATE is the sanctioned
//! `lifecycle_state` flip — so `row_version` (the `ETag`) is frozen with it:
//! content that cannot change needs no new entity tag.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_plan")]
#[secure(tenant_col = "tenant_id", resource_col = "plan_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub plan_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub revision: i64,
    pub tenant_id: Uuid,
    pub sku_id: Option<Uuid>,
    pub plan_tier: Option<String>,
    /// `one_time` | `recurring` | `usage` | `hybrid`, `CHECK`-constrained to
    /// those four; the value set is `domain::plan_shape::BillingCycle`.
    pub billing_cycle: Option<String>,
    /// `monthly` | `quarterly` | `semiannual` | `annual` | `custom_every_n`.
    ///
    /// The custom token is the bare discriminator: its interval rides the two
    /// columns below, and `chk_pricing_plan_custom_interval_pairing` binds the
    /// three together so the only pairings the table admits are the ones
    /// `domain::plan_shape::Frequency` can represent.
    pub frequency: Option<String>,
    /// The interval count of a `custom_every_n` frequency, `> 0`.
    pub custom_interval_n: Option<i32>,
    /// `days` | `months` — what [`Model::custom_interval_n`] counts.
    pub custom_interval_unit: Option<String>,
    /// Whether the tier deliberately diverges from the parent SKU's under an
    /// explicit audited override (§6, P3). `NOT NULL DEFAULT false`, so a plan
    /// that never mentions it is not overriding anything.
    pub plan_tier_override: bool,
    /// Minimum purchasable quantity (one-time plans).
    pub purchase_min_qty: Option<i64>,
    /// Maximum purchasable quantity (one-time plans), `>=`
    /// [`Model::purchase_min_qty`] where both are set.
    pub purchase_max_qty: Option<i64>,
    /// The Billing invoice-layout hint (D-96). NULL or empty means no grouping.
    pub invoice_grouping_key: Option<String>,
    /// `draft` | `abandoned` | `published` | `superseded` | `retired`.
    /// `CHECK`-constrained to those five; the legal edges between them are
    /// `domain::lifecycle::LifecycleState`.
    pub lifecycle_state: String,
    pub available_from: Option<DateTime<Utc>>,
    pub available_to: Option<DateTime<Utc>>,
    /// Pseudonymous principal id of the authoring actor. The Slice-12 history
    /// surface reads it under `plan x read`, so actor identity never requires
    /// the Auditor-only `pricing_audit_log`.
    pub created_by: Uuid,
    pub created_at_utc: DateTime<Utc>,
    /// The `ETag` / optimistic-concurrency row version.
    pub row_version: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
