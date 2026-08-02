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
    pub billing_cycle: Option<String>,
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
