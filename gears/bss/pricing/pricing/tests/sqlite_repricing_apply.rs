//! The mass-repricing apply's own atomicity and idempotency —
//! `design/12-operator-efficiency.md` §3 `inst-mr-apply`, `inst-mr-validate-scope`
//! (D-134), driven at the repository seam rather than through HTTP.
//!
//! `tests/rest_repricing_runs.rs` proves the two REST surfaces and the states
//! `open_repricing_run`/the approve arm leave a run in; what it cannot reach
//! without an enormous fixture is D-134's one property that is easiest to lose
//! and hardest to notice — that a plan whose aggregate pass fails after its
//! rows have already been written **inside the same transaction** applies none
//! of them, while a sibling plan in the same run applies whole. This suite
//! drives [`apply_run_in`] directly, over a run and a journal built at the
//! repository seam, for exactly that reason.
//!
//! # How plan B's aggregate pass is made to fail
//!
//! Not via a defect the repricing adjustment itself introduces — a plain
//! markup/discount/fixed adjustment changes only a row's amount, and none of
//! the Foundation's aggregate-only rules (phase coverage, descriptor
//! completeness, region declaration, window coverage) are sensitive to an
//! amount's *value*, so an adjustment that starts from a clean plan cannot by
//! itself trip one. Plan B instead carries a **stray draft row** — a second,
//! unrelated key with no window scheduled on it, exactly the shape an
//! interactive author mid-edit would leave — which this suite's own module doc
//! (`infra::repricing`) names as the consequence of not taking the bulk lock:
//! `assemble_from`'s candidate set is every `published`/`draft` row of the
//! plan, so the stray draft rides along into the aggregate pass touching
//! nothing this run selected. `WINDOW_COVERAGE_MISSING` (`inst-wc-required`) is
//! genuinely an aggregate-only check — `domain::coverage::window_coverage_rules`
//! is registered in `run_publish_rules` and nowhere in
//! `domain::supersession::plan_supersession`'s row-local set — so plan B's own
//! selected row commits cleanly through its row-local checks and only the
//! aggregate pass, reading the transaction's own writes back, catches it.
//!
//! What this proves is the property D-134 states, not a claim that a plain
//! repricing run can organically produce this shape on a plan nothing else
//! touched: **once written, a row-local pass is not the last word — the
//! aggregate pass over the plan's real, post-commit state is**, and its
//! refusal takes every row of that plan's own selection down with it.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::bulk::{BulkKind, BulkState, JournalState};
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::contracts::{BillingAnchorPolicy, ProrationBasis, ProrationContract};
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::plan_shape::{
    BillingCycle, DescriptorSet, Frequency, PhaseKind, PlanPhase,
};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::repricing::apply_run_in;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::repricing_journal_repo::NewJournalRow;
use bss_pricing::infra::storage::repo::{
    NewBulkOperation, NewPlanDraft, NewPriceDraft, PlanRepo, PlanShapeRepo, PolicyObjectRepo,
    PriceRepo, bulk_repo, repricing_journal_repo,
};
use bss_pricing_sdk::catalog_version_registry::{
    CatalogVersionRegistryError, CatalogVersionRegistryV1, PendingVersionRef,
};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm_migration::MigratorTrait;
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use toolkit_security::SecurityContext;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The registry double — this suite's own, per `sqlite_publish_commit.rs`'s
// convention that no double is shared across binaries.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RegistryDouble {
    issued: Mutex<HashMap<String, String>>,
}

#[async_trait]
impl CatalogVersionRegistryV1 for RegistryDouble {
    async fn request_version(
        &self,
        _ctx: &SecurityContext,
        request_id: &str,
    ) -> Result<PendingVersionRef, CatalogVersionRegistryError> {
        let mut issued = self.issued.lock().expect("no panics in the double");
        let next = issued.len();
        let pending = issued
            .entry(request_id.to_owned())
            .or_insert_with(|| format!("pend-{next}"))
            .clone();
        Ok(PendingVersionRef {
            request_id: request_id.to_owned(),
            pending_ref: pending,
        })
    }

    async fn committed_version(
        &self,
        _ctx: &SecurityContext,
        _pending_ref: &str,
    ) -> Result<Option<bss_pricing_sdk::catalog_version::CatalogVersion>, CatalogVersionRegistryError>
    {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// The harness.
// ---------------------------------------------------------------------------

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const ACTOR: Uuid = Uuid::from_u128(0xac_10);
const CORRELATION: Uuid = Uuid::from_u128(0xc0_11);

fn ctx() -> SecurityContext {
    SecurityContext::builder()
        .subject_id(ACTOR)
        .subject_tenant_id(TENANT)
        .build()
        .expect("a subject and a tenant are all a context needs")
}

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 3, hour, 0, 0).unwrap()
}

/// Far enough out that no wall clock reaches it, and clear of the batching
/// delay floor `ChangeoverMoment::Commit` holds the apply to — the fixtures'
/// standing rule (`tests/rest_repricing_runs.rs` carries the identical
/// constant for the identical reason).
fn changeover() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 20, 0, 0, 0).unwrap()
}

struct Harness {
    provider: DBProvider<DbError>,
    plans: PlanRepo,
    shapes: PlanShapeRepo,
    prices: PriceRepo,
    policies: PolicyObjectRepo,
    registry: Arc<RegistryDouble>,
    scope: AccessScope,
}

async fn harness() -> Harness {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    common::declare_fixture_regions(&provider, TENANT).await;
    Harness {
        plans: PlanRepo::new(provider.clone()),
        shapes: PlanShapeRepo::new(provider.clone()),
        prices: PriceRepo::new(provider.clone()),
        policies: PolicyObjectRepo::new(
            provider.clone(),
            &bss_pricing::config::LimitsConfig::default(),
        ),
        registry: Arc::new(RegistryDouble::default()),
        scope: AccessScope::for_tenant(TENANT),
        provider,
    }
}

/// A row the whole aggregate rule set passes: recurring, flat, tax-exclusive,
/// carrying the billing timing, proration contract and rounding policy
/// `inst-pi-required` and `ROUNDING_POLICY_UNRESOLVED` each demand of one.
fn publishable_row(amount_minor: i64) -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(amount_minor).expect("a non-negative amount"));
    PriceContent {
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: Some("advance".to_owned()),
        proration_contract: Some(ProrationContract {
            billing_anchor_policy: BillingAnchorPolicy::CalendarMonth,
            proration_basis: ProrationBasis::CalendarDaysActual,
            credit_on_downgrade: false,
        }),
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

/// A plan the whole aggregate rule set passes: one evergreen terminal phase, a
/// complete descriptor set, a tier and a frequency — [`seed_publishable_shape`]
/// in `tests/rest_support/mod.rs`'s own recipe, built here at the repository
/// seam instead of through the authoring routes.
async fn seed_plan(h: &Harness, plan_id: Uuid, phase_id: Uuid) {
    let plan = PlanId::new(plan_id);
    let created = h
        .plans
        .create_draft(
            &h.scope,
            NewPlanDraft {
                plan_id: plan,
                tenant_id: TENANT,
                created_by: ACTOR,
                created_at_utc: at(10),
                sku_id: Some(Uuid::from_u128(0x5_c1)),
                plan_tier: Some("gold".to_owned()),
                billing_cycle: Some(BillingCycle::Recurring),
                frequency: Some(Frequency::Monthly),
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
        .expect("create the draft plan");
    let after_phases = h
        .shapes
        .replace_phases(
            &h.scope,
            TENANT,
            plan,
            created.revision,
            created.row_version,
            vec![PlanPhase {
                phase_id: PhaseId::new(phase_id),
                kind: PhaseKind::Evergreen,
                ordinal: 0,
                converts_to_phase_id: None,
                phase_duration_days: None,
                display_trial_days: None,
            }],
            stamp(),
        )
        .await
        .expect("attach the phase chain");
    h.shapes
        .set_descriptor_set(
            &h.scope,
            TENANT,
            plan,
            created.revision,
            after_phases.row_version,
            DescriptorSet {
                invoice_line_template: Some("{plan}".to_owned()),
                gl_code: Some("4000".to_owned()),
                itemization_rule: Some("per_charge".to_owned()),
                additional: std::collections::BTreeMap::new(),
            },
            stamp(),
        )
        .await
        .expect("attach the descriptor set");
    publish_plan(h, plan, created.revision).await;
}

async fn publish_plan(h: &Harness, plan: PlanId, revision: u64) {
    let current = h
        .plans
        .find_revision(&h.scope, TENANT, plan, revision)
        .await
        .expect("read the revision")
        .expect("the revision exists");
    let scope = h.scope.clone();
    let (_, outcome) = h
        .provider
        .db()
        .in_transaction::<_, bss_pricing::infra::storage::RepoError, _>(move |txn| {
            Box::pin(async move {
                bss_pricing::infra::storage::repo::plan_repo::publish_revision(
                    txn,
                    &scope,
                    TENANT,
                    plan,
                    revision,
                    current.row_version,
                )
                .await
            })
        })
        .await;
    outcome.expect("publish the seeded plan revision");
}

fn scope_key(plan: PlanId, phase: Uuid, region: &str) -> ScopeKey {
    ScopeKey::new(
        plan,
        CurrencyCode::new("USD").expect("currency"),
        Region::new(region).expect("region"),
        PhaseId::new(phase),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("scope key")
}

/// Author and publish one row, with a scheduled coverage window
/// (`inst-wc-required`) — the shape every row this suite selects for
/// repricing needs.
async fn seed_published_row(
    h: &Harness,
    plan: PlanId,
    phase: Uuid,
    region: &str,
    amount_minor: i64,
) -> Uuid {
    let price_id = Uuid::now_v7();
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id,
                scope_key: scope_key(plan, phase, region),
                content: publishable_row(amount_minor),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author the price row");
    common::schedule_coverage_window(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        price_id,
        stamp(),
    )
    .await;
    let scope = h.scope.clone();
    let (_, outcome) = h
        .provider
        .db()
        .in_transaction::<_, bss_pricing::infra::storage::RepoError, _>(move |txn| {
            Box::pin(async move {
                bss_pricing::infra::storage::repo::price_repo::publish_rows(
                    txn,
                    &scope,
                    TENANT,
                    plan,
                    &[(price_id, RowVersion::new(0))],
                    &bss_pricing::domain::tax_display::RegionTaxReadiness::empty(),
                )
                .await
            })
        })
        .await;
    outcome.expect("publish the seeded price row");
    price_id
}

/// A **stray draft** row: authored, never published, no window scheduled —
/// exactly the shape this suite's module doc explains as the mechanism that
/// fails plan B's aggregate pass. On a different key from every published row
/// this suite seeds, so it collides with nothing.
async fn seed_stray_draft(h: &Harness, plan: PlanId, phase: Uuid) {
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id: Uuid::now_v7(),
                scope_key: scope_key(plan, phase, "apac"),
                content: publishable_row(5_000),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author the stray draft");
}

fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: ACTOR,
        recorded_at: at(10),
        correlation_id: CORRELATION,
    }
}

fn apply_stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: ACTOR,
        recorded_at: at(12),
        correlation_id: CORRELATION,
    }
}

/// [`api::rest::repricing_runs::frozen_report`]'s exact wire shape — this
/// suite's only way to hand [`apply_run_in`] an adjustment and a changeover,
/// since it takes neither directly and reads both off the run's own stored
/// report (`adjustment_of_report`/`changeover_of_report`, `pub(crate)` to that
/// module and unreachable from an integration test).
fn report() -> serde_json::Value {
    serde_json::json!({
        "selector": serde_json::Value::Null,
        "adjustment": {
            "adjustment_kind": "discount",
            "magnitude_kind": "percent_bp",
            "adjustment_value": 500,
            "amounts": {},
        },
        "changeover": changeover().to_rfc3339(),
        "selected": 0,
    })
}

/// Open a run already `committing`, with `price_ids` frozen into its journal
/// `pending` — [`open_repricing_run`]'s own two writes, built directly rather
/// than through HTTP: this suite's subject is the apply, not the freeze.
async fn open_committing_run(h: &Harness, price_ids: &[Uuid]) -> Uuid {
    let operation_id = Uuid::now_v7();
    let conn = h.provider.conn().expect("conn");
    bulk_repo::open(
        &conn,
        &h.scope,
        NewBulkOperation {
            operation_id,
            tenant_id: TENANT,
            kind: BulkKind::Repricing,
            client_key: operation_id.to_string(),
            report: report(),
            submitted_by: ACTOR,
            submitted_at: at(11),
        },
    )
    .await
    .expect("open the run");
    let rows: Vec<NewJournalRow> = price_ids
        .iter()
        .map(|&price_id| NewJournalRow {
            run_id: operation_id,
            price_id,
            tenant_id: TENANT,
        })
        .collect();
    repricing_journal_repo::open_rows(&conn, &h.scope, &rows)
        .await
        .expect("freeze the journal");
    bulk_repo::advance(
        &conn,
        &h.scope,
        TENANT,
        operation_id,
        BulkState::Committing,
        report(),
        at(11),
    )
    .await
    .expect("enter committing");
    operation_id
}

async fn journal_state(
    h: &Harness,
    run_id: Uuid,
    price_id: Uuid,
) -> (JournalState, Option<String>, Option<Uuid>) {
    let rows = repricing_journal_repo::list_for_run(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        run_id,
    )
    .await
    .expect("read the journal");
    let row = rows
        .into_iter()
        .find(|row| row.price_id == price_id)
        .expect("the row is on this run's journal");
    (row.state, row.failure_reason, row.applied_price_id)
}

// ---------------------------------------------------------------------------
// Step 1's atomicity test.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_plan_whose_aggregate_pass_fails_applies_none_of_that_plans_rows() {
    let h = harness().await;

    // Plan A: one clean row, nothing else on it. Applies whole.
    let plan_a = Uuid::now_v7();
    let phase_a = Uuid::now_v7();
    seed_plan(&h, plan_a, phase_a).await;
    let row_a = seed_published_row(&h, PlanId::new(plan_a), phase_a, "eu", 9_900).await;

    // Plan B: two clean, selected rows, plus a stray draft on a third key —
    // never selected, never touched, and the reason plan B's aggregate pass
    // fails (see the module doc).
    let plan_b = Uuid::now_v7();
    let phase_b = Uuid::now_v7();
    seed_plan(&h, plan_b, phase_b).await;
    let row_b1 = seed_published_row(&h, PlanId::new(plan_b), phase_b, "eu", 10_000).await;
    let row_b2 = seed_published_row(&h, PlanId::new(plan_b), phase_b, "us", 12_000).await;
    seed_stray_draft(&h, PlanId::new(plan_b), phase_b).await;

    let run_id = open_committing_run(&h, &[row_a, row_b1, row_b2]).await;

    let outcome = apply_run_in(
        &h.provider,
        &h.policies,
        &(Arc::clone(&h.registry) as Arc<dyn CatalogVersionRegistryV1>),
        &ctx(),
        &h.scope,
        TENANT,
        run_id,
        apply_stamp(),
    )
    .await
    .expect("apply_run_in itself does not fail — only a plan's own rows do");

    assert_eq!(outcome.applied, 1, "plan A's one row: {outcome:?}");
    assert_eq!(
        outcome.failed, 2,
        "a partial plan is the one outcome D-134 forbids — plan B's whole selection fails: \
         {outcome:?}"
    );

    let (state_a, reason_a, applied_price_id_a) = journal_state(&h, run_id, row_a).await;
    assert_eq!(state_a, JournalState::Applied, "plan A applies whole");
    assert!(reason_a.is_none());
    assert!(
        applied_price_id_a.is_some(),
        "a real successor exists for row A"
    );

    let (state_b1, reason_b1, applied_b1) = journal_state(&h, run_id, row_b1).await;
    let (state_b2, reason_b2, applied_b2) = journal_state(&h, run_id, row_b2).await;
    assert_eq!(
        state_b1,
        JournalState::Failed,
        "plan B's aggregate pass refused it: no partial plan"
    );
    assert_eq!(state_b2, JournalState::Failed);
    assert!(
        applied_b1.is_none(),
        "plan B applied nothing, not even partially"
    );
    assert!(applied_b2.is_none());

    let reasons: BTreeSet<String> = [reason_b1, reason_b2]
        .into_iter()
        .map(|reason| reason.expect("a failed row carries a reason"))
        .collect();
    assert_eq!(
        reasons.len(),
        1,
        "every row of the plan fails with the shared reason: {reasons:?}"
    );
    let reason = reasons.into_iter().next().expect("exactly one");
    assert!(
        reason.contains("WINDOW_COVERAGE_MISSING"),
        "the aggregate-only rule the stray draft trips: {reason}"
    );

    // Plan A's row genuinely left the published plane and a real successor
    // stands on its key — the read that would catch a rollback silently
    // reaching plan A's writes too.
    let plan_a_rows = bss_pricing::infra::storage::repo::price_repo::load_for_plan(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        PlanId::new(plan_a),
        &[
            bss_pricing::domain::lifecycle::LifecycleState::Published,
            bss_pricing::domain::lifecycle::LifecycleState::Superseded,
        ],
    )
    .await
    .expect("read plan A's rows");
    assert_eq!(
        plan_a_rows
            .iter()
            .filter(
                |r| r.lifecycle_state == bss_pricing::domain::lifecycle::LifecycleState::Published
            )
            .count(),
        1,
        "plan A holds exactly one published row — the successor: {plan_a_rows:?}"
    );
    assert_eq!(
        plan_a_rows
            .iter()
            .filter(
                |r| r.lifecycle_state == bss_pricing::domain::lifecycle::LifecycleState::Superseded
            )
            .count(),
        1,
        "and the predecessor, superseded rather than gone: {plan_a_rows:?}"
    );

    // Plan B's two selected rows are untouched: still published, under their
    // original ids, nothing superseded — the rollback the plan's own
    // transaction performed.
    let plan_b_rows = bss_pricing::infra::storage::repo::price_repo::load_for_plan(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        PlanId::new(plan_b),
        &[bss_pricing::domain::lifecycle::LifecycleState::Published],
    )
    .await
    .expect("read plan B's published rows");
    let plan_b_published: BTreeSet<Uuid> = plan_b_rows.iter().map(|r| r.price_id).collect();
    assert!(
        plan_b_published.contains(&row_b1) && plan_b_published.contains(&row_b2),
        "plan B's rows still stand under their own ids — the transaction rolled back whole: \
         {plan_b_rows:?}"
    );

    // ------------------------------------------------------------------
    // Step 5's idempotency test, over this exact run: applied and failed
    // rows both already decided.
    // ------------------------------------------------------------------
    let redrive = apply_run_in(
        &h.provider,
        &h.policies,
        &(Arc::clone(&h.registry) as Arc<dyn CatalogVersionRegistryV1>),
        &ctx(),
        &h.scope,
        TENANT,
        run_id,
        apply_stamp(),
    )
    .await
    .expect("a redrive over an already-decided journal does not fail");

    assert_eq!(
        redrive, outcome,
        "a re-run over a journal containing applied rows applies nothing twice and answers \
         the same outcome"
    );
    let (_, _, applied_price_id_a_again) = journal_state(&h, run_id, row_a).await;
    assert_eq!(
        applied_price_id_a_again, applied_price_id_a,
        "the same successor, not a second one minted by a double-apply"
    );
    let plan_a_rows_again = bss_pricing::infra::storage::repo::price_repo::load_for_plan(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        PlanId::new(plan_a),
        &[bss_pricing::domain::lifecycle::LifecycleState::Published],
    )
    .await
    .expect("read plan A's rows again");
    assert_eq!(
        plan_a_rows_again.len(),
        1,
        "still exactly one published row on plan A's key — a double-apply would mint a second: \
         {plan_a_rows_again:?}"
    );

    let stored = bulk_repo::read(&h.provider.conn().expect("conn"), &h.scope, TENANT, run_id)
        .await
        .expect("read the run")
        .expect("the run exists");
    assert_eq!(
        stored.state,
        BulkState::CompletedWithConflicts,
        "one plan applied, one failed — a success with conflicts (`inst-bk-phase2`'s reading, \
         carried over from bulk import to repricing): {stored:?}"
    );
}
