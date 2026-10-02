//! D-484 on Postgres: the same seeded shapes as `plan_summary.rs`, in schema `bss`.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod pg_support;

use bss_pricing::infra::storage::{entity::plan as plan_e, migrations::Migrator};
use bss_pricing::module::BssPricingGear;
use pg_support::Pg;
use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};
use sea_orm_migration::MigratorTrait;
use toolkit::contracts::DatabaseCapability;
use uuid::Uuid;

const MIGRATION: &str = "m20261002_000020_plan_summary";

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

fn q(id: Uuid) -> String {
    format!("'{id}'")
}

#[tokio::test]
#[ignore = "needs the Postgres harness"]
#[allow(
    clippy::disallowed_methods,
    reason = "the backfill proof reads every plan after the migration, which has no caller scope"
)]
#[allow(
    clippy::too_many_lines,
    reason = "the six seeded shapes and the check refusal stay in one ignored Postgres twin"
)]
async fn postgres_the_backfill_fills_every_seeded_shape_and_the_checks_pair_the_columns() {
    let pg = Pg::empty().await;
    let prior = BssPricingGear::default()
        .migrations()
        .into_iter()
        .filter(|m| m.name() != MIGRATION)
        .collect();
    toolkit_db::migration_runner::run_migrations_for_testing(&pg.db().await, prior)
        .await
        .unwrap();
    let tenant = Uuid::from_u128(0x10);
    let author = Uuid::from_u128(0x11);
    let eur = Uuid::from_u128(0x20);
    let usd = Uuid::from_u128(0x21);
    for (id, code, currency) in [(eur, "eur", "EUR"), (usd, "usd", "USD")] {
        exec(
            &pg,
            &format!(
                "INSERT INTO bss.pricing_price_book (id,tenant_id,code,name,currency,version,created_at,updated_at) \
                 VALUES ({},{},'{code}','{code}','{currency}',1,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
                q(id),
                q(tenant)
            ),
        )
        .await;
    }
    let plan_sql = |id: Uuid, code: &str, updated: &str| {
        format!(
            "INSERT INTO bss.pricing_plan (id,tenant_id,code,name,published_rev,version,created_by,created_at,updated_at) \
             VALUES ({},{},'{code}','{code}',NULL,1,{},'2026-01-01T00:00:00Z','{updated}')",
            q(id),
            q(tenant),
            q(author)
        )
    };
    let revision = |id, plan, book, no, state: &str, from: &str, updated: &str| {
        let from_sql = if from.is_empty() {
            "NULL".to_owned()
        } else {
            format!("'{from}'")
        };
        format!(
            "INSERT INTO bss.pricing_plan_revision (id,tenant_id,plan_id,rev_no,book_id,state,available_from,version,created_by,created_at,updated_at) \
             VALUES ({},{},{},{no},{},'{state}',{from_sql},1,{},'2026-01-01T00:00:00Z','{updated}')",
            q(id),
            q(tenant),
            q(plan),
            q(book),
            q(author)
        )
    };
    let none = Uuid::from_u128(0x30);
    let draft_only = Uuid::from_u128(0x31);
    let beside = Uuid::from_u128(0x32);
    let future = Uuid::from_u128(0x33);
    let due = Uuid::from_u128(0x34);
    let history = Uuid::from_u128(0x35);
    for (id, code, updated) in [
        (none, "NONE", "2026-01-02T00:00:00Z"),
        (draft_only, "DRAFT", "2026-01-03T00:00:00Z"),
        (beside, "BESIDE", "2026-01-04T00:00:00Z"),
        (future, "FUTURE", "2026-01-05T00:00:00Z"),
        (due, "DUE", "2026-01-06T00:00:00Z"),
        (history, "HISTORY", "2026-01-07T00:00:00Z"),
    ] {
        exec(&pg, &plan_sql(id, code, updated)).await;
    }
    exec(
        &pg,
        &revision(
            Uuid::from_u128(0x41),
            draft_only,
            eur,
            1,
            "draft",
            "",
            "2026-02-01T00:00:00Z",
        ),
    )
    .await;
    exec(
        &pg,
        &revision(
            Uuid::from_u128(0x42),
            beside,
            eur,
            1,
            "published",
            "",
            "2026-02-02T00:00:00Z",
        ),
    )
    .await;
    exec(
        &pg,
        &revision(
            Uuid::from_u128(0x43),
            beside,
            usd,
            2,
            "draft",
            "",
            "2026-03-01T00:00:00Z",
        ),
    )
    .await;
    exec(
        &pg,
        &revision(
            Uuid::from_u128(0x44),
            future,
            eur,
            1,
            "published",
            "",
            "2026-02-03T00:00:00Z",
        ),
    )
    .await;
    exec(
        &pg,
        &revision(
            Uuid::from_u128(0x45),
            future,
            usd,
            2,
            "scheduled",
            "2026-12-01",
            "2026-03-02T00:00:00Z",
        ),
    )
    .await;
    exec(
        &pg,
        &revision(
            Uuid::from_u128(0x46),
            due,
            eur,
            1,
            "published",
            "",
            "2026-02-04T00:00:00Z",
        ),
    )
    .await;
    exec(
        &pg,
        &revision(
            Uuid::from_u128(0x47),
            due,
            usd,
            2,
            "scheduled",
            "2020-01-01",
            "2026-03-03T00:00:00Z",
        ),
    )
    .await;
    exec(
        &pg,
        &revision(
            Uuid::from_u128(0x48),
            history,
            eur,
            1,
            "superseded",
            "",
            "2026-04-01T00:00:00Z",
        ),
    )
    .await;
    exec(
        &pg,
        &revision(
            Uuid::from_u128(0x49),
            history,
            usd,
            2,
            "published",
            "",
            "2026-02-05T00:00:00Z",
        ),
    )
    .await;
    let applied = toolkit_db::migration_runner::run_migrations_for_testing(
        &pg.db().await,
        BssPricingGear::default().migrations(),
    )
    .await
    .unwrap();
    assert_eq!(applied.applied_names, [MIGRATION.to_owned()]);
    let conn = sea_orm::Database::connect(pg.url(true)).await.unwrap();
    let rows = plan_e::Entity::find().all(&conn).await.unwrap();
    let row = |code: &str| rows.iter().find(|p| p.code == code).unwrap();
    assert!(row("NONE").work_revision_id.is_none());
    assert_eq!(row("DRAFT").work_state.as_deref(), Some("draft"));
    assert_eq!(row("DRAFT").current_currency.as_deref(), Some("EUR"));
    assert_eq!(row("BESIDE").current_book_id, Some(usd));
    assert_eq!(
        row("FUTURE")
            .scheduled_from
            .map(|d| d.to_string())
            .as_deref(),
        Some("2026-12-01")
    );
    assert_eq!(
        row("DUE").scheduled_from.map(|d| d.to_string()).as_deref(),
        Some("2020-01-01")
    );
    assert_eq!(
        row("HISTORY").published_revision_id,
        Some(Uuid::from_u128(0x49))
    );
    assert!(row("HISTORY").last_activity_at > row("HISTORY").updated_at);
    let refused = pg
        .raw()
        .await
        .execute_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!(
                "UPDATE bss.pricing_plan SET work_state = 'draft' WHERE id = {}",
                q(none)
            ),
        ))
        .await;
    assert!(refused.is_err(), "a state without its revision");
    let raw = pg.raw().await;
    let manager = sea_orm_migration::SchemaManager::new(&raw);
    let step = Migrator::migrations()
        .into_iter()
        .find(|m| m.name() == MIGRATION)
        .unwrap();
    let down = step.down(&manager).await.unwrap_err().to_string();
    assert!(
        down.contains(MIGRATION) && down.contains("irreversible"),
        "{down}"
    );
}
