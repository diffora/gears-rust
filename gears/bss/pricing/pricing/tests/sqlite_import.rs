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

use bss_pricing::domain::approval::PENDING_CHANGE_UNIT_EXISTS;
use bss_pricing::domain::audit::{AuditStamp, AuditSubjectKind};
use bss_pricing::domain::import::{
    BatchReport, IMPORT_TARGETS_PUBLISHED, ImportRow, RowViolation, classify,
};
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{IncludedAllowance, ModelKind, PriceRow, RolloverPolicy};
use bss_pricing::domain::publish::rules::PRIMITIVE_RULES_UNBUILT;
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, DimensionKey, Meter, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::import::classify_against_store;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewApproval, NewPriceDraft, PriceRepo, approval_repo};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use std::collections::BTreeSet;
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

/// A usage row's content, whose own line must agree with its key — the store
/// refuses `UsageLineDisagrees` otherwise, which is the guard that makes a
/// mismatched `ImportRow` unauthorable rather than merely wrong.
fn usage_content(meter: &str, amount: i64) -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(amount).expect("a non-negative amount"));
    row.meter = Some(meter.to_owned());
    "region=eu".clone_into(&mut row.dimension_key);
    PriceContent {
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: Some("arrears".to_owned()),
        proration_contract: None,
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

fn usage_row(scope_key: ScopeKey, meter: &str, amount: i64) -> ImportRow {
    ImportRow {
        scope_key,
        content: usage_content(meter, amount),
        if_match: None,
    }
}

async fn publish_usage(h: &Harness, scope_key: ScopeKey, meter: &str, amount: i64) -> Uuid {
    let price_id = Uuid::now_v7();
    h.prices
        .create_draft(
            &scope(),
            TENANT,
            NewPriceDraft {
                price_id,
                scope_key,
                content: usage_content(meter, amount),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author the usage row");
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
        .rows()
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
    let detail = &report.rows()[0].violations[0].detail;
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
async fn a_row_identical_to_the_published_one_is_refused_too() {
    // **The carve-out that was, and is not** (D-287). `inst-bk-phase1` says "with
    // changed content", and this refuses whatever the content is — because an
    // unchanged row authors a no-op draft beside the published row, and the
    // supersession door refuses a draft occupant, so re-running an unchanged
    // batch would block the repricing run this very message names as the remedy.
    // The re-imported file is served by `inst-bk-idem`'s replay, not here.
    let h = harness().await;
    publish(&h, key("eu"), 9_900).await;

    let report = judged(&h, &[row(key("eu"), 9_900)]).await;

    assert_eq!(codes(&report, 0), vec![IMPORT_TARGETS_PUBLISHED]);
    assert!(report.blocks_the_batch());
}

#[tokio::test]
async fn a_usage_line_key_is_matched_through_the_store_like_any_other() {
    // **The axis pair D-283 was about, checked through the store** — the suite
    // created for that lesson did not exercise it. A published usage row holds
    // its key; a row on a *different* meter is a different key and is untouched.
    let h = harness().await;
    let usage = ScopeKey::new(
        plan(),
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new("eu").expect("a non-blank region"),
        phase(),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Usage,
        Cohort::None,
    )
    .expect("the class pairs with the cohort");
    let metered = usage
        .clone()
        .with_usage_line(
            Some(Meter::new("api-calls").expect("a meter")),
            DimensionKey::new("region=eu"),
        )
        .expect("a usage line on a usage key");
    let other = usage
        .with_usage_line(
            Some(Meter::new("storage-gb").expect("a meter")),
            DimensionKey::new("region=eu"),
        )
        .expect("a usage line on a usage key");
    publish_usage(&h, metered.clone(), "api-calls", 900).await;

    let report = judged(
        &h,
        &[
            usage_row(metered, "api-calls", 1_200),
            usage_row(other, "storage-gb", 1_200),
        ],
    )
    .await;

    assert_eq!(codes(&report, 0), vec![IMPORT_TARGETS_PUBLISHED]);
    assert!(
        codes(&report, 1).is_empty(),
        "a second meter is a second key and holds nothing (D-196, D-283)"
    );
}

#[tokio::test]
async fn a_row_that_is_both_unjudged_and_published_hears_them_in_a_stable_order() {
    // **The ordering the report claims and nothing pinned.** The two codes are
    // inserted `[PRIMITIVE_RULES_UNBUILT, IMPORT_TARGETS_PUBLISHED]` — the
    // batch-only half runs first — and sort to the reverse. Delete the sort in
    // `BatchReport::add` and this is the case that reddens.
    let h = harness().await;
    publish(&h, key("eu"), 9_900).await;
    let mut carrier = row(key("eu"), 12_500);
    carrier.content.row.included_allowance = Some(IncludedAllowance {
        quantity: 100,
        rollover_policy: RolloverPolicy::Carry,
    });

    let report = judged(&h, &[carrier]).await;

    assert_eq!(
        codes(&report, 0),
        vec![IMPORT_TARGETS_PUBLISHED, PRIMITIVE_RULES_UNBUILT],
        "sorted by code, not by the order the halves ran in"
    );
}

#[tokio::test]
async fn a_row_on_a_free_key_is_untouched_by_the_store_half() {
    let h = harness().await;
    publish(&h, key("eu"), 9_900).await;

    let report = judged(&h, &[row(key("us"), 12_500)]).await;

    assert!(
        report.rows().is_empty(),
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
        report.rows().is_empty(),
        "a draft occupant is not a published one: {report:?}"
    );
}

/// Open a `submitted` approval unit holding the given keys, so they are genuinely
/// reserved by the one-pending-unit rule.
async fn hold(h: &Harness, keys: &[ScopeKey]) -> Uuid {
    let approval_id = Uuid::now_v7();
    let conn = h.provider.conn().expect("conn");
    approval_repo::open(
        &conn,
        &scope(),
        NewApproval {
            approval_id,
            tenant_id: TENANT,
            subject_ref: format!("{}/0", plan()),
            subject_kind: AuditSubjectKind::PlanRevision,
            content_hash: vec![0x11; 32],
            materiality: serde_json::json!({ "material": true }),
            held_keys: keys
                .iter()
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>(),
        },
        AuditStamp {
            actor_principal_id: ACTOR,
            recorded_at: at(10),
            correlation_id: CORRELATION,
        },
    )
    .await
    .expect("open the pending unit");
    approval_id
}

#[tokio::test]
async fn a_row_whose_key_a_pending_unit_holds_is_refused_and_names_it() {
    let h = harness().await;
    let unit = hold(&h, &[key("eu")]).await;

    let report = judged(&h, &[row(key("eu"), 12_500)]).await;

    assert_eq!(codes(&report, 0), vec![PENDING_CHANGE_UNIT_EXISTS]);
    let detail = &report.rows()[0].violations[0].detail;
    assert!(
        detail.contains(&unit.to_string()),
        "the operator has to know which unit to decide or withdraw: {detail}"
    );
}

#[tokio::test]
async fn one_unit_holding_two_of_the_batchs_keys_refuses_both_rows() {
    // **The reason the register read needed a plural.** A submit stops at the
    // first conflict because one is enough to refuse; Phase 1 owes a verdict on
    // every row, and a caller that stopped at the first would send the operator
    // round once per held key.
    let h = harness().await;
    hold(&h, &[key("eu"), key("us")]).await;

    let report = judged(&h, &[row(key("eu"), 12_500), row(key("us"), 7_500)]).await;

    assert_eq!(codes(&report, 0), vec![PENDING_CHANGE_UNIT_EXISTS]);
    assert_eq!(codes(&report, 1), vec![PENDING_CHANGE_UNIT_EXISTS]);
}

#[tokio::test]
async fn a_held_key_does_not_taint_its_neighbours() {
    let h = harness().await;
    hold(&h, &[key("eu")]).await;

    let report = judged(&h, &[row(key("eu"), 12_500), row(key("us"), 7_500)]).await;

    assert_eq!(codes(&report, 0), vec![PENDING_CHANGE_UNIT_EXISTS]);
    assert!(
        codes(&report, 1).is_empty(),
        "row 1's key is free and must be reported clean"
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

    assert_eq!(
        report.rows().len(),
        2,
        "one entry per row, not one per rule"
    );
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
        report.rows()[0].violations[0]
            .detail
            .contains(&first_holder.to_string())
    );
    assert!(
        report.rows()[1].violations[0]
            .detail
            .contains(&second_holder.to_string())
    );
}
