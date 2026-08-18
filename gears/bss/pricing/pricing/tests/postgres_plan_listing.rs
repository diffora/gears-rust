//! `plan_repo::list_authoring_page` on Postgres: one row per plan, and the row
//! is the one an **author** is holding.
//!
//! Why this suite is on the Postgres tier rather than the `SQLite` one: what it
//! proves is an interaction between an `ORDER BY` over two columns, a `LIMIT`
//! and the two partial `UNIQUE` indexes that bound how many rows a plan can
//! contribute — `uq_pricing_plan_current` and `uq_pricing_plan_open_draft`. The
//! indexes are what make the collapse's over-fetch sufficient, and they exist
//! as partial indexes only on the engine that has them.
//!
//! The listing deliberately does **not** answer with `current_tokens()`.
//! `LifecycleState::is_current_revision` is `Published | Retired`, so a plan
//! whose only revision is the draft somebody is authoring right now has no
//! current revision at all — and that is exactly the plan an authoring UI most
//! needs to see. `plans.rs::authoring_revision` already decides the rule for one
//! plan; this is the same rule over a page.
//!
//! Ignored by default; they need Docker. Run with
//! `cargo test -p bss-pricing -- --ignored`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod pg_support;

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::storage::repo::{NewPlanDraft, PlanRepo, plan_repo};
use chrono::{DateTime, TimeZone, Utc};
use pg_support::Pg;
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x7e_51);
const ACTOR: Uuid = Uuid::from_u128(0xac_50);
const CORRELATION: Uuid = Uuid::from_u128(0xc0_51);

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 11, hour, 0, 0).unwrap()
}

fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: ACTOR,
        recorded_at: at(11),
        correlation_id: CORRELATION,
    }
}

/// A plan whose revision `0` is `published`, by the direct route
/// `tests/common` documents: this suite is about the listing, and a fixture that
/// went through the publish pipeline would make a pipeline failure look like a
/// listing failure.
async fn seed_published_plan(provider: &DBProvider<DbError>, plan_id: PlanId) {
    let created = PlanRepo::new(provider.clone())
        .create_draft(
            &scope(),
            NewPlanDraft {
                plan_name: None,
                plan_id,
                tenant_id: TENANT,
                created_by: ACTOR,
                created_at_utc: at(10),
                sku_id: None,
                plan_tier: Some("gold".to_owned()),
                billing_cycle: None,
                frequency: None,
                plan_tier_override: false,
                purchase_min_qty: None,
                purchase_max_qty: None,
                invoice_grouping_key: None,
                available_from: None,
                available_to: None,
                cloned_from: None,
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("create the draft");
    common::publish_plan_directly(provider, &scope(), plan_id, created.revision).await;
}

/// **A plan appears once, and as the draft its author is editing.**
///
/// The two-row plan is the whole case: a published revision `0` and the draft
/// revision `1` opened over it. A listing keyed on `current_tokens()` would show
/// the published one, which is the body the author's next `PATCH` would not
/// match; a listing that did not collapse would show the plan twice.
#[tokio::test]
#[ignore = "requires Postgres; run with --ignored"]
async fn a_plan_with_a_draft_over_a_published_revision_lists_once_as_its_draft() {
    let pg = Pg::applied().await;
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let plans = PlanRepo::new(provider.clone());

    let plan_id = PlanId::new(Uuid::from_u128(0x50_51));
    seed_published_plan(&provider, plan_id).await;
    let draft = plans
        .open_revision(&scope(), TENANT, plan_id, stamp())
        .await
        .expect("reopen the plan as a draft");
    assert_eq!(draft.revision, 1, "the successor is revision 1");

    let conn = provider.conn().expect("conn");
    let rows = plan_repo::list_authoring_page(&conn, &scope(), TENANT, &[], None, 100)
        .await
        .expect("list");

    let seen: Vec<_> = rows.iter().filter(|r| r.plan_id == plan_id).collect();
    assert_eq!(seen.len(), 1, "a plan appears once, not once per revision");
    assert_eq!(
        seen[0].revision, 1,
        "the page answers about the revision an author is holding"
    );
    assert_eq!(
        seen[0].lifecycle_state,
        LifecycleState::Draft,
        "the draft is the authoring revision, so it is what the listing shows"
    );
}

/// **The walk visits every plan exactly once.**
///
/// Five plans over pages of two, which is the size that makes the collapse's
/// over-fetch do something: a page is filled from a window twice its size, so a
/// walk that mistook the window for the page would either repeat a plan or skip
/// one at every boundary.
#[tokio::test]
#[ignore = "requires Postgres; run with --ignored"]
async fn the_walk_pages_by_plan_and_never_repeats_one() {
    let pg = Pg::applied().await;
    let provider = DBProvider::<DbError>::new(pg.db().await);

    let mut expected = Vec::new();
    for n in 0..5_u128 {
        let plan_id = PlanId::new(Uuid::from_u128(0x50_60 + n));
        seed_published_plan(&provider, plan_id).await;
        expected.push(plan_id.get());
    }
    expected.sort();

    let conn = provider.conn().expect("conn");
    let mut walked = Vec::new();
    let mut after = None;
    loop {
        let page = plan_repo::list_authoring_page(&conn, &scope(), TENANT, &[], after, 2)
            .await
            .expect("list");
        if page.is_empty() {
            break;
        }
        after = Some(page.last().expect("non-empty").plan_id.get());
        walked.extend(page.into_iter().map(|r| r.plan_id.get()));
    }

    let mut deduped = walked.clone();
    deduped.dedup();
    assert_eq!(walked, deduped, "no plan is returned twice");
    walked.sort();
    assert_eq!(walked, expected, "every plan once, in plan_id order");
}
