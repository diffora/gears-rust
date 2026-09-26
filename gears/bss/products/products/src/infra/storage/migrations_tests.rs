//! Structural migration guards.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::Migrator;
use sea_orm_migration::MigratorTrait;

#[test]
fn every_migration_name_is_unique() {
    let names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        names.len(),
        "a duplicate migration name is a migration that never runs"
    );
}

#[test]
fn vec_order_matches_name_order() {
    // The runner sorts by name; if the vec disagrees, the file order stops
    // describing the execution order and the chain becomes unreadable.
    let names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
}

#[test]
fn the_chain_is_the_guard_coord_then_the_six_pricebook_migrations() {
    let names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    assert_eq!(names[0], "m0000_products_refuse_a_legacy_or_stale_schema");
    assert_eq!(names[1], "m0001_create_coord_leases");
    assert_eq!(
        names[2..],
        [
            "m20260925_000001_create_products_category",
            "m20260925_000002_create_products_sku",
            "m20260925_000003_create_products_approvals",
            "m20260925_000004_create_products_audit_log",
            "m20260925_000005_create_products_idempotency",
            "m20260925_000006_create_products_sku_reference"
        ]
    );
}

/// The runner sorts the gear's WHOLE list by name, outbox and broker included: the guard
/// (P-D-195) must still come first there, or those migrations commit before it refuses.
#[test]
fn the_guard_sorts_first_in_the_gears_whole_list() {
    use toolkit::contracts::DatabaseCapability;
    let mut names: Vec<String> = crate::gear::BssProductsGear::default()
        .migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    names.sort_unstable();
    assert_eq!(names[0], "m0000_products_refuse_a_legacy_or_stale_schema");
}

/// The guard creates nothing and reverses to nothing.
#[tokio::test]
async fn the_guard_creates_nothing() {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    use sea_orm_migration::SchemaManager;
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    let guard = &Migrator::migrations()[0];
    guard.up(&manager).await.unwrap();
    guard.up(&manager).await.unwrap();
    let objects = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master".to_owned(),
        ))
        .await
        .unwrap();
    assert!(objects.is_empty(), "the guard created a schema object");
    guard.down(&manager).await.unwrap();
    guard.down(&manager).await.unwrap();
}
