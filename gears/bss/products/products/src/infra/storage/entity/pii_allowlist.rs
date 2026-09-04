//! `SeaORM` entity for `bss.products_pii_allowlist` — the Legal-governed
//! allow-list of person-named strings the PII detector admits (**P-D-117**
//! items 23 and 31; `design/10-retention-erasure.md` `inst-pp-allowlist`).
//!
//! **The match operand is `value_normalized` and nothing else.** The stored
//! value is always the output of
//! [`crate::domain::retention::normalize_allowlist_value`], and the detector
//! normalizes its own subject through the same function before comparing, so
//! the equality has one definition rather than two that can drift. The rule is
//! exact equality, never a pattern: C2's list is a list of names, and a
//! pattern would let a signed-off entry widen itself after the sign-off.
//!
//! **Revocation is a `state` flip and never a `DELETE`** (P-D-47's reasoning
//! one table over), so a revoked entry keeps its sign-off on record and
//! `uq_products_pii_allowlist_active` scopes the uniqueness to the active
//! rows. A value revoked and later signed off again is two rows and two
//! sign-offs — the audit trail the paper control is.
//!
//! This table is a **PII store by construction** and takes the identity map's
//! posture (**P-D-117** item 12): excluded from every export but the
//! compliance surface, and its two free-text columns go through the
//! content-PII write block at the door.
//!
//! Scoped `resource_col = "entry_id"`: the entry is the governed object, and
//! it is what `PiiAllowlistChanged` partitions on (**P-D-118** item 26).

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

/// The `state` value of an entry the detector consults.
pub const STATE_ACTIVE: &str = "active";

/// The `state` value of an entry Legal has withdrawn. The row stays, so the
/// sign-off that admitted it stays with it.
pub const STATE_REVOKED: &str = "revoked";

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_pii_allowlist")]
#[secure(tenant_col = "tenant_id", resource_col = "entry_id", no_owner, no_type)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// The entry's stable address, surviving a revoke-and-re-sign. The
    /// aggregate `PiiAllowlistChanged` partitions on.
    #[sea_orm(primary_key, auto_increment = false)]
    pub entry_id: Uuid,
    /// The normalized name the detector matches on, exactly.
    pub value_normalized: String,
    /// Why Legal admitted this name. Operator free text, and inside the
    /// content-PII write block.
    pub justification: String,
    /// The reference to the external Legal decision — the artifact, not a
    /// principal. `NOT NULL`, because an entry without it is refused; the
    /// refusal itself rides `01`'s `VALIDATION` naming the field (P-D-64),
    /// and this column is the backstop rather than the message.
    pub signed_off_by: String,
    pub signed_off_at: ChronoDateTimeUtc,
    /// [`STATE_ACTIVE`] or [`STATE_REVOKED`], the `CHECK`'s closed pair.
    pub state: String,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
