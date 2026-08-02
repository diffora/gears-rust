//! Migration set for the bss-pricing gear (schema `bss`).
//!
//! Greenfield chain: the ten Foundation-owned tables of
//! `design/01-foundation.md` §3.7, one migration per table, in dependency
//! order, then the slice-owned tables. Slice 3 adds
//! `pricing_price_tier_band` (`design/03-price-structure.md` §6) — the only
//! part of a price row that is many-per-row; its per-row Slice-3 columns are
//! amended onto `pricing_price` itself. The remaining slice-owned tables (the
//! overlay tables, the window store, `pricing_historical_price`, ...) arrive
//! with their slices and append to this list.
//!
//! **Schema creation.** The chain has no separate `create_bss_schema`
//! migration: `m20260802_000001_create_pricing_plan` issues
//! `CREATE SCHEMA IF NOT EXISTS bss` as its first Postgres statement, and the
//! shared `coord` lease migration (whose `m0001_...` name sorts first under the
//! toolkit runner's name ordering) issues the same statement before its own
//! `CREATE TABLE`. Both are `IF NOT EXISTS`, so the schema exists no matter
//! which of them the runner reaches first.
//!
//! **Ordering.** The toolkit migration runner applies migrations in **name**
//! order, not vec order, and rejects a duplicate `DeriveMigrationName` outright
//! — which is what `tests/module_test.rs` asserts about this list, because a
//! duplicate name would otherwise be a migration that silently never runs.

pub mod m20260802_000001_create_pricing_plan;
pub mod m20260802_000002_create_pricing_price;
pub mod m20260802_000003_create_pricing_read_model;
pub mod m20260802_000004_create_pricing_catalog_version_ref;
pub mod m20260802_000005_create_pricing_pin_frontier;
pub mod m20260802_000006_create_pricing_policy_object;
pub mod m20260802_000007_create_pricing_operator_flag;
pub mod m20260802_000008_create_pricing_idempotency_dedup;
pub mod m20260802_000009_create_pricing_outbox;
pub mod m20260802_000010_create_pricing_audit_log;
pub mod m20260802_000011_create_pricing_price_tier_band;

use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

/// Run the backend's statement list, in order.
///
/// Every migration in this chain is a pair of raw-SQL const arrays (Postgres
/// canonical, `SQLite` mirror) plus this dispatch. Factored out rather than
/// copied ten times: the branch that must never drift is "which backend gets
/// which statements", and there is exactly one copy of it.
///
/// # Errors
/// [`DbErr::Migration`] for `MySQL` (not a supported backend for this gear),
/// or the driver's error for a failing statement.
pub(crate) async fn exec_backend(
    manager: &SchemaManager<'_>,
    postgres: &[&str],
    sqlite: &[&str],
) -> Result<(), DbErr> {
    let backend = manager.get_database_backend();
    let conn = manager.get_connection();
    let statements: &[&str] = match backend {
        sea_orm::DatabaseBackend::Postgres => postgres,
        sea_orm::DatabaseBackend::Sqlite => sqlite,
        sea_orm::DatabaseBackend::MySql => {
            return Err(DbErr::Migration(
                "MySQL is not supported for bss-pricing".to_owned(),
            ));
        }
    };
    for sql in statements {
        conn.execute(Statement::from_string(backend, (*sql).to_owned()))
            .await?;
    }
    Ok(())
}

/// The gear's migration chain.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260802_000001_create_pricing_plan::Migration),
            Box::new(m20260802_000002_create_pricing_price::Migration),
            Box::new(m20260802_000003_create_pricing_read_model::Migration),
            Box::new(m20260802_000004_create_pricing_catalog_version_ref::Migration),
            Box::new(m20260802_000005_create_pricing_pin_frontier::Migration),
            Box::new(m20260802_000006_create_pricing_policy_object::Migration),
            Box::new(m20260802_000007_create_pricing_operator_flag::Migration),
            Box::new(m20260802_000008_create_pricing_idempotency_dedup::Migration),
            Box::new(m20260802_000009_create_pricing_outbox::Migration),
            Box::new(m20260802_000010_create_pricing_audit_log::Migration),
            Box::new(m20260802_000011_create_pricing_price_tier_band::Migration),
            // Shared `coord_leases` table, owned by the `coord` crate. The
            // read-model warm re-drive is a singleton job (§3.8: background work
            // is coordinated as a singleton via the coordination lease library),
            // so this gear needs the lease table the same way the sibling ledger
            // needs it for its recognition run. Qualified into `bss` so it lands
            // there regardless of the connection's `search_path` order; its
            // `m0001_...` name sorts FIRST under the runner's name ordering, so
            // its `up` creates the schema itself before the `CREATE TABLE`.
            Box::new(coord::migration::Migration::in_schema("bss")),
        ]
    }
}
