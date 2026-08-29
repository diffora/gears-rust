//! The roster's own invariants. The runner applies migrations in **name** order
//! and rejects a duplicate name outright, so a duplicate would be a migration
//! that silently never runs — which is what these assertions exist to catch.

use sea_orm_migration::MigratorTrait;

use super::Migrator;

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
fn the_schema_migration_sorts_first() {
    let names: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    let first = names.first().map(String::as_str);
    assert_eq!(first, Some("m20260829_000001_create_bss_schema"));
}
