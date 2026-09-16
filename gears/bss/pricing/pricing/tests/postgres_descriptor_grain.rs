//! The D-373 migration executes the same backfill/refusal contract on Postgres.
mod descriptor_grain_support;
mod pg_support;
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn consensus_backfill_preserves_all_rows_extensions_and_frozen_guards() {
    descriptor_grain_support::consensus_backfills_all_rows_and_preserves_revision_extensions(
        &pg_support::Pg::empty().await.raw().await,
    )
    .await;
}
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn conflicting_revision_descriptors_refuse_without_partial_schema_changes() {
    descriptor_grain_support::conflicting_revisions_refuse_atomically(
        &pg_support::Pg::empty().await.raw().await,
        false,
    )
    .await;
}
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn an_absent_revision_descriptor_is_part_of_the_consensus() {
    descriptor_grain_support::conflicting_revisions_refuse_atomically(
        &pg_support::Pg::empty().await.raw().await,
        true,
    )
    .await;
}
#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn non_draft_rows_without_descriptors_cannot_acquire_invented_snapshot_values() {
    descriptor_grain_support::published_rows_without_descriptors_refuse(
        &pg_support::Pg::empty().await.raw().await,
    )
    .await;
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers)"]
async fn late_migration_failure_restores_previous_schema_data_and_guards() {
    descriptor_grain_support::a_late_failure_rolls_back_columns_data_and_guards(
        &pg_support::Pg::empty().await.raw().await,
    )
    .await;
}
