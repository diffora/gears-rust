//! D-373 replaces the descriptor entity with row fields and revision extensions.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod descriptor_grain_support;
use sea_orm::{Database, DatabaseConnection};
async fn empty() -> DatabaseConnection {
    Database::connect("sqlite::memory:").await.expect("SQLite")
}
#[tokio::test]
async fn consensus_backfill_preserves_all_rows_extensions_and_frozen_guards() {
    descriptor_grain_support::consensus_backfills_all_rows_and_preserves_revision_extensions(
        &empty().await,
    )
    .await;
}
#[tokio::test]
async fn conflicting_revision_descriptors_refuse_without_partial_schema_changes() {
    descriptor_grain_support::conflicting_revisions_refuse_atomically(&empty().await, false).await;
}
#[tokio::test]
async fn an_absent_revision_descriptor_is_part_of_the_consensus() {
    descriptor_grain_support::conflicting_revisions_refuse_atomically(&empty().await, true).await;
}
#[tokio::test]
async fn non_draft_rows_without_descriptors_cannot_acquire_invented_snapshot_values() {
    descriptor_grain_support::published_rows_without_descriptors_refuse(&empty().await).await;
}

#[tokio::test]
async fn late_migration_failure_restores_previous_schema_data_and_guards() {
    descriptor_grain_support::a_late_failure_rolls_back_columns_data_and_guards(&empty().await)
        .await;
}
