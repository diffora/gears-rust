//! `SeaORM` entity for the `version_hash_manifest` table (ADR-0006).
//!
//! One row per `multipart-composite-sha256` version — the durable,
//! self-contained record (offsets + per-part SHA-256 digests) that lets a
//! client or `migrate_backend` independently re-verify `hash_value` without
//! any dependency on `multipart_upload_parts` surviving past the multipart
//! session's own lifecycle. No row exists for `whole-sha256` versions.
//!
//! No `tenant_id` column: reached through the parent `file_versions` row
//! (FK, `ON DELETE CASCADE`), so tenant scoping is enforced there, not
//! re-declared here — mirrors `file_version.rs`'s own rationale.

use sea_orm::entity::prelude::*;
use time::OffsetDateTime;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "version_hash_manifest")]
#[secure(no_tenant, resource_col = "version_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub version_id: Uuid,
    pub manifest: String,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
