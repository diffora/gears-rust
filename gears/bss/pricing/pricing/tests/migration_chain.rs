//! The new chain is replayable and every migration reverses independently; the schema guard
//! (D-423) sorts first and creates nothing.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::infra::storage::migrations::Migrator;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sea_orm_migration::{MigratorTrait, SchemaManager};
use toolkit::contracts::DatabaseCapability;

async fn migration(index: usize, tables: &[&str]) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    let chain = Migrator::migrations();
    for prior in &chain[..index] {
        prior.up(&manager).await.unwrap();
    }
    let step = &chain[index];
    step.up(&manager).await.unwrap();
    step.up(&manager).await.unwrap();
    for table in tables {
        assert!(manager.has_table(*table).await.unwrap());
    }
    step.down(&manager).await.unwrap();
    step.down(&manager).await.unwrap();
    for table in tables {
        assert!(!manager.has_table(*table).await.unwrap());
    }
}

#[tokio::test]
async fn m20260926_000001_settings() {
    migration(2, &["pricing_settings"]).await;
}
#[tokio::test]
async fn m20260926_000002_approvals() {
    migration(
        3,
        &[
            "pricing_approval_policy",
            "pricing_approval_unit",
            "pricing_approval_unit_item",
            "pricing_approval_decision",
        ],
    )
    .await;
}
#[tokio::test]
async fn m20260926_000003_dimension_key() {
    migration(4, &["pricing_dimension_key"]).await;
}
#[tokio::test]
async fn m20260926_000004_price_book() {
    migration(5, &["pricing_price_book"]).await;
}
#[tokio::test]
async fn m20260926_000005_price_book_entry() {
    migration(6, &["pricing_price_book_entry"]).await;
}
#[tokio::test]
async fn m20260926_000006_reference_op() {
    migration(7, &["pricing_reference_op"]).await;
}
#[tokio::test]
async fn m20260926_000007_price() {
    migration(8, &["pricing_price"]).await;
}
#[tokio::test]
async fn m20260926_000008_audit() {
    migration(9, &["pricing_audit"]).await;
}
#[tokio::test]
async fn m20260926_000009_idempotency() {
    migration(10, &["pricing_idempotency"]).await;
}

#[tokio::test]
async fn m20260926_000010_plan() {
    migration(11, &["pricing_plan"]).await;
}
#[tokio::test]
async fn m20260926_000011_plan_revision() {
    migration(12, &["pricing_plan_revision"]).await;
}
#[tokio::test]
async fn m20260926_000012_plan_item() {
    migration(13, &["pricing_plan_item"]).await;
}

/// The guard (D-423) creates nothing and reverses to nothing.
#[tokio::test]
async fn m0000_the_guard_creates_nothing() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    let guard = &Migrator::migrations()[0];
    guard.up(&manager).await.unwrap();
    guard.up(&manager).await.unwrap();
    let tables = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master".to_owned(),
        ))
        .await
        .unwrap();
    assert!(tables.is_empty(), "the guard created a schema object");
    guard.down(&manager).await.unwrap();
    guard.down(&manager).await.unwrap();
}

#[test]
fn gear_chain_is_the_guard_coord_then_twelve_ordered_unique_migrations() {
    let names: Vec<_> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    assert_eq!(names.len(), 14);
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names, sorted);
    assert_eq!(names[0], "m0000_pricing_refuse_a_legacy_or_stale_schema");
    assert_eq!(names[1], "m0001_create_coord_leases");
    assert_eq!(names[2], "m20260926_000001_create_pricing_settings");
    assert_eq!(names[10], "m20260926_000009_create_pricing_idempotency");
    assert_eq!(names[11], "m20260926_000010_create_pricing_plan");
    assert_eq!(names[13], "m20260926_000012_create_pricing_plan_item");
}

/// The runner sorts the gear's WHOLE list by name, outbox and broker included: the guard must
/// still come first there, or those migrations commit before it refuses (plan review M1).
#[test]
fn the_guard_sorts_first_in_the_gears_whole_list() {
    let mut names: Vec<_> = bss_pricing::module::BssPricingGear::default()
        .migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    names.sort();
    assert_eq!(names[0], "m0000_pricing_refuse_a_legacy_or_stale_schema");
}
