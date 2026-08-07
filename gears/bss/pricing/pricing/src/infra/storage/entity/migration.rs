//! `SeaORM` entity for `bss.pricing_migration` — one scheduled plan migration
//! (`design/11-lifecycle.md` §6, `m20260802_000043`).
//!
//! The primary key is **client-supplied** (`inst-ms-api`, M2): a caller mints
//! `migration_id` and the store's uniqueness is what makes a timed-out retry
//! return the original schedule rather than create a second one. There is no
//! surrogate key and no dedup table beside this one.
//!
//! Absence is meaningful on all four nullable columns, and in each case it is a
//! statement about §4's reachable states rather than "not filled in yet":
//! `started_at`/`exclusion_snapshot` are co-nullable and absent exactly while the
//! run has not been declared started (D-65), and the two terminal instants are
//! biconditional with their states. The schema carries all of that; see the
//! migration's module doc for why `started_at` alone is written as two
//! implications instead.

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use serde_json::Value as JsonValue;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "pricing_migration")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "migration_id",
    no_owner,
    no_type
)]
pub struct Model {
    /// Client-supplied (`inst-ms-api`); the idempotency key of M2 and the
    /// primary key at once.
    #[sea_orm(primary_key, auto_increment = false)]
    pub migration_id: Uuid,
    pub tenant_id: Uuid,
    /// The retiring side.
    pub source_plan_id: Uuid,
    /// The source plan's revision at schedule time.
    pub source_revision: i32,
    /// MUST be published when the schedule is created, and re-checked nowhere:
    /// a target that stops being published afterwards surfaces as a D-36
    /// execution-time exclusion rather than as a retroactive refusal.
    pub target_plan_id: Uuid,
    pub effective_at: DateTime<Utc>,
    /// The announcement instant D-49 measures the notice period from — the
    /// scheduling commit. Stored rather than derived from `created_at` so the
    /// rule stays auditable if the two ever diverge.
    pub announced_at: DateTime<Utc>,
    /// `all` or a subscription filter (§6).
    pub scope: JsonValue,
    /// One of `scheduled | in_progress | completed | cancelled`.
    pub state: String,
    /// The deltas **at schedule time** (§6). Frozen by the table's trigger: it
    /// is the evidence of what the operator confirmed against.
    pub delta_report: JsonValue,
    /// D-65's persist-and-replay set, computed once at the first `start` call.
    /// `None` exactly while the run has not started.
    pub exclusion_snapshot: Option<JsonValue>,
    /// The processed / excluded / failed sets on completion, or the partial
    /// (migrated / not-attempted) sets on an in-flight cancel (D-34).
    pub completion_record: Option<JsonValue>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
