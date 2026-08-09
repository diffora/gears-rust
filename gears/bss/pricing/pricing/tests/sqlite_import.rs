//! Phase 1's store-dependent half, executed against a real store
//! (`design/12-operator-efficiency.md` §3 `inst-bk-phase1`; D-118).
//!
//! **Driven through the price repository rather than over a hand-built map**,
//! for `tests/sqlite_clone.rs`'s reason and for one this suite learned the hard
//! way: the rule mirrors what the store holds, and a fixture the test assembled
//! itself would prove only that the test and the rule agree. D-283 is exactly
//! that failure — a duplicate rule judged against an index nobody read, green
//! beside a sibling suite asserting the opposite through the store.
//!
//! What the batch-only half asserts is in `domain::import`'s own tests; nothing
//! here repeats it.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use bss_pricing::domain::import::{
    BatchReport, IMPORT_TARGETS_PUBLISHED, ImportRow, RowViolation, classify,
};
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::import::classify_against_store;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewPriceDraft, PriceRepo};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_31);
const ACTOR: Uuid = Uuid::from_u128(0xac_30);
const CORRELATION: Uuid = Uuid::from_u128(0xc0_31);

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x50_c3))
}
fn phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(0xfa_70))
}
fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 9, hour, 0, 0).unwrap()
}
fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

fn key(region: &str) -> ScopeKey {
    ScopeKey::new(
        plan(),
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new(region).expect("a non-blank region"),
        phase(),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("the class pairs with the cohort")
}

fn content(amount: i64) -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(amount).expect("a non-negative amount"));
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

fn row(scope_key: ScopeKey, amount: i64) -> ImportRow {
    ImportRow {
        scope_key,
        content: content(amount),
        if_match: None,
    }
}

struct Harness {
    provider: DBProvider<DbError>,
    prices: PriceRepo,
}

async fn harness() -> Harness {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    let prices = PriceRepo::new(provider.clone());
    Harness { provider, prices }
}

/// Author a row and publish it past the engine, so the key is genuinely held.
async fn publish(h: &Harness, scope_key: ScopeKey, amount: i64) -> Uuid {
    let price_id = Uuid::now_v7();
    h.prices
        .create_draft(
            &scope(),
            TENANT,
            NewPriceDraft {
                price_id,
                scope_key,
                content: content(amount),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author the row");
    common::publish_row_directly(&h.provider, &scope(), price_id).await;
    price_id
}

async fn judged(h: &Harness, rows: &[ImportRow]) -> BatchReport {
    let mut report = classify(rows);
    let conn = h.provider.conn().expect("conn");
    classify_against_store(&conn, &scope(), TENANT, rows, &mut report)
        .await
        .expect("the store answers");
    report
}

fn codes(report: &BatchReport, row: usize) -> Vec<String> {
    report
        .rows
        .iter()
        .find(|outcome| outcome.row == row)
        .map(|outcome| {
            outcome
                .violations
                .iter()
                .map(|violation: &RowViolation| violation.code.clone())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_row_changing_a_published_rows_content_is_refused_and_named() {
    let h = harness().await;
    let held = publish(&h, key("eu"), 9_900).await;

    let report = judged(&h, &[row(key("eu"), 12_500)]).await;

    assert_eq!(codes(&report, 0), vec![IMPORT_TARGETS_PUBLISHED]);
    let detail = &report.rows[0].violations[0].detail;
    assert!(
        detail.contains(&held.to_string()),
        "the refusal names the row that holds the key: {detail}"
    );
    assert!(
        detail.contains("repricing run"),
        "and the remedy, which exists: {detail}"
    );
    assert!(report.blocks_the_batch());
}

#[tokio::test]
async fn a_row_identical_to_the_published_one_is_not_refused() {
    // **The re-imported file.** `inst-bk-phase1` refuses a row aimed at a
    // published key *with changed content*; running the same batch twice is the
    // ordinary operator act and must not be an error.
    let h = harness().await;
    publish(&h, key("eu"), 9_900).await;

    let report = judged(&h, &[row(key("eu"), 9_900)]).await;

    assert!(
        report.rows.is_empty(),
        "an unchanged row is a no-op draft, not a fault: {report:?}"
    );
    assert!(!report.blocks_the_batch());
}

#[tokio::test]
async fn a_row_on_a_free_key_is_untouched_by_the_store_half() {
    let h = harness().await;
    publish(&h, key("eu"), 9_900).await;

    let report = judged(&h, &[row(key("us"), 12_500)]).await;

    assert!(
        report.rows.is_empty(),
        "no published row holds `us`, so nothing here refuses it: {report:?}"
    );
}

#[tokio::test]
async fn a_draft_row_on_the_key_is_not_a_published_row() {
    // The rule is about the **published** plane. A draft occupant is what an
    // import edits under its version — Phase 2's business, not this one's.
    let h = harness().await;
    h.prices
        .create_draft(
            &scope(),
            TENANT,
            NewPriceDraft {
                price_id: Uuid::now_v7(),
                scope_key: key("eu"),
                content: content(9_900),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author a draft and leave it a draft");

    let report = judged(&h, &[row(key("eu"), 12_500)]).await;

    assert!(
        report.rows.is_empty(),
        "a draft occupant is not a published one: {report:?}"
    );
}

#[tokio::test]
async fn the_two_halves_write_one_report_and_a_row_hears_both() {
    // The merge API's whole point: a row can be a duplicate *and* aimed at a
    // published key, and the operator has to be told both or they fix one and
    // resubmit into the other.
    let h = harness().await;
    publish(&h, key("eu"), 9_900).await;

    let report = judged(&h, &[row(key("eu"), 12_500), row(key("eu"), 12_500)]).await;

    assert_eq!(report.rows.len(), 2, "one entry per row, not one per rule");
    for index in [0, 1] {
        assert_eq!(
            codes(&report, index),
            vec!["DUPLICATE_SCOPE_KEY", IMPORT_TARGETS_PUBLISHED],
            "row {index} is both, in a stable order"
        );
    }
}

#[tokio::test]
async fn one_read_serves_every_row_of_a_plan_and_a_batch_may_span_plans() {
    // Two plans in one batch, each with a held key. The point is not the read
    // count — nothing here can see it — but that a batch spanning plans is
    // judged correctly at all, which a per-plan read written for one plan would
    // get wrong.
    let h = harness().await;
    let other = PlanId::new(Uuid::from_u128(0x50_c4));
    let first_holder = publish(&h, key("eu"), 9_900).await;
    let there = ScopeKey::new(
        other,
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new("eu").expect("a non-blank region"),
        phase(),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("the class pairs with the cohort");
    let second_holder = publish(&h, there.clone(), 5_000).await;

    let report = judged(&h, &[row(key("eu"), 12_500), row(there, 7_500)]).await;

    assert_eq!(codes(&report, 0), vec![IMPORT_TARGETS_PUBLISHED]);
    assert_eq!(codes(&report, 1), vec![IMPORT_TARGETS_PUBLISHED]);
    assert!(
        report.rows[0].violations[0]
            .detail
            .contains(&first_holder.to_string())
    );
    assert!(
        report.rows[1].violations[0]
            .detail
            .contains(&second_holder.to_string())
    );
}
