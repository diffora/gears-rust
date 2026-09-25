//! Runtime migration chain, retaining only the shared coordination table.

use sea_orm_migration::{MigrationTrait, MigratorTrait};

/// The pricing-owned migration chain.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(coord::migration::Migration::in_schema("bss"))]
    }
}
