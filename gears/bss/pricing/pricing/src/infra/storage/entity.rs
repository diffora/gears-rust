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

pub mod approval;
pub mod approval_key;
pub mod approval_threshold;
pub mod approval_threshold_tombstone;
pub mod audit_log;
pub mod brand_taxonomy;
pub mod bundle;
pub mod bundle_component;
pub mod bundle_revshare;
pub mod bundle_revshare_group;
pub mod catalog_version_ref;
pub mod idempotency_dedup;
pub mod migration;
pub mod operator_flag;
pub mod org_tier_taxonomy;
pub mod outbox;
pub mod partner_taxonomy;
pub mod pin_frontier;
pub mod plan;
pub mod plan_addon_rule;
pub mod plan_descriptor_set;
pub mod plan_phase;
pub mod policy_object;
pub mod price;
pub mod price_overlay;
pub mod price_overlay_line;
pub mod price_overlay_line_amount;
pub mod price_tier_band;
pub mod price_window;
pub mod read_model;
pub mod region_taxonomy;
pub mod snapshot_provenance;
