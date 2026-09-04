//! Migration chain for the bss-products gear (schema `bss`).
//!
//! Greenfield chain, **one migration per table**, so the chain reads as a list
//! of decisions rather than a script and every `DOWN` drops what its own `UP`
//! created and nothing else. There is no predecessor state to restore and no
//! earlier `UP` to transcribe, which a chain of patches cannot offer.
//!
//! Each migration is a pair of raw-SQL const arrays — Postgres canonical,
//! `SQLite` mirror — plus the dispatch below. Both dialects ship; every other
//! backend is refused.
//!
//! **The schema statement has its own migration**, first in the roster and the
//! only place `CREATE SCHEMA` appears here.
//!
//! **Ordering.** The toolkit migration runner applies migrations in **name**
//! order, not vec order, and rejects a duplicate migration name outright — a
//! duplicate would otherwise be a migration that silently never runs.

pub mod m20260829_000001_create_bss_schema;
pub mod m20260829_000002_create_products_product;
pub mod m20260829_000003_create_products_sku;
pub mod m20260829_000004_create_products_audit_log;
pub mod m20260829_000005_create_products_identity_ref;
pub mod m20260829_000006_create_products_idempotency;
pub mod m20260829_000007_create_products_entity_version;
pub mod m20260901_000008_create_products_catalog_version_counter;
pub mod m20260901_000009_create_products_reference_watermark;
pub mod m20260901_000010_create_products_catalog_version;
pub mod m20260901_000011_create_products_catalog_version_request;
pub mod m20260901_000012_create_products_freeze_ledger;
pub mod m20260901_000013_create_products_catalog_version_entry;
pub mod m20260901_000014_create_products_bulk;
mod m20260901_000015_create_products_reference_producer;
mod m20260901_000016_create_products_approval;
mod m20260901_000017_create_products_breakglass_session;
mod m20260901_000018_create_products_category;
mod m20260901_000019_create_products_attribute;
mod m20260901_000020_create_products_metadata;
mod m20260901_000021_create_products_recognized_set;
mod m20260901_000022_create_products_correction_override;
mod m20260901_000023_create_products_read_entity;
mod m20260901_000024_create_products_read_stamp;
mod m20260901_000025_create_products_scheduled_transition;
mod m20260901_000026_create_products_deferred_retirement;
mod m20260901_000027_create_products_materiality_policy;
mod m20260901_000028_create_products_pii_allowlist;

use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

/// Run the backend's statement list, in order.
///
/// Factored out rather than copied per migration: the branch that must never
/// drift is "which backend gets which statements", and there is exactly one
/// copy of it.
///
/// # Errors
/// [`DbErr::Migration`] for an unsupported backend, or the driver's error for a
/// failing statement — **wrapped with the migration that owns it and the
/// statement's index within that migration's list**. The operator's first
/// question after a failed boot is which migration was applying and how far it
/// got, and a bare driver string does not answer it.
pub(crate) async fn exec_backend(
    migration: &str,
    manager: &SchemaManager<'_>,
    postgres: &[&str],
    sqlite: &[&str],
) -> Result<(), DbErr> {
    let backend = manager.get_database_backend();
    let conn = manager.get_connection();
    let statements: &[&str] = match backend {
        sea_orm::DatabaseBackend::Postgres => postgres,
        sea_orm::DatabaseBackend::Sqlite => sqlite,
        // A catch-all rather than naming MySQL: `DatabaseBackend` is
        // `#[non_exhaustive]`, so naming the one unsupported variant would stop
        // covering the match the moment sea-orm adds another. The catch-all is
        // what this arm always meant — this gear ships two dialects and refuses
        // every other one.
        _ => {
            return Err(DbErr::Migration(format!(
                "{backend:?} is not a supported backend for bss-products"
            )));
        }
    };
    for (index, sql) in statements.iter().enumerate() {
        conn.execute_raw(Statement::from_string(backend, (*sql).to_owned()))
            .await
            .map_err(|e| {
                DbErr::Migration(format!(
                    "{migration}: statement {} of {} failed on {backend:?}: {e}",
                    index.saturating_add(1),
                    statements.len()
                ))
            })?;
    }
    Ok(())
}

/// The gear's migration chain.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // `coord_leases` — the per-tenant increment lease's table
            // (`dod-coalescer`). Owned by the `coord` crate, qualified into
            // `bss` like the gear's own DDL. NOTE: the toolkit migration
            // runner applies migrations in NAME order, so coord's `m0001_…`
            // name sorts FIRST — before `…000001_create_bss_schema`; coord's
            // `in_schema` `up` therefore runs `CREATE SCHEMA IF NOT EXISTS
            // bss` itself, so the qualification is safe despite running
            // first (and idempotent with the schema migration that follows).
            Box::new(coord::migration::Migration::in_schema("bss")),
            Box::new(m20260829_000001_create_bss_schema::Migration),
            Box::new(m20260829_000002_create_products_product::Migration),
            Box::new(m20260829_000003_create_products_sku::Migration),
            Box::new(m20260829_000004_create_products_audit_log::Migration),
            Box::new(m20260829_000005_create_products_identity_ref::Migration),
            Box::new(m20260829_000006_create_products_idempotency::Migration),
            Box::new(m20260829_000007_create_products_entity_version::Migration),
            Box::new(m20260901_000008_create_products_catalog_version_counter::Migration),
            Box::new(m20260901_000009_create_products_reference_watermark::Migration),
            Box::new(m20260901_000010_create_products_catalog_version::Migration),
            Box::new(m20260901_000011_create_products_catalog_version_request::Migration),
            Box::new(m20260901_000012_create_products_freeze_ledger::Migration),
            Box::new(m20260901_000013_create_products_catalog_version_entry::Migration),
            Box::new(m20260901_000014_create_products_bulk::Migration),
            Box::new(m20260901_000015_create_products_reference_producer::Migration),
            Box::new(m20260901_000016_create_products_approval::Migration),
            Box::new(m20260901_000017_create_products_breakglass_session::Migration),
            Box::new(m20260901_000018_create_products_category::Migration),
            Box::new(m20260901_000019_create_products_attribute::Migration),
            Box::new(m20260901_000020_create_products_metadata::Migration),
            Box::new(m20260901_000021_create_products_recognized_set::Migration),
            Box::new(m20260901_000022_create_products_correction_override::Migration),
            Box::new(m20260901_000023_create_products_read_entity::Migration),
            Box::new(m20260901_000024_create_products_read_stamp::Migration),
            Box::new(m20260901_000025_create_products_scheduled_transition::Migration),
            Box::new(m20260901_000026_create_products_deferred_retirement::Migration),
            Box::new(m20260901_000027_create_products_materiality_policy::Migration),
            Box::new(m20260901_000028_create_products_pii_allowlist::Migration),
        ]
    }
}

#[cfg(test)]
#[path = "migrations_tests.rs"]
mod migrations_tests;
