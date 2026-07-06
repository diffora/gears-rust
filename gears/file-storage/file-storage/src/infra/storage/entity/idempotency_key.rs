//! `SeaORM` entity for the `idempotency_keys` table.
//!
//! Composite PK: `(tenant_id, owner_kind, owner_id, idempotency_key)`.
//! Scoped by `tenant_id` but no single `resource_col` — use `allow_all()` scope.

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

/// An idempotency key row for POST /files deduplication.
#[allow(unknown_lints, de0309_must_have_domain_model)]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "idempotency_keys")]
#[secure(no_tenant, no_resource, no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub owner_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub owner_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub idempotency_key: String,
    /// The authenticated subject (`ctx.subject_id()`) that created this
    /// record. Not part of the primary key — the domain layer fetches by the
    /// existing composite key and then verifies this column matches the
    /// replaying caller, treating a mismatch as `Forbidden` (P2 remediation
    /// 0.10: a caller must never be able to surface another caller's ticket
    /// by reusing/guessing their `(owner_kind, owner_id, key)` tuple).
    pub subject_id: Uuid,
    pub file_id: Uuid,
    pub response_status: i32,
    #[sea_orm(column_type = "Text")]
    pub response_body: String,
    pub response_etag: String,
    /// SHA-256 over a canonicalized, length-prefixed encoding of the
    /// identity-relevant request fields (`owner_kind`, `owner_id`, `name`,
    /// `gts_file_type`, `mime_type`, sorted `custom_metadata`) at insert time
    /// (P2 remediation 2.1: `domain::idempotency::compute_request_hash`). A
    /// replay recomputes this hash from the current request and rejects a
    /// mismatch with `409 Conflict` before returning the stored ticket.
    pub request_hash: Vec<u8>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
