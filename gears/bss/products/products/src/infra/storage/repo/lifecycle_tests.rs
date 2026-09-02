//! Insert-and-readback probes for the two lifecycle stores.
#![allow(clippy::expect_used)]

use chrono::{TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::{
    NewDeferredRetirement, NewScheduledTransition, claim_due_transition, find_deferred_retirement,
    find_live_retire_intents, find_scheduled_transition, finish_scheduled_transition,
    insert_deferred_retirement, insert_scheduled_transition, reclaim_expired_lease,
    resolve_deferred_retirement, supersede_live_intents,
};
use crate::domain::activation::{ClaimLease, DeferralPopulation, RunFinish};
use crate::infra::storage::migrations::Migrator;

const TENANT: Uuid = Uuid::from_u128(0x7e_42);
const ENTITY: Uuid = Uuid::from_u128(0xdd_11);
const TRANSITION: Uuid = Uuid::from_u128(0xaa_01);
const APPROVAL: Uuid = Uuid::from_u128(0xbb_02);
const ACTOR: Uuid = Uuid::from_u128(0xcc_03);
const PRODUCT: Uuid = Uuid::from_u128(0xdd_04);

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

#[tokio::test]
async fn a_scheduled_transition_round_trips_with_separate_reason_columns() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let at = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();

    insert_scheduled_transition(
        &conn,
        &scope,
        &NewScheduledTransition {
            transition_id: TRANSITION,
            tenant_id: TENANT,
            entity_kind: "sku".to_owned(),
            entity_id: ENTITY,
            kind: "retire".to_owned(),
            at,
            approval_ref: APPROVAL,
            retirement_reason: Some("end of sale".to_owned()),
            now,
        },
    )
    .await
    .expect("insert");

    let loaded = find_scheduled_transition(&conn, &scope, TENANT, TRANSITION)
        .await
        .expect("find")
        .expect("row exists");
    assert_eq!(loaded.state, "pending");
    assert_eq!(loaded.attempt, 0);
    assert_eq!(loaded.retirement_reason.as_deref(), Some("end of sale"));
    assert_eq!(loaded.outcome_reason, None);
    assert_eq!(loaded.at, at);
}

#[tokio::test]
async fn a_second_live_intent_for_the_same_entity_and_kind_is_refused() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let at = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
    let row = NewScheduledTransition {
        transition_id: TRANSITION,
        tenant_id: TENANT,
        entity_kind: "sku".to_owned(),
        entity_id: ENTITY,
        kind: "retire".to_owned(),
        at,
        approval_ref: APPROVAL,
        retirement_reason: None,
        now,
    };
    insert_scheduled_transition(&conn, &scope, &row)
        .await
        .expect("first live intent");
    let mut collision = row.clone();
    collision.transition_id = Uuid::from_u128(0xaa_02);
    let err = insert_scheduled_transition(&conn, &scope, &collision)
        .await
        .expect_err("second live intent");
    let text = err.to_string().to_ascii_lowercase();
    assert!(
        text.contains("unique"),
        "the partial UNIQUE is the floor: {err}"
    );
}

#[tokio::test]
async fn a_deferred_retirement_round_trips_unresolved() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let at = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();

    insert_scheduled_transition(
        &conn,
        &scope,
        &NewScheduledTransition {
            transition_id: TRANSITION,
            tenant_id: TENANT,
            entity_kind: "product".to_owned(),
            entity_id: PRODUCT,
            kind: "retire".to_owned(),
            at,
            approval_ref: APPROVAL,
            retirement_reason: Some("cascade".to_owned()),
            now,
        },
    )
    .await
    .expect("parent intent");

    insert_deferred_retirement(
        &conn,
        &scope,
        &NewDeferredRetirement {
            tenant_id: TENANT,
            product_id: PRODUCT,
            cascade_ref: TRANSITION,
            children_snapshot: r#"[{"sku":"a","reason":"referenced"}]"#.to_owned(),
            created_by: ACTOR,
            now,
        },
    )
    .await
    .expect("insert deferral");

    let loaded = find_deferred_retirement(&conn, &scope, TENANT, PRODUCT, TRANSITION)
        .await
        .expect("find")
        .expect("row exists");
    assert_eq!(loaded.resolved_at, None);
    assert_eq!(loaded.resolution, None);
    assert!(loaded.children_snapshot.contains("referenced"));
}

#[tokio::test]
async fn a_due_row_claims_and_finishes_applied() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let at = now - chrono::Duration::hours(1);

    insert_scheduled_transition(
        &conn,
        &scope,
        &NewScheduledTransition {
            transition_id: TRANSITION,
            tenant_id: TENANT,
            entity_kind: "sku".to_owned(),
            entity_id: ENTITY,
            kind: "publish".to_owned(),
            at,
            approval_ref: APPROVAL,
            retirement_reason: None,
            now,
        },
    )
    .await
    .expect("insert");

    assert!(
        claim_due_transition(&conn, &scope, TENANT, TRANSITION, now)
            .await
            .expect("claim")
    );
    let running = find_scheduled_transition(&conn, &scope, TENANT, TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(running.state, "running");

    assert!(
        finish_scheduled_transition(&conn, &scope, TENANT, TRANSITION, &RunFinish::Applied, now)
            .await
            .expect("finish")
    );
    let done = find_scheduled_transition(&conn, &scope, TENANT, TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(done.state, "applied");
    assert_eq!(done.outcome_reason, None);
    assert_eq!(done.retirement_reason, None);
    assert_eq!(done.attempt, 0);
}

#[tokio::test]
async fn reclaim_moves_running_to_pending_and_increments_attempt() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let at = now - chrono::Duration::hours(1);
    let lease = ClaimLease {
        ttl: chrono::Duration::seconds(30),
    };

    insert_scheduled_transition(
        &conn,
        &scope,
        &NewScheduledTransition {
            transition_id: TRANSITION,
            tenant_id: TENANT,
            entity_kind: "sku".to_owned(),
            entity_id: ENTITY,
            kind: "publish".to_owned(),
            at,
            approval_ref: APPROVAL,
            retirement_reason: None,
            now,
        },
    )
    .await
    .expect("insert");
    assert!(
        claim_due_transition(&conn, &scope, TENANT, TRANSITION, now)
            .await
            .expect("claim")
    );

    let too_soon = now + chrono::Duration::seconds(5);
    assert!(
        !reclaim_expired_lease(&conn, &scope, TENANT, TRANSITION, too_soon, lease)
            .await
            .expect("lease still held")
    );

    let later = now + chrono::Duration::seconds(31);
    assert!(
        reclaim_expired_lease(&conn, &scope, TENANT, TRANSITION, later, lease)
            .await
            .expect("reclaim")
    );
    let row = find_scheduled_transition(&conn, &scope, TENANT, TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.state, "pending");
    assert_eq!(row.attempt, 1);
    assert!(row.claimed_at.is_none());
}

#[tokio::test]
async fn a_transient_deferral_finish_increments_attempt() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let at = now - chrono::Duration::hours(1);

    insert_scheduled_transition(
        &conn,
        &scope,
        &NewScheduledTransition {
            transition_id: TRANSITION,
            tenant_id: TENANT,
            entity_kind: "sku".to_owned(),
            entity_id: ENTITY,
            kind: "publish".to_owned(),
            at,
            approval_ref: APPROVAL,
            retirement_reason: None,
            now,
        },
    )
    .await
    .expect("insert");
    assert!(
        claim_due_transition(&conn, &scope, TENANT, TRANSITION, now)
            .await
            .expect("claim")
    );
    assert!(
        finish_scheduled_transition(
            &conn,
            &scope,
            TENANT,
            TRANSITION,
            &RunFinish::Deferred {
                population: DeferralPopulation::TransientDependency,
                reason: "transient: UNAVAILABLE".to_owned(),
            },
            now,
        )
        .await
        .expect("finish")
    );
    let row = find_scheduled_transition(&conn, &scope, TENANT, TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(row.state, "deferred");
    assert_eq!(row.attempt, 1);
    assert_eq!(
        row.outcome_reason.as_deref(),
        Some("transient: UNAVAILABLE")
    );
}

#[tokio::test]
async fn supersede_clears_the_live_slot_and_resolve_flips_the_deferral() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let at = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();

    insert_scheduled_transition(
        &conn,
        &scope,
        &NewScheduledTransition {
            transition_id: TRANSITION,
            tenant_id: TENANT,
            entity_kind: "product".to_owned(),
            entity_id: PRODUCT,
            kind: "retire".to_owned(),
            at,
            approval_ref: APPROVAL,
            retirement_reason: Some("cascade".to_owned()),
            now,
        },
    )
    .await
    .expect("intent");
    let live = find_live_retire_intents(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("scan");
    assert_eq!(live.len(), 1);

    assert_eq!(
        supersede_live_intents(&conn, &scope, TENANT, PRODUCT, "retire", now)
            .await
            .expect("supersede"),
        1
    );
    assert!(
        find_live_retire_intents(&conn, &scope, TENANT, PRODUCT)
            .await
            .expect("scan")
            .is_empty()
    );

    insert_deferred_retirement(
        &conn,
        &scope,
        &NewDeferredRetirement {
            tenant_id: TENANT,
            product_id: PRODUCT,
            cascade_ref: TRANSITION,
            children_snapshot: "[]".to_owned(),
            created_by: ACTOR,
            now,
        },
    )
    .await
    .expect("deferral");
    assert!(
        resolve_deferred_retirement(
            &conn,
            &scope,
            TENANT,
            PRODUCT,
            TRANSITION,
            "cascade_cancelled",
            now,
        )
        .await
        .expect("resolve")
    );
    let resolved = find_deferred_retirement(&conn, &scope, TENANT, PRODUCT, TRANSITION)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(resolved.resolution.as_deref(), Some("cascade_cancelled"));
    assert!(resolved.resolved_at.is_some());
}
