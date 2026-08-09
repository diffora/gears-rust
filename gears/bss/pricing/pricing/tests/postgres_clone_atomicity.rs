//! The clone is one act on Postgres too (D-272, D-274).
//!
//! **Why this exists as its own suite rather than as a line in the register.**
//! `tests/sqlite_clone.rs` proves the whole copy set and proves the rollback, but
//! it proves the rollback on `SQLite`, and the claim D-274 makes is *transactional*
//! — the one class of claim where the two engines are entitled to differ. `SQLite`
//! carries on after a failed statement; Postgres aborts the surrounding
//! transaction outright. A clone assembled from several writers is exactly the
//! shape where that difference decides what survives.
//!
//! The seed is deliberately the **smallest** source that reaches every layer this
//! is about — a published plan revision and one published price row — rather than
//! the sibling suite's full fixture. What is under test here is not what the clone
//! copies, which that suite settles on both engines' shared code; it is whether
//! what it wrote is still there after the caller fails. A larger fixture would add
//! rows to the same assertion, not evidence.
//!
//! The failure is the **caller's**. Every failure the clone could raise from its
//! own data is unreachable by construction: the three axes the reset changes —
//! eligibility, cohort and overlay — are each operand-free by the time a row
//! reaches the copy (D-266, D-268). This is also the route's own shape, where
//! `idempotent::guarded` owns the transaction and its bookkeeping is what fails
//! after the clone.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod pg_support;

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::clone::clone_plan_on;
use bss_pricing::infra::storage::repo::{
    NewPlanDraft, NewPriceDraft, PlanRepo, PriceRepo, plan_repo, price_repo,
};
use chrono::{DateTime, TimeZone, Utc};
use pg_support::Pg;
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_21);
const ACTOR: Uuid = Uuid::from_u128(0xac_20);
const CORRELATION: Uuid = Uuid::from_u128(0xc0_21);
const SOURCE_ROW: Uuid = Uuid::from_u128(0xb_1001);

fn source_plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x50_c2))
}
fn target_plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x7a_6a))
}
fn only_phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(0xfa_60))
}
fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 8, hour, 0, 0).unwrap()
}
fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}
fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: ACTOR,
        recorded_at: at(10),
        correlation_id: CORRELATION,
    }
}

fn source_key() -> ScopeKey {
    ScopeKey::new(
        source_plan(),
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new("eu").expect("a non-blank region"),
        only_phase(),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("the class pairs with the cohort")
}

fn flat_row() -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    PriceContent {
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: Some("advance".to_owned()),
        proration_contract: None,
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

/// A published plan revision carrying one published price row.
///
/// Both publishes are direct `UPDATE`s past the engine, `tests/common`'s
/// helpers, for their stated reason: this suite is about the clone, and a fixture
/// that went through the publish pipeline would make a pipeline failure look like
/// a clone failure.
async fn seed(provider: &DBProvider<DbError>) {
    let created = PlanRepo::new(provider.clone())
        .create_draft(
            &scope(),
            NewPlanDraft {
                plan_id: source_plan(),
                tenant_id: TENANT,
                created_by: ACTOR,
                created_at_utc: at(10),
                sku_id: Some(Uuid::from_u128(0x5_c2)),
                plan_tier: Some("gold".to_owned()),
                billing_cycle: None,
                frequency: None,
                plan_tier_override: false,
                purchase_min_qty: None,
                purchase_max_qty: None,
                invoice_grouping_key: Some("group/pg-source".to_owned()),
                available_from: None,
                available_to: None,
                cloned_from: None,
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("create the source draft");

    PriceRepo::new(provider.clone())
        .create_draft(
            &scope(),
            TENANT,
            NewPriceDraft {
                price_id: SOURCE_ROW,
                scope_key: source_key(),
                content: flat_row(),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author the source row");

    common::publish_row_directly(provider, &scope(), SOURCE_ROW).await;
    common::publish_plan_directly(provider, &scope(), source_plan(), created.revision).await;
}

/// **A clone its caller rolls back leaves no row behind — on Postgres.**
///
/// Before D-274 each step was a repository *method* opening its own transaction,
/// so the plan draft and every row under it committed one at a time and the
/// caller had nothing to take back. The sibling `SQLite` case proves the same
/// property and is the one that was probed against that older shape; this one
/// answers the question that probe could not, which is whether the engine agrees.
#[tokio::test]
#[ignore = "requires Postgres; run with --ignored"]
async fn a_clone_its_caller_rolls_back_leaves_no_row_behind() {
    let pg = Pg::applied().await;
    let provider = DBProvider::<DbError>::new(pg.db().await);
    seed(&provider).await;

    let (_, outcome) = provider
        .db()
        .in_transaction::<(), DomainError, _>(move |txn| {
            Box::pin(async move {
                Box::pin(clone_plan_on(
                    txn,
                    &scope(),
                    TENANT,
                    source_plan(),
                    target_plan(),
                    at(11),
                    stamp(),
                ))
                .await?;
                Err(DomainError::Internal(
                    "the caller fails after the clone".to_owned(),
                ))
            })
        })
        .await;
    assert!(outcome.is_err(), "the caller's failure stands");

    let conn = provider.conn().expect("conn");
    assert!(
        plan_repo::load_open_draft(&conn, &scope(), TENANT, target_plan())
            .await
            .expect("read the draft")
            .is_none(),
        "the draft plan the clone created must not survive its caller's rollback"
    );
    assert!(
        price_repo::load_for_plan(
            &conn,
            &scope(),
            TENANT,
            target_plan(),
            &[LifecycleState::Draft],
        )
        .await
        .expect("read the rows")
        .is_empty(),
        "and neither may the price row it copied"
    );
}

/// The same clone, committed — so the case above is read as *rollback*, not as a
/// clone that never wrote anything.
///
/// Without this pair the assertion "nothing survived" is satisfied just as well
/// by a clone that failed at its first statement, and a suite that cannot tell
/// those apart is not testing atomicity. Its sibling on `SQLite` is the whole
/// nine-case copy-set suite; here it is one row, which is all the distinction
/// needs.
#[tokio::test]
#[ignore = "requires Postgres; run with --ignored"]
async fn the_same_clone_committed_writes_the_plan_and_its_row() {
    let pg = Pg::applied().await;
    let provider = DBProvider::<DbError>::new(pg.db().await);
    seed(&provider).await;

    let (_, outcome) = provider
        .db()
        .in_transaction::<(), DomainError, _>(move |txn| {
            Box::pin(async move {
                Box::pin(clone_plan_on(
                    txn,
                    &scope(),
                    TENANT,
                    source_plan(),
                    target_plan(),
                    at(11),
                    stamp(),
                ))
                .await
                .map(|_| ())
            })
        })
        .await;
    outcome.expect("the clone commits");

    let conn = provider.conn().expect("conn");
    let revision = plan_repo::load_open_draft(&conn, &scope(), TENANT, target_plan())
        .await
        .expect("read the draft")
        .expect("there is one");
    assert_eq!(revision.cloned_from, Some(source_plan()));
    assert_eq!(
        price_repo::load_for_plan(
            &conn,
            &scope(),
            TENANT,
            target_plan(),
            &[LifecycleState::Draft],
        )
        .await
        .expect("read the rows")
        .len(),
        1,
        "the one published source row came across as a draft"
    );
}
