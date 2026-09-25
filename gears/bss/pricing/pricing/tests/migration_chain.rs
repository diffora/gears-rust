//! The new chain is replayable and every migration reverses independently.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::infra::storage::migrations::Migrator;
use sea_orm::Database;
use sea_orm_migration::{MigratorTrait, SchemaManager};

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
    migration(1, &["pricing_settings"]).await;
}
#[tokio::test]
async fn m20260926_000002_approvals() {
    migration(
        2,
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
    migration(3, &["pricing_dimension_key"]).await;
}
#[tokio::test]
async fn m20260926_000004_price_book() {
    migration(4, &["pricing_price_book"]).await;
}
#[tokio::test]
async fn m20260926_000005_price_book_entry() {
    migration(5, &["pricing_price_book_entry"]).await;
}
#[tokio::test]
async fn m20260926_000006_reference_op() {
    migration(6, &["pricing_reference_op"]).await;
}
#[tokio::test]
async fn m20260926_000007_price() {
    migration(7, &["pricing_price"]).await;
}
#[tokio::test]
async fn m20260926_000008_audit() {
    migration(8, &["pricing_audit"]).await;
}
#[tokio::test]
async fn m20260926_000009_idempotency() {
    migration(9, &["pricing_idempotency"]).await;
}

#[tokio::test]
async fn m20260926_000010_plan() {
    migration(10, &["pricing_plan"]).await;
}
#[tokio::test]
async fn m20260926_000011_plan_revision() {
    migration(11, &["pricing_plan_revision"]).await;
}
#[tokio::test]
async fn m20260926_000012_plan_item() {
    migration(12, &["pricing_plan_item"]).await;
}

#[test]
fn gear_chain_is_coord_then_twelve_ordered_unique_migrations() {
    let names: Vec<_> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    assert_eq!(names.len(), 13);
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names, sorted);
    assert_eq!(names[1], "m20260926_000001_create_pricing_settings");
    assert_eq!(names[9], "m20260926_000009_create_pricing_idempotency");
    assert_eq!(names[10], "m20260926_000010_create_pricing_plan");
    assert_eq!(names[12], "m20260926_000012_create_pricing_plan_item");
}
