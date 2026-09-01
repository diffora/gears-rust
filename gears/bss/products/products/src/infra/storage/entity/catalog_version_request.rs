//! `SeaORM` entity for `bss.products_catalog_version_request` — the increment
//! queue (`design/06-catalog-version.md` §4, P-D-50, P-D-52, P-D-60), keyed
//! `(tenant_id, source, request_key)`.
//!
//! The storage shape only; the mechanics live elsewhere. The key's `INSERT`
//! is the request door's idempotency — *"an idempotent replay is caught by
//! the UNIQUE"*, the migration's own words — and the `pending → coalesced`
//! flip is the increment transaction's, stamped together with
//! `satisfied_by_version_id` (the `chk_..._shape` `CHECK` makes the pairing
//! physical on both backends).
//!
//! `requested_at` is the door's stamp, taken at ingress and never accepted
//! from the caller; the interactive lane's SLO measures from it.

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_catalog_version_request")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "request_key",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// The registered requester the demand belongs to. Part of the key with
    /// the tenant axis (`dod-request-queue`'s reason: one source serves many
    /// tenants).
    #[sea_orm(primary_key, auto_increment = false)]
    pub source: String,
    /// The caller's idempotency handle.
    #[sea_orm(primary_key, auto_increment = false)]
    pub request_key: String,
    /// `interactive` or `bulk` — D-47's two demand lanes.
    pub lane: String,
    /// The bulk batch this request coalesces under; interactive requests
    /// carry none.
    pub operation_key: Option<String>,
    /// The door's ingress stamp; the lane SLO's zero point.
    pub requested_at: ChronoDateTimeUtc,
    /// `pending` or `coalesced` — P-D-60 struck `superseded`.
    pub state: String,
    /// The version that satisfied this request; written together with the
    /// `coalesced` flip, `NULL` exactly while `pending` (the shape `CHECK`).
    pub satisfied_by_version_id: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
