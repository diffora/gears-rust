//! `SeaORM` entity for the `retention_rules` table (per-tenant / per-user / per-file
//! retention criteria).
//!
//! `tenant_id` provides the tenant boundary; `scope_target_id` is the target id
//! (`user_id` when `scope = "user"`, `file_id` when `scope = "file"`, NULL when
//! `scope = "tenant"`).

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "retention_rules")]
#[secure(tenant_col = "tenant_id", resource_col = "rule_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub rule_id: Uuid,
    pub tenant_id: Uuid,
    /// `"tenant"`, `"user"`, or `"file"`.
    pub scope: String,
    /// Target id -- NULL for tenant scope, `user_id` for user scope, `file_id` for file scope.
    pub scope_target_id: Option<Uuid>,
    /// Retention rule body serialized as JSON (see `RetentionRuleBody`).
    ///
    /// Stored as `jsonb` on `Postgres` and `TEXT` on `SQLite` (`SeaORM`'s `Json`
    /// column type maps transparently to both), matching the DDL in
    /// `migrations::m20260701_000001_p2_initial`. Mirrors the pattern used by
    /// `audit_outbox::Model::detail` / `events_outbox::Model::payload`.
    #[sea_orm(column_type = "Json")]
    pub body: Json,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the entity/DDL type mismatch: the Postgres DDL
    /// (`migrations::m20260701_000001_p2_initial::POSTGRES_UP`) declares
    /// `body jsonb NOT NULL`. If this column ever drifts back to
    /// `ColumnType::Text`, inserts against Postgres fail with "column is of
    /// type jsonb but expression is of type text" even though the `SQLite`
    /// test suite (where the column really is `TEXT`) would still pass.
    #[test]
    fn body_column_is_json_typed() {
        assert_eq!(Column::Body.def().get_column_type(), &ColumnType::Json);
    }
}
