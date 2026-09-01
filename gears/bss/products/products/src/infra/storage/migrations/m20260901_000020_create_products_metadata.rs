//! Create `bss.products_metadata` — the ungoverned metadata map
//! (`design/02-taxonomy-attributes.md` §4.1; **P-D-06**).
//!
//! # Outside version content, deliberately
//!
//! **P-D-06** puts this map outside the frozen version content: a metadata
//! write is not entity content, so it neither bumps a revision nor lands in a
//! `products_entity_version` row. That is why the table carries no
//! `internal_revision` and no freeze guard — there is nothing here for a
//! version to freeze, and a row is current state that its door overwrites.
//!
//! # The caps live at the door, not in the DDL
//!
//! §4.1: *"caps enforced at the door (`METADATA_LIMIT`)"*. A key count and a
//! value length are policy the door reads from configuration, and a CHECK
//! pinning either would make the limit a schema migration to change. What the
//! DDL does pin is non-emptiness of the key, because a keyless row is
//! addressable by nothing.
//!
//! # The key is polymorphic and carries no entity FK
//!
//! `(tenant_id, entity_kind, entity_id, key)` — `entity_kind ∈ {product, sku}`
//! (categories carry their display values as attribute values, not metadata),
//! and the two kinds live in two tables, so no single FK covers the
//! coordinate. The owning door proves existence, exactly as on
//! `products_attribute_value`.
//!
//! # Backend differences
//!
//! `uuid` becomes `text` on `SQLite`, `timestamptz` becomes `text`, and the
//! `bss.` qualification is dropped. Every CHECK and the key are preserved on
//! both sides.
//!
//! # The `DoD` is not ticked: §7 row 20 is live
//!
//! *"What `entity_kind` values does each table admit, and does a definition
//! scope to entity kinds? The set enumerates them nowhere, while the
//! attribute-value table demonstrably admits `category` and the only named
//! metadata door admits `{products|skus}`."* So `entity_kind` here is pinned
//! **non-empty only** rather than rostered — a CHECK naming the two kinds
//! would answer the row from a migration.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const PG_UP_STATEMENTS: &[&str] = &["CREATE TABLE bss.products_metadata (
            tenant_id   uuid        NOT NULL,
            entity_kind text        NOT NULL,
            entity_id   uuid        NOT NULL,
            key         text        NOT NULL,
            value       text        NOT NULL,
            created_at  timestamptz NOT NULL,
            updated_at  timestamptz NOT NULL,
            CONSTRAINT products_metadata_pkey PRIMARY KEY (tenant_id, entity_kind, entity_id, key),
            CONSTRAINT chk_products_metadata_entity_kind CHECK (entity_kind <> ''),
            CONSTRAINT chk_products_metadata_key CHECK (key <> '')
        )"];

const PG_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS bss.products_metadata"];

const SQLITE_UP_STATEMENTS: &[&str] = &["CREATE TABLE products_metadata (
            tenant_id   text NOT NULL,
            entity_kind text NOT NULL,
            entity_id   text NOT NULL,
            key         text NOT NULL,
            value       text NOT NULL,
            created_at  text NOT NULL,
            updated_at  text NOT NULL,
            PRIMARY KEY (tenant_id, entity_kind, entity_id, key),
            CONSTRAINT chk_products_metadata_entity_kind CHECK (entity_kind <> ''),
            CONSTRAINT chk_products_metadata_key CHECK (key <> '')
        )"];

const SQLITE_DOWN_STATEMENTS: &[&str] = &["DROP TABLE IF EXISTS products_metadata"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(self.name(), manager, PG_UP_STATEMENTS, SQLITE_UP_STATEMENTS).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        super::exec_backend(
            self.name(),
            manager,
            PG_DOWN_STATEMENTS,
            SQLITE_DOWN_STATEMENTS,
        )
        .await
    }
}
