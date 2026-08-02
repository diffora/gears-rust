//! `SeaORM` entities for the bss-pricing gear (schema `bss`).
//!
//! One module per physical table — the ten Foundation-owned ones and the
//! slice-owned tables that follow them — each tenant-scoped through
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
pub mod plan_addon_rule;
pub mod plan_descriptor_set;
pub mod plan_phase;
pub mod policy_object;
pub mod price;
pub mod price_tier_band;
pub mod read_model;
