//! Persistence probes for the staleness stamp host — every obligation in
//! `dod-staleness-stamp` that a domain-only test cannot arm against the
//! table.
#![allow(clippy::expect_used)]

use chrono::{TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    NewReadEntity, apply_read_stamp, count_read_entities, delete_read_entity, insert_read_entity,
    load_read_stamp,
};
use crate::domain::read_model::{
    StampApply, StampCatalogTouch, completeness_rejects_removal, floor_admits_removal,
};
use crate::infra::storage::migrations::Migrator;

const TENANT: Uuid = Uuid::from_u128(0x7e_42);
const ENTITY: Uuid = Uuid::from_u128(0xdd_11);
const VERSION: i64 = 0xee_11;

async fn harness() -> DBProvider<DbError> {
    let opts = ConnectOpts {
        max_conns: Some(1),
        min_conns: Some(1),
        ..Default::default()
    };
    let db = connect_db("sqlite::memory:", opts)
        .await
        .expect("connect in-memory sqlite");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("boot the migration chain");
    DBProvider::<DbError>::new(db)
}

/// A zero-version tenant's first apply stamps `null` with a real
/// `projectedAt`, and a load reads both halves back — absence of the
/// version field would be indistinguishable from a dropped stamp.
#[tokio::test]
async fn a_zero_version_tenant_persists_null_with_a_projected_at() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let at = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();

    let written = apply_read_stamp(
        &conn,
        &scope,
        TENANT,
        StampApply {
            catalog: StampCatalogTouch::Anchorless,
            projected_at: at,
            entities_projected: true,
        },
    )
    .await
    .expect("bootstrap apply");
    assert_eq!(written.as_of_catalog_version, None);
    assert_eq!(written.projected_at, at);

    let loaded = load_read_stamp(&conn, &scope, TENANT)
        .await
        .expect("load")
        .expect("the bootstrap left a row");
    assert_eq!(loaded.as_of_catalog_version, None);
    assert_eq!(loaded.projected_at, at);
}

/// Ordering against the table: stamping a catalog version before the
/// changed-entity list is projected is refused, and no row is written.
#[tokio::test]
async fn a_premature_catalog_stamp_writes_nothing() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let at = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();

    let err = apply_read_stamp(
        &conn,
        &scope,
        TENANT,
        StampApply {
            catalog: StampCatalogTouch::Set(VERSION),
            projected_at: at,
            entities_projected: false,
        },
    )
    .await
    .expect_err("must refuse before entities are projected");
    assert!(
        err.to_string().contains("entities not yet projected"),
        "got {err}"
    );
    assert!(
        load_read_stamp(&conn, &scope, TENANT)
            .await
            .expect("load")
            .is_none(),
        "a refused advance must leave no stamp row"
    );
}

/// Floor vs completeness, armed on a **removal** that hits the tables: a
/// retirement deletes a serving row, the stamp's catalog version stays, and
/// `projected_at` advances. Completeness would alarm; the floor admits it.
#[tokio::test]
async fn a_retirement_removal_advances_projected_at_without_moving_the_version() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let t0 = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 1).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 2).unwrap();

    apply_read_stamp(
        &conn,
        &scope,
        TENANT,
        StampApply {
            catalog: StampCatalogTouch::Set(VERSION),
            projected_at: t0,
            entities_projected: true,
        },
    )
    .await
    .expect("seed the stamp at a catalog version");
    insert_read_entity(
        &conn,
        &scope,
        NewReadEntity {
            tenant_id: TENANT,
            entity_kind: "sku".to_owned(),
            entity_id: ENTITY,
            name: "Fibre 500".to_owned(),
            lifecycle_state: "published".to_owned(),
            published_version: 1,
            projected_at: t0,
        },
    )
    .await
    .expect("seed one serving row");
    assert_eq!(count_read_entities(&conn, &scope, TENANT).await.unwrap(), 1);

    // The retirement flip: content goes, catalog version does not.
    assert_eq!(
        delete_read_entity(&conn, &scope, TENANT, "sku", ENTITY)
            .await
            .expect("remove"),
        1
    );
    let before = load_read_stamp(&conn, &scope, TENANT)
        .await
        .expect("load")
        .expect("stamp");
    let after = apply_read_stamp(
        &conn,
        &scope,
        TENANT,
        StampApply {
            catalog: StampCatalogTouch::Unchanged,
            projected_at: t1,
            entities_projected: true,
        },
    )
    .await
    .expect("the retirement apply still stamps");

    let rows_after = count_read_entities(&conn, &scope, TENANT).await.unwrap();
    assert_eq!(rows_after, 0);
    assert_eq!(after.as_of_catalog_version, Some(VERSION));
    assert_eq!(after.projected_at, t1);
    assert!(completeness_rejects_removal(1, rows_after, true));
    assert!(floor_admits_removal(1, rows_after, &before, &after));

    // And a later apply still advances projected_at with the version held.
    let later = apply_read_stamp(
        &conn,
        &scope,
        TENANT,
        StampApply {
            catalog: StampCatalogTouch::Unchanged,
            projected_at: t2,
            entities_projected: true,
        },
    )
    .await
    .expect("version-or-none apply");
    assert_eq!(later.as_of_catalog_version, Some(VERSION));
    assert_eq!(later.projected_at, t2);
}
