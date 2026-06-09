//! `am_leases` `SeaORM` entity — distributed-lease primitive backing
//! the hierarchy-integrity coordinator.
//!
//! The `key` PK lets multiple coordination domains coexist if a
//! second consumer ever needs one (today the only key is
//! `"hierarchy_integrity"`).
//!
//! Schema (per dialect):
//!
//! * `key`          `TEXT`         PRIMARY KEY            — coordination domain.
//! * `locked_by`    `UUID` / `TEXT` NULL                  — current holder; `NULL` ≡ free.
//! * `locked_until` `TIMESTAMPTZ` NOT NULL                — DB-clock expiry; epoch when free.
//! * `attempts`     `INTEGER`     NOT NULL DEFAULT `0`    — forensic takeover counter.
//!
//! All comparisons happen on the DB clock via dialect-specific
//! `Expr::cust` SQL (see `infra::lease::manager` for the acquire /
//! steal arithmetic). The `OffsetDateTime` field carries the
//! worker's view of `locked_until` for in-process renewal-deadline
//! checks only — the row's truth is whatever the DB committed.
//!
//! `Scopable(no_tenant, no_resource, no_owner, no_type)` because the
//! row is a process-coordination artifact, not a tenant resource. It
//! is never surfaced through the SDK; only the lease module reads or
//! writes it.

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "am_leases")]
#[secure(no_tenant, no_resource, no_owner, no_type)]
pub struct Model {
    /// Coordination domain. Today only `"hierarchy_integrity"`;
    /// `TEXT` PK leaves room for additional singleton jobs without
    /// schema work.
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    /// Current holder's worker id; `NULL` when the row is free.
    pub locked_by: Option<Uuid>,
    /// DB-clock expiry timestamp. When `locked_by IS NULL` this
    /// holds the epoch sentinel (`1970-01-01T00:00:00Z`) — kept
    /// non-nullable to simplify the `WHERE locked_until < NOW()`
    /// steal filter.
    pub locked_until: OffsetDateTime,
    /// Monotonically increases on every steal (expired-lease
    /// takeover). Reset to `0` only on a clean `release`;
    /// `release_with_retry` preserves it as a forensic streak so
    /// operators can detect crash-takeover patterns.
    pub attempts: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
