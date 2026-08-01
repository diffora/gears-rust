//! `SeaORM` entities for the bss-pricing gear (schema `bss`).
//!
//! One module per Foundation-owned table, each tenant-scoped through
//! `SecureORM` (`#[secure(tenant_col = "tenant_id", ...)]`) so cross-tenant
//! reads are denied in SQL rather than by a forgotten `WHERE` clause. Column
//! types are chosen to round-trip on **both** backends: `Uuid` reads from
//! Postgres `uuid` and `SQLite` `text`, `DateTime<Utc>` from `timestamptz` and
//! `text`, `JsonValue` from `jsonb` and `text`, `Vec<u8>` from `bytea` and
//! `blob`.

pub mod audit_log;
pub mod catalog_version_ref;
pub mod idempotency_dedup;
pub mod operator_flag;
pub mod outbox;
pub mod pin_frontier;
pub mod plan;
pub mod policy_object;
pub mod price;
pub mod read_model;
