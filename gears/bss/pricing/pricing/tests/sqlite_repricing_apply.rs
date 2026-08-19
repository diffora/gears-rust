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
use bss_pricing::domain::audit::AuditSubjectKind;
use bss_pricing::domain::bulk::{BulkKind, BulkState, JournalState};
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::contracts::{BillingAnchorPolicy, ProrationBasis, ProrationContract};
use bss_pricing::domain::money::{CurrencyCode, MinorAmount, RateMinor};
use bss_pricing::domain::plan_shape::{
    BillingCycle, DescriptorSet, Frequency, PhaseKind, PlanPhase,
};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{
    BillingGranularity, ModelKind, PriceRow, TierAggregationWindow, TierBand,
};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::repricing::apply_run_in;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::approval_repo::NewApproval;
use bss_pricing::infra::storage::repo::repricing_journal_repo::NewJournalRow;
use bss_pricing::infra::storage::repo::{
    IdempotencyGate, NewBulkOperation, NewPlanDraft, NewPriceDraft, PlanRepo, PlanShapeRepo,
    PolicyObjectRepo, PriceRepo, approval_repo, audit_repo, bulk_repo, price_repo,
    repricing_journal_repo,
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

impl RegistryDouble {
    /// The request ids this double has been asked for, in no order.
    ///
    /// The map is keyed on the request id and `or_insert_with` is idempotent on
    /// it, so this counts **distinct** requests — which is what a claim about a
    /// stranded handle means: one act, one id, however many times it is retried.
    fn requested(&self) -> Vec<String> {
        self.issued
            .lock()
            .expect("no panics in the double")
            .keys()
            .cloned()
            .collect()
    }
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
        // Field order matches the struct definition's, so every field here is
        // a `.clone()` of the cheap handle rather than the one bare move that
        // would otherwise have to come last.
        provider: provider.clone(),
        plans: PlanRepo::new(provider.clone()),
        shapes: PlanShapeRepo::new(provider.clone()),
        prices: PriceRepo::new(provider.clone()),
        policies: PolicyObjectRepo::new(
            provider.clone(),
            &bss_pricing::config::LimitsConfig::default(),
        ),
        registry: Arc::new(RegistryDouble::default()),
        scope: AccessScope::for_tenant(TENANT),
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

/// [`publishable_row`]'s graduated sibling: a two-band ladder, priced from
/// **rates** rather than from `amount_minor` (D-311).
///
/// Its bands are the pair `a_markup_that_overflows_one_band_of_a_ladder_is_out_of_range_for_the_row`
/// uses — one that survives a fat-fingered markup and one that does not — because
/// the defect only exists where the two disagree.
fn publishable_graduated_row() -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("cloudlets".to_owned());
    // `EVAL_POLICY_MISSING`'s two operands for a tiered usage row: the unit the
    // bands are counted in, and the window the tier counter resets on.
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = vec![
        TierBand::closed(
            0,
            1_000,
            RateMinor::from_nano_minor(1_000_000_000).expect("a non-negative rate"),
        ),
        TierBand::open(
            1_000,
            RateMinor::from_nano_minor(100_000_000_000).expect("a non-negative rate"),
        ),
    ];
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

/// [`report`] under a **markup** of `value_bp`, which is the adjustment the
/// out-of-range refusal is about: `magnitude_out_of_range` bounds a discount at
/// 10 000 bp and leaves a markup unbounded above zero.
fn markup_report(value_bp: i64) -> serde_json::Value {
    serde_json::json!({
        "selector": serde_json::Value::Null,
        "adjustment": {
            "adjustment_kind": "markup",
            "magnitude_kind": "percent_bp",
            "adjustment_value": value_bp,
            "amounts": {},
        },
        "changeover": changeover().to_rfc3339(),
        "selected": 0,
    })
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
                plan_name: None,
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

/// [`scope_key`]'s usage sibling: `graduated` and `volume` are usage-only kinds
/// (`MODEL_KIND_CHARGEKIND_MISMATCH`), so a ladder cannot ride a recurring key.
fn usage_scope_key(plan: PlanId, phase: Uuid, region: &str) -> ScopeKey {
    ScopeKey::new(
        plan,
        CurrencyCode::new("USD").expect("currency"),
        Region::new(region).expect("region"),
        PhaseId::new(phase),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Usage,
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
    seed_published_content(
        h,
        scope_key(plan, phase, region),
        publishable_row(amount_minor),
    )
    .await
}

/// [`seed_published_row`] over content the caller chooses, for the cases whose
/// subject is the row's **shape** rather than its amount.
async fn seed_published_content(h: &Harness, key: ScopeKey, content: PriceContent) -> Uuid {
    let price_id = Uuid::now_v7();
    let plan = key.plan_id();
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id,
                scope_key: key,
                content,
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
                    None,
                )
                .await
            })
        })
        .await;
    outcome.expect("publish the seeded price row");
    price_id
}

/// A real, currently-published `existing_grandfathered` generation on
/// `plan`/`phase`/`region` — [`seed_published_row`]'s own recipe (a scheduled
/// coverage window included, so the aggregate pass's `WINDOW_COVERAGE_MISSING`
/// never fires on it), over a key whose `priceEligibility` and `cohort` axes
/// mark it immutable in price (Foundation §4.3, `inst-mp-grandfathered`).
async fn seed_grandfathered_row(
    h: &Harness,
    plan: PlanId,
    phase: Uuid,
    region: &str,
    generation: DateTime<Utc>,
    amount_minor: i64,
) -> Uuid {
    let price_id = Uuid::now_v7();
    let key = ScopeKey::new(
        plan,
        CurrencyCode::new("USD").expect("currency"),
        Region::new(region).expect("region"),
        PhaseId::new(phase),
        PriceEligibility::ExistingGrandfathered,
        ChargeKind::Recurring,
        Cohort::Generation(generation),
    )
    .expect("a grandfathered eligibility pairs with a non-none cohort");
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id,
                scope_key: key,
                content: publishable_row(amount_minor),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author the grandfathered row");
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
                    None,
                )
                .await
            })
        })
        .await;
    outcome.expect("publish the seeded grandfathered row");
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

/// One draft row on `plan`, on a phase of its own so the key is unique, for the
/// only purpose of being lockable: the repricing journal's `price_id` carries a
/// foreign key into `pricing_price`, and
/// [`a_future_dropped_while_it_is_still_taking_its_locks_releases_them_too`]
/// needs many rows on one run's journal rather than many *priced* rows. Nothing
/// publishes these, and nothing reads their content.
async fn seed_lockable_draft(h: &Harness, plan: PlanId) -> Uuid {
    let price_id = Uuid::now_v7();
    h.prices
        .create_draft(
            &h.scope,
            TENANT,
            NewPriceDraft {
                price_id,
                scope_key: scope_key(plan, Uuid::now_v7(), "eu"),
                content: publishable_row(1_000),
                created_by: ACTOR,
                created_at_utc: at(10),
                correlation_id: CORRELATION,
            },
        )
        .await
        .expect("author a lockable draft");
    price_id
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
    open_committing_run_reporting(h, price_ids, report()).await
}

/// [`open_committing_run`] under an adjustment the caller chooses.
async fn open_committing_run_reporting(
    h: &Harness,
    price_ids: &[Uuid],
    report: serde_json::Value,
) -> Uuid {
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
            request_hash: IdempotencyGate::payload_hash(&operation_id.to_string()),
            report: report.clone(),
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
        BulkState::Validating,
        BulkState::Committing,
        report,
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

/// The M-6 refusal on the **apply path**, which is the half nothing drove
/// (review F7, 2026-08-19).
///
/// `projection_out_of_range` has a unit test in `domain::repricing_tests` and the
/// `if` that calls it in `apply_rows_in` had none: deleting the refusal left the
/// whole suite green while the defect it exists for — a `graduated` ladder under a
/// markup that overflows one band and not another, committed `applied` with one
/// band moved and its sibling at its published rate — was live again. A unit test
/// of the callee cannot see that nobody calls it.
#[tokio::test]
async fn a_markup_that_overflows_one_band_fails_the_whole_plan_rather_than_moving_the_other() {
    let h = harness().await;

    let plan = Uuid::now_v7();
    let phase = Uuid::now_v7();
    seed_plan(&h, plan, phase).await;
    let ladder = seed_published_content(
        &h,
        usage_scope_key(PlanId::new(plan), phase, "eu"),
        publishable_graduated_row(),
    )
    .await;
    // A second, ordinary row on the same plan, **in the ladder's own market**:
    // D-134's unit is the plan, so the refusal has to take this one down with it,
    // and a plan selling usage in `eu` with no recurring base row there fails the
    // aggregate pass instead — which would make this case pass for the wrong
    // reason.
    let flat = seed_published_row(&h, PlanId::new(plan), phase, "eu", 12_000).await;

    // The fat-fingered extra six digits nothing in the authoring path refuses.
    let run_id =
        open_committing_run_reporting(&h, &[ladder, flat], markup_report(1_000_000_000_000)).await;

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
    .expect("apply_run_in itself does not fail - only a plan's own rows do");

    assert_eq!(outcome.applied, 0, "nothing applied: {outcome:?}");
    assert_eq!(outcome.failed, 2, "the plan is the unit: {outcome:?}");

    let (ladder_state, ladder_reason, ladder_applied) = journal_state(&h, run_id, ladder).await;
    assert_eq!(ladder_state, JournalState::Failed);
    assert!(
        ladder_applied.is_none(),
        "no successor stands on the ladder's key - a written successor is the unauthored ladder \
         itself"
    );
    let reason = ladder_reason.expect("a failed row carries a reason");
    assert!(
        reason.contains("leaves the representable range"),
        "the refusal names why, so an operator can re-run with a smaller magnitude: {reason}"
    );

    let (flat_state, _, flat_applied) = journal_state(&h, run_id, flat).await;
    assert_eq!(
        flat_state,
        JournalState::Failed,
        "the plan's other row fails with it (D-134): a partial plan is the one outcome forbidden"
    );
    assert!(flat_applied.is_none());
}

/// The **positive control** for the case above: the same ladder under an ordinary
/// markup applies both bands.
///
/// Without it the refusal would pass against an apply that refused every
/// `graduated` row, which is the shape that makes a guard read as coverage.
#[tokio::test]
async fn the_same_ladder_under_an_ordinary_markup_applies() {
    let h = harness().await;

    let plan = Uuid::now_v7();
    let phase = Uuid::now_v7();
    seed_plan(&h, plan, phase).await;
    let ladder = seed_published_content(
        &h,
        usage_scope_key(PlanId::new(plan), phase, "eu"),
        publishable_graduated_row(),
    )
    .await;

    // The recurring base row the aggregate pass requires of a plan selling in this
    // market; not selected for repricing, so it changes nothing this case reads.
    seed_published_row(&h, PlanId::new(plan), phase, "eu", 12_000).await;

    let run_id = open_committing_run_reporting(&h, &[ladder], markup_report(500)).await;

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
    .expect("apply_run_in itself does not fail");

    let (state, reason, applied) = journal_state(&h, run_id, ladder).await;
    assert_eq!(
        outcome.applied, 1,
        "an ordinary markup computes on both bands and must not be refused: {outcome:?}; the \
         journal says {state:?} / {reason:?}"
    );
    assert_eq!(state, JournalState::Applied);
    assert!(reason.is_none());
    assert!(applied.is_some(), "a real successor stands on the key");
}

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
    .expect("apply_run_in itself does not fail - only a plan's own rows do");

    assert_eq!(outcome.applied, 1, "plan A's one row: {outcome:?}");
    assert_eq!(
        outcome.failed, 2,
        "a partial plan is the one outcome D-134 forbids - plan B's whole selection fails: \
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
    let clean_plan_rows = bss_pricing::infra::storage::repo::price_repo::load_for_plan(
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
        clean_plan_rows
            .iter()
            .filter(
                |r| r.lifecycle_state == bss_pricing::domain::lifecycle::LifecycleState::Published
            )
            .count(),
        1,
        "plan A holds exactly one published row - the successor: {clean_plan_rows:?}"
    );
    assert_eq!(
        clean_plan_rows
            .iter()
            .filter(
                |r| r.lifecycle_state == bss_pricing::domain::lifecycle::LifecycleState::Superseded
            )
            .count(),
        1,
        "and the predecessor, superseded rather than gone: {clean_plan_rows:?}"
    );

    // Plan B's two selected rows are untouched: still published, under their
    // original ids, nothing superseded — the rollback the plan's own
    // transaction performed.
    let broken_plan_rows = bss_pricing::infra::storage::repo::price_repo::load_for_plan(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        PlanId::new(plan_b),
        &[bss_pricing::domain::lifecycle::LifecycleState::Published],
    )
    .await
    .expect("read plan B's published rows");
    let broken_plan_published: BTreeSet<Uuid> =
        broken_plan_rows.iter().map(|r| r.price_id).collect();
    assert!(
        broken_plan_published.contains(&row_b1) && broken_plan_published.contains(&row_b2),
        "plan B's rows still stand under their own ids - the transaction rolled back whole: \
         {broken_plan_rows:?}"
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
    let clean_plan_rows_again = bss_pricing::infra::storage::repo::price_repo::load_for_plan(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        PlanId::new(plan_a),
        &[bss_pricing::domain::lifecycle::LifecycleState::Published],
    )
    .await
    .expect("read plan A's rows again");
    assert_eq!(
        clean_plan_rows_again.len(),
        1,
        "still exactly one published row on plan A's key - a double-apply would mint a second: \
         {clean_plan_rows_again:?}"
    );

    let stored = bulk_repo::read(&h.provider.conn().expect("conn"), &h.scope, TENANT, run_id)
        .await
        .expect("read the run")
        .expect("the run exists");
    assert_eq!(
        stored.state,
        BulkState::CompletedWithConflicts,
        "one plan applied, one failed - a success with conflicts (`inst-bk-phase2`'s reading, \
         carried over from bulk import to repricing): {stored:?}"
    );
}

// ---------------------------------------------------------------------------
// `inst-mp-grandfathered` clause 2 — the apply's own per-row refusal of an
// explicitly-selected grandfathered row (task 6).
// ---------------------------------------------------------------------------

/// A selector that names the eligibility axis outright still expands over
/// `existing_grandfathered` rows and freezes them `pending`
/// (`RunSelector::admits_grandfathered`, `domain::repricing`'s own module doc) —
/// dropping them would be the silent skip the clause forbids. This suite drives
/// the apply directly at the repository seam, exactly as the atomicity test
/// above does, so the journal below is built by hand to carry exactly what such
/// a selector would have frozen: the grandfathered row, `pending`, beside an
/// ordinary row the same run also selected.
///
/// The ordinary row is the positive control the task-6 brief names in as many
/// words: without it, a handler that failed every row on the plan (the shape
/// the plan-wide `Err` used by `adjusts_rate`'s own refusal would produce, since
/// both rows share one plan and one transaction) would pass this test too.
#[tokio::test]
async fn a_grandfathered_row_the_selector_named_explicitly_is_refused_while_the_plans_other_row_applies()
 {
    let h = harness().await;

    let plan = Uuid::now_v7();
    let phase = Uuid::now_v7();
    seed_plan(&h, plan, phase).await;

    let ordinary_row = seed_published_row(&h, PlanId::new(plan), phase, "eu", 9_900).await;
    let grandfathered_row =
        seed_grandfathered_row(&h, PlanId::new(plan), phase, "us", at(1), 5_000).await;

    let run_id = open_committing_run(&h, &[ordinary_row, grandfathered_row]).await;

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
    .expect("apply_run_in itself does not fail over a grandfathered row - only that row does");

    assert_eq!(
        outcome.applied, 1,
        "the run's other row still applies - the positive control: {outcome:?}"
    );
    assert_eq!(
        outcome.failed, 1,
        "the grandfathered row is refused outright, neither silently skipped nor applied: \
         {outcome:?}"
    );

    let (ord_state, ord_reason, ord_applied) = journal_state(&h, run_id, ordinary_row).await;
    assert_eq!(
        ord_state,
        JournalState::Applied,
        "one grandfathered row on the plan does not take the plan's other row down with it"
    );
    assert!(ord_reason.is_none());
    assert!(
        ord_applied.is_some(),
        "a real successor exists for the ordinary row"
    );

    let (gf_state, gf_reason, gf_applied) = journal_state(&h, run_id, grandfathered_row).await;
    assert_eq!(
        gf_state,
        JournalState::Failed,
        "inst-mp-grandfathered clause 2: an explicit attempt to include the class fails that row \
         with a per-row validation error"
    );
    assert!(gf_applied.is_none(), "never repriced");
    let reason = gf_reason.expect("a failed row carries a reason an operator reads");
    assert!(
        reason.to_lowercase().contains("grandfathered"),
        "the reason names why: {reason}"
    );

    // The row itself never moved: still published under its own id, and no
    // successor was authored on its key - proof this is a refusal and not a
    // reprice wearing a different journal state.
    let plan_rows = bss_pricing::infra::storage::repo::price_repo::load_for_plan(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        PlanId::new(plan),
        &[
            bss_pricing::domain::lifecycle::LifecycleState::Published,
            bss_pricing::domain::lifecycle::LifecycleState::Superseded,
        ],
    )
    .await
    .expect("read the plan's rows");
    let gf_after = plan_rows
        .iter()
        .find(|r| r.price_id == grandfathered_row)
        .expect("the grandfathered row still exists");
    assert_eq!(
        gf_after.lifecycle_state,
        bss_pricing::domain::lifecycle::LifecycleState::Published,
        "still published, never superseded: {gf_after:?}"
    );
    assert!(
        !plan_rows
            .iter()
            .any(|r| r.scope_key == gf_after.scope_key && r.price_id != grandfathered_row),
        "no successor was authored on the grandfathered row's own key"
    );
}

/// `inst-mp-grandfathered` clause 1's own half, already built
/// (`RunSelector::admits_grandfathered`) — proven here rather than assumed, so
/// this task does not close clause 2 over an expansion that silently regressed
/// clause 1: a selector that does not name the eligibility axis excludes the
/// `existing_grandfathered` class from the expansion entirely, so no such row
/// is ever frozen into a journal for the apply to see.
#[tokio::test]
async fn a_selector_that_does_not_name_the_eligibility_axis_excludes_grandfathered_rows() {
    let h = harness().await;

    let plan = Uuid::now_v7();
    let phase = Uuid::now_v7();
    seed_plan(&h, plan, phase).await;

    let ordinary_row = seed_published_row(&h, PlanId::new(plan), phase, "eu", 9_900).await;
    let grandfathered_row =
        seed_grandfathered_row(&h, PlanId::new(plan), phase, "us", at(1), 5_000).await;

    let selector = bss_pricing::domain::repricing::RunSelector {
        plan_id: Some(PlanId::new(plan)),
        ..Default::default()
    };
    assert!(
        !selector.admits_grandfathered(),
        "an absent eligibility axis excludes the class - RunSelector's own rule"
    );

    let selected = bss_pricing::infra::storage::repo::price_repo::load_published_for_selector(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        &selector,
    )
    .await
    .expect("expand the selector");

    assert!(
        selected.contains(&ordinary_row),
        "the ordinary row is in scope: {selected:?}"
    );
    assert!(
        !selected.contains(&grandfathered_row),
        "clause 1: an unnamed eligibility axis structurally excludes the grandfathered row - it \
         is never frozen into a journal for the apply to see: {selected:?}"
    );
}

/// The one interaction task 6 newly creates, found by review rather than by this
/// suite's first pass: a plan carrying an explicitly-selected grandfathered row
/// **and** an aggregate-pass failure, together.
///
/// `apply_rows_in`'s new check marks the grandfathered row `failed` **inside**
/// the plan's own transaction, before the aggregate pass runs — the first writer
/// to call `mark_failed` from in there rather than from `apply_run_in`'s
/// pre-existing, separate-transaction catch-all. When the aggregate pass then
/// fails (a stray draft, this suite's own mechanism — see the module doc), the
/// whole transaction rolls back, undoing that in-transaction mark along with
/// everything else: the row reads `pending` again the instant the transaction
/// returns, exactly as if the check had never run. It is the **catch-all**, in
/// its own separate transaction, that has to reach the row a second time. This
/// test pins that it actually does — the row ends `failed`, never left
/// `pending` — and, since the in-transaction mark's own reason was rolled back
/// with it, that the row carries the plan's **shared** aggregate-failure reason
/// rather than the `inst-mp-grandfathered` reason it was marked with the first
/// time. Pinning what actually happens rather than the tidier thing a reader
/// might assume.
#[tokio::test]
async fn a_grandfathered_row_whose_plan_also_fails_its_aggregate_pass_still_ends_failed_with_the_plans_shared_reason()
 {
    let h = harness().await;

    let plan = Uuid::now_v7();
    let phase = Uuid::now_v7();
    seed_plan(&h, plan, phase).await;

    let ordinary_row = seed_published_row(&h, PlanId::new(plan), phase, "eu", 9_900).await;
    let grandfathered_row =
        seed_grandfathered_row(&h, PlanId::new(plan), phase, "us", at(1), 5_000).await;
    // This suite's own mechanism (see the module doc) for tripping the
    // aggregate-only `WINDOW_COVERAGE_MISSING`: a stray draft on a third,
    // untouched key, never selected by this run.
    seed_stray_draft(&h, PlanId::new(plan), phase).await;

    let run_id = open_committing_run(&h, &[ordinary_row, grandfathered_row]).await;

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
    .expect("apply_run_in itself does not fail - only the plan's own rows do");

    assert_eq!(
        outcome.applied, 0,
        "the plan's aggregate pass takes its whole selection down, the grandfathered row \
         included: {outcome:?}"
    );
    assert_eq!(
        outcome.failed, 2,
        "both of this plan's rows end failed: {outcome:?}"
    );

    let (ord_state, ord_reason, ord_applied) = journal_state(&h, run_id, ordinary_row).await;
    let (gf_state, gf_reason, gf_applied) = journal_state(&h, run_id, grandfathered_row).await;

    assert_eq!(ord_state, JournalState::Failed);
    assert!(ord_applied.is_none());

    assert_eq!(
        gf_state,
        JournalState::Failed,
        "the in-transaction mark rolled back with the rest of the plan - the catch-all is what \
         has to reach this row a second time, and it does: the row is never left pending"
    );
    assert!(gf_applied.is_none(), "never repriced");

    let ord_reason = ord_reason.expect("a failed row carries a reason");
    let gf_reason = gf_reason.expect("a failed row carries a reason");
    assert_eq!(
        ord_reason, gf_reason,
        "both of this plan's rows share the one reason the catch-all wrote in its own separate \
         transaction, over both price_ids alike"
    );
    assert!(
        gf_reason.contains("WINDOW_COVERAGE_MISSING"),
        "the plan's real aggregate-pass violation - the shared reason a `Err` propagating out of \
         `apply_plan_in` renders, not the row-local check's own text: {gf_reason}"
    );
    assert!(
        !gf_reason.contains("inst-mp-grandfathered"),
        "the row-local reason the in-transaction check wrote does not survive the rollback that \
         also undid the mark itself, so the row must not still be read as carrying it: {gf_reason}"
    );

    // The grandfathered row itself never moved: still published under its own
    // id, no successor authored on its key at all - proof this is a refusal
    // both times, not a reprice that happened to land on the losing side of a
    // rollback.
    let plan_rows = bss_pricing::infra::storage::repo::price_repo::load_for_plan(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        PlanId::new(plan),
        &[
            bss_pricing::domain::lifecycle::LifecycleState::Published,
            bss_pricing::domain::lifecycle::LifecycleState::Superseded,
        ],
    )
    .await
    .expect("read the plan's rows");
    let gf_after = plan_rows
        .iter()
        .find(|r| r.price_id == grandfathered_row)
        .expect("the grandfathered row still exists");
    assert_eq!(
        gf_after.lifecycle_state,
        bss_pricing::domain::lifecycle::LifecycleState::Published,
        "still published, never superseded: {gf_after:?}"
    );
}

// ---------------------------------------------------------------------------
// Task 7 - the bulk lock and its Drop guard.
// ---------------------------------------------------------------------------

/// Block until `price_id`'s bulk lock is visibly held by `run_id`.
///
/// A test that cancelled the apply *before* the lock was ever taken would prove
/// nothing about the drop guard, so this is the precondition both cancellation
/// tests below establish first.
///
/// # Why the bound counts polls and not seconds
///
/// What this waits for is another task's progress, which no amount of polling
/// makes synchronous - so a bound has to exist, and the question is what it
/// should measure. A wall clock is the wrong thing: it measures the box, not the
/// apply. A 5s clock here was measured failing on a box carrying 24 busy
/// processes while the apply had in fact taken its lock - the clock had simply
/// run out first, and the failure said "never took its bulk lock", which was
/// false. **A poll count measures what the assertion actually means.** Every
/// iteration below sleeps, which hands the runtime a chance to poll every other
/// runnable task, and reads the database, which requires the driver to have made
/// real progress; so [`POLL_BUDGET`] iterations mean the apply had that many
/// opportunities to advance and did not take one. A loaded box makes each
/// iteration slower in wall time and *more* forgiving, never less - which is the
/// direction a bound in a test may err in.
async fn await_lock_taken(h: &Harness, run_id: Uuid, price_id: Uuid) {
    for _ in 0..POLL_BUDGET {
        let held = bulk_repo::lock_holder(
            &h.provider.conn().expect("conn"),
            &h.scope,
            TENANT,
            price_id,
        )
        .await
        .expect("read the lock");
        if held == Some(run_id) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    panic!(
        "the apply never took its bulk lock over {price_id} across {POLL_BUDGET} polls - each one \
         a chance for it to run and a read proving the driver was live, so this is the apply \
         wedged rather than a slow box; this test cannot prove anything about the drop guard \
         without first observing the lock taken"
    );
}

/// How many times either wait below hands the runtime a turn before calling the
/// thing it is waiting for wedged.
///
/// Sized for the diagnosis, not for a duration: the measured release takes **one
/// or two** polls (7-9ms), and the measured failure takes *all* of them, because
/// what fails is a release that never ran rather than one that ran late. Any
/// value comfortably past two therefore separates the two outcomes identically,
/// and this one is three orders of magnitude past it.
const POLL_BUDGET: u32 = 2_000;

/// Wait for the run to land terminal with its locks released after a
/// cancellation, and say honestly what it means when that never happens.
///
/// `RunLockGuard`'s release is a detached `Handle::spawn` - its own doc says so -
/// so a caller reading the run back the instant the future drops may still see it
/// `committing` for a beat. Something asynchronous has to be waited on. The trap
/// is what the giving-up bound is allowed to *claim*: a wall-clock deadline here
/// reads as "the release took longer than 5s", and that reading is what cost a
/// day. This suite's flake was reported as exactly that - "the drop guard never
/// released the lock within 5s", about one run in forty - and the finding behind
/// it was not latency at all. The guard's `Drop` had never run, because the
/// future was cancelled while `bulk_repo::take_locks` was still writing lock
/// rows, before the guard existed
/// ([`a_future_dropped_while_it_is_still_taking_its_locks_releases_them_too`]
/// pins that window deliberately). A number raised from 5 to 60 would have hidden
/// it just as well and for longer.
///
/// So the bound counts polls rather than seconds ([`POLL_BUDGET`], and see
/// [`await_lock_taken`] for why that is the honest unit), and the message names
/// the two things reaching it can mean - a release that was never spawned, and
/// one that is wedged - because both are defects in the guard and neither is a
/// slow test box. What it reports is what it read: the run, the locks still held
/// and by which run, and the live task count as a hint rather than a verdict (the
/// `sqlx` pool keeps tasks of its own, so a non-zero count is not proof the
/// release is among them).
async fn await_release_settled(
    h: &Harness,
    run_id: Uuid,
    locks: &[Uuid],
) -> bulk_repo::BulkOperationRecord {
    for _ in 0..POLL_BUDGET {
        match read_release_state(h, run_id, locks).await {
            (Some(run), held) if run.state != BulkState::Committing && held.is_empty() => {
                return run;
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
        }
    }
    let (run, held) = read_release_state(h, run_id, locks).await;
    let alive = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    panic!(
        "the dropped apply's bulk lock was never released and its run never landed terminal, \
         across {POLL_BUDGET} polls - each one a turn for the guard's detached release task and a \
         read proving the driver was live. This is not a slow release: it is one that never ran, \
         or one that is wedged, and the first thing to check is that the guard is armed *before* \
         the first lock row is written rather than after it. Rows still held (price_id, holder): \
         {held:?}. This runtime has {alive} live tasks, the `sqlx` pool's own among them, so read \
         that as a hint and not a verdict. run={run:?}"
    );
}

/// The run and whichever of `locks` is still held, read together - what
/// [`await_release_settled`] tests each turn and reports when it gives up, so the
/// two cannot drift apart.
async fn read_release_state(
    h: &Harness,
    run_id: Uuid,
    locks: &[Uuid],
) -> (Option<bulk_repo::BulkOperationRecord>, Vec<(Uuid, Uuid)>) {
    let run = bulk_repo::read(&h.provider.conn().expect("conn"), &h.scope, TENANT, run_id)
        .await
        .expect("read the run");
    let mut held = Vec::new();
    for &price_id in locks {
        if let Some(holder) = bulk_repo::lock_holder(
            &h.provider.conn().expect("conn"),
            &h.scope,
            TENANT,
            price_id,
        )
        .await
        .expect("read the lock")
        {
            held.push((price_id, holder));
        }
    }
    (run, held)
}

/// **The RED this task is about.** `infra::bulk`'s own sibling releases the
/// bulk lock in its `Ok`/`Err` match arms alone (Z8-8/Z9-5) - a shape that
/// misses two of `apply_run_in`'s three abnormal exits: a panic unwinds past a
/// match arm exactly as it unwinds past everything else, and a dropped future
/// - a client disconnect, a shutdown signal, a losing `select!` arm - never
/// runs any of this crate's own code again, match arm or not. This test is
/// the third one: cancel the future genuinely mid-flight (`JoinHandle::abort`,
/// tokio's own mechanism for exactly that shape) after confirming it has
/// already taken its bulk lock, and prove the lock and the run's state both
/// recover anyway. A green test that dropped the future *before* the lock was
/// ever taken would prove nothing about the guard - the polling loop below
/// exists so this test cannot pass that way.
#[tokio::test]
async fn a_future_dropped_mid_apply_releases_its_bulk_lock_and_lands_the_run_terminal() {
    let h = harness().await;

    // Two plans, so there is real work left in the per-plan loop between the
    // moment the lock becomes visible and the moment the whole run could
    // possibly finish - the margin this test cancels inside.
    let plan_a = Uuid::now_v7();
    let phase_a = Uuid::now_v7();
    seed_plan(&h, plan_a, phase_a).await;
    let row_a = seed_published_row(&h, PlanId::new(plan_a), phase_a, "eu", 9_900).await;

    let plan_b = Uuid::now_v7();
    let phase_b = Uuid::now_v7();
    seed_plan(&h, plan_b, phase_b).await;
    let row_b = seed_published_row(&h, PlanId::new(plan_b), phase_b, "eu", 10_000).await;

    let run_id = open_committing_run(&h, &[row_a, row_b]).await;

    let provider = h.provider.clone();
    let policies = h.policies.clone();
    let registry: Arc<dyn CatalogVersionRegistryV1> =
        Arc::clone(&h.registry) as Arc<dyn CatalogVersionRegistryV1>;
    let scope = h.scope.clone();

    let handle = tokio::spawn(async move {
        apply_run_in(
            &provider,
            &policies,
            &registry,
            &ctx(),
            &scope,
            TENANT,
            run_id,
            apply_stamp(),
        )
        .await
    });

    await_lock_taken(&h, run_id, row_a).await;

    // Cancel it. `JoinHandle::abort` is `select!`'s own mechanism for a losing
    // arm - the future stops running at its next await point and every one of
    // its locals, the drop guard included, drops right there.
    handle.abort();
    let joined = handle.await;
    match joined {
        Err(ref e) if e.is_cancelled() => {}
        other => panic!(
            "the task must have been genuinely cancelled mid-flight for this test to prove \
             anything about the drop guard, not merely finished before the abort landed: \
             {other:?}"
        ),
    }

    // Both rows' locks gone and the run off `committing`. The guard's release
    // is a detached task, so this waits - but on the release's own progress,
    // counted in turns of the runtime rather than in seconds.
    let run = await_release_settled(&h, run_id, &[row_a, row_b]).await;

    assert_ne!(
        run.state,
        BulkState::Committing,
        "no run is left committing after a dropped apply future: {run:?}"
    );
    assert!(
        run.state.is_terminal(),
        "and it lands on one of the seven states that is terminal: {run:?}"
    );
}

/// **The window the test above cannot see.** It cancels the apply once the lock
/// over `row_a` is visible, and with two rows that is *almost always* past the
/// point where the guard exists — so it passes, and passed for weeks, while the
/// same cancellation one statement earlier left the locks held forever. That is
/// where this suite's own flake came from: about one run in forty, the abort
/// landed while `bulk_repo::take_locks` was still inserting, and the run stayed
/// `committing` with its rows frozen until the process died.
///
/// `take_locks` writes **one independent statement per row** on a
/// non-transactional runner — its own doc says so, and says why (a partial set
/// has to be releasable) — so the first lock row is durable while the loop is
/// still awaiting the second insert. Arming the guard after that call therefore
/// leaves a window in which a lock is committed and nothing owns it: a client
/// disconnect there is precisely the "run stuck `committing` until an operator
/// intervenes" outcome the guard was written to prevent.
///
/// This test makes that window wide enough to hit on purpose rather than one in
/// forty: forty rows on the journal, and the cancellation fires the moment the
/// **first** of their locks appears, leaving thirty-nine inserts still to run
/// against a poll loop that notices the first within 2ms. Forty is what made it
/// deterministic in measurement rather than what looked like enough — eleven runs
/// against a guard armed after `take_locks` failed eleven times, each of them with
/// the apply's cancellation landing inside `take_locks`, and six runs against one
/// armed before it passed six times. Nothing else about the apply changes between
/// the two.
#[tokio::test]
async fn a_future_dropped_while_it_is_still_taking_its_locks_releases_them_too() {
    let h = harness().await;

    // Draft rows, not published ones: the journal's own foreign key into
    // `pricing_price` is the only thing the ids owe (bare uuids are refused with
    // `code: 787`), and nothing this test reaches ever prices them — the abort
    // lands while `take_locks` is still running, long before `apply_by_plan`.
    // A distinct phase per row is what keeps each scope key unique; the count is
    // what matters, one INSERT each inside `take_locks`, and the abort has to
    // land among them rather than after the last.
    let plan = Uuid::now_v7();
    let phase = Uuid::now_v7();
    seed_plan(&h, plan, phase).await;
    let mut price_ids: Vec<Uuid> = Vec::new();
    for _ in 0..40 {
        price_ids.push(seed_lockable_draft(&h, PlanId::new(plan)).await);
    }
    price_ids.sort_unstable();
    let first_lock = price_ids[0];
    let last_lock = price_ids[price_ids.len() - 1];

    let run_id = open_committing_run(&h, &price_ids).await;

    let provider = h.provider.clone();
    let policies = h.policies.clone();
    let registry: Arc<dyn CatalogVersionRegistryV1> =
        Arc::clone(&h.registry) as Arc<dyn CatalogVersionRegistryV1>;
    let scope = h.scope.clone();
    let handle = tokio::spawn(async move {
        apply_run_in(
            &provider,
            &policies,
            &registry,
            &ctx(),
            &scope,
            TENANT,
            run_id,
            apply_stamp(),
        )
        .await
    });

    // The *first* lock, so the other 39 inserts are still ahead of the abort.
    await_lock_taken(&h, run_id, first_lock).await;

    handle.abort();
    match handle.await {
        Err(ref e) if e.is_cancelled() => {}
        other => panic!(
            "the task must have been genuinely cancelled mid-flight for this test to prove \
             anything about the drop guard, not merely finished before the abort landed: \
             {other:?}"
        ),
    }

    // Both ends of the set: the first lock, taken before the old guard existed,
    // and the last, on whichever side of the abort it fell - the guard releases
    // the run's whole set, not the part it saw taken.
    let run = await_release_settled(&h, run_id, &[first_lock, last_lock]).await;

    assert_ne!(
        run.state,
        BulkState::Committing,
        "no run is left committing after a future cancelled while it was still taking its \
         locks: {run:?}"
    );
    assert!(
        run.state.is_terminal(),
        "and it lands on one of the seven states that is terminal: {run:?}"
    );
}

// A test for the narrowed ordinary-`Err` behaviour lives beside
// `release_lock_after_ordinary_failure` itself, in `src/infra/repricing.rs`'s
// own `#[cfg(test)]` module (`step0_probe`'s own precedent in this file) —
// not here. Forcing a genuine, non-corrupting `Err` out of `apply_run_in`
// requires racing a direct write against the run's own in-flight processing,
// and this crate's `sqlite::memory:` harness serialises that race away: a
// competing write from a second task can only ever land in the narrow gaps
// *between* the run's transactions, never during one.
//
// **The reason is not a single-connection pool**, which is what this comment
// used to say and what a later reader would have reasoned from. `connect_db`'s
// SQLite arm applies `ConnectOpts::default()` as given — `max_conns: Some(10)`
// (`libs/toolkit-db/src/pool_opts.rs`), with no in-memory special case like the
// config-driven path's (`options.rs`, `max_connections(1)` for a memory DSN) —
// so this harness really does open a second connection, and it was measured
// doing so. What holds the property up instead is one layer down: `sqlx` rewrites
// `sqlite::memory:` into a *named, shared-cache* database, so those connections
// share one store, and a reader or writer that meets an open write transaction's
// table lock **waits** for it (`sqlite3_unlock_notify`) rather than erroring or
// proceeding. Measured, not argued: a read issued from a second connection while
// a transaction held a written table returned only when that transaction ended,
// 451ms later. An earlier version of
// this test tried exactly that race and lost it reliably — the raced row was
// already decided by the time the direct write ran, not merely flaky. The
// property is real and still worth pinning; it is pinned deterministically
// instead, against `release_lock_after_ordinary_failure` directly.

/// Retire the plan's current revision, the way a committed retirement does: the
/// `plan` row's `lifecycle_state` and nothing else.
///
/// **That narrowness is the whole of C3-2.** `retire_revision` does not touch
/// `pricing_price`, so every price row of a retired plan still reads `published`
/// and every reader that keys on the row's own state still finds it.
async fn retire_plan(h: &Harness, plan: PlanId) {
    let revision = bss_pricing::infra::storage::repo::plan_repo::load_current(
        &h.provider.conn().expect("conn"),
        &h.scope,
        TENANT,
        plan,
    )
    .await
    .expect("read the current revision")
    .expect("the plan has one")
    .revision;
    let scope = h.scope.clone();
    let (_, outcome) = h
        .provider
        .db()
        .in_transaction::<_, bss_pricing::infra::storage::RepoError, _>(move |txn| {
            Box::pin(async move {
                bss_pricing::infra::storage::repo::plan_repo::retire_revision(
                    txn, &scope, TENANT, plan, revision,
                )
                .await
            })
        })
        .await;
    outcome.expect("retire the seeded plan revision");
}

#[tokio::test]
async fn a_retired_plan_takes_no_repricing_successor_and_no_version_handle() {
    // **C3-2 on the fourth door.** `refuse_unpublishable_predecessor` was hoisted by
    // `publish::commit` and `supersede_in` and by neither the cutover nor this
    // aggregate. Both resolve the plan through `plan_repo::load_current`, which
    // answers `published` **or** `retired` without distinguishing them, and neither
    // asked `can_transition(Superseded)`.
    //
    // Nothing beneath refuses either: `commit_supersession_rows` runs
    // `refuse_mispaired` / `supersede_row` / `publish_rows` and none of the three
    // reads the plan's lifecycle, while `RepoError::NoSuccessorRevision` — the source
    // of `PLAN_RETIRED_NO_SUCCESSOR` — comes only from the revision-opening path a
    // repricing apply never takes. So the run superseded published rows on a plan
    // nobody may buy and took a `CatalogVersion` handle for them.
    //
    // The handle is the sharper half. The refusal is **permanent**, so a request made
    // past it stands pending forever and trips `pricing.catalogversion.commit_overdue`
    // for a publish that can never happen — which is why this is hoisted rather than
    // left to any later door.
    let h = harness().await;

    // The clean plan is the positive control: without it a probe asserting "the run
    // applied nothing" would pass against an apply that had simply broken.
    let clean = Uuid::now_v7();
    let clean_phase = Uuid::now_v7();
    seed_plan(&h, clean, clean_phase).await;
    let clean_row = seed_published_row(&h, PlanId::new(clean), clean_phase, "eu", 9_900).await;

    let retired = Uuid::now_v7();
    let retired_phase = Uuid::now_v7();
    seed_plan(&h, retired, retired_phase).await;
    let retired_row =
        seed_published_row(&h, PlanId::new(retired), retired_phase, "eu", 10_000).await;
    retire_plan(&h, PlanId::new(retired)).await;

    let run_id = open_committing_run(&h, &[clean_row, retired_row]).await;
    let handles_before = h.registry.requested().len();

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
    .expect("apply_run_in itself does not fail - only a plan's own rows do");

    assert_eq!(outcome.applied, 1, "the clean plan's one row: {outcome:?}");
    assert_eq!(outcome.failed, 1, "the retired plan's: {outcome:?}");

    let (state, reason, applied) = journal_state(&h, run_id, retired_row).await;
    assert_eq!(state, JournalState::Failed);
    assert!(applied.is_none(), "no successor stands on a retired plan");
    let reason = reason.expect("a failed row carries a reason");
    // Matched on `refuse_unpublishable_predecessor`'s own sentence rather than on
    // `PLAN_RETIRED_NO_SUCCESSOR`: the journal records the `DomainError`'s
    // rendering, and only a `ValidationFailed` report renders rule codes into it
    // — which is why the sibling case above can match one and this cannot. The
    // clause is the refusal's own and no other refusal on this path produces it.
    assert!(
        reason.contains("can never be superseded"),
        "the hoisted refusal is what answered: {reason}"
    );

    // **One handle, not two.** Asserted as a count against the pre-call reading
    // rather than as "the registry was asked", because the clean plan asks for one
    // legitimately and a probe that only checked for absence would redden on it.
    assert_eq!(
        h.registry.requested().len() - handles_before,
        1,
        "the clean plan's handle and no second one stranded on the retired plan: {:?}",
        h.registry.requested()
    );
}

/// **A run whose report cannot be decoded stays where it was** (C4-5).
///
/// `apply_run_in` took the `awaiting_approval -> committing` edge and *then*
/// parsed the stored report, and both parses are `DomainError::Internal` on a
/// report it cannot decode. The edge is documented as single-spend — its premise
/// rides into the `UPDATE` so "two approvals landing at once cannot both spend
/// this edge" — so a run with an undecodable report was left in `committing`,
/// and by C4-1 `committing` has no door: the abort route is a bulk-import route,
/// and nothing re-drives a repricing run.
///
/// Neither parse reads the store, so moving them above the `advance` costs
/// nothing and turns an unrecoverable state into a failed call over an unchanged
/// run.
#[tokio::test]
async fn an_undecodable_report_does_not_spend_the_approval_edge() {
    let h = harness().await;
    let plan = Uuid::now_v7();
    let phase = Uuid::now_v7();
    seed_plan(&h, plan, phase).await;
    let row = seed_published_row(&h, PlanId::new(plan), phase, "eu", 10_000).await;

    // Opened and parked in `awaiting_approval` — `validating -> awaiting_approval`
    // is `inst-bs-approval`'s own edge, and the one a material run takes through
    // `advance_on_verdict` — carrying a report with no `adjustment` member, the
    // shape `adjustment_of_report` cannot decode.
    let run_id = Uuid::now_v7();
    let conn = h.provider.conn().expect("conn");
    bulk_repo::open(
        &conn,
        &h.scope,
        NewBulkOperation {
            operation_id: run_id,
            tenant_id: TENANT,
            kind: BulkKind::Repricing,
            client_key: run_id.to_string(),
            request_hash: IdempotencyGate::payload_hash(&run_id.to_string()),
            report: report(),
            submitted_by: ACTOR,
            submitted_at: at(11),
        },
    )
    .await
    .expect("open the run");
    repricing_journal_repo::open_rows(
        &conn,
        &h.scope,
        &[NewJournalRow {
            run_id,
            price_id: row,
            tenant_id: TENANT,
        }],
    )
    .await
    .expect("freeze the journal");
    bulk_repo::advance(
        &conn,
        &h.scope,
        TENANT,
        run_id,
        BulkState::Validating,
        BulkState::AwaitingApproval,
        serde_json::json!({ "selector": serde_json::Value::Null, "selected": 0 }),
        at(11),
    )
    .await
    .expect("park the run awaiting approval");

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
    .await;
    assert!(
        outcome.is_err(),
        "a report the apply cannot decode is a failure, not a run to attempt: {outcome:?}"
    );

    let run = bulk_repo::read(&conn, &h.scope, TENANT, run_id)
        .await
        .expect("read the run")
        .expect("the run is there");
    assert_eq!(
        run.state,
        BulkState::AwaitingApproval,
        "the edge is single-spend and must not be spent on a call that could not \
         have applied anything; `committing` has no door back out"
    );

    // And the journal is untouched, so nothing was half-applied under the
    // spent edge either.
    assert_eq!(
        journal_state(&h, run_id, row).await.0,
        JournalState::Pending
    );
}

/// **A key another unit holds refuses the apply, and the refusal does not spend
/// the approval edge** (C4-6).
///
/// `refuse_held_key` is called by `window`, `supersession`, `cutover`,
/// `grandfather` and five sites in `approval`; `infra::repricing` contained no
/// occurrence of it at all, so `inst-co-single-pending` was asked once at run
/// open, in the API layer, and never again.
///
/// **This is the only level the check is reachable from today, and that is the
/// finding's whole priority argument.** Through HTTP it cannot be: a material
/// run's own batch unit registers every selected key in `held_keys` while it
/// pends, so a competing unit's `approval_repo::open` is refused by
/// `uq_pricing_approval_key_pending` before it can create the situation —
/// measured, not assumed, by an earlier version of this case written at the REST
/// layer, which failed at the seed with `PendingKeyHeld`. A non-material run
/// applies in the same request as the open's own check. What is left is the
/// redrive door: a re-drive of a stalled run arrives arbitrarily later, and
/// building it without this check is the shape the five sibling paths treat as a
/// Critical.
///
/// The second assertion is what the check's *placement* is for. It sits above
/// `inst-bs-commit`'s edge, which is single-spend; below it, a refusal would
/// leave the run `committing`, which section 4 gives it no way out of.
#[tokio::test]
async fn a_key_a_pending_unit_holds_refuses_the_apply_without_spending_the_approval_edge() {
    let h = harness().await;
    let plan = Uuid::now_v7();
    let phase = Uuid::now_v7();
    seed_plan(&h, plan, phase).await;
    let row = seed_published_row(&h, PlanId::new(plan), phase, "eu", 10_000).await;

    let run_id = Uuid::now_v7();
    let conn = h.provider.conn().expect("conn");
    bulk_repo::open(
        &conn,
        &h.scope,
        NewBulkOperation {
            operation_id: run_id,
            tenant_id: TENANT,
            kind: BulkKind::Repricing,
            client_key: run_id.to_string(),
            request_hash: IdempotencyGate::payload_hash(&run_id.to_string()),
            report: report(),
            submitted_by: ACTOR,
            submitted_at: at(11),
        },
    )
    .await
    .expect("open the run");
    repricing_journal_repo::open_rows(
        &conn,
        &h.scope,
        &[NewJournalRow {
            run_id,
            price_id: row,
            tenant_id: TENANT,
        }],
    )
    .await
    .expect("freeze the journal");
    bulk_repo::advance(
        &conn,
        &h.scope,
        TENANT,
        run_id,
        BulkState::Validating,
        BulkState::AwaitingApproval,
        report(),
        at(11),
    )
    .await
    .expect("park the run awaiting approval");

    // The key was free when the run opened. An interactive unit takes it before
    // the apply arrives — the redrive window, in one statement.
    let keys = price_repo::load_scope_keys_for_ids(&conn, &h.scope, TENANT, &[row])
        .await
        .expect("the row's own key");
    let (_, key) = keys.into_iter().next().expect("one row, one key");
    approval_repo::open(
        &conn,
        &h.scope,
        NewApproval {
            approval_id: Uuid::now_v7(),
            tenant_id: TENANT,
            subject_ref: audit_repo::plan_revision_ref(PlanId::new(Uuid::now_v7()), 0),
            subject_kind: AuditSubjectKind::PlanRevision,
            content_hash: vec![0u8; 32],
            materiality: serde_json::json!({ "material": true, "reason": "an interactive unit" }),
            held_keys: std::iter::once(key.to_string()).collect(),
        },
        apply_stamp(),
    )
    .await
    .expect("seed a pending interactive unit holding the key");

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
    .await;
    assert!(
        outcome.is_err(),
        "a row whose key another unit holds is not a row this apply may write: {outcome:?}"
    );

    let run = bulk_repo::read(&conn, &h.scope, TENANT, run_id)
        .await
        .expect("read the run")
        .expect("the run is there");
    assert_eq!(
        run.state,
        BulkState::AwaitingApproval,
        "and the single-spend edge was not spent on a call that refused: \
         `committing` has no door back out"
    );
    assert_eq!(
        journal_state(&h, run_id, row).await.0,
        JournalState::Pending,
        "nothing was applied under it either"
    );
}
