//! `SeaORM` entity for `bss.products_audit_log` — the append-only trail for
//! every act that emits no broker event: a refusal, a read under elevation,
//! and a committed act the design declares eventless.
//!
//! # The reserved platform-sealing seam
//!
//! `seal_state`, `chain_id`, `seq`, `prev_hash` and `row_hash` exist so the
//! platform sealing capability (P-D-08) can activate without a migration.
//! `seal_state` is written `unsealed` at INSERT, always; this gear computes
//! no hash and runs no verification job — that is the platform capability's
//! job. The one admitted `UPDATE`, the one-way `unsealed -> sealed`
//! transition, is enforced by the table's trigger, not by this entity: this
//! entity carries no write rule for it at all.
//!
//! @cpt-cf-bss-products-dod-audit-table

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_audit_log")]
#[secure(tenant_col = "tenant_id", resource_col = "audit_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub audit_id: Uuid,
    pub tenant_id: Uuid,
    /// The pseudonymous ref of whoever (or whatever refused act) this row
    /// attributes to. Never a direct operator identity.
    pub actor_ref: Uuid,
    /// The audit action token (`design/01-foundation.md` §4.4). No
    /// vocabulary `CHECK` yet — an owed debt the migration's own doc names.
    pub action: String,
    /// The kind of thing `subject_id`/`attempted_key` names. Same owed debt
    /// as `action`.
    pub subject_kind: String,
    /// The subject's id, when one was minted. Nullable: a refusal raised
    /// before the mint has no id to carry.
    pub subject_id: Option<Uuid>,
    /// The subject's revision at the time of the act. Nullable for the same
    /// reason as `subject_id`.
    pub subject_revision: Option<i64>,
    /// The refusal's error code. Null on every class that is not a refusal.
    pub error_code: Option<String>,
    /// The attempted `name`, `sku_code` or `product_code` a pre-mint refusal
    /// carries instead of a `subject_id`.
    pub attempted_key: Option<String>,
    /// A free-text reason, where the door supplies one.
    pub reason: Option<String>,
    /// Ties related rows together across a single request, where one exists.
    pub correlation_id: Option<Uuid>,
    /// The operand `10-retention-erasure`'s `RetentionClock` reads.
    pub written_at: ChronoDateTimeUtc,
    /// Present on the elevation class only.
    pub session_id: Option<Uuid>,
    /// `unsealed | sealed`. Written `unsealed` at INSERT, always; this gear
    /// never advances it.
    pub seal_state: String,
    /// Reserved for the platform sealing capability. `NULL` until sealed.
    pub chain_id: Option<Uuid>,
    /// Reserved for the platform sealing capability. `NULL` until sealed.
    pub seq: Option<i64>,
    /// Reserved for the platform sealing capability. `NULL` on the segment
    /// head and until sealed.
    pub prev_hash: Option<Vec<u8>>,
    /// Reserved for the platform sealing capability. `NULL` until sealed.
    pub row_hash: Option<Vec<u8>>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
