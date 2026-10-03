//! P-D-263 on SQLite: 000014 adds the archive mark to SKUs and categories, keeps every row, and
//! `down` takes the mark away again. The twin is `tests/postgres_archive.rs`.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;

async fn prior(db: &sea_orm::DatabaseConnection) -> SchemaManager<'_> {
    let manager = SchemaManager::new(db);
    let chain = super::super::Migrator::migrations();
    let at = chain
        .iter()
        .position(|migration| migration.name() == Migration.name())
        .expect("000014 is in the chain");
    for migration in &chain[..at] {
        migration.up(&manager).await.unwrap();
    }
    manager
}

async fn exec(db: &sea_orm::DatabaseConnection, sql: &str) {
    db.execute_raw(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn strings(db: &sea_orm::DatabaseConnection, sql: &str) -> Vec<String> {
    db.query_all_raw(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .iter()
        .map(|row| row.try_get::<String>("", "v").unwrap())
        .collect()
}

async fn columns(db: &sea_orm::DatabaseConnection, table: &str) -> Vec<String> {
    strings(
        db,
        &format!("SELECT name AS v FROM pragma_table_info('{table}') ORDER BY name"),
    )
    .await
}

async fn indexes(db: &sea_orm::DatabaseConnection) -> Vec<String> {
    strings(
        db,
        "SELECT name AS v FROM sqlite_master WHERE type = 'index' AND tbl_name = 'products_sku' \
         AND sql IS NOT NULL ORDER BY name",
    )
    .await
}

const SEED: &[&str] = &[
    "INSERT INTO products_category (id, tenant_id, code, name, is_default, sort_order, status, \
     version, created_at, updated_at) VALUES ('c1','t1','old','Old',0,0,'retired',2, \
     '2026-10-03T00:00:00Z','2026-10-03T00:00:00Z')",
    "INSERT INTO products_sku (id, tenant_id, code, name, type, description, sellable, \
     lifecycle, revision, published_version, type_change_pending, retire_pending, created_by, \
     created_at, updated_at) VALUES ('s1','t1','gone','Gone','recurring','',1,'retired',3,1,0,0, \
     'a1','2026-10-03T00:00:00Z','2026-10-03T00:00:00Z')",
];

/// Up adds the two columns to both tables and the partial index; the rows stay unarchived. A mark
/// can then be written. Down takes the columns and the index away, the rows stay, and up runs
/// again.
#[tokio::test]
async fn the_mark_is_added_written_and_taken_away_on_sqlite() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = prior(&db).await;
    for sql in SEED {
        exec(&db, sql).await;
    }
    let sku_before = columns(&db, "products_sku").await;
    let category_before = columns(&db, "products_category").await;
    let indexes_before = indexes(&db).await;
    Migration.up(&manager).await.unwrap();
    for table in ["products_sku", "products_category"] {
        let cols = columns(&db, table).await;
        for name in ["archived_at", "archived_by"] {
            assert!(cols.iter().any(|c| c == name), "{table}: {cols:?}");
        }
    }
    assert!(
        indexes(&db)
            .await
            .iter()
            .any(|name| name == "ix_products_sku_unarchived"),
        "the default list's partial index"
    );
    assert_eq!(
        strings(
            &db,
            "SELECT id AS v FROM products_sku WHERE archived_at IS NULL AND archived_by IS NULL"
        )
        .await,
        ["s1"]
    );
    exec(
        &db,
        "UPDATE products_sku SET archived_at = '2026-10-03T01:00:00Z', archived_by = 'a1' \
         WHERE id = 's1'",
    )
    .await;
    exec(
        &db,
        "UPDATE products_category SET archived_at = '2026-10-03T01:00:00Z', archived_by = 'a1' \
         WHERE id = 'c1'",
    )
    .await;
    Migration.down(&manager).await.unwrap();
    assert_eq!(columns(&db, "products_sku").await, sku_before);
    assert_eq!(columns(&db, "products_category").await, category_before);
    assert_eq!(indexes(&db).await, indexes_before);
    assert_eq!(
        strings(&db, "SELECT id AS v FROM products_sku").await,
        ["s1"]
    );
    assert_eq!(
        strings(&db, "SELECT id AS v FROM products_category").await,
        ["c1"]
    );
    Migration.up(&manager).await.unwrap();
    assert!(
        columns(&db, "products_category")
            .await
            .iter()
            .any(|c| c == "archived_at")
    );
}
