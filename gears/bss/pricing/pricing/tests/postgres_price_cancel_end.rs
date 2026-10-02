//! D-520 on Postgres: 000022 adds the cancel and end columns, widens `state`, and reverses.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod pg_support;

use bss_pricing::module::BssPricingGear;
use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::SchemaManager;
use toolkit::contracts::DatabaseCapability;
use uuid::Uuid;

const MIGRATION: &str = "m20261003_000022_price_cancel_and_end";
const TENANT: Uuid = Uuid::from_u128(0x22);
const BOOK: Uuid = Uuid::from_u128(0x2201);
const ENTRY: Uuid = Uuid::from_u128(0x2202);
const PRICE_A: Uuid = Uuid::from_u128(0x2210);
const PRICE_B: Uuid = Uuid::from_u128(0x2211);
const PAIR_A: Uuid = Uuid::from_u128(0x2220);
const PAIR_B: Uuid = Uuid::from_u128(0x2221);
const AUTHOR: Uuid = Uuid::from_u128(0x2230);

fn q(id: Uuid) -> String {
    format!("'{id}'")
}

async fn exec(pg: &Pg, sql: &str) {
    pg.raw()
        .await
        .execute_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            sql.to_owned(),
        ))
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn try_exec(pg: &Pg, sql: &str) -> Result<(), sea_orm::DbErr> {
    pg.raw()
        .await
        .execute_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            sql.to_owned(),
        ))
        .await
        .map(|_| ())
}

async fn strings(pg: &Pg, sql: &str) -> Vec<String> {
    pg.raw()
        .await
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            sql.to_owned(),
        ))
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .iter()
        .map(|row| row.try_get::<String>("", "v").unwrap())
        .collect()
}

async fn prior(pg: &Pg) {
    let chain = BssPricingGear::default()
        .migrations()
        .into_iter()
        .filter(|m| m.name() != MIGRATION)
        .collect();
    toolkit_db::migration_runner::run_migrations_for_testing(&pg.db().await, chain)
        .await
        .unwrap();
}

async fn step(pg: &Pg, down: bool) {
    let migration = BssPricingGear::default()
        .migrations()
        .into_iter()
        .find(|m| m.name() == MIGRATION)
        .expect("000022 is in the chain");
    let conn = pg.raw().await;
    let manager = SchemaManager::new(&conn);
    if down {
        migration.down(&manager).await.unwrap();
    } else {
        migration.up(&manager).await.unwrap();
    }
}

fn seed() -> Vec<String> {
    let price = |id, version, from, state| {
        format!(
            "INSERT INTO bss.pricing_price (id,tenant_id,price_book_entry_id,version_no,price_json,eligibility,effective_from,state,created_by,version,created_at,updated_at) VALUES ({},{},{},{version},'{{}}','all','{from}','{state}',{},1,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
            q(id),
            q(TENANT),
            q(ENTRY),
            q(AUTHOR)
        )
    };
    vec![
        format!(
            "INSERT INTO bss.pricing_price_book (id,tenant_id,code,name,currency,version,created_at,updated_at) VALUES ({},{},'eur','eur','EUR',1,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
            q(BOOK),
            q(TENANT)
        ),
        format!(
            "INSERT INTO bss.pricing_price_book_entry (id,tenant_id,book_id,sku_id,charge_kind,period,reservation_id,reference_state,version,created_at,updated_at,model) VALUES ({},{},{},{},'recurring','month',{},'confirmed',1,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','flat')",
            q(ENTRY),
            q(TENANT),
            q(BOOK),
            q(Uuid::from_u128(0x2240)),
            q(Uuid::from_u128(0x2241))
        ),
        price(PRICE_A, 1, "2026-01-01", "approved"),
        price(PRICE_B, 2, "2026-03-01", "approved"),
        price(PAIR_A, 3, "2026-06-01", "draft"),
        price(PAIR_B, 4, "2026-07-01", "draft"),
        format!(
            "UPDATE bss.pricing_price SET paired_price_id = {} WHERE id = {}",
            q(PAIR_B),
            q(PAIR_A)
        ),
        format!(
            "UPDATE bss.pricing_price SET paired_price_id = {} WHERE id = {}",
            q(PAIR_A),
            q(PAIR_B)
        ),
    ]
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
async fn postgres_adds_cancel_and_end_and_round_trips() {
    let pg = Pg::empty().await;
    prior(&pg).await;
    for sql in seed() {
        exec(&pg, &sql).await;
    }
    step(&pg, false).await;
    let cols = strings(
        &pg,
        "SELECT column_name::text AS v FROM information_schema.columns WHERE table_schema = 'bss' AND table_name = 'pricing_price'",
    )
    .await;
    for name in ["change_kind", "target_price_id", "cancelled_by_unit_id"] {
        assert!(cols.iter().any(|c| c == name), "{cols:?}");
    }
    let kinds = strings(
        &pg,
        "SELECT change_kind AS v FROM bss.pricing_price ORDER BY version_no",
    )
    .await;
    assert_eq!(kinds, vec!["set", "set", "set", "set"]);
    let chain = strings(
        &pg,
        "SELECT effective_from::text AS v FROM bss.pricing_price WHERE state = 'approved' ORDER BY version_no",
    )
    .await;
    assert_eq!(chain, vec!["2026-01-01", "2026-03-01"]);
    let pair = strings(
        &pg,
        &format!(
            "SELECT paired_price_id::text AS v FROM bss.pricing_price WHERE id = {}",
            q(PAIR_A)
        ),
    )
    .await;
    assert_eq!(pair, vec![PAIR_B.to_string()]);
    exec(
        &pg,
        &format!(
            "UPDATE bss.pricing_price SET state = 'cancelled' WHERE id = {}",
            q(PRICE_B)
        ),
    )
    .await;
    assert!(
        try_exec(
            &pg,
            &format!(
                "UPDATE bss.pricing_price SET state = 'retired' WHERE id = {}",
                q(PRICE_A)
            ),
        )
        .await
        .is_err()
    );
    assert!(
        try_exec(
            &pg,
            &format!(
                "UPDATE bss.pricing_price SET change_kind = 'move' WHERE id = {}",
                q(PRICE_A)
            ),
        )
        .await
        .is_err()
    );
    exec(
        &pg,
        &format!(
            "UPDATE bss.pricing_price SET state = 'approved', change_kind = 'end', target_price_id = {} WHERE id = {}",
            q(PRICE_A),
            q(PRICE_B)
        ),
    )
    .await;
    step(&pg, true).await;
    let restored = strings(
        &pg,
        "SELECT column_name::text AS v FROM information_schema.columns WHERE table_schema = 'bss' AND table_name = 'pricing_price'",
    )
    .await;
    for name in ["change_kind", "target_price_id", "cancelled_by_unit_id"] {
        assert!(!restored.iter().any(|c| c == name), "{restored:?}");
    }
    let still = strings(
        &pg,
        "SELECT effective_from::text AS v FROM bss.pricing_price WHERE state = 'approved' ORDER BY version_no",
    )
    .await;
    assert_eq!(still, vec!["2026-01-01", "2026-03-01"]);
    assert!(
        try_exec(
            &pg,
            &format!(
                "UPDATE bss.pricing_price SET state = 'cancelled' WHERE id = {}",
                q(PRICE_B)
            ),
        )
        .await
        .is_err()
    );
    step(&pg, false).await;
    let again = strings(
        &pg,
        "SELECT column_name::text AS v FROM information_schema.columns WHERE table_schema = 'bss' AND table_name = 'pricing_price' AND column_name = 'change_kind'",
    )
    .await;
    assert_eq!(again, vec!["change_kind"]);
}
