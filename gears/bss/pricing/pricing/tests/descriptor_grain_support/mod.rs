//! The D-373 migration contract, shared by the `SQLite` and Postgres engines.
#![allow(dead_code, clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::infra::storage::migrations::Migrator;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;

const TENANT: &str = "11111111-1111-1111-1111-111111111111";
const PLAN: &str = "22222222-2222-2222-2222-222222222222";
const ACTOR: &str = "44444444-4444-4444-4444-444444444444";
const SKU: &str = "55555555-5555-5555-5555-555555555555";

fn sql(conn: &DatabaseConnection, text: &str) -> Statement {
    let qualified = if conn.get_database_backend() == DbBackend::Postgres {
        text.replace("pricing_", "bss.pricing_")
    } else {
        text.to_owned()
    };
    Statement::from_string(conn.get_database_backend(), qualified)
}
async fn run(conn: &DatabaseConnection, text: &str) {
    conn.execute_raw(sql(conn, text)).await.expect(text);
}
async fn scalar(conn: &DatabaseConnection, text: &str) -> String {
    conn.query_one_raw(sql(conn, text))
        .await
        .expect(text)
        .expect("one row")
        .try_get("", "v")
        .expect("scalar v")
}

pub async fn predecessor(conn: &DatabaseConnection) {
    let count = Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m20260916_000045_descriptor_grain")
        .expect("D373 registered");
    Migrator::up(
        conn,
        Some(u32::try_from(count).expect("migration count fits")),
    )
    .await
    .expect("pre-D373 chain");
}
async fn revision(conn: &DatabaseConnection, revision: u64, gl: Option<&str>) {
    run(conn, &format!("INSERT INTO pricing_plan (plan_id, revision, tenant_id, sku_id, lifecycle_state, created_by, created_at_utc) VALUES ('{PLAN}', {revision}, '{TENANT}', '{SKU}', 'draft', '{ACTOR}', '2026-09-16 00:00:00+00')")).await;
    if let Some(gl) = gl {
        run(conn, &format!("INSERT INTO pricing_plan_descriptor_set (plan_id, plan_revision, tenant_id, invoice_line_template, gl_code, additional_fields) VALUES ('{PLAN}', {revision}, '{TENANT}', '{{sku}}', '{gl}', '{{\"costCentre\":\"rev{revision}\"}}')")).await;
    }
    run(conn, &format!("INSERT INTO pricing_plan_phase (plan_id, plan_revision, tenant_id, phase_id, kind, ordinal) VALUES ('{PLAN}', {revision}, '{TENANT}', '33333333-3333-3333-3333-333333333333', 'evergreen', 0)")).await;
    run(conn, &format!("UPDATE pricing_plan SET lifecycle_state = 'published' WHERE plan_id = '{PLAN}' AND revision = {revision}")).await;
}
async fn price(conn: &DatabaseConnection, suffix: u64, state: &str) {
    run(conn, &format!("INSERT INTO pricing_price (price_id, plan_id, tenant_id, sku_id, phase, currency, region, charge_kind, lifecycle_state, created_by, created_at_utc) VALUES ('66666666-6666-6666-6666-{suffix:012}', '{PLAN}', '{TENANT}', '{SKU}', '33333333-3333-3333-3333-333333333333', 'USD', 'region{suffix}', 'recurring', '{state}', '{ACTOR}', '2026-09-16 00:00:00+00')")).await;
}

#[allow(
    clippy::cognitive_complexity,
    reason = "one migrated fixture checks backfill arithmetic, revision extensions, FK children and each frozen column together"
)]
pub async fn consensus_backfills_all_rows_and_preserves_revision_extensions(
    conn: &DatabaseConnection,
) {
    predecessor(conn).await;
    revision(conn, 0, Some("4000")).await;
    run(conn, &format!("UPDATE pricing_plan SET lifecycle_state = 'superseded' WHERE plan_id = '{PLAN}' AND revision = 0")).await;
    revision(conn, 1, Some("4000")).await;
    price(conn, 1, "draft").await;
    price(conn, 2, "published").await;
    price(conn, 3, "superseded").await;
    Migrator::up(conn, None).await.expect("consensus backfill");
    assert_eq!(scalar(conn, "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price WHERE gl_code_ref = '4000' AND invoice_line_template = '{sku}'").await, "3");
    assert_eq!(scalar(conn, "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price WHERE lifecycle_state <> 'draft' AND resolved_gl_code = '4000' AND resolved_invoice_line_template = '{sku}'").await, "2");
    assert_eq!(scalar(conn, "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_price WHERE lifecycle_state = 'draft' AND resolved_gl_code IS NULL AND resolved_invoice_line_template IS NULL").await, "1");
    for revision in 0..2 {
        let raw = scalar(conn, &format!("SELECT CAST(descriptor_ext AS TEXT) AS v FROM pricing_plan WHERE revision = {revision}")).await;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&raw).expect("extension JSON"),
            serde_json::json!({"costCentre": format!("rev{revision}")})
        );
    }
    assert_eq!(
        scalar(
            conn,
            "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_plan_phase"
        )
        .await,
        "2",
        "SQLite rebuild retains FK descendants"
    );
    for column in [
        "invoice_line_template",
        "gl_code_ref",
        "resolved_invoice_line_template",
        "resolved_gl_code",
    ] {
        let error = conn.execute_raw(sql(conn, &format!("UPDATE pricing_price SET {column} = 'changed' WHERE lifecycle_state = 'published'"))).await.expect_err("all descriptor columns frozen");
        assert!(error.to_string().contains("pricing_price"), "{error}");
    }
    run(conn, "UPDATE pricing_price SET invoice_line_template = '{plan}', gl_code_ref = '4100' WHERE lifecycle_state = 'draft'").await;
}

pub async fn conflicting_revisions_refuse_atomically(conn: &DatabaseConnection, missing: bool) {
    predecessor(conn).await;
    revision(conn, 0, Some("4000")).await;
    run(conn, &format!("UPDATE pricing_plan SET lifecycle_state = 'superseded' WHERE plan_id = '{PLAN}' AND revision = 0")).await;
    revision(conn, 1, if missing { None } else { Some("4100") }).await;
    price(conn, 1, "published").await;
    let error = Migrator::up(conn, None)
        .await
        .expect_err("ambiguous grain must refuse");
    let message = error.to_string();
    assert!(
        message.contains(TENANT) && message.contains(PLAN),
        "named owner: {message}"
    );
    assert_eq!(
        scalar(
            conn,
            "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_plan_descriptor_set"
        )
        .await,
        if missing { "1" } else { "2" }
    );
    assert!(
        conn.execute_raw(sql(conn, "SELECT descriptor_ext FROM pricing_plan"))
            .await
            .is_err(),
        "refusal leaves the prior schema intact"
    );
}

pub async fn published_rows_without_descriptors_refuse(conn: &DatabaseConnection) {
    predecessor(conn).await;
    revision(conn, 0, None).await;
    price(conn, 1, "published").await;
    let error = Migrator::up(conn, None)
        .await
        .expect_err("published snapshot needs descriptors");
    assert!(error.to_string().contains("descriptor"), "{error}");
}

pub async fn a_late_failure_rolls_back_columns_data_and_guards(conn: &DatabaseConnection) {
    predecessor(conn).await;
    revision(conn, 0, Some("4000")).await;
    price(conn, 1, "published").await;
    // Force failure after guards are suspended and five columns are added.
    run(
        conn,
        "ALTER TABLE pricing_policy_object ADD COLUMN default_gl_code_ref text",
    )
    .await;
    Migrator::up(conn, None)
        .await
        .expect_err("duplicate later column aborts migration");
    for query in [
        "SELECT invoice_line_template FROM pricing_price",
        "SELECT descriptor_ext FROM pricing_plan",
    ] {
        assert!(
            conn.query_all_raw(sql(conn, query)).await.is_err(),
            "new column rolled back: {query}"
        );
    }
    assert_eq!(scalar(conn, "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_plan_descriptor_set WHERE gl_code = '4000'").await, "1");
    let error = conn
        .execute_raw(sql(
            conn,
            "UPDATE pricing_price SET currency = 'EUR' WHERE lifecycle_state = 'published'",
        ))
        .await
        .expect_err("old frozen guard restored");
    assert!(error.to_string().contains("pricing_price"), "{error}");
}
