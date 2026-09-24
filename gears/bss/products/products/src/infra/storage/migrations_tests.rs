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
fn the_chain_is_coord_then_the_six_pricebook_migrations() {
    let names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    assert_eq!(
        names[1..],
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
