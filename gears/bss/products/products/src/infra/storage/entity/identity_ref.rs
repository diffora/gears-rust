//! `SeaORM` entity for `bss.products_identity_ref` — the pseudonym-to-identity
//! map (`design/10-retention-erasure.md` `inst-im-map`), keyed `(tenant_id,
//! actor_ref)`.
//!
//! The only table in the gear where PII may live. `identity_payload` is
//! nullable because a tombstone destroys it while `principal_ref` — the
//! pseudonym, not the identity — stands, which is what lets a repeat DSAR and
//! the age predicate keep working after an erasure. `last_seen_at` is
//! advanced by every act that **resolves** the ref, never by minting it
//! alone: minting happens once per active ref, on the first appearance of a
//! principal with no live ref, not on first appearance ever. This table
//! carries no append-only guard — `last_seen_at` and the tombstone columns
//! are mutable by design.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-actor-ref:p1

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_identity_ref")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "actor_ref",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// Composite primary key with `tenant_id`. The pseudonymous ref every
    /// append-only record in the gear is stamped with.
    #[sea_orm(primary_key, auto_increment = false)]
    pub actor_ref: Uuid,
    /// The pseudonymous principal handle this ref resolves — NOT NULL so
    /// erasure's resolve, the DSAR export and the first-appearance predicate
    /// all have an operand to read by principal.
    pub principal_ref: String,
    /// The identity side of the map. Nullable: a tombstone destroys it, and
    /// `chk_products_identity_ref_tombstone` requires it absent once
    /// `tombstoned_at` is set.
    pub identity_payload: Option<String>,
    /// Set once, by erasure, and never cleared. A tombstoned ref is retired
    /// permanently — a principal acting after its erasure mints a fresh row
    /// rather than reusing this one.
    pub tombstoned_at: Option<ChronoDateTimeUtc>,
    /// Stamped once, at mint, and never moved again.
    pub first_seen_at: ChronoDateTimeUtc,
    /// The M2 age operand. Advanced by every act that resolves this ref, not
    /// by minting it — see this module's doc for the recorded failure that
    /// rule fixes.
    pub last_seen_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
