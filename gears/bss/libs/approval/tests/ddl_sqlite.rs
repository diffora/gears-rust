#![allow(clippy::expect_used, clippy::unwrap_used)]
use bss_approval::ddl::{apply_down, apply_up, up};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sea_orm_migration::SchemaManager;

#[tokio::test]
async fn the_template_creates_four_tables_and_its_keys_hold_on_sqlite() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    apply_up(&manager, "products_", Some("ignored_on_sqlite"))
        .await
        .unwrap();
    apply_up(&manager, "products_", Some("ignored_on_sqlite"))
        .await
        .unwrap(); // idempotent: IF NOT EXISTS everywhere
    let names: Vec<String> = db.query_all_raw(Statement::from_string(DbBackend::Sqlite,
        "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'products_approval_%' ORDER BY name".to_owned())).await.unwrap()
        .iter().map(|r| r.try_get::<String>("", "name").unwrap()).collect();
    assert_eq!(
        names,
        [
            "products_approval_decision",
            "products_approval_policy",
            "products_approval_unit",
            "products_approval_unit_item"
        ]
    );
    let unit = "INSERT INTO products_approval_unit (id, tenant_id, kind, ref_type, ref_id, state, quorum_required, generation, submitted_by, submitted_at, snapshot, snapshot_hash, version) VALUES ('u1','t1','sku_publish','sku','s1','pending',1,1,'a0','2026-09-24T10:00:00Z','{}','h',1)";
    db.execute_raw(Statement::from_string(DbBackend::Sqlite, unit.to_owned()))
        .await
        .unwrap();
    let vote = "INSERT INTO products_approval_decision (unit_id, tenant_id, actor, generation, decision, note, at, stale) VALUES ('u1','t1','a1',1,'approve',NULL,'2026-09-24T10:05:00Z',0)";
    db.execute_raw(Statement::from_string(DbBackend::Sqlite, vote.to_owned()))
        .await
        .unwrap();
    assert!(
        db.execute_raw(Statement::from_string(DbBackend::Sqlite, vote.to_owned()))
            .await
            .is_err(),
        "one vote per actor per generation"
    );
    let orphan = vote.replace("'u1'", "'missing'");
    assert!(
        db.execute_raw(Statement::from_string(DbBackend::Sqlite, orphan))
            .await
            .is_err(),
        "decisions require a unit"
    );
    let item = "INSERT INTO products_approval_unit_item (unit_id, tenant_id, item_type, item_id, created_by, after_json) VALUES ('u1','t1','sku','s1','a0','{}')";
    db.execute_raw(Statement::from_string(DbBackend::Sqlite, item.to_owned()))
        .await
        .unwrap();
    assert!(
        db.execute_raw(Statement::from_string(DbBackend::Sqlite, item.to_owned()))
            .await
            .is_err(),
        "unique item within the unit"
    );
    assert!(
        db.execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            item.replace("'u1'", "'missing'")
        ))
        .await
        .is_err(),
        "items require a unit"
    );
    let next_gen = vote.replace("'a1',1,", "'a1',2,"); // same actor, generation 2
    db.execute_raw(Statement::from_string(DbBackend::Sqlite, next_gen))
        .await
        .unwrap(); // the same actor may vote again in generation 2
    apply_down(&manager, "products_", Some("ignored_on_sqlite"))
        .await
        .unwrap();
    assert!(!manager.has_table("products_approval_unit").await.unwrap());
}

#[test]
fn the_postgres_template_carries_the_schema_and_the_queue_index() {
    let pg = up("pricing_", Some("bss"), DbBackend::Postgres).join("\n");
    assert!(pg.contains("CREATE TABLE IF NOT EXISTS bss.pricing_approval_unit"));
    assert!(pg.contains("CREATE INDEX IF NOT EXISTS ix_pricing_approval_unit_queue ON bss.pricing_approval_unit USING btree (tenant_id, state, kind, submitted_at)"));
    assert!(pg.contains("CHECK (state IN ('pending','approved','rejected','withdrawn'))"));
    assert!(pg.contains("PRIMARY KEY (unit_id, actor, generation)"));
}
