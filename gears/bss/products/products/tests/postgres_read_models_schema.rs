//! The read projection's schema oracle on Postgres (`design/08` §4; P-D-161):
//! the eight read tables' rosters — the two of `m20260901_000023/24` by name,
//! the six of `m20260901_000029` by name and nullability — pinned as literals
//! computed from the migrations' own DDL, so a drift between the two engines'
//! statements fails here rather than at the first projected row. The `SQLite`
//! tier pins the same two rosters in `migrations_tests`; this is the other
//! engine's half, with its own perturbation case.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use pg_support::Pg;
use sea_orm::{ConnectionTrait, FromQueryResult as _, Statement};

#[derive(Debug, sea_orm::FromQueryResult)]
struct ColumnRow {
    column_name: String,
    is_nullable: String,
}

async fn roster(conn: &impl ConnectionTrait, table: &str) -> Vec<(String, bool)> {
    let rows = ColumnRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT column_name, is_nullable FROM information_schema.columns \
             WHERE table_schema = 'bss' AND table_name = '{table}' ORDER BY column_name"
        ),
    ))
    .all(conn)
    .await
    .expect("information_schema answers");
    rows.into_iter()
        .map(|row| (row.column_name, row.is_nullable == "YES"))
        .collect()
}

fn names(rows: &[(String, bool)]) -> Vec<&str> {
    rows.iter().map(|(n, _)| n.as_str()).collect()
}

fn golden(rows: &[(&str, bool)]) -> Vec<(String, bool)> {
    rows.iter()
        .map(|(name, nullable)| ((*name).to_owned(), *nullable))
        .collect()
}

/// `products_read_entity`, by name — `inst-ps-shape`'s list, the same roster
/// the `SQLite` oracle pins.
const READ_ENTITY: &[&str] = &[
    "brand_scope",
    "category_paths",
    "composition_pending",
    "deprecated",
    "deprecation_provenance",
    "display_attributes",
    "entity_code",
    "entity_id",
    "entity_kind",
    "generation",
    "lifecycle_state",
    "metering_unit",
    "name",
    "plan_tier_label",
    "projected_at",
    "published_version",
    "region_scope",
    "replaced_by_sku_id",
    "sellable",
    "sku_type",
    "tenant_id",
];

const READ_STAMP: &[&str] = &["catalog_version_id", "projected_at", "tenant_id"];

const READ_INBOX: &[(&str, bool)] = &[
    ("actor_ref", false),
    ("aggregate_id", false),
    ("created_at", false),
    ("inbox_id", false),
    ("partition", false),
    ("payload", false),
    ("payload_type", false),
    ("tenant_id", false),
];

const READ_CHECKPOINT: &[(&str, bool)] = &[
    ("generation", false),
    ("inbox_id", false),
    ("tenant_id", false),
    ("updated_at", false),
];

const READ_POISON: &[(&str, bool)] = &[
    ("attempts", false),
    ("inbox_id", false),
    ("last_error", false),
    ("parked_at", false),
    ("payload_type", false),
    ("released_at", true),
    ("tenant_id", false),
];

const READ_DEFERRED_INTENT: &[(&str, bool)] = &[
    ("age_secs", false),
    ("cascade_ref", false),
    ("children_count", false),
    ("created_at", false),
    ("polled_at", false),
    ("product_id", false),
    ("tenant_id", false),
];

const READ_FREEZE_STATUS: &[(&str, bool)] = &[
    ("acked", false),
    ("catalog_version_id", false),
    ("forced", false),
    ("freeze_state", false),
    ("pending", false),
    ("polled_at", false),
    ("published_at", false),
    ("released", false),
    ("tenant_id", false),
];

const READ_DELIVERY_STATE: &[(&str, bool)] = &[
    ("inbox_pending", false),
    ("oldest_pending_age_secs", false),
    ("parked", false),
    ("polled_at", false),
    ("tenant_id", false),
];

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_read_projection_rosters_match_on_postgres() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    assert_eq!(
        names(&roster(&conn, "products_read_entity").await),
        READ_ENTITY
    );
    assert_eq!(
        names(&roster(&conn, "products_read_stamp").await),
        READ_STAMP
    );
    for (table, expected) in [
        ("products_read_inbox", READ_INBOX),
        ("products_read_checkpoint", READ_CHECKPOINT),
        ("products_read_poison", READ_POISON),
        ("products_read_deferred_intent", READ_DEFERRED_INTENT),
        ("products_read_freeze_status", READ_FREEZE_STATUS),
        ("products_read_delivery_state", READ_DELIVERY_STATE),
    ] {
        assert_eq!(roster(&conn, table).await, golden(expected), "{table}");
    }
}

/// The perturbation: the oracle can fail — a wrong table reads empty, a
/// golden with one flipped nullability does not match, and two different
/// tables never compare equal.
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn the_read_projection_oracle_can_fail() {
    let pg = Pg::applied().await;
    let conn = pg.raw().await;
    assert!(roster(&conn, "products_read_nope").await.is_empty());
    let mut flipped = golden(READ_POISON);
    flipped.retain(|(n, _)| n != "released_at");
    flipped.push(("released_at".to_owned(), false));
    flipped.sort();
    assert_ne!(roster(&conn, "products_read_poison").await, flipped);
    assert_ne!(
        roster(&conn, "products_read_checkpoint").await,
        roster(&conn, "products_read_delivery_state").await
    );
}
