//! `SeaORM` migrations for the `credstore` module.
//!
//! * `m0001_initial_schema` — `credstore_secrets` table (incl. the monotonic
//!   `version` column) with all indexes.

use sea_orm_migration::prelude::*;

pub mod m0001_initial_schema;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m0001_initial_schema::Migration)]
    }
}
