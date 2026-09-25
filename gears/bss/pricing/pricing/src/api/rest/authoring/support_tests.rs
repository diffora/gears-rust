//! Exhausted contention answers the door's 409, injected through the retry classifier.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

async fn db() -> Db {
    toolkit_db::connect_db("sqlite::memory:", toolkit_db::ConnectOpts::default())
        .await
        .unwrap()
}
fn driver(message: &str) -> DoorError {
    DoorError::Repo(RepoError::Driver {
        context: "test".into(),
        source: sea_orm::DbErr::Custom(message.into()),
    })
}
/// The body fails every attempt with `error`; answer the door's error and the attempt count.
async fn run(unit: bool, error: fn() -> DoorError) -> (CanonicalError, u32) {
    let db = db().await;
    let attempts = Arc::new(AtomicU32::new(0));
    let seen = attempts.clone();
    let result = if unit {
        unit_transaction(&db, move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Err::<(), _>(error()) })
        })
        .await
    } else {
        transaction(&db, move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Err::<(), _>(error()) })
        })
        .await
    };
    (result.unwrap_err(), attempts.load(Ordering::SeqCst))
}
fn reason(error: &CanonicalError) -> Option<String> {
    match error {
        CanonicalError::Aborted { ctx, .. } => Some(ctx.reason.clone()),
        _ => None,
    }
}
#[tokio::test]
async fn contention_that_outlasts_the_retries_is_409_contended() {
    let busy = || driver("error returned from database: (code: 5) database is locked");
    let (error, attempts) = run(false, busy).await;
    assert_eq!(
        attempts,
        toolkit_db::DEFAULT_TX_RETRY_ATTEMPTS,
        "it was retried"
    );
    assert_eq!(error.status_code(), 409);
    assert_eq!(reason(&error).as_deref(), Some("CONTENDED"));
    let (error, _) = run(true, busy).await;
    assert_eq!(error.status_code(), 409);
    assert_eq!(reason(&error).as_deref(), Some("UNIT_CONTENDED"));
}
#[tokio::test]
async fn any_other_driver_failure_stays_a_500() {
    let broken = || driver("error returned from database: (code: 1) no such table: x");
    let (error, attempts) = run(false, broken).await;
    assert_eq!(attempts, 1, "not retried");
    assert_eq!(error.status_code(), 500);
    let (error, _) = run(true, broken).await;
    assert_eq!(error.status_code(), 500);
}
#[test]
fn the_classifier_follows_the_backend() {
    let pg = || driver("could not serialize access due to concurrent update");
    assert!(matches!(
        exhausted_contention(sea_orm::DbBackend::Postgres, CONTENDED, pg()),
        DoorError::Api(_)
    ));
    assert!(matches!(
        exhausted_contention(sea_orm::DbBackend::Sqlite, CONTENDED, pg()),
        DoorError::Repo(_)
    ));
}
/// The unit doors answer `UNIT_CONTENDED`; every other mutation door keeps `CONTENDED`.
#[test]
fn every_approval_unit_door_runs_a_unit_transaction() {
    let code = crate::source_scan::blank_comments_and_literals(include_str!("approvals.rs"));
    for door in [
        "pub async fn submit_row(",
        "pub async fn publish(",
        "pub async fn vote(",
    ] {
        let start = code.find(door).unwrap();
        let rest = &code[start + door.len()..];
        let body = &rest[..rest.find("\npub async fn ").unwrap_or(rest.len())];
        assert!(
            body.contains("support::unit_transaction"),
            "{door} must run a unit transaction"
        );
        assert!(
            !body.contains("support::transaction(") && !body.contains("support::transaction_door("),
            "{door} must not run a plain transaction"
        );
    }
}
