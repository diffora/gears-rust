//! The retirement flip against a real database (`inst-rt-cancel`, `inst-rt-event`,
//! D-90, D-128, D-145).
//!
//! Everything here is a property of a **statement** rather than of a branch in
//! Rust: the transition rides inside a compare-and-swap the database evaluates
//! under the row lock, and what makes "the plan's current revision" well defined
//! is the partial `uq_pricing_plan_current` index — which spans
//! `('published','retired')`, so a retired revision **stays** current. That last
//! fact is D-128's load-bearing one and no mock can see it: it is why the
//! projector still finds a retired plan's revision, and therefore why an arrears
//! charge for an in-flight subscriber still resolves after the plan stops
//! selling.
//!
//! `sqlite::memory:` serializes writers, so nothing here distinguishes isolation
//! levels. What it does prove is that a second retirement is refused by the
//! predicate rather than by a Rust `if` — delete the `lifecycle_state =
//! 'published'` clause from the swap and this suite reddens.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::plan_shape::{BillingCycle, Frequency};
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewPlanDraft, PlanRepo, plan_repo};
use bss_pricing::infra::storage::{RepoError, repo_failure};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

const TENANT: Uuid = Uuid::from_u128(0x_7e_11_51);
const CORRELATION: Uuid = Uuid::from_u128(0x_c0_11_a7_11);

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 7, hour, 0, 0).unwrap()
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

async fn harness() -> (PlanRepo, DBProvider<DbError>) {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations(&db).await;
    let provider = DBProvider::<DbError>::new(db);
    (PlanRepo::new(provider.clone()), provider)
}

async fn run_migrations(db: &toolkit_db::Db) {
    toolkit_db::migration_runner::run_migrations_for_testing(db, Migrator::migrations())
        .await
        .expect("run migrator");
}

fn new_draft(plan_id: PlanId) -> NewPlanDraft {
    NewPlanDraft {
        plan_id,
        tenant_id: TENANT,
        created_by: Uuid::from_u128(0xac_11),
        created_at_utc: at(10),
        sku_id: Some(Uuid::from_u128(0x5_c1_11)),
        plan_tier: Some("gold".to_owned()),
        billing_cycle: Some(BillingCycle::Recurring),
        frequency: Some(Frequency::Monthly),
        plan_tier_override: false,
        purchase_min_qty: None,
        purchase_max_qty: None,
        invoice_grouping_key: None,
        available_from: None,
        available_to: None,
        correlation_id: CORRELATION,
    }
}

/// A plan standing at a published revision `0`, the state retirement acts on.
async fn published_plan(repo: &PlanRepo, provider: &DBProvider<DbError>, plan_id: PlanId) {
    let created = repo
        .create_draft(&scope(), new_draft(plan_id))
        .await
        .expect("create the first revision");
    let (_, published) = provider
        .db()
        .in_transaction::<(), RepoError, _>(move |txn| {
            Box::pin(async move {
                plan_repo::publish_revision(
                    txn,
                    &scope(),
                    TENANT,
                    plan_id,
                    created.revision,
                    created.row_version,
                )
                .await
                .map(|_| ())
            })
        })
        .await;
    published.expect("publish the first revision");
}

/// Retire through the repository, returning whatever it answered.
async fn retire(
    provider: &DBProvider<DbError>,
    plan_id: PlanId,
    revision: u64,
) -> Result<LifecycleState, RepoError> {
    let (_, outcome) = provider
        .db()
        .in_transaction::<LifecycleState, RepoError, _>(move |txn| {
            Box::pin(async move {
                plan_repo::retire_revision(txn, &scope(), TENANT, plan_id, revision)
                    .await
                    .map(|row| row.lifecycle_state)
            })
        })
        .await;
    outcome.map_err(|e| e.into_domain(|infra| RepoError::Db(format!("retire: {infra}"))))
}

#[tokio::test]
async fn retirement_flips_the_current_published_revision_and_leaves_its_tag() {
    let (repo, provider) = harness().await;
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a5));
    published_plan(&repo, &provider, plan_id).await;

    let before =
        plan_repo::load_current(&provider.conn().expect("conn"), &scope(), TENANT, plan_id)
            .await
            .expect("read")
            .expect("a published plan has a current revision");
    assert_eq!(before.lifecycle_state, LifecycleState::Published);

    let state = retire(&provider, plan_id, before.revision)
        .await
        .expect("the published revision retires");
    assert_eq!(state, LifecycleState::Retired);

    let after = plan_repo::load_current(&provider.conn().expect("conn"), &scope(), TENANT, plan_id)
        .await
        .expect("read")
        .expect("a retired revision is still the plan's current one");
    assert_eq!(after.lifecycle_state, LifecycleState::Retired);
    // **The tag does not move**, and the store is what decides that: every
    // content column of a row past `draft` is frozen by
    // `trg_pricing_plan_frozen_columns`, `row_version` among them, so the only
    // update a published row admits is the bare flip. This assertion was
    // originally written the other way round - a bump alongside the flip - and
    // every case in this file reddened on `(code: 1811) revision is frozen`,
    // a driver refusal rather than an assertion. `supersede_current` had already
    // recorded the same rule on the same table.
    assert_eq!(after.row_version, before.row_version);
    // Same revision number: retirement mints nothing and consumes nothing.
    assert_eq!(after.revision, before.revision);
}

#[tokio::test]
async fn a_retired_revision_stays_the_plans_current_one() {
    // D-128's load-bearing consequence: `uq_pricing_plan_current` spans
    // `('published','retired')`, so the projector still sources the revision and
    // an in-flight subscriber's arrears charge still resolves after the plan
    // stops selling. A retirement that left no current revision would make the
    // plan unresolvable rather than unsellable.
    let (repo, provider) = harness().await;
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a6));
    published_plan(&repo, &provider, plan_id).await;

    retire(&provider, plan_id, 0).await.expect("retire");

    let current =
        plan_repo::load_current(&provider.conn().expect("conn"), &scope(), TENANT, plan_id)
            .await
            .expect("read");
    assert!(
        current.is_some(),
        "a retired plan must still answer with a current revision"
    );
    assert_eq!(
        current.expect("checked").lifecycle_state,
        LifecycleState::Retired
    );
}

#[tokio::test]
async fn a_second_retirement_is_refused_by_the_predicate() {
    // The `lifecycle_state = 'published'` clause is the guard. Delete it and the
    // second call succeeds, flipping `retired -> retired` and bumping the tag of
    // a row nothing changed about.
    let (repo, provider) = harness().await;
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a7));
    published_plan(&repo, &provider, plan_id).await;

    retire(&provider, plan_id, 0).await.expect("the first");
    let refusal = retire(&provider, plan_id, 0)
        .await
        .expect_err("the second finds no published row");

    assert!(
        matches!(refusal, RepoError::ConcurrentMutation { .. }),
        "the loser is a contention on the current-revision slot: {refusal:?}"
    );
    assert!(
        refusal.to_string().contains(&plan_id.to_string()),
        "and it names the aggregate: {refusal}"
    );
}

#[tokio::test]
async fn retiring_a_plan_that_never_published_moves_nothing() {
    // Revision `0` is a draft here. The swap names `published`, so it matches no
    // row - and the caller is told so rather than the draft being retired, which
    // is the flip `domain::lifecycle` and the store's trigger both refuse.
    let (repo, provider) = harness().await;
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a8));
    repo.create_draft(&scope(), new_draft(plan_id))
        .await
        .expect("create the draft");

    let refusal = retire(&provider, plan_id, 0)
        .await
        .expect_err("a draft is not retirable");
    assert!(
        matches!(refusal, RepoError::ConcurrentMutation { .. }),
        "{refusal:?}"
    );

    let draft = repo
        .find_revision(&scope(), TENANT, plan_id, 0)
        .await
        .expect("read")
        .expect("the draft is still there");
    assert_eq!(
        draft.lifecycle_state,
        LifecycleState::Draft,
        "the refusal left the draft alone"
    );
}

#[tokio::test]
async fn the_refusal_reaches_a_caller_as_a_lifecycle_answer_and_not_a_storage_fault() {
    // `repo_failure` is the single ladder every surface maps through; a
    // contention that arrived as `Internal` would tell an operator to open a
    // ticket for a retry they could make themselves.
    let (repo, provider) = harness().await;
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a9));
    published_plan(&repo, &provider, plan_id).await;
    retire(&provider, plan_id, 0).await.expect("the first");

    let refusal = retire(&provider, plan_id, 0).await.expect_err("the second");
    let domain = repo_failure(&refusal);
    assert!(
        matches!(
            domain,
            bss_pricing::domain::error::DomainError::ConcurrentMutation(_)
        ),
        "{domain:?}"
    );
}
