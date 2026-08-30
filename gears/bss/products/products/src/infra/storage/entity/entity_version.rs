//! `SeaORM` entity for `bss.products_entity_version` — the frozen
//! published-version history (`design/01-foundation.md` §4.3), keyed
//! `(tenant_id, entity_kind, entity_id, published_version)`.
//!
//! This entity lays down the storage shape only. Freezing a row — computing
//! the canonical rendering and its digest — is the publish act's, not this
//! module's, and no repository function reads or writes this table yet.
//!
//! [`Model::content`] holds **the canonical rendering itself**, exactly the
//! bytes [`Model::content_digest`] was computed over, rather than one column
//! per content field. The migration's module doc carries the full argument;
//! the short form is that slice 10's restore drill re-verifies the digest
//! **byte-for-byte**, which a re-serialisation from typed columns cannot
//! guarantee, and that content grows per slice while §4.3 already makes a
//! widening a `digest_version` bump.
//!
//! [`Model::digest_version`] is carried on the row, not deduced, so that
//! "digest-version bump, not a silent change" is checkable at all
//! (**P-D-29**, **P-D-33**).
//!
//! The four key columns are the primary key: §4.3 states the key as a
//! `UNIQUE`, and a primary key over exactly those columns is that uniqueness
//! without a second structure enforcing it.
//!
//! @cpt-cf-bss-products-dod-version-history-table

use sea_orm::entity::prelude::*;
use toolkit_db_macros::Scopable;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Scopable)]
#[sea_orm(table_name = "products_entity_version")]
#[secure(
    tenant_col = "tenant_id",
    resource_col = "entity_id",
    no_owner,
    no_type
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    /// Composite primary key. `product` or `sku`, constrained by
    /// `chk_products_entity_version_entity_kind` — the roster is closed on
    /// both engines, so a third kind is a migration, never a convention.
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_kind: String,
    /// Composite primary key. The head row's own identifier — `product_id`
    /// or `sku_id` according to [`Model::entity_kind`]. It carries no
    /// foreign key, the two heads living in two tables.
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_id: Uuid,
    /// Composite primary key. The version this row freezes, `>= 1`: version
    /// `0` is the unpublished head's counter value and has no frozen row.
    #[sea_orm(primary_key, auto_increment = false)]
    pub published_version: i64,
    /// The frozen content in §4.3's engine-canonical rendering — `JSON`,
    /// keys sorted lexicographically, absent values written `null` rather
    /// than omitted, numbers as bare decimal strings, timestamps RFC 3339
    /// UTC at microsecond precision — excluding the metadata map and
    /// excluding `lifecycle_state`, `deprecation_provenance`,
    /// `replaced_by_sku_id` and `internal_revision`, which move on
    /// transitions and are read from the head row (**P-D-24**, **P-D-35**).
    ///
    /// Typed `String`, not a parsed `JSON` value: a parsed value re-rendered
    /// on write is no longer guaranteed to be the bytes
    /// [`Model::content_digest`] was taken over, which is the one property
    /// this column exists to have.
    ///
    /// The column is `text` on **both** engines. It is deliberately not
    /// `jsonb`, which re-renders its input and so cannot hold digested
    /// bytes, and deliberately not `json` either: this field is bound as a
    /// `text` parameter, and Postgres has no assignment cast from `text` to
    /// `json`, so a `json` column would refuse every insert on the
    /// production engine while the `SQLite`-only suite stayed green. The
    /// migration's module doc carries the full chain.
    pub content: String,
    /// `SHA-256` over [`Model::content`] as stored, computed at freeze
    /// (**P-D-35**). Re-verifiable from the row alone, which is what slice
    /// 10's restore drill needs.
    pub content_digest: Vec<u8>,
    /// The digest scheme [`Model::content_digest`] was computed under,
    /// starting at `1` and pinned by §5's golden vector as a code constant
    /// rather than by configuration (**P-D-33**).
    pub digest_version: i32,
    /// The authorizing `ApprovalRecord`'s id on a yes verdict
    /// (`inst-fd-gate-verdict`). Nullable while the gate that mints one is
    /// slice 05's; see the migration's module doc for why the tightening is
    /// owed rather than taken now.
    pub approval_ref: Option<Uuid>,
    /// The pseudonymous reference of the principal who published, resolved
    /// through `products_identity_ref` (`inst-fd-actor-ref`).
    pub actor_ref: Uuid,
    pub published_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
